#ifndef VOICERECORDER_H
#define VOICERECORDER_H

#include <QAudioRecorder>
#include <QObject>
#include <QString>

/// Records a voice message into the cache. Qt 5.6 has no QML recorder, and the
/// codec is chosen from what the device offers rather than assumed.
class VoiceRecorder : public QObject
{
    Q_OBJECT

    Q_PROPERTY(bool recording READ recording NOTIFY recordingChanged)
    Q_PROPERTY(qint64 duration READ duration NOTIFY durationChanged)
    Q_PROPERTY(QString mimeType READ mimeType CONSTANT)

public:
    explicit VoiceRecorder(const QString &cacheDirectory, QObject *parent = nullptr);

    bool recording() const;
    qint64 duration() const;
    QString mimeType() const { return m_mimeType; }

    /// Starts a new recording, discarding any previous one.
    Q_INVOKABLE void start();

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

private:
    void chooseFormat();

    QAudioRecorder *m_recorder;
    QString m_cacheDirectory;
    QString m_currentPath;
    QString m_mimeType = QStringLiteral("audio/ogg");
    /// Read when the recording is stopped: the recorder's own duration is back
    /// to zero by the time the file has been written out.
    qint64 m_lastDuration = 0;
    bool m_discard = false;
};

#endif // VOICERECORDER_H
