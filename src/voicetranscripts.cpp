#include "voicetranscripts.h"

#include <QCoreApplication>
#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusReply>
#include <QDBusVariant>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJSValue>
#include <QProcess>
#include <QProcessEnvironment>
#include <QUuid>

#include <sailfishapp.h>

#include "voicedecode.h"

namespace {

const char *Service = "org.mkiol.Speech";
const char *ServicePath = "/";
const char *ServiceInterface = "org.mkiol.Speech";

const char *BusService = "org.freedesktop.DBus";
const char *BusPath = "/org/freedesktop/DBus";
const char *PropertiesInterface = "org.freedesktop.DBus.Properties";

/// Declared values are claims: refused on early here, enforced by the decoder.
const qint64 MaxFileBytes = 20 * 1024 * 1024;
const qint64 MaxDurationMs = 10 * 60 * 1000;
const int MaxTranscriptChars = 5000;

const int CallTimeoutMs = 10000;
const int KeepAliveMs = 2000;
const int DownloadTimeoutMs = 120000;
const int DecodeTimeoutMs = 150000;
const int IdleAttempts = 20;

/// The service's own states that matter here.
const int StateNotConfigured = 1;
const int StateIdle = 3;

/// A line of speech, as plain text: bidi overrides reorder what the reader
/// sees, control characters have no business in it, and it has an end.
QString plainTranscript(const QString &raw)
{
    QString out;
    out.reserve(raw.size());
    for (const QChar character : raw) {
        const ushort code = character.unicode();
        if ((code >= 0x202A && code <= 0x202E) || (code >= 0x2066 && code <= 0x2069)
                || code == 0x200E || code == 0x200F || code == 0x061C) {
            continue;
        }
        const QChar::Category category = character.category();
        if (category == QChar::Other_Control || category == QChar::Other_Format) {
            out.append(QLatin1Char(' '));
            continue;
        }
        out.append(character);
    }
    out = out.simplified();
    if (out.size() > MaxTranscriptChars) {
        out.truncate(MaxTranscriptChars - 1);
        out.append(QChar(0x2026));
    }
    return out;
}

/// One entry of the service's model map: id, name, options, language.
QStringList stringsOf(const QVariant &value)
{
    QVariant plain = value;
    if (plain.userType() == qMetaTypeId<QDBusVariant>()) {
        plain = plain.value<QDBusVariant>().variant();
    }
    if (plain.userType() != qMetaTypeId<QDBusArgument>()) {
        return plain.toStringList();
    }
    const QDBusArgument argument = plain.value<QDBusArgument>();
    QStringList out;
    if (argument.currentSignature() == QLatin1String("as")) {
        argument >> out;
    } else if (argument.currentSignature() == QLatin1String("av")) {
        argument.beginArray();
        while (!argument.atEnd()) {
            QDBusVariant element;
            argument >> element;
            out.append(element.variant().toString());
        }
        argument.endArray();
    }
    return out;
}

/// The magic, not the declared type: an Ogg page that carries an Opus header.
bool looksLikeOggOpus(const QString &path)
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        return false;
    }
    const QByteArray head = file.read(256);
    return head.startsWith("OggS") && head.contains("OpusHead");
}

QVariant replyValue(const QDBusMessage &reply)
{
    return reply.arguments().value(0).value<QDBusVariant>().variant();
}

} // namespace

VoiceTranscripts::VoiceTranscripts(const QString &workDirectory, QObject *parent)
    : QObject(parent)
    , m_workDirectory(workDirectory)
{
    m_keepAlive.setInterval(KeepAliveMs);
    connect(&m_keepAlive, &QTimer::timeout, this, &VoiceTranscripts::keepAlive);

    m_deadline.setSingleShot(true);
    connect(&m_deadline, &QTimer::timeout, this, [this]() {
        if (m_busy) {
            fail(m_deadlineReason);
        }
    });
}

void VoiceTranscripts::transcribe(const QString &itemId, const QString &eventId,
                                  const QVariant &media)
{
    if (itemId.isEmpty() || eventId.isEmpty()) {
        return;
    }
    const QString current = m_states.value(eventId);
    if (current == QLatin1String("working") || current == QLatin1String("done")) {
        return;
    }

    // QML hands nested objects over as QJSValue.
    QVariant plain = media;
    if (plain.canConvert<QJSValue>()) {
        plain = plain.value<QJSValue>().toVariant();
    }
    const QVariantMap fields = plain.toMap();

    Job job;
    job.eventId = eventId;
    // The player's key: a file already fetched for listening is not fetched twice.
    job.mediaKey = itemId + QStringLiteral("/full");
    job.source = fields.value(QStringLiteral("source"));
    job.declaredSize = fields.value(QStringLiteral("size")).toLongLong();

    m_texts.remove(eventId);
    m_failures.remove(eventId);
    if (!job.source.isValid()) {
        markFailed(eventId, tr("This voice message has no file."));
        return;
    }
    if (job.declaredSize > MaxFileBytes
            || fields.value(QStringLiteral("duration")).toLongLong() > MaxDurationMs) {
        markFailed(eventId, tr("The voice message is too long to convert."));
        return;
    }

    m_states.insert(eventId, QStringLiteral("working"));
    bump();
    m_queue.append(job);
    next();
}

QString VoiceTranscripts::state(const QString &eventId) const
{
    return m_states.value(eventId);
}

QString VoiceTranscripts::text(const QString &eventId) const
{
    return m_texts.value(eventId);
}

QString VoiceTranscripts::failure(const QString &eventId) const
{
    return m_failures.value(eventId);
}

void VoiceTranscripts::check()
{
    if (m_checkState == QLatin1String("running")) {
        return;
    }
    setCheck(QStringLiteral("running"), QString());
    Job job;
    job.check = true;
    m_queue.append(job);
    next();
}

void VoiceTranscripts::clear()
{
    m_queue.clear();
    if (m_busy) {
        endJob();
    }
    // Best effort: what a transcript said should not linger in freed memory.
    for (auto it = m_texts.begin(); it != m_texts.end(); ++it) {
        it.value().fill(QChar(0));
    }
    m_texts.clear();
    m_states.clear();
    m_failures.clear();
    QDir(m_workDirectory).removeRecursively();
    bump();
}

void VoiceTranscripts::mediaArrived(const QString &key, const QString &path)
{
    if (!m_busy || m_job.check || key != m_job.mediaKey || !m_job.mediaPath.isEmpty()) {
        return;
    }
    m_job.mediaPath = path;
    decode();
}

void VoiceTranscripts::markFailed(const QString &eventId, const QString &reason)
{
    m_states.insert(eventId, QStringLiteral("failed"));
    m_failures.insert(eventId, reason);
    bump();
}

void VoiceTranscripts::bump()
{
    ++m_revision;
    emit revisionChanged();
}

void VoiceTranscripts::next()
{
    if (m_busy || m_queue.isEmpty()) {
        return;
    }
    m_job = m_queue.takeFirst();
    m_busy = true;
    ++m_generation;

    if (m_job.check) {
        m_job.mediaPath = SailfishApp::pathTo(QStringLiteral("data/voice-check.opus")).toLocalFile();
        decode();
        return;
    }
    // Armed first: a cached file answers within this very call.
    arm(DownloadTimeoutMs, tr("The voice message could not be downloaded."));
    emit mediaWanted(m_job.mediaKey, m_job.source, m_job.declaredSize);
}

void VoiceTranscripts::decode()
{
    const QFileInfo input(m_job.mediaPath);
    if (!input.isFile()) {
        fail(tr("The voice message could not be read."));
        return;
    }
    if (input.size() > MaxFileBytes) {
        fail(tr("The voice message is too long to convert."));
        return;
    }
    if (!looksLikeOggOpus(m_job.mediaPath)) {
        fail(tr("This is not a voice message that can be converted."));
        return;
    }
    if (!QDir().mkpath(m_workDirectory)) {
        fail(tr("The voice message could not be decoded."));
        return;
    }
    m_job.wavPath = m_workDirectory + QLatin1Char('/')
            + QString::fromLatin1(QUuid::createUuid().toRfc4122().toHex())
            + QStringLiteral(".wav");

    m_helper = new QProcess(this);
    QProcessEnvironment environment = QProcessEnvironment::systemEnvironment();
    environment.insert(QStringLiteral(XMATIC_VOICE_DECODE_ENV), QStringLiteral("1"));
    environment.insert(QStringLiteral(XMATIC_VOICE_IN_ENV), m_job.mediaPath);
    environment.insert(QStringLiteral(XMATIC_VOICE_OUT_ENV), m_job.wavPath);
    m_helper->setProcessEnvironment(environment);
    m_helper->setStandardOutputFile(QProcess::nullDevice());
    m_helper->setStandardErrorFile(QProcess::nullDevice());

    const quint64 generation = m_generation;
    QProcess *helper = m_helper;
    connect(helper,
            static_cast<void (QProcess::*)(int, QProcess::ExitStatus)>(&QProcess::finished),
            this, [this, helper, generation](int code, QProcess::ExitStatus status) {
                helper->deleteLater();
                if (m_helper == helper) {
                    m_helper = nullptr;
                }
                if (generation == m_generation && m_busy) {
                    decoded(code, status != QProcess::NormalExit);
                }
            });
    connect(helper, &QProcess::errorOccurred, this,
            [this, helper, generation](QProcess::ProcessError error) {
                if (error != QProcess::FailedToStart) {
                    return;
                }
                helper->deleteLater();
                if (m_helper == helper) {
                    m_helper = nullptr;
                }
                if (generation == m_generation && m_busy) {
                    qWarning("xmatic: voice decoder did not start");
                    fail(tr("The voice message could not be decoded."));
                }
            });

    arm(DecodeTimeoutMs, tr("Converting took too long and was stopped."));
    m_helper->start(QCoreApplication::applicationFilePath(), QStringList());
}

void VoiceTranscripts::decoded(int exitCode, bool crashed)
{
    if (crashed) {
        qWarning("xmatic: voice decoder crashed");
        fail(tr("The voice message could not be decoded."));
        return;
    }
    switch (exitCode) {
    case VoiceDecodeOk:
        activate();
        return;
    case VoiceDecodeNotVoice:
        fail(tr("This is not a voice message that can be converted."));
        return;
    case VoiceDecodeTooLong:
        fail(tr("The voice message is too long to convert."));
        return;
    case VoiceDecodeTimedOut:
        fail(tr("Converting took too long and was stopped."));
        return;
    default:
        fail(tr("The voice message could not be decoded."));
        return;
    }
}

void VoiceTranscripts::activate()
{
    arm(3 * CallTimeoutMs, tr("Speech Note is not installed, or its service does not start."));
    ask(QString::fromLatin1(BusService), QString::fromLatin1(BusPath), QString::fromLatin1(BusService),
        QStringLiteral("StartServiceByName"),
        QVariantList() << QString::fromLatin1(Service) << QVariant::fromValue(0u),
        [this](const QDBusMessage &reply) {
            if (reply.type() == QDBusMessage::ErrorMessage) {
                fail(serviceProblem(reply));
                return;
            }
            verify();
        });
}

void VoiceTranscripts::verify()
{
    // Bound to the unique name that answers now: a name can change owner during
    // a job, a unique name cannot. Who may own it at all: docs/PITFALLS.md.
    QDBusConnectionInterface *bus = QDBusConnection::sessionBus().interface();
    const QDBusReply<QString> owner = bus->serviceOwner(QString::fromLatin1(Service));
    if (!owner.isValid() || owner.value().isEmpty()) {
        fail(tr("Speech Note is not installed, or its service does not start."));
        return;
    }
    m_job.owner = owner.value();

    ask(m_job.owner, QString::fromLatin1(ServicePath), QString::fromLatin1(PropertiesInterface),
        QStringLiteral("Get"),
        QVariantList() << QString::fromLatin1(ServiceInterface) << QStringLiteral("SttModels"),
        [this](const QDBusMessage &reply) {
            if (reply.type() == QDBusMessage::ErrorMessage) {
                fail(serviceProblem(reply));
                return;
            }
            QVariantMap models;
            replyValue(reply).value<QDBusArgument>() >> models;
            // Recognising the language by itself is the point: a message is in
            // whatever language its sender spoke.
            QString chosen;
            for (auto it = models.constBegin(); it != models.constEnd(); ++it) {
                const QStringList fields = stringsOf(it.value());
                const QString id = fields.value(0, it.key());
                if (!id.startsWith(QLatin1String("auto_")) && fields.value(3) != QLatin1String("auto")) {
                    continue;
                }
                if (chosen.isEmpty() || id.contains(QLatin1String("_small"))) {
                    chosen = id;
                }
            }
            if (chosen.isEmpty()) {
                fail(tr("Speech Note has no model that recognises the language by itself. Download \"Auto (WhisperCpp Small)\" in Speech Note."));
                return;
            }
            m_job.model = chosen;
            waitIdle(IdleAttempts);
        });
}

void VoiceTranscripts::waitIdle(int attemptsLeft)
{
    ask(m_job.owner, QString::fromLatin1(ServicePath), QString::fromLatin1(PropertiesInterface),
        QStringLiteral("Get"),
        QVariantList() << QString::fromLatin1(ServiceInterface) << QStringLiteral("State"),
        [this, attemptsLeft](const QDBusMessage &reply) {
            if (reply.type() == QDBusMessage::ErrorMessage) {
                fail(serviceProblem(reply));
                return;
            }
            const int state = replyValue(reply).toInt();
            if (state == StateIdle) {
                startTranscription();
                return;
            }
            if (state == StateNotConfigured) {
                fail(tr("Speech Note has no model that recognises the language by itself. Download \"Auto (WhisperCpp Small)\" in Speech Note."));
                return;
            }
            if (attemptsLeft <= 0) {
                fail(tr("The speech service is busy. Try again in a moment."));
                return;
            }
            keepAlive();
            const quint64 generation = m_generation;
            QTimer::singleShot(1000, this, [this, generation, attemptsLeft]() {
                if (generation == m_generation && m_busy) {
                    waitIdle(attemptsLeft - 1);
                }
            });
        });
}

void VoiceTranscripts::startTranscription()
{
    subscribe(true);
    ask(m_job.owner, QString::fromLatin1(ServicePath), QString::fromLatin1(ServiceInterface),
        QStringLiteral("SttTranscribeFile"),
        QVariantList() << m_job.wavPath << m_job.model << QString()
                       << QVariant::fromValue(QVariantMap()),
        [this](const QDBusMessage &reply) {
            if (reply.type() == QDBusMessage::ErrorMessage) {
                fail(serviceProblem(reply));
                return;
            }
            const int task = reply.arguments().value(0).toInt();
            if (task < 0) {
                fail(tr("The speech service refused the voice message."));
                return;
            }
            m_job.task = task;
            m_keepAlive.start();

            // About 0.4 of the length on a current phone; well past that is stuck.
            const qint64 seconds = qMax<qint64>(1, (QFileInfo(m_job.wavPath).size() - 44) / 32000);
            arm(int(qMin<qint64>(600000, 60000 + seconds * 3000)),
                tr("Converting took too long and was stopped."));

            for (const auto &piece : m_early) {
                if (piece.first == task) {
                    m_job.pieces.append(piece.second);
                }
            }
            m_early.clear();
            if (m_earlyDone.contains(task)) {
                finish();
            }
        });
}

void VoiceTranscripts::keepAlive()
{
    if (!m_busy || m_job.owner.isEmpty()) {
        return;
    }
    ask(m_job.owner, QString::fromLatin1(ServicePath), QString::fromLatin1(ServiceInterface),
        QStringLiteral("KeepAliveService"), QVariantList(),
        [this](const QDBusMessage &reply) {
            if (reply.type() == QDBusMessage::ErrorMessage) {
                fail(serviceProblem(reply));
            }
        });
    // The service kills a task whose timer runs out, silently and mid-file.
    if (m_job.task >= 0) {
        ask(m_job.owner, QString::fromLatin1(ServicePath), QString::fromLatin1(ServiceInterface),
            QStringLiteral("KeepAliveTask"), QVariantList() << m_job.task,
            [](const QDBusMessage &) {});
    }
}

void VoiceTranscripts::onTextDecoded(const QString &text, const QString &, int task)
{
    if (!m_busy) {
        return;
    }
    if (m_job.task < 0) {
        m_early.append(qMakePair(task, text));
        return;
    }
    if (task == m_job.task) {
        m_job.pieces.append(text);
    }
}

void VoiceTranscripts::onTranscribeFinished(int task)
{
    if (!m_busy) {
        return;
    }
    if (m_job.task < 0) {
        m_earlyDone.insert(task);
        return;
    }
    if (task == m_job.task) {
        finish();
    }
}

void VoiceTranscripts::onServiceError(int)
{
    // The signal names no task; the service was idle when this one began.
    if (m_busy && !m_job.owner.isEmpty()) {
        fail(tr("The speech service reported an error."));
    }
}

void VoiceTranscripts::finish()
{
    QString joined = m_job.pieces.join(QLatin1Char(' '));
    const QString plain = plainTranscript(joined);
    joined.fill(QChar(0));

    if (m_job.check) {
        setCheck(QStringLiteral("ok"), tr("Everything works: voice messages can be converted."));
    } else if (plain.isEmpty()) {
        m_states.insert(m_job.eventId, QStringLiteral("failed"));
        m_failures.insert(m_job.eventId, tr("No speech was recognised."));
        bump();
    } else {
        m_texts.insert(m_job.eventId, plain);
        m_states.insert(m_job.eventId, QStringLiteral("done"));
        bump();
    }
    // Length only: what was said never goes into a log.
    qInfo("xmatic: voice transcript ready, %d characters", plain.size());
    endJob();
}

void VoiceTranscripts::fail(const QString &reason)
{
    if (!m_busy) {
        return;
    }
    if (m_job.check) {
        setCheck(QStringLiteral("failed"), reason);
    } else {
        markFailed(m_job.eventId, reason);
    }
    qInfo("xmatic: voice transcript failed");
    endJob();
}

void VoiceTranscripts::endJob()
{
    ++m_generation;
    m_keepAlive.stop();
    m_deadline.stop();
    subscribe(false);
    if (m_helper) {
        QProcess *helper = m_helper;
        m_helper = nullptr;
        helper->disconnect(this);
        helper->kill();
        helper->deleteLater();
    }
    // The decoded audio of a message from an encrypted room: gone whatever happened.
    if (!m_job.wavPath.isEmpty()) {
        QFile::remove(m_job.wavPath);
    }
    for (QString &piece : m_job.pieces) {
        piece.fill(QChar(0));
    }
    m_job = Job();
    m_early.clear();
    m_earlyDone.clear();
    m_busy = false;
    QTimer::singleShot(0, this, &VoiceTranscripts::next);
}

void VoiceTranscripts::arm(int milliseconds, const QString &reason)
{
    m_deadlineReason = reason;
    m_deadline.start(milliseconds);
}

void VoiceTranscripts::setCheck(const QString &state, const QString &message)
{
    m_checkState = state;
    m_checkMessage = message;
    emit checkChanged();
}

void VoiceTranscripts::subscribe(bool on)
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    const QString owner = on ? m_job.owner : m_subscribed;
    if (owner.isEmpty() || (on && m_subscribed == owner)) {
        return;
    }
    const QString path = QString::fromLatin1(ServicePath);
    const QString interface = QString::fromLatin1(ServiceInterface);
    if (on) {
        bus.connect(owner, path, interface, QStringLiteral("SttTextDecoded"), this,
                    SLOT(onTextDecoded(QString,QString,int)));
        bus.connect(owner, path, interface, QStringLiteral("SttFileTranscribeFinished"), this,
                    SLOT(onTranscribeFinished(int)));
        bus.connect(owner, path, interface, QStringLiteral("ErrorOccured"), this,
                    SLOT(onServiceError(int)));
        m_subscribed = owner;
    } else {
        bus.disconnect(owner, path, interface, QStringLiteral("SttTextDecoded"), this,
                       SLOT(onTextDecoded(QString,QString,int)));
        bus.disconnect(owner, path, interface, QStringLiteral("SttFileTranscribeFinished"), this,
                       SLOT(onTranscribeFinished(int)));
        bus.disconnect(owner, path, interface, QStringLiteral("ErrorOccured"), this,
                       SLOT(onServiceError(int)));
        m_subscribed.clear();
    }
}

QString VoiceTranscripts::serviceProblem(const QDBusMessage &reply) const
{
    const QString name = reply.errorName();
    if (name == QLatin1String("org.freedesktop.DBus.Error.AccessDenied")) {
        // The sandbox refused, not the service: our own permission file is missing.
        qWarning("xmatic: speech service refused by the sandbox");
        return tr("xmatic may not talk to the speech service. Reinstalling xmatic puts the permission back.");
    }
    if (name == QLatin1String("org.freedesktop.DBus.Error.ServiceUnknown")
            || name == QLatin1String("org.freedesktop.DBus.Error.NameHasNoOwner")
            || name == QLatin1String("org.freedesktop.DBus.Error.Spawn.ExecFailed")) {
        return tr("Speech Note is not installed, or its service does not start.");
    }
    qWarning("xmatic: speech service error %s", qPrintable(name));
    return tr("The speech service does not answer.");
}

void VoiceTranscripts::ask(const QString &destination, const QString &path,
                           const QString &interface, const QString &method,
                           const QVariantList &arguments,
                           std::function<void(const QDBusMessage &)> done)
{
    QDBusMessage message = QDBusMessage::createMethodCall(destination, path, interface, method);
    message.setArguments(arguments);
    auto *watcher = new QDBusPendingCallWatcher(
            QDBusConnection::sessionBus().asyncCall(message, CallTimeoutMs), this);
    const quint64 generation = m_generation;
    connect(watcher, &QDBusPendingCallWatcher::finished, this,
            [this, watcher, generation, done]() {
                watcher->deleteLater();
                if (generation == m_generation && m_busy) {
                    done(watcher->reply());
                }
            });
}
