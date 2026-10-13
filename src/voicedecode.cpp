#include "voicedecode.h"

#include <atomic>
#include <cerrno>
#include <cstdio>
#include <cstdlib>

#include <gst/gst.h>
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <cstddef>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <dirent.h>
#include <fcntl.h>
#include <unistd.h>
#include <vector>

// A received voice message is a stranger's file. It is decoded here, in a
// short-lived process inside the app's jail, so a decoder fault does not take
// the app down. The jail grants the app's rights, the secrets daemon included;
// a filter leaves it reading and computing only: no socket, no file it may write.
// The service that recognises the speech is handed only a WAV of known shape.

namespace {

/// Ten minutes of 16 kHz mono S16LE. Longer is refused, not cut.
const guint64 MaxSeconds = 600;
const guint64 BytesPerSecond = 16000 * 2;
/// A healthy decode of ten minutes takes seconds; this bounds a stuck one.
const gint64 WallLimitSeconds = 120;

struct Context {
    GstElement *decoder = nullptr;
    std::atomic<bool> linked{false};
    std::atomic<bool> foreign{false};
    std::atomic<bool> tooLong{false};
    std::atomic<guint64> bytes{0};
};

/// A decoder bug may loop, allocate or write; it may not take the phone with it.
void limitResources()
{
    const rlim_t memory = 1024UL * 1024 * 1024;
    const struct rlimit address = {memory, memory};
    setrlimit(RLIMIT_AS, &address);
    const struct rlimit cpu = {rlim_t(WallLimitSeconds), rlim_t(WallLimitSeconds)};
    setrlimit(RLIMIT_CPU, &cpu);
    const rlim_t file = (MaxSeconds + 10) * BytesPerSecond;
    const struct rlimit size = {file, file};
    setrlimit(RLIMIT_FSIZE, &size);
}

#if defined(__aarch64__)
const unsigned int OwnArch = AUDIT_ARCH_AARCH64;
#elif defined(__arm__)
const unsigned int OwnArch = AUDIT_ARCH_ARM;
#elif defined(__x86_64__)
const unsigned int OwnArch = AUDIT_ARCH_X86_64;
#else
#error "voice decode: no seccomp architecture for this target"
#endif

/// Whatever the parent left open beyond the standard three: a writable file or
/// a socket inherited is a way around the filter. `false` if it cannot tell.
bool closeInherited(int keepIn, int keepOut)
{
    std::vector<int> open;
    DIR *dir = opendir("/proc/self/fd");
    if (!dir) {
        return false;
    }
    while (struct dirent *entry = readdir(dir)) {
        const int fd = atoi(entry->d_name);
        if (fd > 2 && fd != keepIn && fd != keepOut && fd != dirfd(dir)) {
            open.push_back(fd);
        }
    }
    closedir(dir);
    for (int fd : open) {
        close(fd);
    }
    return true;
}

/// From here on the decoder may read and compute, nothing else: no new socket
/// (bus, secrets daemon, network), no file opened for writing, renamed, linked
/// or removed, no metadata changed, no signal sent, no lock taken, no io_uring.
/// Its two files are open already. `false` if refused.
bool confine()
{
    unsetenv("DBUS_SESSION_BUS_ADDRESS");
    unsetenv("DBUS_SYSTEM_BUS_ADDRESS");

    std::vector<int> denied = {
        __NR_socket, __NR_connect, __NR_renameat, __NR_unlinkat, __NR_linkat,
        __NR_symlinkat, __NR_mkdirat, __NR_mknodat, __NR_truncate, __NR_fchmod,
        __NR_fchmodat, __NR_fchownat, __NR_fchown, __NR_utimensat, __NR_kill,
        __NR_tkill, __NR_tgkill, __NR_rt_sigqueueinfo, __NR_rt_tgsigqueueinfo,
        __NR_ptrace, __NR_process_vm_writev, __NR_flock,
        __NR_setxattr, __NR_lsetxattr, __NR_fsetxattr,
        __NR_removexattr, __NR_lremovexattr, __NR_fremovexattr,
        // Newer than the SDK's kernel headers, one number on every architecture:
        // pidfd_send_signal, io_uring_setup/enter/register, openat2.
        424, 425, 426, 427, 437,
#ifdef __NR_renameat2
        __NR_renameat2,
#endif
#ifdef __NR_socketcall
        __NR_socketcall,
#endif
#ifdef __NR_rename
        __NR_rename, __NR_unlink, __NR_link, __NR_symlink, __NR_mkdir, __NR_creat,
        __NR_mknod, __NR_chmod, __NR_chown, __NR_lchown,
#endif
#ifdef __NR_truncate64
        __NR_truncate64,
#endif
#ifdef __NR_utime
        __NR_utime,
#endif
#ifdef __NR_utimes
        __NR_utimes,
#endif
#ifdef __NR_futimesat
        __NR_futimesat,
#endif
#ifdef __NR_chown32
        __NR_chown32, __NR_lchown32, __NR_fchown32,
#endif
    };
    const unsigned int writeFlags = O_WRONLY | O_RDWR | O_CREAT | O_TRUNC | O_APPEND;
    const unsigned int errno_ = SECCOMP_RET_ERRNO | (EPERM & SECCOMP_RET_DATA);

    std::vector<sock_filter> program;
    program.push_back(BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, arch)));
    program.push_back(BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, OwnArch, 1, 0));
    program.push_back(BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL));
    program.push_back(BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)));
    // Each denial jumps to the errno return; its offset is patched below.
    std::vector<size_t> toErrno;
    for (int nr : denied) {
        toErrno.push_back(program.size());
        program.push_back(BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, static_cast<unsigned>(nr), 0, 0));
    }
    // Opening: allowed only without a write flag. Flags are the third argument
    // of openat and the second of open.
    struct Opener { int nr; int flagsArg; };
    std::vector<Opener> openers = { { __NR_openat, 2 } };
#ifdef __NR_open
    openers.push_back({ __NR_open, 1 });
#endif
    std::vector<size_t> toCheck;
    for (const Opener &opener : openers) {
        toCheck.push_back(program.size());
        program.push_back(BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, static_cast<unsigned>(opener.nr), 0, 0));
    }
    program.push_back(BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
    std::vector<size_t> checks;
    std::vector<size_t> toErrnoFromCheck;
    for (const Opener &opener : openers) {
        checks.push_back(program.size());
        const unsigned int flagsAt = static_cast<unsigned int>(
                offsetof(struct seccomp_data, args) + static_cast<size_t>(opener.flagsArg) * 8);
        program.push_back(BPF_STMT(BPF_LD | BPF_W | BPF_ABS, flagsAt));
        toErrnoFromCheck.push_back(program.size());
        program.push_back(BPF_JUMP(BPF_JMP | BPF_JSET | BPF_K, writeFlags, 0, 0));
        program.push_back(BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
    }
    const size_t errnoAt = program.size();
    program.push_back(BPF_STMT(BPF_RET | BPF_K, errno_));

    for (size_t at : toErrno) {
        program[at].jt = static_cast<unsigned char>(errnoAt - at - 1);
    }
    for (size_t i = 0; i < toCheck.size(); ++i) {
        program[toCheck[i]].jt = static_cast<unsigned char>(checks[i] - toCheck[i] - 1);
    }
    for (size_t at : toErrnoFromCheck) {
        program[at].jt = static_cast<unsigned char>(errnoAt - at - 1);
    }
    if (program.size() > 250) {
        return false;
    }

    struct sock_fprog filter = {
        static_cast<unsigned short>(program.size()),
        program.data(),
    };
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0) {
        return false;
    }
    return prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &filter) == 0;
}

void onPadAdded(GstElement *, GstPad *pad, gpointer data)
{
    auto *context = static_cast<Context *>(data);
    GstCaps *caps = gst_pad_get_current_caps(pad);
    if (!caps) {
        caps = gst_pad_query_caps(pad, nullptr);
    }
    const GstStructure *structure = caps && gst_caps_get_size(caps) > 0
            ? gst_caps_get_structure(caps, 0)
            : nullptr;
    const bool opus = structure && gst_structure_has_name(structure, "audio/x-opus");
    if (caps) {
        gst_caps_unref(caps);
    }
    // One Opus stream is a voice message. A second stream, or any other kind, is not.
    if (!opus || context->linked.exchange(true)) {
        context->foreign = true;
        return;
    }
    GstPad *sink = gst_element_get_static_pad(context->decoder, "sink");
    if (gst_pad_link(pad, sink) != GST_PAD_LINK_OK) {
        context->foreign = true;
    }
    gst_object_unref(sink);
}

GstPadProbeReturn countBytes(GstPad *, GstPadProbeInfo *info, gpointer data)
{
    auto *context = static_cast<Context *>(data);
    const guint64 total = context->bytes += gst_buffer_get_size(GST_PAD_PROBE_INFO_BUFFER(info));
    if (total > MaxSeconds * BytesPerSecond) {
        context->tooLong = true;
        return GST_PAD_PROBE_DROP;
    }
    return GST_PAD_PROBE_OK;
}

int decode(int in, int out)
{
    Context context;
    GstElement *pipeline = gst_pipeline_new(nullptr);
    GstElement *source = gst_element_factory_make("fdsrc", nullptr);
    GstElement *demuxer = gst_element_factory_make("oggdemux", nullptr);
    context.decoder = gst_element_factory_make("opusdec", nullptr);
    GstElement *convert = gst_element_factory_make("audioconvert", nullptr);
    GstElement *resample = gst_element_factory_make("audioresample", nullptr);
    GstElement *filter = gst_element_factory_make("capsfilter", nullptr);
    GstElement *encoder = gst_element_factory_make("wavenc", nullptr);
    GstElement *sink = gst_element_factory_make("fdsink", nullptr);
    if (!pipeline || !source || !demuxer || !context.decoder || !convert || !resample
            || !filter || !encoder || !sink) {
        return VoiceDecodeFailed;
    }

    // Descriptors opened before the filter: from here nothing opens for writing.
    g_object_set(source, "fd", in, nullptr);
    g_object_set(sink, "fd", out, nullptr);
    GstCaps *caps = gst_caps_from_string("audio/x-raw,format=S16LE,rate=16000,channels=1");
    g_object_set(filter, "caps", caps, nullptr);
    gst_caps_unref(caps);

    gst_bin_add_many(GST_BIN(pipeline), source, demuxer, context.decoder, convert, resample,
                     filter, encoder, sink, nullptr);
    if (!gst_element_link(source, demuxer)
            || !gst_element_link_many(context.decoder, convert, resample, filter, encoder, sink,
                                      nullptr)) {
        gst_object_unref(pipeline);
        return VoiceDecodeFailed;
    }
    g_signal_connect(demuxer, "pad-added", G_CALLBACK(onPadAdded), &context);

    GstPad *counted = gst_element_get_static_pad(filter, "src");
    gst_pad_add_probe(counted, GST_PAD_PROBE_TYPE_BUFFER, countBytes, &context, nullptr);
    gst_object_unref(counted);

    GstBus *bus = gst_element_get_bus(pipeline);
    int result = VoiceDecodeFailed;
    if (gst_element_set_state(pipeline, GST_STATE_PLAYING) != GST_STATE_CHANGE_FAILURE) {
        const gint64 deadline = g_get_monotonic_time() + WallLimitSeconds * G_USEC_PER_SEC;
        for (;;) {
            if (context.foreign) {
                result = VoiceDecodeNotVoice;
                break;
            }
            if (context.tooLong) {
                result = VoiceDecodeTooLong;
                break;
            }
            if (g_get_monotonic_time() > deadline) {
                result = VoiceDecodeTimedOut;
                break;
            }
            GstMessage *message = gst_bus_timed_pop_filtered(
                    bus, 200 * GST_MSECOND,
                    static_cast<GstMessageType>(GST_MESSAGE_EOS | GST_MESSAGE_ERROR));
            if (!message) {
                continue;
            }
            const bool finished = GST_MESSAGE_TYPE(message) == GST_MESSAGE_EOS;
            gst_message_unref(message);
            if (context.foreign) {
                result = VoiceDecodeNotVoice;
            } else if (context.tooLong) {
                result = VoiceDecodeTooLong;
            } else {
                result = finished ? VoiceDecodeOk : VoiceDecodeFailed;
            }
            break;
        }
    }
    gst_element_set_state(pipeline, GST_STATE_NULL);
    gst_object_unref(bus);
    gst_object_unref(pipeline);
    return result;
}

} // namespace

int runVoiceDecode()
{
    limitResources();

    const char *inPath = std::getenv(XMATIC_VOICE_IN_ENV);
    const char *outPath = std::getenv(XMATIC_VOICE_OUT_ENV);
    if (!inPath || !outPath || !*inPath || !*outPath) {
        return VoiceDecodeFailed;
    }
    const int in = open(inPath, O_RDONLY | O_CLOEXEC);
    const int out = open(outPath, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600);
    if (in < 0 || out < 0) {
        return VoiceDecodeFailed;
    }
    if (!closeInherited(in, out)) {
        return VoiceDecodeFailed;
    }
    // Before the filter: the registry may want to write its cache.
    GError *error = nullptr;
    if (!gst_init_check(nullptr, nullptr, &error)) {
        if (error) {
            g_error_free(error);
        }
        return VoiceDecodeFailed;
    }
    // Refused rather than run open: the filter is the promise this helper makes.
    if (!confine()) {
        return VoiceDecodeFailed;
    }

    const int result = decode(in, out);
    // A decode that produced nothing is a failure; the caller removes the file.
    struct stat written;
    if (result != VoiceDecodeOk || fstat(out, &written) != 0 || written.st_size <= 44) {
        if (ftruncate(out, 0) != 0) {
            // Nothing more to do here: the caller deletes the file in any case.
        }
        return result == VoiceDecodeOk ? VoiceDecodeFailed : result;
    }
    return VoiceDecodeOk;
}
