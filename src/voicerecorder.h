#ifndef VOICERECORDER_H
#define VOICERECORDER_H

#include <QAudioRecorder>
#include <QObject>
#include <QString>
#include <QTimer>

class QAudioBuffer;
class QAudioProbe;

/// Records a voice message into the cache. Qt 5.6 has no QML recorder, and the
/// codec is chosen from what the device offers rather than assumed.
class VoiceRecorder : public QObject
{
    Q_OBJECT

    Q_PROPERTY(bool recording READ recording NOTIFY recordingChanged)
    /// Recording without a finger on the button, until a tap or silence ends it.
    Q_PROPERTY(bool handsFree READ handsFree NOTIFY recordingChanged)
    Q_PROPERTY(qint64 duration READ duration NOTIFY durationChanged)
    Q_PROPERTY(QString mimeType READ mimeType CONSTANT)

public:
    explicit VoiceRecorder(const QString &cacheDirectory, QObject *parent = nullptr);

    bool recording() const;
    bool handsFree() const { return m_handsFree; }
    qint64 duration() const;
    QString mimeType() const { return m_mimeType; }

    /// Starts a new recording, discarding any previous one.
    Q_INVOKABLE void start();

    /// Starts one that runs on its own: a second tap, or seven seconds of
    /// silence after speech, sends it; silence from the start drops it.
    Q_INVOKABLE void startHandsFree();

    /// Stops and reports the finished file through finished().
    Q_INVOKABLE void stop();

    /// Stops and throws the recording away.
    Q_INVOKABLE void cancel();

signals:
    void recordingChanged();
    void durationChanged();

    /// A recording is ready, with its length: without one it is a plain audio
    /// attachment to every other client, and the bridges refuse it.
    void finished(const QString &path, const QString &mimeType, qint64 duration);

    void failed(const QString &message);

    /// Hands-free ended by silence or by the ceiling, not by the user.
    void autoStopped();
    /// Hands-free heard nothing above the silence line and was dropped.
    void nothingHeard();

private:
    void chooseFormat();
    void measure(const QAudioBuffer &buffer);
    void endHandsFree();

    QAudioRecorder *m_recorder;
    QAudioProbe *m_probe = nullptr;
    QString m_cacheDirectory;
    QString m_currentPath;
    QString m_mimeType = QStringLiteral("audio/ogg");
    /// Read when the recording is stopped: the recorder's own duration is back
    /// to zero by the time the file has been written out.
    qint64 m_lastDuration = 0;
    bool m_discard = false;

    bool m_handsFree = false;
    bool m_heardSpeech = false;
    qint64 m_quietMs = 0;
    qint64 m_loudRunMs = 0;
    bool m_bufferLogged = false;
    qint64 m_levelWindowMs = 0;
    double m_levelWindowPeak = 0.0;
    QTimer m_ceiling;
};

#endif // VOICERECORDER_H
