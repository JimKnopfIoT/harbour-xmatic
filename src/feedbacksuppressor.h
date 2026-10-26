#ifndef FEEDBACKSUPPRESSOR_H
#define FEEDBACKSUPPRESSOR_H

#include <cstddef>
#include <cstdint>
#include <vector>

/// Howling between two loudspeakers is one tone that stands and grows; speech
/// never holds a lone pitch that long. Each such tone gets a narrow notch.
class FeedbackSuppressor
{
public:
    explicit FeedbackSuppressor(int rate);

    /// In place, S16 mono at the rate given.
    void process(int16_t *samples, size_t count);

private:
    struct Notch {
        float frequency = 0;
        double b0 = 1, b1 = 0, b2 = 0, a1 = 0, a2 = 0;
        double x1 = 0, x2 = 0, y1 = 0, y2 = 0;
        int64_t until = 0;
    };
    struct Track {
        int bin;
        int frames;
        float firstLevel;
    };

    void analyse();
    void engage(float frequency);

    int m_rate;
    int64_t m_clock = 0;
    std::vector<float> m_window;
    std::vector<float> m_history;
    size_t m_filled = 0;
    size_t m_sinceAnalysis = 0;
    std::vector<Track> m_tracks;
    std::vector<Notch> m_notches;
};

#endif // FEEDBACKSUPPRESSOR_H
