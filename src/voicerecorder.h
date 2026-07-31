#ifndef VOICERECORDER_H
#define VOICERECORDER_H

#include <QAudioRecorder>
#include <QObject>
#include <QString>

/// Records a voice message into the cache directory.
///
/// Qt 5.6 has no QML recorder element, so this wraps QAudioRecorder. The codec
/// is chosen from what the device actually offers rather than assumed: Opus in
/// Ogg is what other Matrix clients expect, but a device that cannot encode it
/// still has to be able to record something.
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

    /// A recording is complete and ready to be sent.
    void finished(const QString &path, const QString &mimeType);

    void failed(const QString &message);

private:
    void chooseFormat();

    QAudioRecorder *m_recorder;
    QString m_cacheDirectory;
    QString m_currentPath;
    QString m_mimeType = QStringLiteral("audio/ogg");
    bool m_discard = false;
};

#endif // VOICERECORDER_H
