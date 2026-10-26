#include "feedbacksuppressor.h"

#include <QtGlobal>

#include <algorithm>
#include <cmath>
#include <complex>

namespace {

const size_t kSize = 2048;            // ~43 ms at 48 kHz, 23 Hz per bin
const float kLowest = 200.0f;
const float kHighest = 8000.0f;
const float kProminence = 25.0f;      // dB over the spectrum's median
const float kRivalRange = 15.0f;      // speech has several partials this close
const int kMaxRivals = 2;
const int kStanding = 20;             // analyses, 20 ms apart: 400 ms
const double kDepth = -18.0;          // dB
const double kQ = 30.0;
const size_t kMaxNotches = 6;
const double kHoldSeconds = 10.0;

void fft(std::vector<std::complex<float>> &a)
{
    const size_t n = a.size();
    for (size_t i = 1, j = 0; i < n; ++i) {
        size_t bit = n >> 1;
        for (; j & bit; bit >>= 1) {
            j ^= bit;
        }
        j ^= bit;
        if (i < j) {
            std::swap(a[i], a[j]);
        }
    }
    for (size_t len = 2; len <= n; len <<= 1) {
        const float angle = -2.0f * float(M_PI) / float(len);
        const std::complex<float> step(std::cos(angle), std::sin(angle));
        for (size_t i = 0; i < n; i += len) {
            std::complex<float> w(1.0f, 0.0f);
            for (size_t k = 0; k < len / 2; ++k) {
                const std::complex<float> u = a[i + k];
                const std::complex<float> v = a[i + k + len / 2] * w;
                a[i + k] = u + v;
                a[i + k + len / 2] = u - v;
                w *= step;
            }
        }
    }
}

} // namespace

FeedbackSuppressor::FeedbackSuppressor(int rate)
    : m_rate(rate)
    , m_window(kSize)
    , m_history(kSize, 0.0f)
{
    for (size_t i = 0; i < kSize; ++i) {
        m_window[i] = 0.5f - 0.5f * std::cos(2.0f * float(M_PI) * float(i) / float(kSize - 1));
    }
}

void FeedbackSuppressor::process(int16_t *samples, size_t count)
{
    const size_t hop = size_t(m_rate / 50);
    for (size_t i = 0; i < count; ++i) {
        m_history[m_filled % kSize] = samples[i];
        ++m_filled;
        if (++m_sinceAnalysis >= hop && m_filled >= kSize) {
            m_sinceAnalysis = 0;
            analyse();
        }
    }

    m_notches.erase(std::remove_if(m_notches.begin(), m_notches.end(),
                                   [this](const Notch &n) { return n.until < m_clock; }),
                    m_notches.end());
    m_clock += int64_t(count);
    if (m_notches.empty()) {
        return;
    }
    for (size_t i = 0; i < count; ++i) {
        double v = samples[i];
        for (Notch &n : m_notches) {
            const double y = n.b0 * v + n.b1 * n.x1 + n.b2 * n.x2 - n.a1 * n.y1 - n.a2 * n.y2;
            n.x2 = n.x1;
            n.x1 = v;
            n.y2 = n.y1;
            n.y1 = y;
            v = y;
        }
        samples[i] = int16_t(std::max(-32768.0, std::min(32767.0, std::round(v))));
    }
}

void FeedbackSuppressor::analyse()
{
    std::vector<std::complex<float>> spectrum(kSize);
    const size_t oldest = m_filled % kSize;
    for (size_t i = 0; i < kSize; ++i) {
        spectrum[i] = m_history[(oldest + i) % kSize] * m_window[i];
    }
    fft(spectrum);

    const float binHz = float(m_rate) / float(kSize);
    const int low = int(kLowest / binHz);
    const int high = std::min(int(kHighest / binHz), int(kSize / 2) - 2);
    std::vector<float> level(size_t(high + 2));
    for (int b = low - 1; b <= high + 1; ++b) {
        level[size_t(b)] = 10.0f * std::log10(std::norm(spectrum[size_t(b)]) + 1e-3f);
    }
    std::vector<float> band(level.begin() + low, level.begin() + high + 1);
    std::nth_element(band.begin(), band.begin() + long(band.size() / 2), band.end());
    const float median = band[band.size() / 2];

    std::vector<int> peaks;
    for (int b = low; b <= high; ++b) {
        if (level[size_t(b)] > level[size_t(b - 1)] && level[size_t(b)] >= level[size_t(b + 1)]) {
            peaks.push_back(b);
        }
    }

    std::vector<Track> next;
    for (int b : peaks) {
        if (level[size_t(b)] - median < kProminence) {
            continue;
        }
        int rivals = -1;
        for (int other : peaks) {
            rivals += level[size_t(other)] > level[size_t(b)] - kRivalRange;
        }
        if (rivals > kMaxRivals) {
            continue;
        }
        Track track{b, 1, level[size_t(b)]};
        for (const Track &old : m_tracks) {
            if (std::abs(old.bin - b) <= 1) {
                track.frames = old.frames + 1;
                track.firstLevel = old.firstLevel;
                break;
            }
        }
        // Standing long enough and not fading: refreshed while it stays.
        if (track.frames >= kStanding && level[size_t(b)] >= track.firstLevel) {
            const float l = level[size_t(b - 1)];
            const float c = level[size_t(b)];
            const float r = level[size_t(b + 1)];
            const float denominator = l - 2.0f * c + r;
            const float offset = denominator != 0.0f ? 0.5f * (l - r) / denominator : 0.0f;
            engage((float(b) + offset) * binHz);
        }
        next.push_back(track);
    }
    m_tracks.swap(next);
}

void FeedbackSuppressor::engage(float frequency)
{
    const int64_t until = m_clock + int64_t(kHoldSeconds * m_rate);
    for (Notch &n : m_notches) {
        if (std::fabs(n.frequency - frequency) < 30.0f) {
            n.until = until;
            return;
        }
    }
    if (m_notches.size() >= kMaxNotches) {
        m_notches.erase(m_notches.begin());
    }
    Notch n;
    n.frequency = frequency;
    n.until = until;
    const double a = std::pow(10.0, kDepth / 40.0);
    const double w0 = 2.0 * M_PI * frequency / m_rate;
    const double alpha = std::sin(w0) / (2.0 * kQ);
    const double a0 = 1.0 + alpha / a;
    n.b0 = (1.0 + alpha * a) / a0;
    n.b1 = -2.0 * std::cos(w0) / a0;
    n.b2 = (1.0 - alpha * a) / a0;
    n.a1 = n.b1;
    n.a2 = (1.0 - alpha / a) / a0;
    m_notches.push_back(n);
    qInfo("xmatic: feedback at %.0f Hz, notch on", double(frequency));
}
