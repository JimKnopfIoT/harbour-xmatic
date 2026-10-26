#ifndef ECHOCANCELLER_H
#define ECHOCANCELLER_H

#include <gst/gst.h>

#include <memory>

/// Echo cancellation (the system's speexdsp) between the call's playback and microphone.
/// Both sides run S16LE, 48 kHz, mono; the pipeline pins that with caps.
/// Without it two loudspeaker phones feed each other until they howl.
class EchoCanceller
{
public:
    EchoCanceller();
    ~EchoCanceller();

    /// Playback, read only: the far end the canceller subtracts.
    void watchPlayback(GstPad *pad);
    /// Microphone, rewritten in place, ten milliseconds late.
    void processCapture(GstPad *pad);

    bool available() const;

private:
    struct Private;
    // Shared with the probes: a released pipeline can outlive the call.
    std::shared_ptr<Private> d;
};

#endif // ECHOCANCELLER_H
