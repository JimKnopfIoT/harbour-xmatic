#include "echocanceller.h"

#include <speex/speex_echo.h>
#include <speex/speex_preprocess.h>

#include "feedbacksuppressor.h"

#include <QtGlobal>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <vector>

namespace {

const int kRate = 48000;
const size_t kFrame = kRate / 100; // 10 ms
// Behind the estimated delay: covers the room's reverberation.
const int kTail = kRate / 5;
const int64_t kMargin = kRate * 3 / 100;
// Measured 300 to 530 ms; too short reads a reference that has not arrived.
const int64_t kDefaultDelay = kRate * 42 / 100;
const int64_t kAgree = kRate / 100;
const size_t kHistory = 7;
const size_t kQuorum = 4;
// Inside this the filter's own window absorbs the change; a reset costs more.
const int64_t kMoveAfter = kRate / 40;
const int kCheckEvery = 100; // frames
// The estimator learns half a second back: the far end's newest audio is
// still in the jitter buffer when the microphone hears now.
const int64_t kLearnBehind = kRate / 2;
// The sink's buffer and the device between arrival and the loudspeaker.
const int64_t kPlayout = kRate * 4 / 10;
const float kMinProminence = 20.0f;

/// Samples by absolute position in running time; what is not there reads as silence.
class Ring
{
public:
    explicit Ring(size_t size) : m_data(size, 0) {}

    /// One past the newest sample, or -1 while empty.
    int64_t end() const { return m_end == kUnset ? -1 : m_end; }

    void write(int64_t pos, const int16_t *samples, size_t count)
    {
        const int64_t size = int64_t(m_data.size());
        if (m_end == kUnset || pos - m_end > size) {
            std::fill(m_data.begin(), m_data.end(), 0);
            m_end = pos;
        }
        for (; m_end < pos; ++m_end) {
            m_data[size_t(m_end % size)] = 0;
        }
        for (size_t i = 0; i < count; ++i) {
            const int64_t p = pos + int64_t(i);
            if (p > m_end - size) {
                m_data[size_t(p % size)] = samples[i];
            }
        }
        m_end = std::max(m_end, pos + int64_t(count));
    }

    void read(int64_t pos, int16_t *out, size_t count) const
    {
        const int64_t size = int64_t(m_data.size());
        for (size_t i = 0; i < count; ++i) {
            const int64_t p = pos + int64_t(i);
            out[i] = (m_end != kUnset && p >= 0 && p < m_end && p >= m_end - size)
                         ? m_data[size_t(p % size)] : 0;
        }
    }

private:
    static const int64_t kUnset = INT64_MIN;
    std::vector<int16_t> m_data;
    int64_t m_end = kUnset;
};

int64_t positionOf(GstPad *pad, GstBuffer *buffer)
{
    if (!GST_BUFFER_PTS_IS_VALID(buffer)) {
        return -1;
    }
    GstEvent *event = gst_pad_get_sticky_event(pad, GST_EVENT_SEGMENT, 0);
    if (!event) {
        return -1;
    }
    const GstSegment *segment = nullptr;
    gst_event_parse_segment(event, &segment);
    const guint64 rt = gst_segment_to_running_time(segment, GST_FORMAT_TIME, GST_BUFFER_PTS(buffer));
    gst_event_unref(event);
    return rt == GST_CLOCK_TIME_NONE ? -1 : int64_t(gst_util_uint64_scale_round(rt, kRate, GST_SECOND));
}

/// The echo path's delay, learnt: a coarse adaptive filter over 800 ms at
/// 2 kHz whose strongest tap is where the far end comes back.
class DelayEstimator
{
public:
    void push(const int16_t *mic, const int16_t *far, size_t count)
    {
        for (size_t i = 0; i < count; ++i) {
            m_micSum += mic[i];
            m_farSum += far[i];
            if (++m_summed == kDecimate) {
                step(m_farSum / kDecimate, m_micSum / kDecimate);
                m_micSum = m_farSum = 0;
                m_summed = 0;
            }
        }
    }

    /// Delay in samples at the full rate, or -1 while no tap stands out.
    /// Only between earliest and latest: the far end cannot echo before it
    /// arrives, nor long after the sink has played it.
    int64_t peak(float *prominence, int64_t earliest, int64_t latest)
    {
        const size_t first = std::max(size_t(kEdge), size_t(std::max<int64_t>(0, earliest) / kDecimate));
        const size_t last = std::min(kTaps - kEdge, size_t(std::max<int64_t>(0, latest) / kDecimate));
        float top = 0;
        double sum = 0;
        size_t at = 0;
        for (size_t k = 0; k < kTaps; ++k) {
            const float v = m_filter[k] * m_filter[k];
            sum += v;
            // A peak at either edge is the filter's, not the room's.
            if (v > top && k >= first && k < last) {
                top = v;
                at = k;
            }
        }
        // Rounding piles up in the running sum; once a second it starts over.
        m_energy = 0;
        for (size_t k = 0; k < kTaps; ++k) {
            m_energy += m_history[m_head + k] * m_history[m_head + k];
        }
        *prominence = sum > 0 ? float(top / (sum / kTaps)) : 0;
        return *prominence >= kMinProminence ? int64_t(at) * kDecimate : -1;
    }

private:
    static const int kDecimate = 24;
    static const size_t kTaps = 1600;
    static const size_t kEdge = 40;

    void step(float far, float mic)
    {
        m_head = (m_head + kTaps - 1) % kTaps;
        const float old = m_history[m_head];
        m_history[m_head] = m_history[m_head + kTaps] = far;
        m_energy += far * far - old * old;
        // Learn only while the far end is there to learn from.
        if (m_energy < kTaps * 30.0f * 30.0f) {
            return;
        }
        const float *x = &m_history[m_head];
        float predicted = 0;
        for (size_t k = 0; k < kTaps; ++k) {
            predicted += m_filter[k] * x[k];
        }
        const float gain = 0.3f * (mic - predicted) / m_energy;
        for (size_t k = 0; k < kTaps; ++k) {
            m_filter[k] += gain * x[k];
        }
    }

    // Twice over, so the newest kTaps samples always lie in one piece.
    std::vector<float> m_history = std::vector<float>(2 * kTaps, 0.0f);
    std::vector<float> m_filter = std::vector<float>(kTaps, 0.0f);
    size_t m_head = 0;
    float m_energy = 0;
    float m_micSum = 0;
    float m_farSum = 0;
    int m_summed = 0;
};

} // namespace

struct EchoCanceller::Private {
    SpeexEchoState *echo = nullptr;
    SpeexPreprocessState *preprocess = nullptr;

    std::mutex renderLock;
    Ring render{size_t(kRate * 2)};

    // Capture thread only.
    std::vector<int16_t> captureIn;
    int64_t capturePos = -1;
    // Primed with one frame of silence: what goes out lags what comes in by
    // exactly one frame, so every buffer can be filled whatever its size.
    std::vector<int16_t> captureOut = std::vector<int16_t>(kFrame, 0);
    std::vector<int16_t> reference = std::vector<int16_t>(kFrame, 0);
    Ring heard{size_t(kRate)};
    std::vector<int16_t> learnMic = std::vector<int16_t>(kFrame, 0);
    std::vector<int16_t> learnFar = std::vector<int16_t>(kFrame, 0);
    std::vector<int16_t> cleaned = std::vector<int16_t>(kFrame, 0);
    DelayEstimator estimator;
    FeedbackSuppressor suppressor{kRate};
    int64_t delay = kDefaultDelay;
    std::vector<int64_t> found;
    // How far the newest far-end audio trails the microphone, worst this second.
    int64_t arrival = 0;
    int frames = 0;
    double heardEnergy = 0;
    double leftEnergy = 0;

    ~Private()
    {
        if (preprocess) {
            speex_preprocess_state_destroy(preprocess);
        }
        if (echo) {
            speex_echo_state_destroy(echo);
        }
    }

    void process(int16_t *frame);
    void check();

    static GstPadProbeReturn onPlayback(GstPad *pad, GstPadProbeInfo *info, gpointer user);
    static GstPadProbeReturn onCapture(GstPad *pad, GstPadProbeInfo *info, gpointer user);
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
    // While the near end talks too: Speex's -15 dB let two loudspeakers build up.
    int suppressActive = -25;
    speex_preprocess_ctl(d->preprocess, SPEEX_PREPROCESS_SET_ECHO_SUPPRESS_ACTIVE, &suppressActive);
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

GstPadProbeReturn EchoCanceller::Private::onPlayback(GstPad *pad, GstPadProbeInfo *info, gpointer user)
{
    Private *p = of(user);
    GstBuffer *buffer = GST_PAD_PROBE_INFO_BUFFER(info);
    if (!buffer) {
        return GST_PAD_PROBE_OK;
    }
    const int64_t pos = positionOf(pad, buffer);
    GstMapInfo map;
    if (pos < 0 || !gst_buffer_map(buffer, &map, GST_MAP_READ)) {
        return GST_PAD_PROBE_OK;
    }
    std::lock_guard<std::mutex> lock(p->renderLock);
    p->render.write(pos, reinterpret_cast<const int16_t *>(map.data), map.size / sizeof(int16_t));
    gst_buffer_unmap(buffer, &map);
    return GST_PAD_PROBE_OK;
}

GstPadProbeReturn EchoCanceller::Private::onCapture(GstPad *pad, GstPadProbeInfo *info, gpointer user)
{
    Private *p = of(user);
    GstBuffer *buffer = GST_PAD_PROBE_INFO_BUFFER(info);
    if (!buffer) {
        return GST_PAD_PROBE_OK;
    }
    const int64_t pos = positionOf(pad, buffer);
    buffer = gst_buffer_make_writable(buffer);
    GST_PAD_PROBE_INFO_DATA(info) = buffer;
    GstMapInfo map;
    if (!gst_buffer_map(buffer, &map, GST_MAP_READWRITE)) {
        return GST_PAD_PROBE_OK;
    }
    auto *samples = reinterpret_cast<int16_t *>(map.data);
    const size_t count = map.size / sizeof(int16_t);

    // A gap or a jump: what is half a frame goes out as it came.
    const int64_t expected = p->capturePos + int64_t(p->captureIn.size());
    if (pos >= 0 && (p->capturePos < 0 || std::llabs(pos - expected) > int64_t(kFrame / 2))) {
        p->captureOut.insert(p->captureOut.end(), p->captureIn.begin(), p->captureIn.end());
        p->captureIn.clear();
        p->capturePos = pos;
    }
    p->captureIn.insert(p->captureIn.end(), samples, samples + count);
    size_t done = 0;
    for (; done + kFrame <= p->captureIn.size(); done += kFrame) {
        p->process(p->captureIn.data() + done);
    }
    p->captureIn.erase(p->captureIn.begin(), p->captureIn.begin() + done);

    // captureOut holds one frame plus count minus what is still waiting: never short.
    std::memcpy(samples, p->captureOut.data(), count * sizeof(int16_t));
    p->captureOut.erase(p->captureOut.begin(), p->captureOut.begin() + count);
    gst_buffer_unmap(buffer, &map);
    return GST_PAD_PROBE_OK;
}

void EchoCanceller::Private::process(int16_t *frame)
{
    heard.write(capturePos, frame, kFrame);
    heard.read(capturePos - kLearnBehind, learnMic.data(), kFrame);
    {
        std::lock_guard<std::mutex> lock(renderLock);
        render.read(capturePos - delay, reference.data(), kFrame);
        render.read(capturePos - kLearnBehind, learnFar.data(), kFrame);
        if (render.end() >= 0) {
            arrival = std::max(arrival, capturePos - render.end());
        }
    }
    estimator.push(learnMic.data(), learnFar.data(), kFrame);
    double far = 0;
    double before = 0;
    for (size_t i = 0; i < kFrame; ++i) {
        far += double(reference[i]) * reference[i];
        before += double(frame[i]) * frame[i];
    }
    speex_echo_cancellation(echo, frame, reference.data(), cleaned.data());
    speex_preprocess_run(preprocess, cleaned.data());
    suppressor.process(cleaned.data(), kFrame);
    // What the canceller takes away, counted while the far end is talking.
    if (far > double(kFrame) * 300.0 * 300.0) {
        double after = 0;
        for (int16_t v : cleaned) {
            after += double(v) * v;
        }
        heardEnergy += before;
        leftEnergy += after;
    }
    captureOut.insert(captureOut.end(), cleaned.begin(), cleaned.end());
    capturePos += kFrame;
    if (++frames % kCheckEvery == 0) {
        check();
    }
}

// The median of the last checks, once most of them agree on it.
void EchoCanceller::Private::check()
{
    float prominence = 0;
    const int64_t earliest = arrival;
    arrival = 0;
    const int64_t peak = estimator.peak(&prominence, earliest, earliest + kPlayout);
    if (frames % (10 * kCheckEvery) == 0) {
        const double removed = leftEnergy > 0 ? 10.0 * std::log10(heardEnergy / leftEnergy) : 0.0;
        qInfo("xmatic: echo path at %lld ms (peak %.0fx), far end %lld ms behind, shift %lld ms, removed %.1f dB",
              (long long)(peak * 1000 / kRate), prominence, (long long)(earliest * 1000 / kRate),
              (long long)((delay + kMargin) * 1000 / kRate), removed);
        heardEnergy = leftEnergy = 0;
    }
    if (peak < 0) {
        return;
    }
    found.push_back(peak);
    if (found.size() > kHistory) {
        found.erase(found.begin());
    }
    std::vector<int64_t> sorted = found;
    std::sort(sorted.begin(), sorted.end());
    const int64_t median = sorted[sorted.size() / 2];
    const size_t near = size_t(std::count_if(found.begin(), found.end(), [median](int64_t f) {
        return std::llabs(f - median) <= kAgree;
    }));
    if (near < kQuorum) {
        return;
    }
    const int64_t shift = std::max<int64_t>(earliest, median - kMargin);
    if (std::llabs(shift - delay) > kMoveAfter) {
        qInfo("xmatic: echo delay %lld ms (was %lld ms)", (long long)(median * 1000 / kRate),
              (long long)((delay + kMargin) * 1000 / kRate));
        delay = shift;
        speex_echo_state_reset(echo);
    }
}
