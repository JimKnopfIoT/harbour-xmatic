#include "voicerecorder.h"

#include <QAudioBuffer>
#include <QAudioEncoderSettings>
#include <QAudioProbe>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QUrl>

#include <cmath>

namespace {

/// Seven seconds without speech end a hands-free take; five minutes end any.
const qint64 SilenceMs = 7000;
const int CeilingMs = 5 * 60 * 1000;
/// A buffer's RMS, 0..1, from which on it counts as speech. Tuned on the device.
const double SpeechLevel = 0.01;
/// A syllable lasts this long above the line; a click or a knock does not.
const qint64 SpeechRunMs = 200;

} // namespace

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
                endHandsFree();
                emit failed(m_recorder->errorString());
            });

    // The levels decide when a hands-free take has ended. Asked, not assumed:
    // without them a tap or the ceiling ends it.
    m_probe = new QAudioProbe(this);
    const bool probing = m_probe->setSource(m_recorder);
    qInfo("xmatic: recorder levels %s", probing ? "available" : "unavailable");
    connect(m_probe, &QAudioProbe::audioBufferProbed, this, &VoiceRecorder::measure);

    m_ceiling.setSingleShot(true);
    m_ceiling.setInterval(CeilingMs);
    connect(&m_ceiling, &QTimer::timeout, this, [this]() {
        if (m_handsFree && recording()) {
            stop();
            emit autoStopped();
        }
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

void VoiceRecorder::startHandsFree()
{
    if (recording()) {
        return;
    }
    m_heardSpeech = false;
    m_quietMs = 0;
    m_loudRunMs = 0;
    m_bufferLogged = false;
    m_levelWindowMs = 0;
    m_levelWindowPeak = 0.0;
    m_handsFree = true;
    m_ceiling.start();
    start();
}

void VoiceRecorder::stop()
{
    endHandsFree();
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
    endHandsFree();
    if (!recording()) {
        return;
    }
    m_discard = true;
    m_recorder->stop();
}

void VoiceRecorder::endHandsFree()
{
    if (!m_handsFree) {
        return;
    }
    m_handsFree = false;
    m_ceiling.stop();
    emit recordingChanged();
}

void VoiceRecorder::measure(const QAudioBuffer &buffer)
{
    if (!m_handsFree || !buffer.isValid() || buffer.sampleCount() <= 0) {
        return;
    }
    const QAudioFormat format = buffer.format();
    const int samples = buffer.sampleCount();
    double sum = 0.0;
    if (format.sampleType() == QAudioFormat::SignedInt && format.sampleSize() == 16) {
        const qint16 *data = buffer.constData<qint16>();
        for (int i = 0; i < samples; ++i) {
            const double value = data[i] / 32768.0;
            sum += value * value;
        }
    } else if (format.sampleType() == QAudioFormat::Float && format.sampleSize() == 32) {
        const float *data = buffer.constData<float>();
        for (int i = 0; i < samples; ++i) {
            sum += double(data[i]) * data[i];
        }
    } else if (format.sampleType() == QAudioFormat::UnSignedInt && format.sampleSize() == 8) {
        const quint8 *data = buffer.constData<quint8>();
        for (int i = 0; i < samples; ++i) {
            const double value = (int(data[i]) - 128) / 128.0;
            sum += value * value;
        }
    } else {
        return;
    }
    const double level = std::sqrt(sum / samples);
    const qint64 milliseconds = buffer.duration() / 1000;
    if (!m_bufferLogged) {
        m_bufferLogged = true;
        qDebug("xmatic: hands-free buffers of %lld ms", milliseconds);
    }

    // One line a second, to tune the threshold against a real room.
    m_levelWindowPeak = qMax(m_levelWindowPeak, level);
    m_levelWindowMs += milliseconds;
    if (m_levelWindowMs >= 1000) {
        qDebug("xmatic: hands-free level %.3f", m_levelWindowPeak);
        m_levelWindowMs = 0;
        m_levelWindowPeak = 0.0;
    }

    if (level >= SpeechLevel) {
        m_loudRunMs += milliseconds;
        if (m_loudRunMs >= SpeechRunMs) {
            m_heardSpeech = true;
            m_quietMs = 0;
            return;
        }
    } else {
        m_loudRunMs = 0;
    }
    // Until a sound has lasted, it counts as quiet: a knock must not keep a take alive.
    m_quietMs += milliseconds;
    if (m_quietMs < SilenceMs) {
        return;
    }
    // Silence after speech is the end of what was said; silence alone is nothing.
    if (m_heardSpeech) {
        stop();
        emit autoStopped();
    } else {
        cancel();
        emit nothingHeard();
    }
}
