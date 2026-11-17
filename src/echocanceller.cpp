// Compiled as C++17 (the `aec` compiler in harbour-xmatic.pro): webrtc-audio-processing needs it.
#include "echocanceller.h"

#include <api/audio/audio_processing.h>

#include <QtGlobal>

#include <atomic>
#include <cmath>
#include <cstdint>

namespace {

const int kRate = 48000;
const size_t kFrame = kRate / 100; // the canceller's 10 ms
// A hint only: AEC3 finds the echo path itself, within about a second.
const int kDelayHintMs = 100;
const int kReportEvery = 1000; // frames, ten seconds

} // namespace

struct EchoCanceller::Private {
    rtc::scoped_refptr<webrtc::AudioProcessing> apm;
    webrtc::StreamConfig stream{kRate, 1};
    // One far end: a second audio stream would interleave with the first.
    std::atomic<bool> watching{false};

    // Capture thread only.
    int frames = 0;
    double heardEnergy = 0;
    double leftEnergy = 0;

    void process(int16_t *frame);

    static GstPadProbeReturn onPlayback(GstPad *pad, GstPadProbeInfo *info, gpointer user);
    static GstPadProbeReturn onCapture(GstPad *pad, GstPadProbeInfo *info, gpointer user);
    static void release(gpointer user) { delete static_cast<std::shared_ptr<Private> *>(user); }
    static Private *of(gpointer user) { return static_cast<std::shared_ptr<Private> *>(user)->get(); }
};

EchoCanceller::EchoCanceller()
    : d(std::make_shared<Private>())
{
    d->apm = webrtc::AudioProcessingBuilder().Create();
    if (!d->apm) {
        qWarning("xmatic: echo cancellation unavailable");
        return;
    }
    webrtc::AudioProcessing::Config config;
    // The full canceller, not the mobile one: a loudspeaker's echo is loud and not linear.
    config.echo_canceller.enabled = true;
    config.echo_canceller.mobile_mode = false;
    config.noise_suppression.enabled = true;
    config.noise_suppression.level = webrtc::AudioProcessing::Config::NoiseSuppression::kHigh;
    config.high_pass_filter.enabled = true;
    config.gain_controller2.enabled = true;
    config.gain_controller2.adaptive_digital.enabled = true;
    d->apm->ApplyConfig(config);
    qInfo("xmatic: echo cancellation AEC3");
}

EchoCanceller::~EchoCanceller() = default;

bool EchoCanceller::available() const
{
    return d->apm != nullptr;
}

void EchoCanceller::watchPlayback(GstPad *pad)
{
    if (d->apm && !d->watching.exchange(true)) {
        gst_pad_add_probe(pad, GST_PAD_PROBE_TYPE_BUFFER, &Private::onPlayback,
                          new std::shared_ptr<Private>(d), &Private::release);
    }
}

void EchoCanceller::processCapture(GstPad *pad)
{
    if (d->apm) {
        gst_pad_add_probe(pad, GST_PAD_PROBE_TYPE_BUFFER, &Private::onCapture,
                          new std::shared_ptr<Private>(d), &Private::release);
    }
}

// The canceller keeps render and capture apart internally; each side has its thread.
// Both taps sit behind a 10 ms split: a buffer is whole frames.
GstPadProbeReturn EchoCanceller::Private::onPlayback(GstPad *, GstPadProbeInfo *info, gpointer user)
{
    Private *p = of(user);
    GstBuffer *buffer = GST_PAD_PROBE_INFO_BUFFER(info);
    GstMapInfo map;
    if (!buffer || !gst_buffer_map(buffer, &map, GST_MAP_READ)) {
        return GST_PAD_PROBE_OK;
    }
    const auto *samples = reinterpret_cast<const int16_t *>(map.data);
    const size_t count = map.size / sizeof(int16_t);
    int16_t scratch[kFrame];
    for (size_t i = 0; i + kFrame <= count; i += kFrame) {
        p->apm->ProcessReverseStream(samples + i, p->stream, p->stream, scratch);
    }
    gst_buffer_unmap(buffer, &map);
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
    for (size_t i = 0; i + kFrame <= count; i += kFrame) {
        p->process(samples + i);
    }
    gst_buffer_unmap(buffer, &map);
    return GST_PAD_PROBE_OK;
}

void EchoCanceller::Private::process(int16_t *frame)
{
    double before = 0;
    for (size_t i = 0; i < kFrame; ++i) {
        before += double(frame[i]) * frame[i];
    }
    apm->set_stream_delay_ms(kDelayHintMs);
    apm->ProcessStream(frame, stream, stream, frame);
    double after = 0;
    for (size_t i = 0; i < kFrame; ++i) {
        after += double(frame[i]) * frame[i];
    }
    heardEnergy += before;
    leftEnergy += after;
    if (++frames % kReportEvery == 0) {
        // What the canceller reports itself, and what left the microphone path.
        const webrtc::AudioProcessingStats stats = apm->GetStatistics();
        const double removed = leftEnergy > 0 ? 10.0 * std::log10(heardEnergy / leftEnergy) : 0.0;
        qInfo("xmatic: echo delay %d ms, echo return loss enhancement %.1f dB, removed %.1f dB",
              stats.delay_ms.value_or(-1),
              stats.echo_return_loss_enhancement.value_or(0.0), removed);
        heardEnergy = leftEnergy = 0;
    }
}
