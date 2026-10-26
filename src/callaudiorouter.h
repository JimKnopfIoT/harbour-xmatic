#ifndef CALLAUDIOROUTER_H
#define CALLAUDIOROUTER_H

#include <QObject>
#include <QString>

class QTimer;

/// Received call audio is born on the media sink and muted by the adaptation.
/// This finds the stream by name, moves it to the real sink and unmutes it.
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
    /// Loudspeaker or earpiece; applied now and on every later routing.
    void setSpeaker(bool on);

private:
    void ensureConnection();
    bool routeCallStream();
    void applyPort(bool speaker);

    QTimer *m_retryTimer;
    void *m_mainloop = nullptr; // pa_threaded_mainloop*
    void *m_context = nullptr;  // pa_context*
    QString m_sink;
    QString m_speakerPort;
    QString m_earpiecePort;
    bool m_speakerOn = true;
    bool m_routed = false;
};

#endif // CALLAUDIOROUTER_H
