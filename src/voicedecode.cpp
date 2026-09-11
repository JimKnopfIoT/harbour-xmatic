#include "voicedecode.h"

#include <atomic>
#include <cstdio>
#include <cstdlib>

#include <gst/gst.h>
#include <sys/resource.h>
#include <sys/stat.h>

// A received voice message is a stranger's file. It is decoded here, in a
// short-lived process inside the app's jail, so a decoder fault neither takes
// the app down nor runs next to its keys - and the service that recognises the
// speech, which runs outside any jail, is handed only a WAV of known shape.

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

int decode(const char *in, const char *out)
{
    GError *error = nullptr;
    if (!gst_init_check(nullptr, nullptr, &error)) {
        if (error) {
            g_error_free(error);
        }
        return VoiceDecodeFailed;
    }

    Context context;
    GstElement *pipeline = gst_pipeline_new(nullptr);
    GstElement *source = gst_element_factory_make("filesrc", nullptr);
    GstElement *demuxer = gst_element_factory_make("oggdemux", nullptr);
    context.decoder = gst_element_factory_make("opusdec", nullptr);
    GstElement *convert = gst_element_factory_make("audioconvert", nullptr);
    GstElement *resample = gst_element_factory_make("audioresample", nullptr);
    GstElement *filter = gst_element_factory_make("capsfilter", nullptr);
    GstElement *encoder = gst_element_factory_make("wavenc", nullptr);
    GstElement *sink = gst_element_factory_make("filesink", nullptr);
    if (!pipeline || !source || !demuxer || !context.decoder || !convert || !resample
            || !filter || !encoder || !sink) {
        return VoiceDecodeFailed;
    }

    // Set as properties, never through a pipeline string: a path is not syntax.
    g_object_set(source, "location", in, nullptr);
    g_object_set(sink, "location", out, nullptr);
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

    const char *in = std::getenv(XMATIC_VOICE_IN_ENV);
    const char *out = std::getenv(XMATIC_VOICE_OUT_ENV);
    if (!in || !out || !*in || !*out) {
        return VoiceDecodeFailed;
    }

    const int result = decode(in, out);
    // A decode that produced nothing is a failure, not an empty message.
    struct stat written;
    if (result != VoiceDecodeOk || stat(out, &written) != 0 || written.st_size <= 44) {
        std::remove(out);
        return result == VoiceDecodeOk ? VoiceDecodeFailed : result;
    }
    return VoiceDecodeOk;
}
