#include "matrixbridge.h"

#include "emojistore.h"

#include "appsettings.h"
#include "secretskeeper.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QJSValue>
#include <QJsonArray>
#include <QJsonValue>
#include <QJsonDocument>
#include <QLoggingCategory>
#include <QMetaObject>
#include <QMimeDatabase>
#include <QMimeType>
#include <QSettings>
#include <QStandardPaths>
#include <QStringList>
#include <QUrl>

namespace {

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
                           const QString &storeKey,
                           AppSettings *settings,
                           QObject *parent)
    : QObject(parent)
    , m_dataDirectory(dataDirectory)
    , m_cacheDirectory(cacheDirectory)
    , m_settings(settings)
{
    // The read status can only be chosen when a timeline is built, so the one
    // already open keeps the old setting until it is built again. Rebuilt here
    // so the switch acts on the room the user came from, not on the next one.
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

    // A banner that waited in vain for its text goes out as a count. Without
    // this a room whose latest event never resolves - an event kind the
    // preview has no words for, a decryption that does not come - would
    // announce nothing at all.
    m_bannerTimeout.setSingleShot(true);
    m_bannerTimeout.setInterval(900);
    connect(&m_bannerTimeout, &QTimer::timeout, this, [this]() {
        const auto pending = m_pendingBanners;
        m_pendingBanners.clear();
        for (auto it = pending.constBegin(); it != pending.constEnd(); ++it) {
            publishBanner(it.key(), it.value(), QJsonObject());
        }
    });

    // Started before anything can be sent: every pending command is stamped
    // against this clock, and reading an unstarted QElapsedTimer is undefined.
    m_uptime.start();

    m_coreVersion = takeCoreString(xm_version());

    QJsonObject config;
    config.insert(QStringLiteral("dataDir"), dataDirectory);
    config.insert(QStringLiteral("cacheDir"), cacheDirectory);
    if (!storeKey.isEmpty()) {
        config.insert(QStringLiteral("storeKey"), storeKey);
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

    // Signing out takes the decrypted media with it, unless the user asked
    // for "never" - that setting means never. The lists that name people go in
    // every case: they belong to the account that is leaving.
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

    // Asked once at start, before anything else: whether the files on this
    // device are encrypted is a property of the disk, not of a session, and
    // the UI must be able to say so while signed out too.
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

    // Held in a named variable rather than chained: the pointer belongs to the
    // temporary QByteArray, which would otherwise live only to the end of the
    // statement. That is long enough today, because the core copies the string
    // before returning — but it is a lifetime that depends on a detail of the
    // callee, and nothing here would notice if that changed.
    QByteArray payload = jsonToCompactString(message).toUtf8();
    xm_core_send(m_core, payload.constData());
    if (wipePayload) {
        // The buffer held a password; the core has taken its copy (and wipes
        // its own), so this copy must not linger on the heap. The transient
        // QString and QJsonObject copies above cannot be reached from here —
        // that residual is documented in docs/PASSWORD-LOGIN.md.
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
    // Ten seconds is well past anything a healthy homeserver takes and well
    // short of the SDK's retry budget, so a rate-limited request is named while
    // it is still being retried rather than a quarter of an hour later.
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

    // A command that never came back used to sit in the list for good, and
    // `busy` sits on that list: one lost answer froze the sign-in page, text
    // field included, until the app was restarted. Two minutes is far past any
    // retry budget the SDK has.
    static const qint64 giveUp = 120000;
    QStringList abandoned;
    QStringList lostMedia;
    for (auto it = m_pending.begin(); it != m_pending.end();) {
        if (now - it->sentAt < giveUp) {
            ++it;
            continue;
        }
        abandoned.append(it->command);
        // The two states that gate a page are not in this list: a lost login
        // reply left the sign-in page frozen, and a lost pagination reply left
        // "load older messages" dead for the rest of the session.
        if (it.key() == m_paginateId) {
            m_paginateId = 0;
            emit paginatingChanged();
        }
        if (it->command.startsWith(QLatin1String("login."))) {
            setLoginRunning(false);
        }
        // Everything the answer would have cleared. Without this a lost
        // `media.fetch` kept its key in `m_mediaRequests` for good, and the
        // "one request per key at a time" test then refused every further
        // attempt at that picture for the rest of the session - a permanently
        // empty frame from one dropped reply.
        //
        // Collected, not emitted here: this loop erases from the very hash a
        // slot could add to through `send()`, and a rehash under the iterator
        // is how that ends. Said out loud after the loop.
        if (m_mediaRequests.contains(it.key())) {
            lostMedia.append(m_mediaRequests.value(it.key()));
        }
        // The recording stays on disk. Putting the entry back was pointless -
        // the reply that would have read it looks the command up in `m_pending`,
        // which this loop has just erased, so the branch that deletes the file
        // can never run again for this id. It is cleared with the media cache
        // like everything else in there.
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

void MatrixBridge::retryUnlock()
{
    setLastError(QString());
    QString key = obtainStoreKey(m_dataDirectory);
    QJsonObject arguments;
    if (!key.isEmpty()) {
        arguments.insert(QStringLiteral("storeKey"), key);
        key.fill(QChar('0'));
    }
    // The payload carries the key; wiped like a password after the core took
    // its copy. Without a key the plain restore runs and reports "locked"
    // again, which keeps the page and its retry on screen.
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
    // The password is deliberately not trimmed: whitespace in it is the
    // user's business. It is also deliberately not validated, logged or kept
    // — it goes into one command and the send buffer is wiped.
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
        // Already subscribed: the rows are still in the model, so re-entering
        // the room shows them at once. A focused view never takes this path —
        // it is a different view of the same room.
        //
        // Asked anyway, and without clearing anything. The core has this same
        // case and answers it by keeping the rows and reporting where reading
        // stopped (`rebuilt: false`); returning here instead meant a freshly
        // built page never heard that, so opening at the first unread message
        // worked on the first visit to a room and silently did nothing on every
        // return to it - "sometimes it works", reported exactly that way.
        qInfo("xmatic: room already open, %d rows kept", m_timeline.count());
        QJsonObject kept;
        kept.insert(QStringLiteral("roomId"), roomId);
        kept.insert(QStringLiteral("receipts"),
                    m_settings && m_settings->showReadStatus());
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
        // Downloaded attachments are remembered under the timeline row's id,
        // and those ids are only unique within one timeline. The core now
        // prefixes them per room, so a stale entry can no longer be mistaken
        // for this room's — but nothing would ever drop them either, and a
        // long session would keep every attachment of every room it visited.
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
    send(QStringLiteral("timeline.open"), arguments);

    // Pull this room's keys out of the backup as well. Messages that predate
    // this device can only be read that way, and the timeline retries the
    // decryption once the keys land.
    if (m_encryptionStatus.value(QStringLiteral("backupEnabled")).toBool()) {
        fetchRoomKeys(roomId);
    }
}

void MatrixBridge::closeRoom()
{
    if (m_openRoomId.isEmpty()) {
        return;
    }
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
    send(QStringLiteral("timeline.close"));
}

void MatrixBridge::loadOlder()
{
    // One at a time. The core serialises paginations on the open timeline
    // anyway, and a second request in flight would make the first one's answer
    // impossible to attribute — which is what the automatic fill needs it for.
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
    // Reopening the same thread has to be told apart from the one before it:
    // the previous core task is aborted at its next await, so a diff of its
    // making can still be in the Qt queue when this one's reset has landed.
    // The root alone does not distinguish those two.
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
    // Named, because commands run as independent tasks: a close and an open
    // issued back to back can reach the core in either order, and an
    // unqualified close would shut down the thread just opened.
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

    // The name is built from the reaction, the way every emoji set names its
    // files: code points in hex, joined by a dash. A variation selector is
    // dropped unless the sequence is a joined one, which is the rule the sets
    // themselves follow.
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

    // The second name is the first with every variation selector gone. Sets
    // agree on the rule above but not on every sequence: one of them names the
    // eye in a speech bubble that way, and a name that is merely spelled
    // differently should not read as "no picture".
    QStringList names;
    names.append(parts.join(QLatin1Char('-')));
    if (bare != parts) {
        names.append(bare.join(QLatin1Char('-')));
    }

    QString found;
    // A set that was read in through the app is served by the provider, which
    // checks each picture against the checksum taken when it was written. A
    // set somebody copied into the directory by hand has no checksums to check
    // against and is opened as a plain file, exactly as before - that path is
    // unchecked, and the setting's own text says what it costs.
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
        // PNG only. A hand-copied set has no checksums, and handing an SVG
        // from outside to Qt 5.6's parser at display time - once per row, per
        // redraw - is the one thing this feature must not do. Reading a set in
        // through Appearance rasterises it and lifts this restriction.
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
    // The registers that remember what a command was about. Emptied on the
    // reply, on the error, and - since a dropped answer is neither - when the
    // watchdog gives the command up.
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
    // The same question the room's own mark-read asks. It used to go without,
    // and the core then sent the public receipt unconditionally - so the
    // privacy switch held while reading a room and was ignored by the entry in
    // the chat list, which is the one that exists precisely so a room need not
    // be opened. A setting that a second path does not ask is worse than none.
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
    // Reading without telling anybody is a choice the privacy page offers, and
    // it governs the receipt others see - not the fully-read marker, which is
    // private account data. Holding that one back as well hid nothing from
    // anybody and cost this device the line in the conversation and the
    // position the room opens at: both follow the marker.
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
    // Reading a set in, or throwing it away, changes what every lookup should
    // answer - and the answers are cached. Both signals may come from the
    // image thread, hence queued.
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

void MatrixBridge::recoverKeys(const QString &key)
{
    if (key.trimmed().isEmpty()) {
        setLastError(tr("Enter your recovery key first."));
        return;
    }
    setLastError(QString());

    QJsonObject arguments;
    arguments.insert(QStringLiteral("key"), key.trimmed());
    // Same treatment as the login password: the recovery key unlocks the whole
    // key backup, so the serialised payload must not stay on the heap after the
    // core has taken its copy. What cannot be wiped is documented with the
    // password's residual in docs/PASSWORD-LOGIN.md.
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

void MatrixBridge::sendMedia(const QString &path, const QString &mimeType,
                             const QString &caption, const QString &replyTo,
                             qint64 voiceDuration)
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
    arguments.insert(QStringLiteral("path"), local);
    arguments.insert(QStringLiteral("mimeType"),
                     mimeType.isEmpty() ? QStringLiteral("application/octet-stream") : mimeType);
    arguments.insert(QStringLiteral("caption"), caption);
    arguments.insert(QStringLiteral("replyTo"), replyTo);
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
                                 const QString &mimeType)
{
    if (roomId.isEmpty()) {
        return;
    }

    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }

    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("body"), body);
    arguments.insert(QStringLiteral("path"), local);
    arguments.insert(QStringLiteral("mimeType"), mimeType);
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
        // Remembered is not the same as still there: the core sweeps the media
        // cache back to its budget, oldest first, and a long session in a
        // picture-heavy room crosses that. Handing out the path anyway left an
        // empty frame with a spinner over it and no second attempt for the rest
        // of the visit.
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
        // The same check the attachment path makes: the cache sweep may have
        // taken the file, and a picture that is gone must be asked for again
        // rather than handed out as a path to nothing.
        if (QFile::exists(known)) {
            emit mediaReady(url, known);
            return;
        }
        m_media.remove(url);
    }

    if (m_mediaRequests.values().contains(url)) {
        return;
    }

    // The SDK's media source is a struct, not a bare address: the unencrypted
    // variant is `{"url": …}`, the encrypted one carries a `file` object
    // instead. A profile picture is never encrypted, so the former it is.
    QJsonObject source;
    source.insert(QStringLiteral("url"), url);

    QJsonObject arguments;
    arguments.insert(QStringLiteral("source"), source);
    arguments.insert(QStringLiteral("thumbnail"), true);

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

    // The other half of the stall report: how long it took in the end.
    // Without it the journal says a command hung and never says whether it
    // came back, which is the difference between slow and broken.
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
        if (command == QLatin1String("thread.open")
            || command == QLatin1String("thread.paginate")
            || command == QLatin1String("thread.send")) {
            emit threadFailed(error);
        }
        // A failed media fetch has to say so, or the row keeps a spinner
        // turning over a download that will never arrive - including the ones
        // this app refuses on purpose, like an oversized attachment.
        if (m_mediaRequests.contains(id)) {
            emit mediaFailed(m_mediaRequests.value(id));
        }
        forgetRequest(id);
        if (entry.command == QLatin1String("private.set")) {
            // The local copy was updated before the round trip; a refused
            // write must not leave the caller looking allowed.
            send(QStringLiteral("private.get"));
        }
        // The recording stays where it is: the send failed, and it is the only
        // copy. `forgetRequest` above has already let go of the bookkeeping.
        // "command not understood" means the two halves of the app do not
        // match - a stale core against a newer bridge. The list of every
        // command it does know belongs in the journal, not in a banner.
        if (error.startsWith(QLatin1String("command not understood"))) {
            qWarning("xmatic: %s rejected by the core: %s",
                     qPrintable(command), qPrintable(error.left(120)));
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
    if (replyTimeline(id, command, data)) {
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
        // A server without OAuth that offers the classic password flow
        // answers with a flag instead of a browser URL; the login page
        // then shows the credentials form. Not busy while the user types.
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
        // The mode writes a push rule, which the SDK's room list does not
        // consider a notable change, so no diff follows and the row would
        // keep the old state until something else touched the room —
        // entering it, for instance. Write it into the models directly.
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
        // Same reason as the notify mode above: the receipt may not come back
        // as a diff, and the badge would stand until something else touched
        // the room.
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

    // No diff follows core-made state changes, and the local store only
    // learns of the state event on a later sync — so the result is
    // written into the model and handed to the page as data, never as a
    // hint to re-read.
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

        // Whether the file could be read at all. Locked or damaged is not
        // empty, and nothing may be written over it - the store key is bound
        // to the device lock and is missing after every reboot until the
        // system dialog has run.
        m_privateListsReadable = data.value(QStringLiteral("readable")).toBool();

        // Anything the plain settings file still holds moves over once, and is
        // removed there: a list of names does not belong in a file that any
        // backup copies in the clear. Only where the encrypted file was
        // readable - otherwise the move would delete the only copy.
        if (command == QLatin1String("private.get") && m_settings && m_privateListsReadable) {
            const QStringList callers = m_settings->legacyAllowedCallers();
            const QStringList trusted = m_settings->legacyTrustedRecipients();
            if (!callers.isEmpty() || !trusted.isEmpty()) {
                // Both lists in one write. Two commands would be two
                // read-modify-writes racing each other, and the second would
                // overwrite the first - on the very migration that then
                // deletes the plaintext original.
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
        // The same reason the chat list's own "mark read" writes into the
        // model: a receipt is not necessarily answered with a room-list diff,
        // and the badge then stood on a room that had just been read to the
        // end - which is how "it does not mark as read on its own" was
        // reported. Only where the core says it marked something: a room whose
        // newest event this device never saw has nothing to point a marker at.
        if (data.value(QStringLiteral("read")).toBool()) {
            const QString roomId = data.value(QStringLiteral("roomId")).toString();
            m_rooms.clearUnread(roomId);
            m_spaceRooms.clearUnread(roomId);
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
            // Both come back from the core rather than being remembered here:
            // one message can carry several reactions, and the answer has to
            // say which of them it is about.
            emit reactorsReady(data.value(QStringLiteral("eventId")).toString(),
                               data.value(QStringLiteral("key")).toString(),
                               data.value(QStringLiteral("reactors")).toArray().toVariantList());
        }
        return true;
    }

    if (command == QLatin1String("timeline.open")) {
        setTimelineReady(true);
        // What this account may do in the room that was just opened. Menus
        // ask this instead of offering everything and letting the server
        // refuse - a menu entry that always fails is a dead end, and this app
        // does not build those.
        // A rebuilt timeline is a different room (or a fresh view of this
        // one), and the core asks the server for its thread roots as part of
        // building it. A kept timeline keeps its roots: the answer would not
        // come a second time, and clearing them would take the markers away.
        if (data.value(QStringLiteral("rebuilt")).toBool()) {
            m_timeline.setThreadRoots(QHash<QString, int>());
        }
        const QVariantMap can = data.value(QStringLiteral("can")).toObject().toVariantMap();
        if (can != m_roomPermissions) {
            m_roomPermissions = can;
            emit roomPermissionsChanged();
        }
        // Only for the live view. A focused open - the pinned overview, a
        // permalink - shows a different slice of the room, and the room's
        // read marker means nothing in it.
        if (m_timelineFocus.isEmpty()) {
            emit timelineOpened(data.value(QStringLiteral("readMarker")).toString(),
                                data.value(QStringLiteral("rebuilt")).toBool());
        }
        return true;
    }

    if (command == QLatin1String("timeline.paginate")) {
        setTimelineAtStart(data.value(QStringLiteral("reachedStart")).toBool());
        // The count is the model's at this moment, not the page's yield —
        // the rows of this pagination may still be on their way through the
        // diff stream. Two of these lines with the same count and no diff
        // between them is what a fruitless round looks like.
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
        // The invitation still raised the server's notification counter, and
        // the chat list turns that into a banner. Refusing the call and then
        // beeping about it is the opposite of what the setting promises.
        const QString roomId = data.value(QStringLiteral("roomId")).toString();
        if (!roomId.isEmpty()) {
            // Short, and once per room per minute. Longer or repeatable turns
            // a refused call into a way of silencing a room's messages: the
            // caller chooses when to send, and every invitation would extend
            // the silence.
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
    } else if (name == QLatin1String("encryption.changed")) {
        m_encryptionStatus = data.toVariantMap();
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
        // Late diffs from a room that was closed in the meantime are dropped.
        if (data.value(QStringLiteral("roomId")).toString() == m_openRoomId) {
            const QJsonArray ops = data.value(QStringLiteral("ops")).toArray();
            m_timeline.applyOperations(ops);

            // Operation names and the resulting row count, no content. A
            // timeline that empties itself is otherwise indistinguishable from
            // a rendering fault.
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
        // The server's list of this room's thread roots. It is the way into a
        // thread whose root the store knows without a summary - the case every
        // room is in after an update, because every event cached before
        // threading was switched on lacks one.
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
            // Only worth a line when the room names pins that cannot be read:
            // the pinned view is then empty for a reason outside this app, and
            // that reason is otherwise invisible.
            const int loaded = data.value(QStringLiteral("loaded")).toInt();
            // Against what the core actually looked at, not against the room's
            // whole list: the check is capped, so comparing with the full count
            // reported a failure in every room with more pins than the cap.
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
    if (name == QLatin1String("session.expired")) {
        // Emitted by the core and, until now, understood by nobody: the room
        // list then sat there empty and quietly not syncing.
        qWarning("xmatic: the session expired");
        applySession(QJsonObject { { QStringLiteral("state"), QStringLiteral("none") } });
        setLastError(tr("Your session has ended. Please sign in again."));
        return true;
    }

    if (name == QLatin1String("sync.support")) {
        // A server that cannot do this app's sync fails every sync, which the
        // offline mode turns into "offline" plus a restart loop — a flashing
        // banner over an empty room list that reads as a network fault. Say
        // what it is instead.
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
    } else if (name == QLatin1String("session.changed")) {
        setLoginRunning(false);
        if (data.value(QStringLiteral("state")).toString() != QLatin1String("signed-in")) {
            m_rooms.clear();
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
    // A message the queue gave up on is the one thing in a diff worth a line
    // of its own: it stays in the room until the user acts, and "not sent" is
    // all the screen can say about it. The core scrubbed the reason already.
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

        // The banner follows the notifying events, counted client-side
        // against the account's push rules — `unread` counts everything, so
        // a room set to "mentions only" (here or in any other client) kept
        // notifying for every message. The unread badge deliberately keeps
        // counting everything; only the banner is governed by the rules.
        const int notifications = room.value(QStringLiteral("notifications")).toInt();
        const int previous = m_notified.value(id, notifications);
        m_notified.insert(id, notifications);

        // Reading the room somewhere else clears the server's counter for
        // this device too, and the count arrives here as any other change.
        // Reported before the mute and on-screen tests below: those decide
        // whether a room may *raise* a banner, not whether a banner that
        // stands may go.
        if (notifications == 0 && previous > 0) {
            // A banner still waiting for its text is pointless now: the room
            // was read somewhere else, and what it would announce is gone.
            m_pendingBanners.remove(id);
            emit roomRead(id);
        }

        // Muted and low-priority rooms still count their unread badge but
        // never raise a banner — that is the whole point of both.
        //
        // Muting has to be honoured here and not only on the server: the mute
        // is a push rule, and push rules govern the notifications the server
        // pushes out. This app has no push service, it raises its own banners
        // from the unread counts, so a rule the server never gets to apply
        // would leave a muted room notifying exactly as before.
        if (room.value(QStringLiteral("muted")).toBool()
            || room.value(QStringLiteral("lowPriority")).toBool()) {
            continue;
        }

        // The room on screen is being read right now, so it never notifies —
        // but only while it really is on screen and the app really is in front.
        // This used to test the *open* room, which is the room whose timeline
        // the core still has subscribed: the subscription deliberately outlives
        // the page, so the last room visited went silent for good. That is
        // precisely the room someone tests with, which is how it was found.
        const bool onScreen = !m_visibleRoomId.isEmpty() && id == m_visibleRoomId
                && QGuiApplication::applicationState() == Qt::ApplicationActive;
        // The room's latest event, by the timestamp that travels with it. The
        // preview in this very object belongs to that event, so a timestamp
        // that moved is the proof that the text is the new message's and not
        // its predecessor's.
        const quint64 timestamp = static_cast<quint64>(
            room.value(QStringLiteral("timestamp")).toDouble());

        const auto waiting = m_pendingBanners.find(id);
        if (waiting != m_pendingBanners.end() && timestamp > waiting->timestamp) {
            const PendingBanner ready = *waiting;
            m_pendingBanners.erase(waiting);
            publishBanner(id, ready, room);
        }

        // The count shown is the notifying one too: in a mentions-only room
        // "50 new messages" over one lone mention would point at the wrong
        // thing.
        // A call this device refused a moment ago: its own counter rise is
        // what would ring here.
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

/// One banner, from what was known when the counts rose plus whatever text the
/// room carries now. An empty `room` is the timeout's way of saying "no text
/// arrived" — the count line then stands on its own.
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

    // A login creates the stores (and with them the encryption marker), a
    // sign-out deletes them again. Both change the answer, and neither sends a
    // diff of its own.
    refreshStorageStatus();

    emit sessionChanged();
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
