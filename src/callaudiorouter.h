#ifndef CALLAUDIOROUTER_H
#define CALLAUDIOROUTER_H

#include <QObject>
#include <QString>

class QTimer;

/// Makes received call audio audible on Sailfish/Halium.
///
/// The playback stream a call creates is born on the media sink and MUTED by
/// the hardware adaptation — reaching PulseAudio but silent. This connects to
/// PulseAudio in-process (Sailjail blocks an external `pactl` but allows the
/// app's own audio access), finds the call's playback stream by application
/// name, moves it onto the real output sink and unmutes it. It retries until
/// the stream appears, since that happens a moment after the call connects.
///
/// The libpulse calls here are ordinary API use; the approach was learned from
/// the sibling Fernschreiber project, but no code is shared, so this file
/// stays under the project's own licence.
class CallAudioRouter : public QObject
{
    Q_OBJECT
public:
    explicit CallAudioRouter(QObject *parent = nullptr);
    ~CallAudioRouter() override;

    /// Begin routing; safe to call again, it just keeps retrying.
    void start();
    /// Stop retrying. The stream goes away with the pipeline.
    void stop();

private:
    void ensureConnection();
    bool routeCallStream();

    QTimer *m_retryTimer;
    void *m_mainloop = nullptr; // pa_threaded_mainloop*
    void *m_context = nullptr;  // pa_context*
    QString m_sink;
    QString m_speakerPort;
};

#endif // CALLAUDIOROUTER_H
