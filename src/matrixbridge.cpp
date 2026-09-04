#include "matrixbridge.h"

#include "emojistore.h"

#include "appsettings.h"
#include "secretskeeper.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImageReader>

#include "outgoingimage.h"
#include <QJSValue>
#include <QJsonArray>
#include <QJsonValue>
#include <QJsonDocument>
#include <QLoggingCategory>
#include <QMetaObject>
#include <QMimeDatabase>
#include <QTime>
#include <QMimeType>
#include <QSettings>
#include <QStandardPaths>
#include <QStringList>
#include <QUrl>

namespace {
/// What a profile picture may weigh: past any thumbnail, below what one
/// allocation costs. Weighed before the request and after the bytes arrive.
const qint64 MaximumAvatarBytes = 10 * 1024 * 1024;


/// Takes ownership of a string handed out by the core and releases it again.
QString takeCoreString(char *owned)
{
    if (!owned) {
        return QString();
    }
    const QString value = QString::fromUtf8(owned);
    xm_string_free(owned);
    return value;
}

QString jsonToCompactString(const QJsonObject &object)
{
    return QString::fromUtf8(QJsonDocument(object).toJson(QJsonDocument::Compact));
}

} // namespace

MatrixBridge::MatrixBridge(const QString &dataDirectory,
                           const QString &cacheDirectory,
                           const StoreKeyResult &storeKey,
                           AppSettings *settings,
                           QObject *parent)
    : QObject(parent)
    , m_dataDirectory(dataDirectory)
    , m_storeKeyState(storeKey.state)
    , m_storeKeyReason(storeKey.errorMessage)
    // `::` deliberately: unqualified lookup finds the member of the same name,
    // so this read itself instead of the filesystem.
    , m_secretsDaemonPresent(::secretsDaemonPresent())
    , m_cacheDirectory(cacheDirectory)
    , m_settings(settings)
{
    // The read status is fixed when a timeline is built, so the open one is
    // rebuilt here - the switch acts on the room the user came from.
    if (m_settings) {
        connect(m_settings, &AppSettings::showReadStatusChanged, this, [this]() {
            if (!m_openRoomId.isEmpty()) {
                openRoom(m_openRoomId, m_timelineFocus);
            }
        });
    }

    // The call rules travel to the core, where a refused call is dropped
    // before it can ring, and follow every change of the privacy page.
    if (m_settings) {
        connect(m_settings, &AppSettings::callPolicyChanged, this,
                &MatrixBridge::pushCallPolicy);
    }

    // A banner that waited in vain for its text goes out as a count. Otherwise
    // an event the preview has no words for announces nothing at all.
    m_bannerTimeout.setSingleShot(true);
    m_bannerTimeout.setInterval(900);
    connect(&m_bannerTimeout, &QTimer::timeout, this, [this]() {
        const auto pending = m_pendingBanners;
        m_pendingBanners.clear();
        for (auto it = pending.constBegin(); it != pending.constEnd(); ++it) {
            publishBanner(it.key(), it.value(), QJsonObject());
        }
    });

    // A "still finishing" that never ends is a lie of its own. Generous for a
    // slow device against a rate-limiting server, but bounded.
    m_recoverySettleTimeout.setSingleShot(true);
    m_recoverySettleTimeout.setInterval(90000);
    connect(&m_recoverySettleTimeout, &QTimer::timeout, this, [this]() {
        if (m_recoverySettling) {
            m_recoverySettling = false;
            qWarning("xmatic: recovery did not settle within 90 s");
            emit encryptionChanged();
        }
    });

    // Started before anything can be sent: every pending command is stamped
    // against this clock, and reading an unstarted QElapsedTimer is undefined.
    m_uptime.start();

    m_coreVersion = takeCoreString(xm_version());

    QJsonObject config;
    config.insert(QStringLiteral("dataDir"), dataDirectory);
    config.insert(QStringLiteral("cacheDir"), cacheDirectory);
    if (!storeKey.key.isEmpty()) {
        config.insert(QStringLiteral("storeKey"), storeKey.key);
    }
    // Not const: the buffer holds the store key and is wiped once the core
    // has taken its copy. Same rule as the password command's payload.
    QByteArray configJson = jsonToCompactString(config).toUtf8();
    m_core = xm_core_new(configJson.constData());
    configJson.fill('\0');

    if (!m_core) {
        setLastError(tr("The protocol core could not be started."));
        return;
    }

    xm_core_set_callback(m_core, &MatrixBridge::deliver, this);

    // Signing out takes the decrypted media, unless the setting says never.
    // The lists that name people go in every case - they belong to the account.
    connect(this, &MatrixBridge::sessionChanged, this, [this]() {
        if (m_sessionState == QLatin1String("signed-in") && !m_privateListsReadable) {
            // A key that arrived late - the collection was locked at start.
            send(QStringLiteral("private.get"));
            return;
        }
        if (m_sessionState != QLatin1String("none") || !m_settings) {
            return;
        }
        m_privateLists.clear();
        m_privateListsReadable = false;
        emit privateListsChanged();
        if (m_settings->mediaWipe() != QLatin1String("never")) {
            clearMediaCache();
        }
    });

    // Leftovers from a run that was killed rather than closed.
    if (m_settings) {
        const QString when = m_settings->mediaWipe();
        if (when == QLatin1String("exit") || when == QLatin1String("background")) {
            clearMediaCache();
        }
    }

    // The strict stage: gone as soon as the app is not in front any more.
    connect(qGuiApp, &QGuiApplication::applicationStateChanged, this,
            [this](Qt::ApplicationState state) {
                if (state == Qt::ApplicationActive || !m_settings) {
                    return;
                }
                if (m_settings->mediaWipe() == QLatin1String("background")) {
                    clearMediaCache();
                }
            });

    // The encrypted lists, and with them the callers the policy allows.
    send(QStringLiteral("private.get"));

    // Before any sync can deliver a call: the core refuses what the privacy
    // page refuses, and it has to know the rules from the first event on.
    pushCallPolicy();

    // Asked once at start: whether the files are encrypted is a property of the
    // disk, not of a session, and the UI must say so while signed out.
    refreshStorageStatus();

    // Runs only while commands are outstanding, so an idle app does not wake
    // up for this.
    m_stallWatch = new QTimer(this);
    m_stallWatch->setInterval(5000);
    connect(m_stallWatch, &QTimer::timeout, this, &MatrixBridge::checkStalledCommands);

    // Recordings are throwaway files; they live in the cache next to the
    // downloaded attachments.
    m_calls = new CallEngine(this);

    // The engine produces what has to be signalled and is fed what arrives;
    // neither half knows about the other.
    connect(m_calls, &CallEngine::inviteReady, this,
            [this](const QString &room, const QString &call, const QString &party,
                   const QString &sdp) {
                QJsonObject arguments;
                arguments.insert(QStringLiteral("roomId"), room);
                arguments.insert(QStringLiteral("callId"), call);
                arguments.insert(QStringLiteral("partyId"), party);
                arguments.insert(QStringLiteral("sdp"), sdp);
                send(QStringLiteral("call.invite"), arguments);
            });
    connect(m_calls, &CallEngine::answerReady, this,
            [this](const QString &room, const QString &call, const QString &party,
                   const QString &sdp) {
                QJsonObject arguments;
                arguments.insert(QStringLiteral("roomId"), room);
                arguments.insert(QStringLiteral("callId"), call);
                arguments.insert(QStringLiteral("partyId"), party);
                arguments.insert(QStringLiteral("sdp"), sdp);
                send(QStringLiteral("call.answer"), arguments);
            });
    connect(m_calls, &CallEngine::candidatesReady, this,
            [this](const QString &room, const QString &call, const QString &party,
                   const QVariantList &candidates) {
                QJsonObject arguments;
                arguments.insert(QStringLiteral("roomId"), room);
                arguments.insert(QStringLiteral("callId"), call);
                arguments.insert(QStringLiteral("partyId"), party);
                arguments.insert(QStringLiteral("candidates"),
                                 QJsonArray::fromVariantList(candidates));
                send(QStringLiteral("call.candidates"), arguments);
            });
    connect(m_calls, &CallEngine::hangupReady, this,
            [this](const QString &room, const QString &call, const QString &party) {
                QJsonObject arguments;
                arguments.insert(QStringLiteral("roomId"), room);
                arguments.insert(QStringLiteral("callId"), call);
                arguments.insert(QStringLiteral("partyId"), party);
                send(QStringLiteral("call.hangup"), arguments);
            });

    m_voiceDirectory = cacheDirectory + QStringLiteral("/voice");
    m_recorder = new VoiceRecorder(m_voiceDirectory, this);
    connect(m_recorder, &VoiceRecorder::finished, this, [this](const QString &path,
                                                               const QString &mimeType,
                                                               qint64 duration) {
        sendMedia(path, mimeType, QString(), QString(), duration);
    });
    connect(m_recorder, &VoiceRecorder::failed, this, [this](const QString &message) {
        setLastError(message);
    });

    // The UI reads the grouped proxy; the core keeps filling the flat source.
    m_roomsSorted.setSourceModel(&m_rooms);

    connect(&m_rooms, &RoomListModel::unreadTotalsChanged,
            this, &MatrixBridge::unreadTotalsChanged);

    // Diagnostics only, and only the number of rooms — never their names or
    // identifiers.
    connect(&m_rooms, &RoomListModel::countChanged, this, [this]() {
        qInfo("xmatic: room list holds %d rooms", m_rooms.count());
    });
}

MatrixBridge::~MatrixBridge()
{
    if (m_core) {
        // Clears the callback before tearing the runtime down, so no message
        // can arrive while this object is being destroyed.
        xm_core_set_callback(m_core, nullptr, nullptr);
        xm_core_free(m_core);
        m_core = nullptr;
    }
}

void MatrixBridge::deliver(void *userData, const char *json)
{
    if (!userData || !json) {
        return;
    }

    // Called from a core worker thread: copy the message and hand it to the Qt
    // event loop. Nothing else may touch this object from here.
    auto *bridge = static_cast<MatrixBridge *>(userData);
    QMetaObject::invokeMethod(bridge,
                              "handleMessage",
                              Qt::QueuedConnection,
                              Q_ARG(QString, QString::fromUtf8(json)));
}

quint64 MatrixBridge::send(const QString &command, const QJsonObject &arguments, bool wipePayload)
{
    if (!m_core) {
        setLastError(tr("The protocol core is not available."));
        return 0;
    }

    const quint64 id = m_nextId++;
    QJsonObject message = arguments;
    message.insert(QStringLiteral("id"), static_cast<double>(id));
    message.insert(QStringLiteral("cmd"), command);

    PendingCommand entry;
    entry.command = command;
    entry.sentAt = m_uptime.elapsed();
    m_pending.insert(id, entry);
    emit busyChanged();
    updateStallWatch();

    // Named variable, not chained: the pointer belongs to the temporary, whose
    // lifetime past the statement depends on the callee copying it.
    QByteArray payload = jsonToCompactString(message).toUtf8();
    xm_core_send(m_core, payload.constData());
    if (wipePayload) {
        // The buffer held a password and the core has its own copy. The transient
        // QString and QJsonObject residuals: docs/PASSWORD-LOGIN.md.
        payload.fill('\0');
    }
    return id;
}

void MatrixBridge::updateStallWatch()
{
    if (!m_stallWatch) {
        return;
    }
    if (m_pending.isEmpty()) {
        m_stallWatch->stop();
    } else if (!m_stallWatch->isActive()) {
        m_stallWatch->start();
    }
}

void MatrixBridge::checkStalledCommands()
{
    // Past anything a healthy homeserver takes, short of the SDK's retry budget:
    // a rate-limited request is named while it is still being retried.
    static const qint64 threshold = 10000;

    const qint64 now = m_uptime.elapsed();
    for (auto it = m_pending.begin(); it != m_pending.end(); ++it) {
        const qint64 waited = now - it->sentAt;
        if (it->reported || waited < threshold) {
            continue;
        }
        it->reported = true;
        qWarning("xmatic: %s has been waiting %d s for an answer",
                 qPrintable(it->command),
                 static_cast<int>(waited / 1000));
    }

    // A command that never came back used to sit here for good, and `busy` sits
    // on this list - one lost answer froze the sign-in page until a restart.
    static const qint64 giveUp = 120000;
    QStringList abandoned;
    QStringList lostMedia;
    for (auto it = m_pending.begin(); it != m_pending.end();) {
        if (now - it->sentAt < giveUp) {
            ++it;
            continue;
        }
        abandoned.append(it->command);
        // The two states that gate a page are not in this list: a lost login reply
        // froze the sign-in page, a lost pagination killed "load older messages".
        if (it.key() == m_paginateId) {
            m_paginateId = 0;
            emit paginatingChanged();
        }
        if (it->command.startsWith(QLatin1String("login."))) {
            setLoginRunning(false);
        }
        // Everything the answer would have cleared - a lost `media.fetch` kept its
        // key and refused that picture for good. Collected, not emitted: rehash.
        if (m_mediaRequests.contains(it.key())) {
            lostMedia.append(m_mediaRequests.value(it.key()));
        }
        // The recording stays on disk: the reply that would have deleted it looks
        // the command up in `m_pending`, which this loop has just erased.
        forgetRequest(it.key());
        it = m_pending.erase(it);
    }
    // Outside the loop, for the reason given inside it.
    for (const QString &key : lostMedia) {
        emit mediaFailed(key);
    }
    if (!abandoned.isEmpty()) {
        qWarning("xmatic: gave up waiting for %d command(s): %s",
                 int(abandoned.count()), qPrintable(abandoned.join(QStringLiteral(", "))));
        emit busyChanged();
        updateStallWatch();
    }
}

void MatrixBridge::restoreSession()
{
    send(QStringLiteral("session.restore"));
}

/// Sailfish 4's Gecko cannot render the MAS sign-in pages - measured on 4.6,
/// and the reason the device-code grant exists. Unreadable release: assume it works.
bool MatrixBridge::browserLoginReliable() const
{
    QFile release(QStringLiteral("/etc/sailfish-release"));
    if (!release.open(QIODevice::ReadOnly | QIODevice::Text)) {
        return true;
    }
    while (!release.atEnd()) {
        const QByteArray line = release.readLine().trimmed();
        if (!line.startsWith("VERSION_ID=")) {
            continue;
        }
        const QString value = QString::fromLatin1(line.mid(11)).remove(QChar('"'));
        bool ok = false;
        const int major = value.section(QChar('.'), 0, 0).toInt(&ok);
        return !ok || major >= 5;
    }
    return true;
}

/// Only this app's own encryption half counts. On the global `busy` a picture
/// stuck in a retry made the verify buttons dead for as long as it took.
bool MatrixBridge::encryptionBusy() const
{
    for (auto it = m_pending.constBegin(); it != m_pending.constEnd(); ++it) {
        if (it.value().command.startsWith(QLatin1String("encryption."))
            || it.value().command.startsWith(QLatin1String("verification."))) {
            return true;
        }
    }
    return false;
}

/// True while a store must not be created. See the property's documentation.
bool MatrixBridge::storageBlocked() const
{
    if (m_storeKeyState == StoreKeyState::Available) {
        return false;
    }
    // Deliberately not `encryptedDataPresent`: an install predating the store key
    // has plaintext data and no key, and blocking it would strand its keys.
    if (localDataPresent(m_dataDirectory)) {
        return false;
    }
    // No way past this. The "continue without encryption" of 0.26.0 was for the
    // image without the daemon, which the package's `Requires` already keeps out.
    return true;
}

void MatrixBridge::retryStoreKey()
{
    StoreKeyResult result = obtainStoreKey(m_dataDirectory);
    m_storeKeyState = result.state;
    m_storeKeyReason = result.errorMessage;
    m_secretsDaemonPresent = ::secretsDaemonPresent();
    if (result.state == StoreKeyState::Available) {
        QJsonObject arguments;
        arguments.insert(QStringLiteral("storeKey"), result.key);
        result.key.fill(QChar('0'));
        // `session.restore` installs a key handed to it before it looks for a
        // session; on an empty device it answers "none" and the login follows.
        send(QStringLiteral("session.restore"), arguments, true);
    }
    emit storageChanged();
    emit storeKeyChecked(result.state == StoreKeyState::Available);
    refreshStorageStatus();
}

void MatrixBridge::retryUnlock()
{
    setLastError(QString());
    StoreKeyResult result = obtainStoreKey(m_dataDirectory);
    m_storeKeyState = result.state;
    m_storeKeyReason = result.errorMessage;
    m_secretsDaemonPresent = ::secretsDaemonPresent();
    emit storageChanged();
    QJsonObject arguments;
    if (!result.key.isEmpty()) {
        arguments.insert(QStringLiteral("storeKey"), result.key);
        // The original, not a copy of it: `fill()` resizes, which detaches a
        // shared QString - the copy would be wiped and the key would live on.
        result.key.fill(QChar('0'));
    }
    // The payload carries the key, wiped like a password. Without one the plain
    // restore reports "locked" again and the page keeps its retry.
    send(QStringLiteral("session.restore"), arguments, !arguments.isEmpty());
}

void MatrixBridge::startLogin(const QString &homeserver)
{
    const QString trimmed = homeserver.trimmed();
    if (trimmed.isEmpty()) {
        setLastError(tr("Enter a homeserver first."));
        return;
    }

    setLastError(QString());
    setLoginRunning(true);

    QJsonObject arguments;
    arguments.insert(QStringLiteral("homeserver"), trimmed);
    send(QStringLiteral("login.start"), arguments);
}

void MatrixBridge::startPasswordLogin(const QString &homeserver,
                                      const QString &user,
                                      const QString &password)
{
    const QString trimmedServer = homeserver.trimmed();
    const QString trimmedUser = user.trimmed();
    // The password is not trimmed - whitespace in it is the user's business -
    // and not validated, logged or kept: one command, then the buffer is wiped.
    if (trimmedServer.isEmpty() || trimmedUser.isEmpty() || password.isEmpty()) {
        setLastError(tr("Enter username and password first."));
        return;
    }

    setLastError(QString());
    setLoginRunning(true);

    QJsonObject arguments;
    arguments.insert(QStringLiteral("homeserver"), trimmedServer);
    arguments.insert(QStringLiteral("user"), trimmedUser);
    arguments.insert(QStringLiteral("password"), password);
    send(QStringLiteral("login.password"), arguments, true);
}

void MatrixBridge::startDeviceCodeLogin(const QString &homeserver)
{
    const QString trimmed = homeserver.trimmed();
    if (trimmed.isEmpty()) {
        setLastError(tr("Enter a homeserver first."));
        return;
    }

    setLastError(QString());
    setLoginRunning(true);

    QJsonObject arguments;
    arguments.insert(QStringLiteral("homeserver"), trimmed);
    send(QStringLiteral("login.deviceCode"), arguments);
}

void MatrixBridge::requestRegistrationUrl(const QString &homeserver)
{
    const QString trimmed = homeserver.trimmed();
    if (trimmed.isEmpty()) {
        setLastError(tr("Enter a homeserver first."));
        return;
    }
    setLastError(QString());

    QJsonObject arguments;
    arguments.insert(QStringLiteral("homeserver"), trimmed);
    send(QStringLiteral("login.registrationUrl"), arguments);
}

void MatrixBridge::abortLogin()
{
    setLoginRunning(false);
    send(QStringLiteral("login.abort"));
}

void MatrixBridge::logout()
{
    // Nothing of what was typed and never sent outlives the account it was
    // meant for.
    m_drafts.clear();
    send(QStringLiteral("logout"));
}

void MatrixBridge::setDraft(const QString &roomId, const QString &text)
{
    if (roomId.isEmpty()) {
        return;
    }
    if (text.isEmpty()) {
        m_drafts.remove(roomId);
        return;
    }
    m_drafts.insert(roomId, text);
}

QString MatrixBridge::draft(const QString &roomId) const
{
    return m_drafts.value(roomId);
}

void MatrixBridge::startRoomList()
{
    send(QStringLiteral("roomlist.start"));
}

void MatrixBridge::stopRoomList()
{
    send(QStringLiteral("roomlist.stop"));
}

void MatrixBridge::setRoomFilter(const QString &pattern)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("pattern"), pattern);
    send(QStringLiteral("roomlist.filter"), arguments);
}

void MatrixBridge::loadMoreRooms()
{
    send(QStringLiteral("roomlist.more"));
}

void MatrixBridge::startSpaces()
{
    send(QStringLiteral("spaces.start"));
}

void MatrixBridge::stopSpaces()
{
    send(QStringLiteral("spaces.stop"));
}

void MatrixBridge::openSpace(const QString &roomId)
{
    if (roomId.isEmpty()) {
        qWarning("xmatic: open space requested without an id");
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("space.open"), arguments);
}

void MatrixBridge::closeSpace()
{
    send(QStringLiteral("space.close"));
}

void MatrixBridge::createSpace(const QString &name)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("name"), name);
    send(QStringLiteral("space.create"), arguments);
}

void MatrixBridge::leaveSpace(const QString &roomId)
{
    if (roomId.isEmpty()) {
        qWarning("xmatic: leave space requested without an id");
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("space.leave"), arguments);
}

void MatrixBridge::moveRoomToSpace(const QString &fromSpaceId,
                                   const QString &toSpaceId,
                                   const QString &roomId)
{
    if (fromSpaceId.isEmpty() || toSpaceId.isEmpty() || roomId.isEmpty()) {
        qWarning("xmatic: move-to-space requested without ids");
        return;
    }
    // Added before removed on purpose: if one of the two writes fails, the
    // room is in both spaces rather than in none.
    addRoomToSpace(toSpaceId, roomId);
    removeRoomFromSpace(fromSpaceId, roomId);
}

void MatrixBridge::addRoomToSpace(const QString &spaceId, const QString &roomId)
{
    if (spaceId.isEmpty() || roomId.isEmpty()) {
        qWarning("xmatic: add-to-space requested without ids");
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("spaceId"), spaceId);
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("space.addChild"), arguments);
}

void MatrixBridge::removeRoomFromSpace(const QString &spaceId, const QString &roomId)
{
    if (spaceId.isEmpty() || roomId.isEmpty()) {
        qWarning("xmatic: remove-from-space requested without ids");
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("spaceId"), spaceId);
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("space.removeChild"), arguments);
}

void MatrixBridge::openRoom(const QString &roomId, const QString &focus)
{
    if (roomId.isEmpty()) {
        qWarning("xmatic: open room requested without an id");
        return;
    }

    if (roomId == m_openRoomId && focus.isEmpty() && m_timelineFocus.isEmpty()) {
        // Already subscribed, rows kept. Asked anyway and without clearing: the core
        // answers `rebuilt: false` with where reading stopped, which a fresh page needs.
        qInfo("xmatic: room already open, %d rows kept", m_timeline.count());
        QJsonObject kept;
        kept.insert(QStringLiteral("roomId"), roomId);
        kept.insert(QStringLiteral("receipts"),
                    m_settings && m_settings->showReadStatus());
        // *Not* bumped here: the same view carries on and the core keeps the
        // stream it has. Raising it silenced every diff until a restart.
        kept.insert(QStringLiteral("token"), QString::number(m_timelineGeneration));
        send(QStringLiteral("timeline.open"), kept);
        return;
    }

    qInfo("xmatic: opening a room%s", focus.isEmpty() ? "" : " (focused)");
    closeThread();
    setTimelineReady(false);

    // The model is emptied right away so the previous room's messages never
    // flash up under the new room's header.
    m_timeline.clear();
    if (m_openRoomId != roomId && !m_pinnedEventIds.isEmpty()) {
        m_pinnedEventIds.clear();
        m_pinnedPreview.clear();
        emit pinnedChanged();
    }
    if (m_openRoomId != roomId && !m_successorRoomId.isEmpty()) {
        // The core answers for the new room within the same open, but the
        // banner must not stand over the wrong conversation until it does.
        m_successorRoomId.clear();
        m_replacementReason.clear();
        m_replacementJoined = false;
        emit tombstoneChanged();
    }
    if (m_openRoomId != roomId) {
        // Memory hygiene, not correctness: the core prefixes attachment keys per
        // room, but nothing else would ever drop them over a long session.
        m_media.clear();
    }
    m_openRoomId = roomId;
    m_timelineFocus = focus;
    setTimelineAtStart(false);
    emit openRoomChanged();

    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    if (!focus.isEmpty()) {
        arguments.insert(QStringLiteral("focus"), focus);
    }
    arguments.insert(QStringLiteral("receipts"),
                     m_settings && m_settings->showReadStatus());
    arguments.insert(QStringLiteral("token"), QString::number(++m_timelineGeneration));
    m_openTimelineId = send(QStringLiteral("timeline.open"), arguments);

    // Pull this room's keys out of the backup too: messages older than this
    // device can only be read that way, and the timeline retries on arrival.
    if (m_encryptionStatus.value(QStringLiteral("backupEnabled")).toBool()) {
        fetchRoomKeys(roomId);
    }
}

void MatrixBridge::closeRoom()
{
    if (m_openRoomId.isEmpty()) {
        return;
    }
    const QString closing = m_openRoomId;
    m_openRoomId.clear();
    m_timelineFocus.clear();
    if (!m_pinnedEventIds.isEmpty()) {
        m_pinnedEventIds.clear();
        m_pinnedPreview.clear();
        emit pinnedChanged();
    }
    setTimelineReady(false);
    emit openRoomChanged();
    m_timeline.clear();
    // The core closes the thread with the room; only the model remains.
    m_openThreadRoot.clear();
    m_threadTimeline.clear();
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), closing);
    send(QStringLiteral("timeline.close"), arguments);
}

void MatrixBridge::loadOlder()
{
    // One at a time. A second request in flight makes the first answer
    // impossible to attribute, which is what the automatic fill needs.
    if (m_paginateId != 0) {
        return;
    }

    qInfo("xmatic: loading older messages, %d rows so far", m_timeline.count());
    const quint64 id = send(QStringLiteral("timeline.paginate"));
    if (id == 0) {
        return;
    }
    m_paginateId = id;
    emit paginatingChanged();
}

void MatrixBridge::openThread(const QString &roomId, const QString &rootEventId)
{
    if (roomId.isEmpty() || rootEventId.isEmpty()) {
        qWarning("xmatic: open thread requested without ids");
        return;
    }
    setLastError(QString());
    m_threadTimeline.clear();
    m_openThreadRoot = rootEventId;
    // Reopening the same thread has to be told from the one before: the aborted
    // task's diffs can still be in the Qt queue. The root alone does not.
    ++m_threadGeneration;
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("rootEventId"), rootEventId);
    arguments.insert(QStringLiteral("token"), QString::number(m_threadGeneration));
    send(QStringLiteral("thread.open"), arguments);
}

void MatrixBridge::closeThread()
{
    if (m_openThreadRoot.isEmpty()) {
        return;
    }
    // Named, because commands are independent tasks: a close and an open issued
    // back to back can arrive in either order.
    QJsonObject arguments;
    arguments.insert(QStringLiteral("rootEventId"), m_openThreadRoot);
    m_openThreadRoot.clear();
    m_threadTimeline.clear();
    send(QStringLiteral("thread.close"), arguments);
}

void MatrixBridge::sendThreadMessage(const QString &body)
{
    if (body.trimmed().isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("body"), body);
    send(QStringLiteral("thread.send"), arguments);
}

void MatrixBridge::threadLoadOlder()
{
    send(QStringLiteral("thread.paginate"));
}

void MatrixBridge::pinMessage(const QString &eventId, bool pin)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("pin"), pin);
    send(QStringLiteral("timeline.pin"), arguments);
}

void MatrixBridge::followSuccessor(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.followSuccessor"), arguments);
}

void MatrixBridge::loadRoomInfo(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.info"), arguments);
}

void MatrixBridge::setRoomMuted(const QString &roomId, bool muted)
{
    // The quick toggle in the room list: muting sets the explicit rule,
    // unmuting returns the room to the account default.
    setRoomNotifyMode(roomId, muted ? QStringLiteral("mute") : QStringLiteral("default"));
}

void MatrixBridge::setRoomNotifyMode(const QString &roomId, const QString &mode)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("mode"), mode);
    send(QStringLiteral("room.setNotifyMode"), arguments);
}

void MatrixBridge::setRoomFavourite(const QString &roomId, bool favourite)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("favourite"), favourite);
    send(QStringLiteral("room.setFavourite"), arguments);
}

void MatrixBridge::setRoomLowPriority(const QString &roomId, bool lowPriority)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("lowPriority"), lowPriority);
    send(QStringLiteral("room.setLowPriority"), arguments);
}

QString MatrixBridge::emojiDirectory() const
{
    return m_dataDirectory + QStringLiteral("/emoji");
}

QString MatrixBridge::emojiSource(const QString &key) const
{
    if (key.isEmpty()) {
        return QString();
    }
    const auto cached = m_emojiSources.constFind(key);
    if (cached != m_emojiSources.constEnd()) {
        return *cached;
    }

    // Name built from the reaction the way emoji sets name their files: code
    // points in hex, joined by a dash, variation selector dropped unless joined.
    const QVector<uint> points = key.toUcs4();
    const bool joined = points.contains(0x200D);
    QStringList parts;
    QStringList bare;
    for (uint point : points) {
        const QString hex = QString::number(point, 16);
        if (point != 0xFE0F) {
            bare.append(hex);
        }
        if (point == 0xFE0F && !joined) {
            continue;
        }
        parts.append(hex);
    }

    // Second name: the first without any variation selector. Sets disagree on
    // some sequences, and a different spelling should not read as "no picture".
    QStringList names;
    names.append(parts.join(QLatin1Char('-')));
    if (bare != parts) {
        names.append(bare.join(QLatin1Char('-')));
    }

    QString found;
    // A set read in through the app is served by the provider against its
    // checksums; a hand-copied one is opened as a plain file, unchecked.
    const bool checked = m_emojiStore && m_emojiStore->verified();
    for (const QString &name : names) {
        if (name.isEmpty()) {
            continue;
        }
        if (checked) {
            if (m_emojiStore->knows(name + QStringLiteral(".png"))) {
                found = QStringLiteral("image://xmatic-emoji/") + name;
                break;
            }
            continue;
        }
        const QString base = emojiDirectory() + QLatin1Char('/') + name;
        // PNG only: a hand-copied set has no checksums, and handing an SVG to Qt
        // 5.6's parser once per row per redraw is what this must not do.
        for (const QString &extension : { QStringLiteral(".png") }) {
            const QFileInfo file(base + extension);
            if (file.exists() && file.isFile() && file.size() <= 64 * 1024) {
                found = QUrl::fromLocalFile(file.absoluteFilePath()).toString();
                break;
            }
        }
        if (!found.isEmpty()) {
            break;
        }
    }


    m_emojiSources.insert(key, found);
    return found;
}

void MatrixBridge::clearLastError()
{
    setLastError(QString());
}

void MatrixBridge::forgetRequest(quint64 id)
{
    // The registers that remember what a command was about. Emptied on the reply,
    // on the error, and - a dropped answer being neither - by the watchdog.
    m_mediaRequests.remove(id);
    m_hierarchyRequests.remove(id);
    m_removeRequests.remove(id);
    m_readerRequests.remove(id);
    m_reactorRequests.remove(id);
    m_voiceSends.remove(id);
}

void MatrixBridge::markRoomRead(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    // The same question the room's own mark-read asks. Without it the core sent
    // the receipt regardless, so the privacy switch held in one path only.
    arguments.insert(QStringLiteral("receipt"),
                     !m_settings || m_settings->sendReadReceipts());
    send(QStringLiteral("room.markRead"), arguments);
}

void MatrixBridge::resolveRoom(const QString &address)
{
    if (address.trimmed().isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("address"), address.trimmed());
    send(QStringLiteral("room.resolve"), arguments);
}

void MatrixBridge::checkRecipients(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.checkRecipients"), arguments);
}

void MatrixBridge::enableEncryption(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.enableEncryption"), arguments);
}

void MatrixBridge::fetchProfile()
{
    send(QStringLiteral("account.get"));
}

void MatrixBridge::setDisplayName(const QString &name)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("name"), name);
    send(QStringLiteral("account.setDisplayName"), arguments);
}

void MatrixBridge::setAvatarFile(const QString &path)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("path"), path);
    send(QStringLiteral("account.setAvatar"), arguments);
}

void MatrixBridge::searchDirectory(const QString &pattern, const QString &server)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("pattern"), pattern);
    if (!server.trimmed().isEmpty()) {
        arguments.insert(QStringLiteral("server"), server.trimmed());
    }
    send(QStringLiteral("directory.search"), arguments);
}

// A server name as the user may type it: with a scheme, a trailing slash or
// stray spaces. Reduced to the bare name the directory API expects.
void MatrixBridge::directoryLoadMore()
{
    send(QStringLiteral("directory.loadMore"));
}

void MatrixBridge::stopDirectory()
{
    send(QStringLiteral("directory.stop"));
    m_directory.clear();
    if (!m_directoryAtEnd) {
        m_directoryAtEnd = true;
        emit directoryStateChanged();
    }
}

void MatrixBridge::indexRoom(const QString &roomId)
{
    if (m_indexRequest != 0) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    m_indexRequest = send(QStringLiteral("search.index"), arguments);
    emit indexingChanged();
}

void MatrixBridge::searchRoom(const QString &roomId, const QString &query)
{
    setLastError(QString());
    m_searchRoomId = roomId;
    m_searchQuery = query.trimmed();
    m_searchOffset = 0;
    m_searchResults.clear();
    if (m_searchHasMore) {
        m_searchHasMore = false;
        emit searchHasMoreChanged();
    }
    // An emptied box is not a search that found nothing, and it keeps the core
    // out of a query the user is in the middle of deleting.
    if (m_searchQuery.isEmpty()) {
        if (m_searchRequest != 0) {
            m_searchRequest = 0;
            emit searchingChanged();
        }
        return;
    }
    sendSearchPage();
}

void MatrixBridge::searchMore()
{
    if (m_searchRequest != 0 || !m_searchHasMore || m_searchQuery.isEmpty()) {
        return;
    }
    sendSearchPage();
}

void MatrixBridge::clearSearch()
{
    m_searchResults.clear();
    m_searchRoomId.clear();
    m_searchQuery.clear();
    m_searchOffset = 0;
    if (m_searchHasMore) {
        m_searchHasMore = false;
        emit searchHasMoreChanged();
    }
    if (m_searchRequest != 0) {
        m_searchRequest = 0;
        emit searchingChanged();
    }
}

/// Asks for the page at the current offset and remembers which request it is.
/// Only that answer may touch the model.
void MatrixBridge::sendSearchPage()
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), m_searchRoomId);
    arguments.insert(QStringLiteral("query"), m_searchQuery);
    arguments.insert(QStringLiteral("offset"), m_searchOffset);
    arguments.insert(QStringLiteral("limit"), SearchPageSize);
    m_searchRequest = send(QStringLiteral("search.room"), arguments);
    emit searchingChanged();
}

void MatrixBridge::loadMembers(const QString &roomId)
{
    // A stale error from elsewhere would show up on the freshly opened page.
    setLastError(QString());
    m_members.clear();
    m_membersRoomId = roomId;
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("members.load"), arguments);
}

void MatrixBridge::removeMember(const QString &roomId, const QString &userId)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId);
    const quint64 id = send(QStringLiteral("member.remove"), arguments);
    m_removeRequests.insert(id, userId);
}

void MatrixBridge::loadMemberProfile(const QString &roomId, const QString &userId)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId);
    send(QStringLiteral("member.profile"), arguments);
}

void MatrixBridge::banMember(const QString &roomId, const QString &userId)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId);
    send(QStringLiteral("member.ban"), arguments);
}

void MatrixBridge::unbanMember(const QString &roomId, const QString &userId)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId);
    send(QStringLiteral("member.unban"), arguments);
}

void MatrixBridge::setMemberPower(const QString &roomId, const QString &userId, int power)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId);
    arguments.insert(QStringLiteral("power"), power);
    send(QStringLiteral("member.setPower"), arguments);
}

void MatrixBridge::setMemberIgnored(const QString &userId, bool ignored)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("userId"), userId);
    arguments.insert(QStringLiteral("ignored"), ignored);
    send(QStringLiteral("member.setIgnored"), arguments);
}

void MatrixBridge::withdrawMemberVerification(const QString &userId)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("userId"), userId);
    send(QStringLiteral("member.withdrawVerification"), arguments);
}

void MatrixBridge::loadIgnoredUsers()
{
    setLastError(QString());
    send(QStringLiteral("account.ignoredUsers"));
}

void MatrixBridge::resetRoomKeys(const QString &roomId)
{
    if (roomId.isEmpty()) {
        return;
    }
    setLastError(QString());
    qInfo("xmatic: resetting the room key");
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.resetKeys"), arguments);
}

void MatrixBridge::fetchSpaceHierarchy(const QString &spaceId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), spaceId);
    const quint64 id = send(QStringLiteral("space.hierarchy"), arguments);
    m_hierarchyRequests.insert(id, spaceId);
}

void MatrixBridge::sendMessage(const QString &body)
{
    // Length only, never content: this exists to tell "the UI never asked"
    // apart from "the server refused".
    qInfo("xmatic: send requested, %d characters", body.trimmed().length());

    if (body.trimmed().isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("body"), body);
    send(QStringLiteral("timeline.send"), arguments);
}

void MatrixBridge::markRead()
{
    // The privacy switch governs the receipt others see, not the fully-read
    // marker: holding that back hid nothing and cost the position the room opens at.
    QJsonObject arguments;
    arguments.insert(QStringLiteral("receipt"),
                     !m_settings || m_settings->sendReadReceipts());
    send(QStringLiteral("timeline.markRead"), arguments);
}

bool MatrixBridge::shareableFile(const QString &path) const
{
    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }
    const QFileInfo info(local);
    const QString resolved = info.canonicalFilePath().isEmpty()
            ? info.absoluteFilePath()
            : info.canonicalFilePath();
    if (resolved.isEmpty()) {
        return false;
    }

    // Canonical on both sides, so a link cannot point in from outside.
    const QStringList mine {
        m_dataDirectory,
        m_cacheDirectory,
        QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation),
    };
    for (const QString &directory : mine) {
        if (directory.isEmpty()) {
            continue;
        }
        const QFileInfo own(directory);
        const QString ownPath = own.canonicalFilePath().isEmpty()
                ? own.absoluteFilePath()
                : own.canonicalFilePath();
        if (!ownPath.isEmpty()
            && (resolved == ownPath || resolved.startsWith(ownPath + QLatin1Char('/')))) {
            qWarning("xmatic: a share pointed at this app's own data and was refused");
            return false;
        }
    }
    return true;
}

void MatrixBridge::clearMediaCache()
{
    if (m_cacheDirectory.isEmpty()) {
        return;
    }
    QDir(m_cacheDirectory + QStringLiteral("/media")).removeRecursively();
    QDir(m_cacheDirectory + QStringLiteral("/voice")).removeRecursively();
    m_media.clear();
}

bool MatrixBridge::callerAllowed(const QString &userId) const
{
    return m_privateLists.value(QStringLiteral("callers")).contains(userId.trimmed());
}

void MatrixBridge::allowCaller(const QString &userId)
{
    const QString id = userId.trimmed();
    // A Matrix address and nothing that could be a pattern: the list is
    // compared literally.
    if (!id.startsWith(QLatin1Char('@')) || !id.contains(QLatin1Char(':'))) {
        return;
    }
    QStringList callers = m_privateLists.value(QStringLiteral("callers"));
    if (callers.contains(id)) {
        return;
    }
    callers.append(id);
    sendPrivateList(QStringLiteral("callers"), callers);
}

void MatrixBridge::forbidCaller(const QString &userId)
{
    QStringList callers = m_privateLists.value(QStringLiteral("callers"));
    if (callers.removeAll(userId.trimmed()) == 0) {
        return;
    }
    sendPrivateList(QStringLiteral("callers"), callers);
}

bool MatrixBridge::recipientTrusted(const QString &userId) const
{
    return m_privateLists.value(QStringLiteral("trusted")).contains(userId);
}

void MatrixBridge::trustRecipient(const QString &userId)
{
    QStringList trusted = m_privateLists.value(QStringLiteral("trusted"));
    if (userId.isEmpty() || trusted.contains(userId)) {
        return;
    }
    trusted.append(userId);
    sendPrivateList(QStringLiteral("trusted"), trusted);
}

int MatrixBridge::resetRecipientWarnings()
{
    const int count = m_privateLists.value(QStringLiteral("trusted")).count();
    if (count > 0) {
        sendPrivateList(QStringLiteral("trusted"), QStringList());
    }
    return count;
}

void MatrixBridge::sendPrivateList(const QString &list, const QStringList &values)
{
    if (!m_privateListsReadable) {
        // Locked or damaged: writing now would replace what could not be read.
        setLastError(tr("The stored lists cannot be read right now."));
        return;
    }

    // Local first, so the UI does not wait for a round trip; the answer
    // carries the stored truth and overwrites it.
    m_privateLists.insert(list, values);
    sendPrivateLists();
}

/// Writes every list in one command. The core replaces the file with what
/// arrives, so there is no read-modify-write to race with.
void MatrixBridge::sendPrivateLists()
{
    if (!m_privateListsReadable) {
        setLastError(tr("The stored lists cannot be read right now."));
        return;
    }

    emit privateListsChanged();
    pushCallPolicy();

    QJsonObject lists;
    for (auto it = m_privateLists.constBegin(); it != m_privateLists.constEnd(); ++it) {
        lists.insert(it.key(), QJsonArray::fromStringList(it.value()));
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("lists"), lists);
    send(QStringLiteral("private.set"), arguments);
}

void MatrixBridge::pushCallPolicy()
{
    if (!m_settings) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("policy"), m_settings->callPolicy());
    arguments.insert(QStringLiteral("groups"), m_settings->groupCalls());
    arguments.insert(QStringLiteral("video"), m_settings->videoCalls());
    arguments.insert(QStringLiteral("flood"), m_settings->callFlood());
    arguments.insert(QStringLiteral("allowed"),
                     QJsonArray::fromStringList(allowedCallers()));
    send(QStringLiteral("calls.setPolicy"), arguments);
}

void MatrixBridge::setEmojiStore(EmojiStore *store)
{
    m_emojiStore = store;
    if (!m_emojiStore) {
        return;
    }
    // Reading a set in, or dropping it, changes every cached lookup. Both signals
    // may come from the image thread, hence queued.
    const auto invalidate = [this]() {
        m_emojiSources.clear();
        ++m_emojiRevision;
        emit emojiRevisionChanged();
    };
    connect(m_emojiStore, &EmojiStore::contentChanged, this, invalidate, Qt::QueuedConnection);
    connect(m_emojiStore, &EmojiStore::tamperedChanged, this, invalidate, Qt::QueuedConnection);
}

void MatrixBridge::loadRoomLink(const QString &roomId)
{
    if (roomId.isEmpty()) {
        return;
    }
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.permalink"), arguments);
}

void MatrixBridge::loadReaders(const QString &eventId)
{
    if (eventId.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    const quint64 id = send(QStringLiteral("timeline.readers"), arguments);
    m_readerRequests.insert(id, eventId);
}

void MatrixBridge::loadReactors(const QString &eventId, const QString &key)
{
    if (eventId.isEmpty() || key.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("key"), key);
    m_reactorRequests.insert(send(QStringLiteral("timeline.reactors"), arguments));
}

void MatrixBridge::joinRoom(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.join"), arguments);
}

void MatrixBridge::requestVerification(const QString &userId)
{
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("userId"), userId.trimmed());
    send(QStringLiteral("verification.request"), arguments);
}

void MatrixBridge::joinRoomByAlias(const QString &alias)
{
    if (alias.trimmed().isEmpty()) {
        return;
    }
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("alias"), alias.trimmed());
    send(QStringLiteral("room.joinByAlias"), arguments);
}

void MatrixBridge::createRoom(const QVariantMap &options)
{
    const QString name = options.value(QStringLiteral("name")).toString().trimmed();
    if (name.isEmpty()) {
        return;
    }
    setLastError(QString());

    QJsonObject arguments;
    arguments.insert(QStringLiteral("name"), name);
    arguments.insert(QStringLiteral("topic"),
                     options.value(QStringLiteral("topic")).toString().trimmed());
    arguments.insert(QStringLiteral("alias"),
                     options.value(QStringLiteral("alias")).toString().trimmed());
    arguments.insert(QStringLiteral("encrypted"),
                     options.value(QStringLiteral("encrypted")).toBool());
    arguments.insert(QStringLiteral("public"),
                     options.value(QStringLiteral("public")).toBool());
    arguments.insert(QStringLiteral("historyVisibility"),
                     options.value(QStringLiteral("historyVisibility")).toString());
    arguments.insert(QStringLiteral("readOnly"),
                     options.value(QStringLiteral("readOnly")).toBool());
    arguments.insert(QStringLiteral("equalPower"),
                     options.value(QStringLiteral("equalPower")).toBool());
    // Absent means yes for this one, so the default cannot be read off a
    // missing key the way the flags above are.
    arguments.insert(QStringLiteral("federate"),
                     options.contains(QStringLiteral("federate"))
                             ? options.value(QStringLiteral("federate")).toBool()
                             : true);

    QJsonArray invited;
    const QStringList entries = options.value(QStringLiteral("invite")).toStringList();
    for (const QString &entry : entries) {
        const QString trimmed = entry.trimmed();
        if (!trimmed.isEmpty()) {
            invited.append(trimmed);
        }
    }
    arguments.insert(QStringLiteral("invite"), invited);

    send(QStringLiteral("room.create"), arguments);
}

void MatrixBridge::leaveRoom(const QString &roomId)
{
    if (roomId.isEmpty()) {
        qWarning("xmatic: leave room requested without an id");
        return;
    }
    setLastError(QString());
    // Leaving the room that is open would leave the timeline subscribed to a
    // room the account is no longer in, so it is closed first.
    if (roomId == m_openRoomId) {
        closeRoom();
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.leave"), arguments);
}

void MatrixBridge::inviteToRoom(const QString &roomId, const QString &userId)
{
    if (roomId.isEmpty() || userId.trimmed().isEmpty()) {
        return;
    }
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId.trimmed());
    send(QStringLiteral("room.invite"), arguments);
}

void MatrixBridge::startDirectChat(const QString &userId)
{
    if (userId.trimmed().isEmpty()) {
        return;
    }
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("userId"), userId.trimmed());
    send(QStringLiteral("room.directChat"), arguments);
}

void MatrixBridge::acceptVerification()
{
    send(QStringLiteral("verification.accept"));
}

void MatrixBridge::confirmVerification()
{
    send(QStringLiteral("verification.confirm"));
}

void MatrixBridge::cancelVerification()
{
    send(QStringLiteral("verification.cancel"));
}

void MatrixBridge::reportVerificationMismatch()
{
    send(QStringLiteral("verification.mismatch"));
}

void MatrixBridge::clearVerification()
{
    m_verificationState = QStringLiteral("none");
    m_verificationUser.clear();
    m_verificationIsSelf = false;
    m_verificationWeStarted = false;
    m_verificationEmoji.clear();
    emit verificationChanged();
}

void MatrixBridge::refreshEncryptionStatus()
{
    send(QStringLiteral("encryption.status"));
}

void MatrixBridge::refreshStorageStatus()
{
    send(QStringLiteral("storage.status"));
}

void MatrixBridge::refreshPushStatus()
{
    send(QStringLiteral("push.status"));
}

void MatrixBridge::enablePush(const QString &gateway)
{
    if (gateway.trimmed().isEmpty()) {
        setLastError(tr("Enter a push gateway first."));
        return;
    }
    // Kept for the second half: the distributor answers in its own time, and by
    // then the field the user typed into may be gone with its page.
    m_pushGateway = gateway.trimmed();
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("gateway"), m_pushGateway);
    send(QStringLiteral("push.enable"), arguments);
}

void MatrixBridge::disablePush()
{
    QJsonObject arguments;
    // Sent along so the pusher can be deleted while the endpoint is still
    // known: giving the registration back drops it.
    arguments.insert(QStringLiteral("endpoint"), m_pushEndpoint);
    send(QStringLiteral("push.disable"), arguments);
    m_pushEndpoint.clear();
    m_pushP256dh.clear();
    m_pushAuth.clear();
    emit pushStatusChanged();
}

void MatrixBridge::recoverKeys(const QString &key)
{
    if (key.trimmed().isEmpty()) {
        setLastError(tr("Enter your recovery key first."));
        return;
    }
    setLastError(QString());

    QJsonObject arguments;
    arguments.insert(QStringLiteral("key"), key.trimmed());
    // Like the login password: the recovery key unlocks the whole backup, so the
    // payload must not stay on the heap. Residuals: docs/PASSWORD-LOGIN.md.
    send(QStringLiteral("encryption.recover"), arguments, true);
}

void MatrixBridge::enableKeyBackup()
{
    setLastError(QString());
    send(QStringLiteral("encryption.enableBackup"));
}

void MatrixBridge::fetchRoomKeys(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("encryption.fetchKeys"), arguments);
}

void MatrixBridge::replyToMessage(const QString &eventId, const QString &body)
{
    if (eventId.isEmpty() || body.trimmed().isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("body"), body);
    send(QStringLiteral("timeline.reply"), arguments);
}

void MatrixBridge::editMessage(const QString &eventId, const QString &body)
{
    if (eventId.isEmpty() || body.trimmed().isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("body"), body);
    send(QStringLiteral("timeline.edit"), arguments);
}

void MatrixBridge::toggleReaction(const QString &eventId, const QString &key)
{
    if (eventId.isEmpty() || key.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("key"), key);
    send(QStringLiteral("timeline.react"), arguments);
}

void MatrixBridge::retryMessage(const QString &txnId)
{
    if (txnId.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("txnId"), txnId);
    send(QStringLiteral("timeline.retry"), arguments);
}

void MatrixBridge::deleteMessage(const QString &eventId, const QString &txnId)
{
    if (eventId.isEmpty() && txnId.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("txnId"), txnId);
    send(QStringLiteral("timeline.redact"), arguments);
}

/// Measurements from the header, `QImageReader::size()`, no decode. Without
/// them the SDK writes an empty `info` and every picture goes out undescribed.
static void insertDimensions(QJsonObject &arguments, const QString &localPath,
                             const QString &mimeType)
{
    if (!mimeType.startsWith(QLatin1String("image/"))) {
        return;
    }

    QImageReader reader(localPath);
    const QSize size = reader.size();
    if (!size.isValid() || size.isEmpty()) {
        return;
    }

    arguments.insert(QStringLiteral("width"), double(size.width()));
    arguments.insert(QStringLiteral("height"), double(size.height()));
}

void MatrixBridge::sendMedia(const QString &path, const QString &mimeType,
                             const QString &caption, const QString &replyTo,
                             qint64 voiceDuration, bool original)
{
    // The type, never the name: a file name carries whatever the user called
    // it, and the log is not the place for that.
    qInfo("xmatic: sending an attachment of type %s", qPrintable(mimeType));

    QJsonObject arguments;
    // The picker hands out URLs; the core wants a plain path.
    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }
    // A copy in the cache, or the file itself. Measured afterwards, never
    // before: the re-encode is what decides the dimensions that go out.
    const OutgoingImage outgoing = prepareOutgoingImage(local, mimeType, original);
    arguments.insert(QStringLiteral("path"), outgoing.path);
    arguments.insert(QStringLiteral("mimeType"),
                     outgoing.mimeType.isEmpty() ? QStringLiteral("application/octet-stream")
                                                 : outgoing.mimeType);
    arguments.insert(QStringLiteral("caption"), caption);
    arguments.insert(QStringLiteral("replyTo"), replyTo);
    insertDimensions(arguments, outgoing.path, outgoing.mimeType);
    if (voiceDuration > 0) {
        arguments.insert(QStringLiteral("voice"), true);
        arguments.insert(QStringLiteral("duration"), double(voiceDuration));
    }
    const quint64 id = send(QStringLiteral("timeline.sendMedia"), arguments);
    // A recording of one's own is not a document the user keeps: it goes as
    // soon as it is out.
    if (!m_voiceDirectory.isEmpty() && local.startsWith(m_voiceDirectory)) {
        m_voiceSends.insert(id, local);
    }
}

void MatrixBridge::forwardToRoom(const QString &roomId,
                                 const QString &body,
                                 const QString &path,
                                 const QString &mimeType,
                                 bool original)
{
    if (roomId.isEmpty()) {
        return;
    }

    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }

    // A forwarded picture has been through someone's encoder already; one from
    // another application has not, and goes the same way the paper clip does.
    const OutgoingImage outgoing = prepareOutgoingImage(local, mimeType, original);

    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("body"), body);
    arguments.insert(QStringLiteral("path"), outgoing.path);
    arguments.insert(QStringLiteral("mimeType"), outgoing.mimeType);
    insertDimensions(arguments, outgoing.path, outgoing.mimeType);
    send(QStringLiteral("room.forward"), arguments);
}

QString MatrixBridge::mimeTypeForPath(const QString &path) const
{
    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }

    // Content first, name second: the share dialog hands over whatever the
    // other application had, and a temporary copy may well carry no suffix.
    const QMimeType type = QMimeDatabase().mimeTypeForFile(local);
    return type.isValid() ? type.name() : QStringLiteral("application/octet-stream");
}

QString MatrixBridge::saveToPictures(const QString &path, const QString &suggestedName)
{
    return saveInto(QStandardPaths::PicturesLocation, path, suggestedName);
}

QString MatrixBridge::saveToDownloads(const QString &path, const QString &suggestedName)
{
    return saveInto(QStandardPaths::DownloadLocation, path, suggestedName);
}

QString MatrixBridge::saveInto(QStandardPaths::StandardLocation location,
                               const QString &path,
                               const QString &suggestedName)
{
    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }

    const QString pictures = QStandardPaths::writableLocation(location);
    if (pictures.isEmpty() || !QFile::exists(local)) {
        setLastError(tr("The file could not be saved."));
        return QString();
    }
    QDir().mkpath(pictures);

    // The cache file name is a hash, so a readable name is derived from the
    // message and made unique rather than overwriting silently.
    QString base = QFileInfo(suggestedName).fileName();
    if (base.isEmpty()) {
        base = QStringLiteral("xmatic-attachment");
    }
    if (!base.contains(QLatin1Char('.'))) {
        const QString suffix = QFileInfo(local).suffix();
        if (!suffix.isEmpty()) {
            base += QLatin1Char('.') + suffix;
        }
    }

    const QFileInfo info(base);
    QString candidate = pictures + QLatin1Char('/') + base;
    for (int n = 1; QFile::exists(candidate); ++n) {
        candidate = QStringLiteral("%1/%2-%3.%4")
                        .arg(pictures, info.completeBaseName())
                        .arg(n)
                        .arg(info.suffix());
    }

    if (!QFile::copy(local, candidate)) {
        setLastError(tr("The file could not be saved."));
        return QString();
    }
    return candidate;
}

void MatrixBridge::requestMedia(const QString &key, const QVariant &source, bool thumbnail,
                                qint64 declaredSize)
{
    if (key.isEmpty() || !source.isValid()) {
        return;
    }

    const QString known = m_media.value(key);
    if (!known.isEmpty()) {
        // Remembered is not still there: the cache sweep drops oldest first, and
        // handing out the path anyway left an empty frame with no second attempt.
        if (QFile::exists(known)) {
            emit mediaReady(key, known);
            return;
        }
        m_media.remove(key);
    }

    // One request per key at a time; the timeline asks again on every rebind.
    if (m_mediaRequests.values().contains(key)) {
        return;
    }

    // QML hands nested objects over as QJSValue, which QJsonValue::fromVariant
    // silently turns into null. Unwrap it first.
    QVariant plain = source;
    if (plain.canConvert<QJSValue>()) {
        plain = plain.value<QJSValue>().toVariant();
    }

    const QJsonValue encoded = QJsonValue::fromVariant(plain);
    if (!encoded.isObject()) {
        qWarning("xmatic: media request has no usable source (variant type %s)", source.typeName());
        return;
    }

    QJsonObject arguments;
    arguments.insert(QStringLiteral("source"), encoded);
    arguments.insert(QStringLiteral("thumbnail"), thumbnail);
    if (declaredSize > 0) {
        arguments.insert(QStringLiteral("size"), double(declaredSize));
    }

    const quint64 id = m_nextId;
    m_mediaRequests.insert(id, key);
    send(QStringLiteral("media.fetch"), arguments);
}

void MatrixBridge::requestAvatar(const QString &url)
{
    if (url.isEmpty()) {
        return;
    }

    const QString known = m_media.value(url);
    if (!known.isEmpty()) {
        // Same check as the attachment path: the sweep may have taken the file, and
        // a picture that is gone must be asked for again.
        if (QFile::exists(known)) {
            emit mediaReady(url, known);
            return;
        }
        m_media.remove(url);
    }

    if (m_mediaRequests.values().contains(url)) {
        return;
    }

    // The SDK's media source is a struct: `{"url": …}` unencrypted, a `file`
    // object encrypted. A profile picture is never encrypted.
    QJsonObject source;
    source.insert(QStringLiteral("url"), url);

    QJsonObject arguments;
    arguments.insert(QStringLiteral("source"), source);
    arguments.insert(QStringLiteral("thumbnail"), true);
    // An avatar declares no size in an event, so the pre-download gate had
    // nothing to weigh. This is what one may cost.
    arguments.insert(QStringLiteral("size"), double(MaximumAvatarBytes));

    const quint64 id = m_nextId;
    m_mediaRequests.insert(id, url);
    send(QStringLiteral("media.fetch"), arguments);
}
void MatrixBridge::handleMessage(const QString &json)
{
    const QJsonDocument document = QJsonDocument::fromJson(json.toUtf8());
    if (!document.isObject()) {
        return;
    }
    const QJsonObject message = document.object();
    const QString type = message.value(QStringLiteral("type")).toString();

    if (type == QLatin1String("reply")) {
        handleReply(message);
    } else if (type == QLatin1String("event")) {
        handleEvent(message);
    }
}

/// A command's answer: the bookkeeping every reply needs, then the one
/// handler that knows this command. Each group answers whether it took it.
void MatrixBridge::handleReply(const QJsonObject &message)
{
    const quint64 id = static_cast<quint64>(message.value(QStringLiteral("id")).toDouble());
    const PendingCommand entry = m_pending.take(id);
    const QString command = entry.command;
    emit busyChanged();
    updateStallWatch();

    // The other half of the stall report: how long it took in the end. Slow and
    // broken look the same without it.
    if (entry.reported) {
        qWarning("xmatic: %s answered after %d s",
                 qPrintable(command),
                 static_cast<int>((m_uptime.elapsed() - entry.sentAt) / 1000));
    }

    // Released before anything else looks at the reply, so a handler that
    // reacts to the result already sees the timeline as idle.
    if (id != 0 && id == m_paginateId) {
        m_paginateId = 0;
        emit paginatingChanged();
    }

    if (!message.value(QStringLiteral("ok")).toBool()) {
        const QString error = message.value(QStringLiteral("error")).toString();
        qWarning("xmatic: %s failed: %s", qPrintable(command), qPrintable(error));
        if (command == QLatin1String("login.start")
            || command == QLatin1String("login.deviceCode")
            || command == QLatin1String("login.password")) {
            setLoginRunning(false);
            emit loginFailed(error);
        }
        if (command == QLatin1String("member.profile")) {
            emit memberProfileFailed(error);
        }
        if (id == m_indexRequest && m_indexRequest != 0) {
            m_indexRequest = 0;
            emit indexingChanged();
            // Not fatal: the search still works on whatever the index has.
            qWarning("xmatic: folding stored messages into the search index failed");
        }
        if (id == m_searchRequest && m_searchRequest != 0) {
            m_searchRequest = 0;
            emit searchingChanged();
            emit searchFailed(error);
        }
        if (command == QLatin1String("thread.open")
            || command == QLatin1String("thread.paginate")
            || command == QLatin1String("thread.send")) {
            emit threadFailed(error);
        }
        // A failed media fetch has to say so, or the row keeps a spinner over a
        // download that will never arrive - refused oversized ones included.
        const bool wasMedia = m_mediaRequests.contains(id);
        if (wasMedia) {
            emit mediaFailed(m_mediaRequests.value(id));
        }
        forgetRequest(id);
        if (entry.command == QLatin1String("private.set")) {
            // The local copy was updated before the round trip; a refused
            // write must not leave the caller looking allowed.
            send(QStringLiteral("private.get"));
        }
        // The recording stays: the send failed and it is the only copy. "command not
        // understood" means a stale core against a newer bridge - journal, not banner.
        if (error.startsWith(QLatin1String("command not understood"))) {
            qWarning("xmatic: %s rejected by the core: %s",
                     qPrintable(command), qPrintable(error.left(120)));
            return;
        }
        // Everything that failed goes in the log, whether or not it is worth
        // interrupting somebody with.
        noteError(command, error);

        // "Token is not active" is a rotation under a request in flight, not an ended
        // session; the log keeps it, the page does not. Media has `mediaFailed`.
        if (id == m_pushNotifyRequest) {
            m_pushNotifyRequest = 0;
            emit pushNotificationFailed(error);
            return;
        }
        // Between closing one room and opening the next there is a moment with no
        // timeline, and a request already in flight comes back into it. The user
        // switched rooms; there is nothing to act on. The log keeps it.
        if (error.startsWith(QLatin1String("no timeline is open"))
            || error.startsWith(QLatin1String("no thread is open"))) {
            qWarning("xmatic: %s came back to a view that had closed",
                     qPrintable(command));
            return;
        }
        const bool tokenRotated = error.contains(QLatin1String("M_UNKNOWN_TOKEN"));
        if (tokenRotated || wasMedia) {
            qWarning("xmatic: %s failed: %s", qPrintable(command),
                     qPrintable(error.left(160)));
            return;
        }
        setLastError(error);
        return;
    }

    const QJsonObject data = message.value(QStringLiteral("data")).toObject();

    if (replyLogin(command, data)) {
        return;
    }
    if (replyAccount(command, data)) {
        return;
    }
    if (replyRoom(id, command, data)) {
        return;
    }
    if (replyMember(id, command, data)) {
        return;
    }
    if (replySearch(id, command, data)) {
        return;
    }
    if (replyTimeline(id, command, data)) {
        return;
    }
    if (command == QLatin1String("push.notify")) {
        if (id == m_pushNotifyRequest) {
            m_pushNotifyRequest = 0;
            QVariantMap notification = data.toVariantMap();
            // The banner's two lines, built as from a room-list preview, so a push reads
            // like an ordinary arrival. Text only where allowed - lock screen.
            const QString kind = data.value(QStringLiteral("previewKind")).toString();
            const QString text = data.value(QStringLiteral("previewText")).toString();
            notification.insert(QStringLiteral("body"),
                                m_settings && m_settings->notificationPreview()
                                    ? previewLine(kind, text)
                                    : tr("New message"));
            if (!(m_settings && m_settings->notificationPreview())) {
                notification.insert(QStringLiteral("roomName"), QString());
            }
            emit pushNotificationReady(notification);
        }
        return;
    }
    if (replyEncryption(command, data)) {
        return;
    }
    if (replyCall(command, data)) {
        return;
    }

    if (data.contains(QStringLiteral("state"))) {
        applySession(data);
    }
}

/// Replies about the login flow.
bool MatrixBridge::replyLogin(const QString &command, const QJsonObject &data)
{

    if (command == QLatin1String("login.registrationUrl")) {
        const QString url = data.value(QStringLiteral("url")).toString();
        if (!url.isEmpty()) {
            emit registrationUrlReady(url);
        }
        return true;
    }

    if (command == QLatin1String("login.start")) {
        // A server without OAuth that offers the password flow answers with a flag
        // instead of a URL. Not busy while the user types.
        if (data.value(QStringLiteral("passwordLogin")).toBool()) {
            setLoginRunning(false);
            emit passwordLoginNeeded();
            return true;
        }
        const QString url = data.value(QStringLiteral("url")).toString();
        if (url.isEmpty()) {
            setLoginRunning(false);
            emit loginFailed(tr("The homeserver did not return a login page."));
        } else {
            emit loginUrlReady(url);
        }
        return true;
    }

    if (command == QLatin1String("login.password")) {
        // The session.changed event that follows this reply carries the
        // whole outcome; nothing to do here.
        return true;
    }

    if (command == QLatin1String("login.deviceCode")) {
        // Prefer the short URL: the user has to type it on another
        // device, and the complete variant embeds the code anyway.
        const QString url = data.value(QStringLiteral("verificationUri")).toString();
        const QString code = data.value(QStringLiteral("userCode")).toString();
        if (url.isEmpty() || code.isEmpty()) {
            setLoginRunning(false);
            emit loginFailed(tr("The homeserver did not return a sign-in code."));
        } else {
            emit deviceCodeReady(url, code);
        }
        return true;
    }
    return false;
}

/// Replies about the account and its storage.
bool MatrixBridge::replyAccount(const QString &command, const QJsonObject &data)
{

    if (command == QLatin1String("account.get")) {
        const QString name = data.value(QStringLiteral("displayName")).toString();
        const QString avatar = data.value(QStringLiteral("avatarUrl")).toString();
        if (name != m_profileName || avatar != m_profileAvatar) {
            m_profileName = name;
            m_profileAvatar = avatar;
            emit profileChanged();
        }
        return true;
    }

    if (command == QLatin1String("account.ignoredUsers")) {
        QStringList users;
        const QJsonArray list = data.value(QStringLiteral("users")).toArray();
        for (const QJsonValue &value : list) {
            users.append(value.toString());
        }
        emit ignoredUsersReady(users);
        return true;
    }

    if (command == QLatin1String("storage.status")) {
        m_storageStatus = data.toVariantMap();
        // Says what is on disk, names no path and no key.
        qInfo("xmatic: local storage: store=%s session=%s key=%s",
              m_storageStatus.value(QStringLiteral("storeEncrypted")).toBool()
                  ? "encrypted" : "plain",
              !m_storageStatus.value(QStringLiteral("sessionPresent")).toBool()
                  ? "none"
                  : (m_storageStatus.value(QStringLiteral("sessionEncrypted")).toBool()
                         ? "encrypted" : "plain"),
              m_storageStatus.value(QStringLiteral("keyAvailable")).toBool()
                  ? "available" : "missing");
        emit storageChanged();
        return true;
    }
    return false;
}

/// Replies about rooms and spaces.
bool MatrixBridge::replyRoom(quint64 id, const QString &command, const QJsonObject &data)
{

    if (command == QLatin1String("space.hierarchy")) {
        const QString spaceId = m_hierarchyRequests.take(id);
        emit spaceHierarchyReady(spaceId,
                                 data.value(QStringLiteral("rooms")).toArray().toVariantList());
        return true;
    }

    if (command == QLatin1String("room.checkRecipients")) {
        emit recipientsChecked(data.value(QStringLiteral("roomId")).toString(),
                               data.value(QStringLiteral("users")).toArray().toVariantList());
        return true;
    }

    if (command == QLatin1String("room.setNotifyMode")) {
        // A push rule is not a notable change to the SDK's room list, so no diff
        // follows and the row would keep the old state. Write it into the models.
        const QString roomId = data.value(QStringLiteral("roomId")).toString();
        const QString mode = data.value(QStringLiteral("mode")).toString();
        m_rooms.setNotifyMode(roomId, mode);
        m_spaceRooms.setNotifyMode(roomId, mode);
        return true;
    }

    if (command == QLatin1String("room.resolve")) {
        emit roomResolved(data.value(QStringLiteral("address")).toString(),
                          data.value(QStringLiteral("roomId")).toString(),
                          data.value(QStringLiteral("joined")).toBool());
        return true;
    }

    if (command == QLatin1String("room.markRead")) {
        // Same as the notify mode above: no diff need follow, and the badge would
        // stand until something else touched the room.
        const QString roomId = data.value(QStringLiteral("roomId")).toString();
        m_rooms.clearUnread(roomId);
        m_spaceRooms.clearUnread(roomId);
        return true;
    }

    if (command == QLatin1String("room.permalink")) {
        emit roomLinkReady(data.value(QStringLiteral("link")).toString());
        return true;
    }

    if (command == QLatin1String("room.directChat")) {
        emit directChatReady(data.value(QStringLiteral("roomId")).toString());
        return true;
    }

    if (command == QLatin1String("room.followSuccessor")) {
        emit successorReady(data.value(QStringLiteral("roomId")).toString());
        return true;
    }

    if (command == QLatin1String("room.info")) {
        emit roomInfoReady(data.toVariantMap());
        return true;
    }

    if (command == QLatin1String("room.create")) {
        emit roomCreated(data.value(QStringLiteral("roomId")).toString(),
                         data.value(QStringLiteral("name")).toString(),
                         data.value(QStringLiteral("encrypted")).toBool());
        return true;
    }
    return false;
}

/// The answer to one page of a search.
bool MatrixBridge::replySearch(quint64 id, const QString &command, const QJsonObject &data)
{
    if (command == QLatin1String("search.index")) {
        const int count = data.value(QStringLiteral("count")).toInt();
        // The number, not the room: when a search comes back thin, how much of the
        // room is on the device is the first thing worth knowing.
        qInfo("xmatic: search index holds %d stored events", count);
        m_indexRequest = 0;
        emit indexingChanged();
        emit indexReady(count);
        return true;
    }
    if (command != QLatin1String("search.room")) {
        return false;
    }
    // Not ours any more - the user typed on, or the box was cleared. Dropping it
    // is the whole point of remembering the id.
    if (id != m_searchRequest) {
        return true;
    }
    m_searchRequest = 0;

    const QJsonArray rows = data.value(QStringLiteral("rows")).toArray();
    const bool more = data.value(QStringLiteral("more")).toBool();
    const bool first = data.value(QStringLiteral("offset")).toInt() == 0;

    QJsonObject operation;
    operation.insert(QStringLiteral("op"),
                     first ? QStringLiteral("reset") : QStringLiteral("append"));
    operation.insert(QStringLiteral("values"), rows);
    m_searchResults.applyOperations(QJsonArray{ operation });

    // The offset counts what was asked for. A row whose event could not be loaded
    // is dropped on the way, and counting survivors asks for the same page again.
    m_searchOffset += SearchPageSize;
    if (more != m_searchHasMore) {
        m_searchHasMore = more;
        emit searchHasMoreChanged();
    }
    emit searchingChanged();
    return true;
}

/// Replies about members and moderation.
bool MatrixBridge::replyMember(quint64 id, const QString &command, const QJsonObject &data)
{

    if (command == QLatin1String("member.remove")) {
        m_members.removeUser(m_removeRequests.take(id));
        return true;
    }

    if (command == QLatin1String("member.profile")) {
        emit memberProfileReady(data.toVariantMap());
        return true;
    }

    // No diff follows core-made state changes, and the store learns of them on a
    // later sync - so the result is data for the page, never a hint to re-read.
    if (command == QLatin1String("member.ban")) {
        const QString userId = data.value(QStringLiteral("userId")).toString();
        // Only the model of the room this actually happened in.
        if (data.value(QStringLiteral("roomId")).toString() == m_membersRoomId) {
            m_members.removeUser(userId);
        }
        emit memberActionDone(QStringLiteral("ban"), data.toVariantMap());
        emit memberChanged(userId);
        return true;
    }

    if (command == QLatin1String("member.setPower")) {
        const QString userId = data.value(QStringLiteral("userId")).toString();
        if (data.value(QStringLiteral("roomId")).toString() == m_membersRoomId) {
            m_members.setPower(userId, data.value(QStringLiteral("power")).toInt());
        }
        emit memberActionDone(QStringLiteral("setPower"), data.toVariantMap());
        emit memberChanged(userId);
        return true;
    }

    if (command == QLatin1String("member.unban")
            || command == QLatin1String("member.setIgnored")
            || command == QLatin1String("member.withdrawVerification")) {
        const QString action = command == QLatin1String("member.unban")
                ? QStringLiteral("unban")
                : (command == QLatin1String("member.setIgnored")
                   ? QStringLiteral("setIgnored")
                   : QStringLiteral("withdrawVerification"));
        emit memberActionDone(action, data.toVariantMap());
        emit memberChanged(data.value(QStringLiteral("userId")).toString());
        return true;
    }
    return false;
}

/// Replies about the timeline and its media.
bool MatrixBridge::replyTimeline(quint64 id, const QString &command, const QJsonObject &data)
{

    if (command == QLatin1String("media.fetch")) {
        const QString key = m_mediaRequests.take(id);
        const QString path = data.value(QStringLiteral("path")).toString();
        if (!key.isEmpty() && !path.isEmpty()) {
            m_media.insert(key, path);
            emit mediaReady(key, path);
        }
        return true;
    }

    if (command == QLatin1String("private.set") && m_legacyDropPending && m_settings) {
        // The encrypted copy exists now, so the plaintext one may go.
        m_legacyDropPending = false;
        m_settings->dropLegacyLists();
    }

    if (command == QLatin1String("private.get") || command == QLatin1String("private.set")) {
        const QJsonObject lists = data.value(QStringLiteral("lists")).toObject();
        m_privateLists.clear();
        for (auto it = lists.constBegin(); it != lists.constEnd(); ++it) {
            QStringList values;
            for (const QJsonValue &value : it.value().toArray()) {
                values.append(value.toString());
            }
            m_privateLists.insert(it.key(), values);
        }

        // Whether the file could be read at all. Locked or damaged is not empty, and
        // the key is missing after every reboot until the system dialog has run.
        m_privateListsReadable = data.value(QStringLiteral("readable")).toBool();

        // What the plain settings file still holds moves over once and is removed
        // there - only where the encrypted file was readable, or the copy is gone.
        if (command == QLatin1String("private.get") && m_settings && m_privateListsReadable) {
            const QStringList callers = m_settings->legacyAllowedCallers();
            const QStringList trusted = m_settings->legacyTrustedRecipients();
            if (!callers.isEmpty() || !trusted.isEmpty()) {
                // Both lists in one write: two commands would be two read-modify-writes
                // racing, on the very migration that then deletes the original.
                for (const QString &caller : callers) {
                    QStringList merged = m_privateLists.value(QStringLiteral("callers"));
                    if (!merged.contains(caller)) {
                        merged.append(caller);
                        m_privateLists.insert(QStringLiteral("callers"), merged);
                    }
                }
                for (const QString &user : trusted) {
                    QStringList merged = m_privateLists.value(QStringLiteral("trusted"));
                    if (!merged.contains(user)) {
                        merged.append(user);
                        m_privateLists.insert(QStringLiteral("trusted"), merged);
                    }
                }
                sendPrivateLists();
                // Dropped only when the encrypted write came back ok; see
                // the reply handler for `private.set`.
                m_legacyDropPending = true;
            }
        }

        emit privateListsChanged();
        pushCallPolicy();
        return true;
    }

    if (command == QLatin1String("timeline.markRead")) {
        // A receipt is not necessarily answered with a room-list diff, so the badge
        // stood on a room just read. Only where the core says it marked something.
        const bool read = data.value(QStringLiteral("read")).toBool();
        if (read) {
            const QString roomId = data.value(QStringLiteral("roomId")).toString();
            m_rooms.clearUnread(roomId);
            m_spaceRooms.clearUnread(roomId);
        } else {
            // Nothing to point a marker at. Said out loud: from outside this looks like
            // the badge simply not clearing, and the two are repaired elsewhere.
            qWarning("xmatic: nothing to mark read - the timeline has no newest event");
        }
        return true;
    }

    if (command == QLatin1String("timeline.sendMedia")) {
        const QString recording = m_voiceSends.take(id);
        if (!recording.isEmpty()) {
            QFile::remove(recording);
        }
        return false;
    }

    if (command == QLatin1String("timeline.readers")) {
        const QString eventId = m_readerRequests.take(id);
        if (!eventId.isEmpty()) {
            emit readersReady(eventId,
                              data.value(QStringLiteral("readers")).toArray().toVariantList());
        }
        return true;
    }

    if (command == QLatin1String("timeline.reactors")) {
        if (m_reactorRequests.remove(id)) {
            // Both come from the core rather than being remembered here: one message can
            // carry several reactions, and the answer says which.
            emit reactorsReady(data.value(QStringLiteral("eventId")).toString(),
                               data.value(QStringLiteral("key")).toString(),
                               data.value(QStringLiteral("reactors")).toArray().toVariantList());
        }
        return true;
    }

    if (command == QLatin1String("timeline.open")) {
        m_openTimelineId = 0;
        setTimelineReady(true);
        // Permissions so menus ask instead of offering what the server will refuse.
        // Thread roots only on a rebuilt timeline - a kept one keeps its own.
        if (data.value(QStringLiteral("rebuilt")).toBool()) {
            m_timeline.setThreadRoots(QHash<QString, int>());
        }
        const QVariantMap can = data.value(QStringLiteral("can")).toObject().toVariantMap();
        // Absent for every room that is not an encrypted two-party chat, which
        // is what takes the entry away again on the next room.
        const QString peer = data.value(QStringLiteral("directWith")).toString();
        if (can != m_roomPermissions || peer != m_roomDirectPeer) {
            m_roomPermissions = can;
            m_roomDirectPeer = peer;
            emit roomPermissionsChanged();
        }
        // Live view only: a focused open shows a different slice, and the room's
        // read marker means nothing in it.
        if (m_timelineFocus.isEmpty()) {
            const QString marker = data.value(QStringLiteral("readMarker")).toString();
            const QString receipt = data.value(QStringLiteral("readReceipt")).toString();
            const bool rebuilt = data.value(QStringLiteral("rebuilt")).toBool();
            // Whether a marker came, never which one. Without it "opened at the newest
            // message" cannot be told from "said and not found", which want opposite fixes.
            qInfo("xmatic: room opened, read marker %s, rebuilt %d, %d rows",
                  marker.isEmpty() ? "none" : "present",
                  rebuilt ? 1 : 0,
                  m_timeline.count());
            emit timelineOpened(marker, receipt, rebuilt);
        }
        return true;
    }

    if (command == QLatin1String("timeline.paginate")) {
        setTimelineAtStart(data.value(QStringLiteral("reachedStart")).toBool());
        // The model's count now, not the page's yield - those rows may still be in
        // the diff stream. Two equal counts is what a fruitless round looks like.
        qInfo("xmatic: older messages answered, %d rows in the model, at start %d",
              m_timeline.count(), m_timelineAtStart ? 1 : 0);
        emit paginated();
        return true;
    }

    return false;
}

/// Replies about encryption.
bool MatrixBridge::replyEncryption(const QString &command, const QJsonObject &data)
{

    if (command == QLatin1String("encryption.status")
            || command == QLatin1String("encryption.recover")) {
        m_encryptionStatus = data.toVariantMap();
        // The import is done, but the recovery state rides the SDK's stream and lags.
        // Until it says enabled, the app says "still finishing".
        if (command == QLatin1String("encryption.recover")) {
            const bool settled =
                m_encryptionStatus.value(QStringLiteral("recovery")).toString()
                == QLatin1String("enabled");
            m_recoverySettling = !settled;
            if (m_recoverySettling) {
                m_recoverySettleTimeout.start();
            } else {
                m_recoverySettleTimeout.stop();
            }
        }
        emit encryptionChanged();
        return true;
    }

    if (command == QLatin1String("encryption.enableBackup")) {
        emit recoveryKeyReady(data.value(QStringLiteral("recoveryKey")).toString());
        return true;
    }
    return false;
}

/// Replies about calls.
bool MatrixBridge::replyCall(const QString &command, const QJsonObject &data)
{
    if (command == QLatin1String("call.invite")) {
        // The one member who may answer, from the room's own membership.
        m_calls->setExpectedPeer(data.value(QStringLiteral("peer")).toString());
        return true;
    }


    if (command == QLatin1String("call.turnServers")) {
        QVariantList servers;
        const QJsonArray uris = data.value(QStringLiteral("uris")).toArray();
        for (const QJsonValue &uri : uris) {
            QVariantMap server;
            server.insert(QStringLiteral("uri"), uri.toString());
            server.insert(QStringLiteral("username"),
                          data.value(QStringLiteral("username")).toString());
            server.insert(QStringLiteral("password"),
                          data.value(QStringLiteral("password")).toString());
            servers.append(server);
        }
        m_calls->setTurnServers(servers);
        qInfo("xmatic: %d relay servers available", servers.size());
        return true;
    }
    return false;
}
/// A message the core sent on its own. Same shape as the replies: one
/// handler per domain, the first that recognises the name takes it.
void MatrixBridge::handleEvent(const QJsonObject &message)
{
    const QString name = message.value(QStringLiteral("event")).toString();
    const QJsonObject data = message.value(QStringLiteral("data")).toObject();

    if (eventCall(name, data)) {
        return;
    }
    if (eventVerification(name, data)) {
        return;
    }
    if (eventTimeline(name, data)) {
        return;
    }
    if (eventLists(name, data)) {
        return;
    }
    if (eventSession(name, data)) {
        return;
    }
}

/// Events about a call's signalling.
bool MatrixBridge::eventCall(const QString &name, const QJsonObject &data)
{
    if (name == QLatin1String("call.invite")) {
        // A late ring is either our fault or the delivery's; this number says
        // which, without naming anybody.
        qInfo("xmatic: incoming call, %lld ms after it was sent",
              static_cast<long long>(data.value(QStringLiteral("ageMs")).toDouble()));
        m_calls->onRemoteInvite(data.value(QStringLiteral("roomId")).toString(),
                                data.value(QStringLiteral("video")).toBool(),
                                data.value(QStringLiteral("videoOffered")).toBool(),
                                data.value(QStringLiteral("sender")).toString(),
                                data.value(QStringLiteral("callId")).toString(),
                                data.value(QStringLiteral("sdp")).toString());
    } else if (name == QLatin1String("call.answer")) {
        // Room and sender travel with the event and are checked in the engine:
        // a call id is public to everybody in the room.
        m_calls->onRemoteAnswer(data.value(QStringLiteral("roomId")).toString(),
                                data.value(QStringLiteral("sender")).toString(),
                                data.value(QStringLiteral("callId")).toString(),
                                data.value(QStringLiteral("sdp")).toString());
    } else if (name == QLatin1String("call.candidates")) {
        m_calls->onRemoteCandidates(data.value(QStringLiteral("roomId")).toString(),
                                    data.value(QStringLiteral("sender")).toString(),
                                    data.value(QStringLiteral("callId")).toString(),
                                    data.value(QStringLiteral("candidates")).toArray().toVariantList());
    } else if (name == QLatin1String("call.blocked")) {
        // The invitation still raised the server's counter, and the chat list turns
        // that into a banner. Refusing a call and then beeping about it is the opposite.
        const QString roomId = data.value(QStringLiteral("roomId")).toString();
        if (!roomId.isEmpty()) {
            // Short, once per room per minute: longer or repeatable turns a refused call
            // into a way of silencing a room, and the caller picks the timing.
            const qint64 now = m_uptime.elapsed();
            const qint64 last = m_blockedCallRooms.value(roomId, -60000);
            if (now - last >= 60000) {
                m_blockedCallRooms.insert(roomId, now);
            }
        }
        // No identifier, only the reason: this is the line that tells a silent
        // phone from a phone nobody called.
        qInfo("xmatic: an incoming call was refused (%s)",
              qPrintable(data.value(QStringLiteral("reason")).toString()));
    } else if (name == QLatin1String("call.hangup")) {
        m_calls->onRemoteHangup(data.value(QStringLiteral("roomId")).toString(),
                                data.value(QStringLiteral("sender")).toString(),
                                data.value(QStringLiteral("callId")).toString());
    } else {
        return false;
    }
    return true;
}

/// Events about verification and encryption.
bool MatrixBridge::eventVerification(const QString &name, const QJsonObject &data)
{
    if (name == QLatin1String("verification.request")) {
        m_verificationState = QStringLiteral("requested");
        m_verificationUser = data.value(QStringLiteral("userId")).toString();
        m_verificationIsSelf = data.value(QStringLiteral("isSelf")).toBool();
        m_verificationWeStarted = data.value(QStringLiteral("weStarted")).toBool();
        m_verificationEmoji.clear();
        qInfo("xmatic: verification requested (self=%d, inRoom=%d, weStarted=%d)",
              m_verificationIsSelf ? 1 : 0,
              data.value(QStringLiteral("inRoom")).toBool() ? 1 : 0,
              data.value(QStringLiteral("weStarted")).toBool() ? 1 : 0);
        emit verificationChanged();
    } else if (name == QLatin1String("push.state")) {
        // Merged, not replaced: an event carrying only a state must not take the
        // distributor list with it - the page then said "no distributor".
        const QVariantMap update = data.toVariantMap();
        // An error belongs to the state that carried it: a later state without
        // one clears it.
        if (!update.contains(QStringLiteral("error"))) {
            m_pushStatus.remove(QStringLiteral("error"));
        }
        for (auto it = update.constBegin(); it != update.constEnd(); ++it) {
            m_pushStatus.insert(it.key(), it.value());
        }
        const QString state = m_pushStatus.value(QStringLiteral("state")).toString();
        // The distributor's bus names, not the endpoint: one is what is
        // installed on the device, the other is a secret.
        qInfo("xmatic: push state -> %s (%d distributor(s))", qPrintable(state),
              m_pushStatus.value(QStringLiteral("distributors")).toList().count());
        const QString error = m_pushStatus.value(QStringLiteral("error")).toString();
        if (!error.isEmpty()) {
            noteError(QStringLiteral("push"), error);
        }
        emit pushStatusChanged();
    } else if (name == QLatin1String("push.endpoint")) {
        // The second half of turning it on, here because the gateway is a setting.
        // An endpoint nobody was told about is a secret held for nothing.
        m_pushEndpoint = data.value(QStringLiteral("endpoint")).toString();
        m_pushP256dh = data.value(QStringLiteral("p256dh")).toString();
        m_pushAuth = data.value(QStringLiteral("auth")).toString();
        // Never the endpoint itself: it is the one string here that lets a
        // stranger push to this phone.
        qInfo("xmatic: push endpoint received (%d bytes)", m_pushEndpoint.size());
        emit pushStatusChanged();
        if (!m_pushGateway.isEmpty()) {
            QJsonObject arguments;
            arguments.insert(QStringLiteral("endpoint"), m_pushEndpoint);
            arguments.insert(QStringLiteral("p256dh"), m_pushP256dh);
            arguments.insert(QStringLiteral("auth"), m_pushAuth);
            arguments.insert(QStringLiteral("gateway"), m_pushGateway);
            send(QStringLiteral("push.pusher"), arguments);
        }
    } else if (name == QLatin1String("push.message")) {
        m_pushMessageSeen = true;
        const QString roomId = data.value(QStringLiteral("roomId")).toString();
        const QString eventId = data.value(QStringLiteral("eventId")).toString();
        // Never the identifiers themselves.
        qInfo("xmatic: push received (decrypted=%d, matrix=%d)",
              data.value(QStringLiteral("decrypted")).toBool() ? 1 : 0,
              roomId.isEmpty() ? 0 : 1);
        if (roomId.isEmpty() || eventId.isEmpty()) {
            // Another app's push, a gateway that reshaped the body, or the
            // distributor's own test. Not ours to show.
            emit pushNotificationFailed(QStringLiteral("not a matrix notification"));
        } else {
            QJsonObject arguments;
            arguments.insert(QStringLiteral("roomId"), roomId);
            arguments.insert(QStringLiteral("eventId"), eventId);
            m_pushNotifyRequest = send(QStringLiteral("push.notify"), arguments);
        }
    } else if (name == QLatin1String("encryption.changed")) {
        m_encryptionStatus = data.toVariantMap();
        if (m_recoverySettling
            && m_encryptionStatus.value(QStringLiteral("recovery")).toString()
                   == QLatin1String("enabled")) {
            m_recoverySettling = false;
            m_recoverySettleTimeout.stop();
        }
        qInfo("xmatic: encryption: recovery=%s backup=%s enabled=%d",
              qPrintable(m_encryptionStatus.value(QStringLiteral("recovery")).toString()),
              qPrintable(m_encryptionStatus.value(QStringLiteral("backup")).toString()),
              m_encryptionStatus.value(QStringLiteral("backupEnabled")).toBool() ? 1 : 0);
        emit encryptionChanged();
    } else if (name == QLatin1String("verification.stage")) {
        // Stage names only — enough to tell a stalled flow from a declined one.
        qInfo("xmatic: verification stage: %s",
              qPrintable(data.value(QStringLiteral("stage")).toString()));
    } else if (name == QLatin1String("verification.failed")) {
        const QString error = data.value(QStringLiteral("message")).toString();
        qWarning("xmatic: verification failed: %s", qPrintable(error));
        setLastError(error);
    } else if (name == QLatin1String("verification.emoji")) {
        m_verificationEmoji.clear();
        const QJsonArray emoji = data.value(QStringLiteral("emoji")).toArray();
        for (const QJsonValue &value : emoji) {
            m_verificationEmoji.append(value.toObject().toVariantMap());
        }
        m_verificationState = QStringLiteral("comparing");
        emit verificationChanged();
    } else if (name == QLatin1String("verification.done")) {
        m_verificationState = QStringLiteral("done");
        qInfo("xmatic: verification done");
        emit verificationChanged();
    } else if (name == QLatin1String("verification.cancelled")) {
        m_verificationState = QStringLiteral("cancelled");
        qInfo("xmatic: verification cancelled");
        emit verificationChanged();
    } else {
        return false;
    }
    return true;
}

/// Events about the timeline and threads.
bool MatrixBridge::eventTimeline(const QString &name, const QJsonObject &data)
{
    if (name == QLatin1String("timeline.diff")) {
        // Room *and* generation: the pinned view opens the same room under a
        // different focus, so the diffs of the view being left pass a room test.
        const QString diffToken = data.value(QStringLiteral("token")).toString();
        const bool mine = data.value(QStringLiteral("roomId")).toString() == m_openRoomId
                && diffToken == QString::number(m_timelineGeneration);
        if (!mine) {
            // Once per token: a stream whose diffs are all dropped is a room
            // that stops moving, which reads as a broken sync.
            if (diffToken != m_lastDroppedToken) {
                m_lastDroppedToken = diffToken;
                qWarning("xmatic: dropping timeline diffs of another view (token %s, current %s)",
                         qPrintable(diffToken),
                         qPrintable(QString::number(m_timelineGeneration)));
            }
        }
        if (mine) {
            const QJsonArray ops = data.value(QStringLiteral("ops")).toArray();
            m_timeline.applyOperations(ops);

            // Operation names and the row count, no content: a timeline that empties
            // itself is otherwise indistinguishable from a rendering fault.
            QStringList names;
            for (const QJsonValue &value : ops) {
                names.append(value.toObject().value(QStringLiteral("op")).toString());
            }
            qInfo("xmatic: timeline diff [%s] -> %d rows",
                  qPrintable(names.join(QStringLiteral(","))),
                  m_timeline.count());
            reportSendFailures(ops);
        }
    } else if (name == QLatin1String("thread.diff")) {
        if (data.value(QStringLiteral("token")).toString()
                == QString::number(m_threadGeneration)) {
            const QJsonArray ops = data.value(QStringLiteral("ops")).toArray();
            m_threadTimeline.applyOperations(ops);
            qInfo("xmatic: thread diff -> %d rows", m_threadTimeline.count());
        }
    } else if (name == QLatin1String("thread.error")) {
        if (data.value(QStringLiteral("root")).toString() == m_openThreadRoot) {
            const QString message = data.value(QStringLiteral("message")).toString();
            qWarning("xmatic: thread could not be loaded: %s", qPrintable(message));
            emit threadFailed(message);
        }
    } else if (name == QLatin1String("timeline.detailError")) {
        // The quoted event of a reply could not be fetched; ids arrive
        // pre-truncated, the error pre-scrubbed.
        qInfo("xmatic: reply details failed for %s: %s",
              qPrintable(data.value(QStringLiteral("eventId")).toString()),
              qPrintable(data.value(QStringLiteral("error")).toString()));
    } else if (name == QLatin1String("timeline.threads")) {
        // The server's thread roots - the way into a thread whose root the store
        // knows without a summary, which is every event cached before threading.
        if (data.value(QStringLiteral("roomId")).toString() == m_openRoomId) {
            QHash<QString, int> roots;
            const QJsonArray listed = data.value(QStringLiteral("roots")).toArray();
            for (const QJsonValue &value : listed) {
                const QJsonObject entry = value.toObject();
                const QString id = entry.value(QStringLiteral("eventId")).toString();
                if (!id.isEmpty()) {
                    roots.insert(id, entry.value(QStringLiteral("count")).toInt(1));
                }
            }
            qInfo("xmatic: the server lists %d thread root(s) in this room", roots.count());
            m_timeline.setThreadRoots(roots);
        }
        return true;
    } else if (name == QLatin1String("timeline.pinned")) {
        if (data.value(QStringLiteral("roomId")).toString() == m_openRoomId) {
            QStringList ids;
            const QJsonArray eventIds = data.value(QStringLiteral("eventIds")).toArray();
            for (const QJsonValue &value : eventIds) {
                ids.append(value.toString());
            }
            const QString preview = data.value(QStringLiteral("preview")).toString();
            // Only worth a line where the room names pins that cannot be read: the view
            // is then empty for a reason outside this app.
            const int loaded = data.value(QStringLiteral("loaded")).toInt();
            // Against what the core looked at, not the room's whole list: the check is
            // capped, and comparing with the full count reported a failure everywhere.
            const int checked = data.value(QStringLiteral("checked")).toInt();
            if (checked > 0 && loaded < checked) {
                const QString loadError =
                    data.value(QStringLiteral("loadError")).toString();
                qWarning("xmatic: %d pinned, %d checked, only %d readable, "
                         "first failure: %s",
                         ids.count(), checked, loaded, qPrintable(loadError));
            }
            if (ids != m_pinnedEventIds || preview != m_pinnedPreview) {
                m_pinnedEventIds = ids;
                m_pinnedPreview = preview;
                emit pinnedChanged();
            }
        }
    } else if (name == QLatin1String("timeline.tombstone")) {
        if (data.value(QStringLiteral("roomId")).toString() == m_openRoomId) {
            // Sent for every room opened, the negative answer included: an
            // absent successor is what clears the banner of the room before.
            const QJsonObject successor =
                data.value(QStringLiteral("successor")).toObject();
            const QString roomId = successor.value(QStringLiteral("roomId")).toString();
            const QString reason = successor.value(QStringLiteral("reason")).toString();
            const bool joined = successor.value(QStringLiteral("joined")).toBool();
            if (roomId != m_successorRoomId || reason != m_replacementReason
                || joined != m_replacementJoined) {
                m_successorRoomId = roomId;
                m_replacementReason = reason;
                m_replacementJoined = joined;
                emit tombstoneChanged();
            }
        }
    } else {
        return false;
    }
    return true;
}

/// Events about the lists: rooms, spaces, directory, members.
bool MatrixBridge::eventLists(const QString &name, const QJsonObject &data)
{
    if (name == QLatin1String("roomlist.diff")) {
        const QJsonArray ops = data.value(QStringLiteral("ops")).toArray();
        m_rooms.applyOperations(ops);
        reportNewMessages(ops);
        // Unread counts feed the space badges, so a room list change may change
        // them even though the space list itself did not.
        bumpSpaceCounts();
    } else if (name == QLatin1String("spaces.diff")) {
        m_spaces.applyOperations(data.value(QStringLiteral("ops")).toArray());
    } else if (name == QLatin1String("space.diff")) {
        // A space's rooms are a subset of the main list, which already drives
        // notifications, so this view does not report new messages of its own.
        m_spaceRooms.applyOperations(data.value(QStringLiteral("ops")).toArray());
    } else if (name == QLatin1String("spaces.children")) {
        updateSpaceChildren(data.value(QStringLiteral("spaces")).toObject());
    } else if (name == QLatin1String("directory.diff")) {
        m_directory.applyOperations(data.value(QStringLiteral("ops")).toArray());
    } else if (name == QLatin1String("members.diff")) {
        m_members.applyOperations(data.value(QStringLiteral("ops")).toArray());
    } else if (name == QLatin1String("directory.state")) {
        const bool atEnd = data.value(QStringLiteral("atEnd")).toBool();
        if (atEnd != m_directoryAtEnd) {
            m_directoryAtEnd = atEnd;
            emit directoryStateChanged();
        }
        const QString error = data.value(QStringLiteral("error")).toString();
        if (!error.isEmpty()) {
            setLastError(error);
        }
    } else {
        return false;
    }
    return true;
}

/// Events about the session, sync and profile.
bool MatrixBridge::eventSession(const QString &name, const QJsonObject &data)
{
    if (name == QLatin1String("session.refreshed")) {
        // Nothing to do, everything to record: a request in flight across a refresh
        // comes back as "token is not active" while the sync carries on.
        qInfo("xmatic: session tokens refreshed");
        return true;
    }

    if (name == QLatin1String("session.expired")) {
        // Emitted by the core and, until now, understood by nobody: the room
        // list then sat there empty and quietly not syncing.
        qWarning("xmatic: the session expired");
        applySession(QJsonObject { { QStringLiteral("state"), QStringLiteral("none") } });
        setLastError(tr("Your session has ended. Please sign in again."));
        return true;
    }

    if (name == QLatin1String("sync.support")) {
        // A server that cannot do this app's sync fails every one, which offline mode
        // turns into a flashing banner over an empty list. Say what it is.
        const bool supported = data.value(QStringLiteral("supported")).toBool(true);
        const QString error = data.value(QStringLiteral("error")).toString();
        if (!error.isEmpty()) {
            qWarning("xmatic: could not ask the server what it supports: %s",
                     qPrintable(error));
        } else {
            qInfo("xmatic: homeserver supports the required sync: %d", supported ? 1 : 0);
        }
        if (m_serverSupported != supported) {
            m_serverSupported = supported;
            emit serverSupportedChanged();
        }
    } else if (name == QLatin1String("profile.changed")) {
        const QString displayName = data.value(QStringLiteral("displayName")).toString();
        const QString avatar = data.value(QStringLiteral("avatarUrl")).toString();
        if (displayName != m_profileName || avatar != m_profileAvatar) {
            m_profileName = displayName;
            m_profileAvatar = avatar;
            emit profileChanged();
        }
    } else if (name == QLatin1String("sync.state")) {
        const QString state = data.value(QStringLiteral("state")).toString();
        if (state != m_syncState) {
            qInfo("xmatic: sync state -> %s", qPrintable(state));
            m_syncState = state;
            emit syncStateChanged();
        }
    } else if (name == QLatin1String("roomlist.total")) {
        // The server's room count next to the rows the list holds: a short list is
        // either an ungrown sync window or one page asked for, and only both say which.
        const QJsonValue total = data.value(QStringLiteral("total"));
        const int count = total.isDouble() ? total.toInt() : -1;
        if (count != m_roomTotal) {
            qInfo("xmatic: server counts %d rooms", count);
            m_roomTotal = count;
            emit roomTotalChanged();
        }
    } else if (name == QLatin1String("session.changed")) {
        setLoginRunning(false);
        if (data.value(QStringLiteral("state")).toString() != QLatin1String("signed-in")) {
            m_rooms.clear();
            if (m_roomTotal != -1) {
                m_roomTotal = -1;
                emit roomTotalChanged();
            }
            m_spaces.clear();
            m_spaceRooms.clear();
            m_unread.clear();
            m_notified.clear();
            m_spaceChildRooms.clear();
            m_spaceSubspaces.clear();
            bumpSpaceCounts();
            m_timeline.clear();
            m_openRoomId.clear();
            emit openRoomChanged();
            m_directory.clear();
            if (!m_profileName.isEmpty() || !m_profileAvatar.isEmpty()) {
                m_profileName.clear();
                m_profileAvatar.clear();
                emit profileChanged();
            }
        }
        applySession(data);
    } else if (name == QLatin1String("login.failed")) {
        setLoginRunning(false);
        const QString error = data.value(QStringLiteral("message")).toString();
        qWarning("xmatic: login failed: %s", qPrintable(error));
        setLastError(error);
        emit loginFailed(error);
    } else if (name == QLatin1String("login.aborted")) {
        setLoginRunning(false);
    } else if (name == QLatin1String("session.warning")) {
        setLastError(data.value(QStringLiteral("message")).toString());
    } else {
        return false;
    }
    return true;
}
void MatrixBridge::reportSendFailures(const QJsonArray &operations)
{
    // A message the queue gave up on stays in the room until the user acts, and
    // "not sent" is all the screen can say. The core scrubbed the reason.
    for (const QJsonValue &value : operations) {
        const QJsonObject op = value.toObject();
        QVector<QJsonObject> rows;
        if (op.value(QStringLiteral("value")).isObject()) {
            rows.append(op.value(QStringLiteral("value")).toObject());
        }
        for (const QJsonValue &entry : op.value(QStringLiteral("values")).toArray()) {
            rows.append(entry.toObject());
        }
        for (const QJsonObject &row : rows) {
            const QJsonObject error = row.value(QStringLiteral("sendError")).toObject();
            if (error.isEmpty()) {
                continue;
            }
            const QString key = row.value(QStringLiteral("txnId")).toString();
            if (key.isEmpty() || m_reportedSendFailures.contains(key)) {
                continue;
            }
            m_reportedSendFailures.insert(key);
            qWarning("xmatic: a message could not be sent (%s): %s",
                     error.value(QStringLiteral("recoverable")).toBool() ? "will retry"
                                                                        : "given up",
                     qPrintable(error.value(QStringLiteral("reason")).toString()));
        }
    }
}

void MatrixBridge::reportNewMessages(const QJsonArray &operations)
{
    // Every operation that carries rooms carries their full state, so a rising
    // unread count is visible without asking the core for anything.
    QVector<QJsonObject> rooms;
    for (const QJsonValue &value : operations) {
        const QJsonObject op = value.toObject();
        const QJsonValue single = op.value(QStringLiteral("value"));
        if (single.isObject()) {
            rooms.append(single.toObject());
        }
        const QJsonArray many = op.value(QStringLiteral("values")).toArray();
        for (const QJsonValue &entry : many) {
            rooms.append(entry.toObject());
        }
    }

    for (const QJsonObject &room : rooms) {
        const QString id = room.value(QStringLiteral("id")).toString();
        if (id.isEmpty()) {
            continue;
        }
        const int unread = room.value(QStringLiteral("unread")).toInt();
        m_unread.insert(id, unread);

        // The banner follows the notifying events, counted against the push rules;
        // `unread` counts everything, so "mentions only" kept notifying.
        const int notifications = room.value(QStringLiteral("notifications")).toInt();
        const int previous = m_notified.value(id, notifications);
        m_notified.insert(id, notifications);

        // Reading the room elsewhere clears the counter here too. Before the mute and
        // on-screen tests: those govern raising a banner, not taking one down.
        if (notifications == 0 && previous > 0) {
            // A banner still waiting for its text is pointless now: the room
            // was read somewhere else, and what it would announce is gone.
            m_pendingBanners.remove(id);
            emit roomRead(id);
        }

        // Muted and low-priority rooms count their badge but never notify. Honoured
        // here because this app raises its own banners, so the server's rule never applies.
        if (room.value(QStringLiteral("muted")).toBool()
            || room.value(QStringLiteral("lowPriority")).toBool()) {
            continue;
        }

        // Only while the room really is on screen and the app in front. This tested
        // the *open* room, whose subscription outlives the page - so it went silent for good.
        const bool onScreen = !m_visibleRoomId.isEmpty() && id == m_visibleRoomId
                && QGuiApplication::applicationState() == Qt::ApplicationActive;
        // The room's latest event by its own timestamp: the preview belongs to that
        // event, so a moved timestamp proves the text is the new message's.
        const quint64 timestamp = static_cast<quint64>(
            room.value(QStringLiteral("timestamp")).toDouble());

        const auto waiting = m_pendingBanners.find(id);
        if (waiting != m_pendingBanners.end() && timestamp > waiting->timestamp) {
            const PendingBanner ready = *waiting;
            m_pendingBanners.erase(waiting);
            publishBanner(id, ready, room);
        }

        // The count shown is the notifying one - "50 new" over one mention points at
        // the wrong thing. A call refused a moment ago: its own rise would ring here.
        const auto blocked = m_blockedCallRooms.constFind(id);
        const bool afterBlockedCall = blocked != m_blockedCallRooms.constEnd()
                && m_uptime.elapsed() - *blocked < 5000;

        if (notifications > previous && !onScreen && !afterBlockedCall) {
            PendingBanner pending;
            pending.name = room.value(QStringLiteral("name")).toString();
            pending.notifications = notifications;
            pending.mentions = room.value(QStringLiteral("mentions")).toInt();
            pending.timestamp = timestamp;

            // Without the message text there is nothing to wait for: the
            // banner says how many arrived, and that number is right already.
            if (!m_settings || !m_settings->notificationPreview()) {
                publishBanner(id, pending, room);
                continue;
            }

            m_pendingBanners.insert(id, pending);
            if (!m_bannerTimeout.isActive()) {
                m_bannerTimeout.start();
            }
        }
    }
}

/// One banner from what was known when the counts rose plus the room's text
/// now. An empty `room` is the timeout saying no text arrived.
void MatrixBridge::publishBanner(const QString &roomId, const PendingBanner &pending,
                                 const QJsonObject &room)
{
    emit roomActivity(roomId,
                      pending.name,
                      pending.notifications,
                      pending.mentions,
                      room.value(QStringLiteral("previewKind")).toString(),
                      room.value(QStringLiteral("previewText")).toString(),
                      room.value(QStringLiteral("previewSender")).toString());
}

void MatrixBridge::updateSpaceChildren(const QJsonObject &spaces)
{
    m_spaceChildRooms.clear();
    m_spaceSubspaces.clear();
    for (auto it = spaces.constBegin(); it != spaces.constEnd(); ++it) {
        const QJsonObject entry = it.value().toObject();
        QStringList rooms;
        const QJsonArray roomArray = entry.value(QStringLiteral("rooms")).toArray();
        for (const QJsonValue &value : roomArray) {
            rooms.append(value.toString());
        }
        m_spaceChildRooms.insert(it.key(), rooms);
        m_spaceSubspaces.insert(it.key(),
                                entry.value(QStringLiteral("subspaces")).toInt());
    }
    bumpSpaceCounts();
}

void MatrixBridge::bumpSpaceCounts()
{
    ++m_spaceCountsRevision;
    emit spaceCountsChanged();
}

QString MatrixBridge::spaceBadge(const QString &spaceId) const
{
    const int subspaces = m_spaceSubspaces.value(spaceId, 0);

    // The member rooms' unread counts come from the same numbers the room list
    // shows, so the badge matches what the user sees inside the space.
    int messages = 0;
    const QStringList rooms = m_spaceChildRooms.value(spaceId);
    for (const QString &roomId : rooms) {
        messages += m_unread.value(roomId, 0);
    }

    if (subspaces > 0) {
        return QStringLiteral("(%1/%2)").arg(subspaces).arg(messages);
    }
    if (messages > 0) {
        return QStringLiteral("(%1)").arg(messages);
    }
    return QString();
}

void MatrixBridge::applySession(const QJsonObject &data)
{
    const QString state = data.value(QStringLiteral("state")).toString();
    const QString user = data.value(QStringLiteral("userId")).toString();
    const QString device = data.value(QStringLiteral("deviceId")).toString();

    if (state == m_sessionState && user == m_userId && device == m_deviceId) {
        return;
    }

    m_sessionState = state;
    m_userId = user;
    m_deviceId = device;

    // Only the state, never the identifiers: this ends up in the system
    // journal, which is not the place for a Matrix ID or a device ID.
    qInfo("xmatic: session state: %s", qPrintable(m_sessionState));

    // Relay credentials are short-lived and only useful once signed in.
    if (m_sessionState == QLatin1String("signed-in")) {
        send(QStringLiteral("call.turnServers"));
    }

    // A login creates the stores and their marker, a sign-out deletes them.
    // Both change the answer and neither sends a diff.
    refreshStorageStatus();

    emit sessionChanged();
}

QString MatrixBridge::previewLine(const QString &kind, const QString &text) const
{
    if (kind == QLatin1String("text")) {
        return text;
    }
    if (kind == QLatin1String("emote")) {
        return text;
    }
    if (kind == QLatin1String("image")) {
        return tr("Picture");
    }
    if (kind == QLatin1String("video")) {
        return tr("Video");
    }
    if (kind == QLatin1String("audio")) {
        return tr("Voice message");
    }
    if (kind == QLatin1String("file")) {
        return tr("File");
    }
    if (kind == QLatin1String("location")) {
        return tr("Location");
    }
    if (kind == QLatin1String("encrypted")) {
        return tr("Encrypted message");
    }
    if (kind == QLatin1String("invite")) {
        return tr("Invitation");
    }
    return text;
}

void MatrixBridge::clearErrorLog()
{
    if (m_errorLog.isEmpty()) {
        return;
    }
    m_errorLog.clear();
    emit errorLogChanged();
}

void MatrixBridge::noteError(const QString &command, const QString &message)
{
    if (message.isEmpty()) {
        return;
    }
    QVariantMap entry;
    // The time of day only. A date would be one more thing to read on a
    // narrow row, and everything here happened in this run of the app.
    entry.insert(QStringLiteral("time"),
                 QTime::currentTime().toString(QStringLiteral("HH:mm:ss")));
    entry.insert(QStringLiteral("command"), command);
    entry.insert(QStringLiteral("message"), message);
    // Newest first: a log is read from the top, and the interesting entry is
    // the one that just happened.
    m_errorLog.prepend(entry);
    while (m_errorLog.size() > ErrorLogSize) {
        m_errorLog.removeLast();
    }
    emit errorLogChanged();
}

void MatrixBridge::setLastError(const QString &message)
{
    if (m_lastError == message) {
        return;
    }
    m_lastError = message;
    emit lastErrorChanged();
}

void MatrixBridge::setTimelineAtStart(bool atStart)
{
    if (m_timelineAtStart == atStart) {
        return;
    }
    m_timelineAtStart = atStart;
    emit timelineAtStartChanged();
}

void MatrixBridge::setTimelineReady(bool ready)
{
    if (m_timelineReady == ready) {
        return;
    }
    m_timelineReady = ready;
    emit timelineReadyChanged();
}

void MatrixBridge::setLoginRunning(bool running)
{
    if (m_loginRunning == running) {
        return;
    }
    m_loginRunning = running;
    emit busyChanged();
}
