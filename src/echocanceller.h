#ifndef ECHOCANCELLER_H
#define ECHOCANCELLER_H

#include <gst/gst.h>

#include <memory>

/// Echo cancellation (WebRTC's AEC3) between the call's playback and microphone.
/// Both sides run S16LE, 48 kHz, mono, in 10 ms buffers; the pipeline pins that. The
/// canceller finds the echo path's delay itself; without it two loudspeaker
/// phones feed each other until they howl.
class EchoCanceller
{
public:
    EchoCanceller();
    ~EchoCanceller();

    /// Playback, read only, as it enters the sink: the far end the canceller subtracts.
    /// The first pad only; later ones are ignored.
    void watchPlayback(GstPad *pad);
    /// Microphone, rewritten in place.
    void processCapture(GstPad *pad);

    bool available() const;

private:
    struct Private;
    // Shared with the probes: a released pipeline can outlive the call.
    std::shared_ptr<Private> d;
};

#endif // ECHOCANCELLER_H
