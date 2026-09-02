#include "voicerecorder.h"

#include <QAudioEncoderSettings>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QUrl>

VoiceRecorder::VoiceRecorder(const QString &cacheDirectory, QObject *parent)
    : QObject(parent)
    , m_recorder(new QAudioRecorder(this))
    , m_cacheDirectory(cacheDirectory)
{
    chooseFormat();

    connect(m_recorder, &QAudioRecorder::durationChanged, this, &VoiceRecorder::durationChanged);
    // The container is written after recording stops: the state is Stopped while
    // the status is still Finalizing, and waiting for the state loses the file.
    connect(m_recorder,
            &QAudioRecorder::statusChanged,
            this,
            [this](QMediaRecorder::Status status) {
                emit recordingChanged();

                if (status != QMediaRecorder::LoadedStatus
                    && status != QMediaRecorder::UnloadedStatus) {
                    return;
                }
                if (m_currentPath.isEmpty()) {
                    return;
                }

                const QString path = m_currentPath;
                m_currentPath.clear();

                if (m_discard) {
                    QFile::remove(path);
                    m_discard = false;
                    qInfo("xmatic: recording discarded");
                    return;
                }

                const qint64 size = QFile::exists(path) ? QFile(path).size() : -1;
                qInfo("xmatic: recording finished, %lld bytes", size);

                // Nothing recorded usually means the microphone was never granted; the empty
                // file would be an unplayable message.
                if (size <= 0) {
                    emit failed(tr("Nothing was recorded."));
                    return;
                }

                emit finished(path, m_mimeType, m_lastDuration);
            });

    connect(m_recorder, &QAudioRecorder::stateChanged, this, [this](QMediaRecorder::State) {
        emit recordingChanged();
    });

    connect(m_recorder,
            static_cast<void (QAudioRecorder::*)(QMediaRecorder::Error)>(&QAudioRecorder::error),
            this,
            [this](QMediaRecorder::Error) {
                qWarning("xmatic: recorder error: %s", qPrintable(m_recorder->errorString()));
                emit failed(m_recorder->errorString());
            });
}

void VoiceRecorder::chooseFormat()
{
    // Preference order: what Matrix clients expect first, then whatever the
    // device can actually do.
    const QStringList codecs = m_recorder->supportedAudioCodecs();
    qInfo("xmatic: audio codecs offered: %s", qPrintable(codecs.join(QStringLiteral(","))));

    QAudioEncoderSettings settings;
    settings.setQuality(QMultimedia::NormalQuality);

    if (codecs.contains(QStringLiteral("audio/x-opus"))) {
        settings.setCodec(QStringLiteral("audio/x-opus"));
        m_recorder->setContainerFormat(QStringLiteral("ogg"));
        m_mimeType = QStringLiteral("audio/ogg");
    } else if (codecs.contains(QStringLiteral("audio/opus"))) {
        settings.setCodec(QStringLiteral("audio/opus"));
        m_recorder->setContainerFormat(QStringLiteral("ogg"));
        m_mimeType = QStringLiteral("audio/ogg");
    } else if (codecs.contains(QStringLiteral("audio/vorbis"))) {
        settings.setCodec(QStringLiteral("audio/vorbis"));
        m_recorder->setContainerFormat(QStringLiteral("ogg"));
        m_mimeType = QStringLiteral("audio/ogg");
    } else if (codecs.contains(QStringLiteral("audio/mpeg"))) {
        settings.setCodec(QStringLiteral("audio/mpeg"));
        m_recorder->setContainerFormat(QStringLiteral("mp3"));
        m_mimeType = QStringLiteral("audio/mpeg");
    } else if (!codecs.isEmpty()) {
        settings.setCodec(codecs.first());
        m_mimeType = QStringLiteral("audio/ogg");
    }

    m_recorder->setAudioSettings(settings);
}

bool VoiceRecorder::recording() const
{
    return m_recorder->state() == QMediaRecorder::RecordingState;
}

qint64 VoiceRecorder::duration() const
{
    return m_recorder->duration();
}

void VoiceRecorder::start()
{
    if (recording()) {
        return;
    }

    QDir().mkpath(m_cacheDirectory);
    const QString stamp = QDateTime::currentDateTimeUtc().toString(QStringLiteral("yyyyMMdd-hhmmss"));
    const QString suffix = m_mimeType == QStringLiteral("audio/mpeg") ? QStringLiteral("mp3")
                                                                      : QStringLiteral("ogg");
    m_currentPath = QStringLiteral("%1/voice-%2.%3").arg(m_cacheDirectory, stamp, suffix);
    m_discard = false;

    m_lastDuration = 0;
    m_recorder->setOutputLocation(QUrl::fromLocalFile(m_currentPath));
    m_recorder->record();
    qInfo("xmatic: recording started");
    emit recordingChanged();
}

void VoiceRecorder::stop()
{
    if (!recording()) {
        return;
    }
    // Before stopping: the recorder answers 0 once it has stopped, and the
    // file is only written out later still.
    m_lastDuration = m_recorder->duration();
    m_recorder->stop();
}

void VoiceRecorder::cancel()
{
    if (!recording()) {
        return;
    }
    m_discard = true;
    m_recorder->stop();
}
