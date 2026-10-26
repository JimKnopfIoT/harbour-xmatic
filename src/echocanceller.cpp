#include "echocanceller.h"

#include <speex/speex_echo.h>
#include <speex/speex_preprocess.h>

#include <QtGlobal>

#include <algorithm>
#include <cstring>
#include <mutex>
#include <vector>

namespace {

const int kRate = 48000;
const size_t kFrame = kRate / 100; // 10 ms
// Speaker to microphone, after the playback the reference queue already holds.
const int kTail = kRate / 4;
// Playback runs ahead by what pulsesink buffers; more is a stall, dropped.
const size_t kMaxQueued = kFrame * 40;

} // namespace

struct EchoCanceller::Private {
    SpeexEchoState *echo = nullptr;
    SpeexPreprocessState *preprocess = nullptr;

    std::mutex renderLock;
    std::vector<int16_t> render;

    std::vector<int16_t> captureIn;
    // Primed with one frame of silence: what goes out lags what comes in by
    // exactly one frame, so every buffer can be filled whatever its size.
    std::vector<int16_t> captureOut = std::vector<int16_t>(kFrame, 0);
    std::vector<int16_t> reference = std::vector<int16_t>(kFrame, 0);
    std::vector<int16_t> cleaned = std::vector<int16_t>(kFrame, 0);

    ~Private()
    {
        if (preprocess) {
            speex_preprocess_state_destroy(preprocess);
        }
        if (echo) {
            speex_echo_state_destroy(echo);
        }
    }

    static GstPadProbeReturn onPlayback(GstPad *, GstPadProbeInfo *info, gpointer user);
    static GstPadProbeReturn onCapture(GstPad *, GstPadProbeInfo *info, gpointer user);
    static void release(gpointer user) { delete static_cast<std::shared_ptr<Private> *>(user); }
    static Private *of(gpointer user) { return static_cast<std::shared_ptr<Private> *>(user)->get(); }
};

EchoCanceller::EchoCanceller()
    : d(std::make_shared<Private>())
{
    d->echo = speex_echo_state_init(kFrame, kTail);
    d->preprocess = speex_preprocess_state_init(kFrame, kRate);
    if (!d->echo || !d->preprocess) {
        qWarning("xmatic: echo cancellation unavailable");
        d.reset(new Private);
        return;
    }
    int rate = kRate;
    speex_echo_ctl(d->echo, SPEEX_ECHO_SET_SAMPLING_RATE, &rate);
    // Residual echo suppression and noise reduction ride on the preprocessor.
    speex_preprocess_ctl(d->preprocess, SPEEX_PREPROCESS_SET_ECHO_STATE, d->echo);
    int on = 1;
    speex_preprocess_ctl(d->preprocess, SPEEX_PREPROCESS_SET_DENOISE, &on);
}

EchoCanceller::~EchoCanceller() = default;

bool EchoCanceller::available() const
{
    return d->echo != nullptr;
}

void EchoCanceller::watchPlayback(GstPad *pad)
{
    if (d->echo) {
        gst_pad_add_probe(pad, GST_PAD_PROBE_TYPE_BUFFER, &Private::onPlayback,
                          new std::shared_ptr<Private>(d), &Private::release);
    }
}

void EchoCanceller::processCapture(GstPad *pad)
{
    if (d->echo) {
        gst_pad_add_probe(pad, GST_PAD_PROBE_TYPE_BUFFER, &Private::onCapture,
                          new std::shared_ptr<Private>(d), &Private::release);
    }
}

GstPadProbeReturn EchoCanceller::Private::onPlayback(GstPad *, GstPadProbeInfo *info, gpointer user)
{
    Private *p = of(user);
    GstBuffer *buffer = GST_PAD_PROBE_INFO_BUFFER(info);
    GstMapInfo map;
    if (!buffer || !gst_buffer_map(buffer, &map, GST_MAP_READ)) {
        return GST_PAD_PROBE_OK;
    }
    const auto *samples = reinterpret_cast<const int16_t *>(map.data);
    std::lock_guard<std::mutex> lock(p->renderLock);
    p->render.insert(p->render.end(), samples, samples + map.size / sizeof(int16_t));
    gst_buffer_unmap(buffer, &map);
    if (p->render.size() > kMaxQueued) {
        p->render.erase(p->render.begin(), p->render.end() - kMaxQueued);
    }
    return GST_PAD_PROBE_OK;
}

GstPadProbeReturn EchoCanceller::Private::onCapture(GstPad *, GstPadProbeInfo *info, gpointer user)
{
    Private *p = of(user);
    GstBuffer *buffer = GST_PAD_PROBE_INFO_BUFFER(info);
    if (!buffer) {
        return GST_PAD_PROBE_OK;
    }
    buffer = gst_buffer_make_writable(buffer);
    GST_PAD_PROBE_INFO_DATA(info) = buffer;
    GstMapInfo map;
    if (!gst_buffer_map(buffer, &map, GST_MAP_READWRITE)) {
        return GST_PAD_PROBE_OK;
    }
    auto *samples = reinterpret_cast<int16_t *>(map.data);
    const size_t count = map.size / sizeof(int16_t);

    p->captureIn.insert(p->captureIn.end(), samples, samples + count);
    size_t done = 0;
    for (; done + kFrame <= p->captureIn.size(); done += kFrame) {
        {
            // Nothing played back: silence is the reference.
            std::lock_guard<std::mutex> lock(p->renderLock);
            if (p->render.size() >= kFrame) {
                std::memcpy(p->reference.data(), p->render.data(), kFrame * sizeof(int16_t));
                p->render.erase(p->render.begin(), p->render.begin() + kFrame);
            } else {
                std::fill(p->reference.begin(), p->reference.end(), 0);
            }
        }
        speex_echo_cancellation(p->echo, p->captureIn.data() + done, p->reference.data(),
                                p->cleaned.data());
        speex_preprocess_run(p->preprocess, p->cleaned.data());
        p->captureOut.insert(p->captureOut.end(), p->cleaned.begin(), p->cleaned.end());
    }
    p->captureIn.erase(p->captureIn.begin(), p->captureIn.begin() + done);

    // captureOut holds one frame plus count minus what is still waiting: never short.
    std::memcpy(samples, p->captureOut.data(), count * sizeof(int16_t));
    p->captureOut.erase(p->captureOut.begin(), p->captureOut.begin() + count);
    gst_buffer_unmap(buffer, &map);
    return GST_PAD_PROBE_OK;
}
