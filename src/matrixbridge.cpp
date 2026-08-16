#include "matrixbridge.h"

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
                           QObject *parent)
    : QObject(parent)
{
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

    m_recorder = new VoiceRecorder(cacheDirectory + QStringLiteral("/voice"), this);
    connect(m_recorder, &VoiceRecorder::finished, this, [this](const QString &path,
                                                               const QString &mimeType) {
        sendMedia(path, mimeType);
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
}

void MatrixBridge::restoreSession()
{
    send(QStringLiteral("session.restore"));
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
    send(QStringLiteral("logout"));
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

// Sailjail only lets the app write inside its own config directory
// (AppConfigLocation → ~/.config/org.xmatic/xmatic). QSettings' UserScope path
// sits one level above that and the sandbox blocks it silently, so the file is
// placed explicitly inside the allowed directory.
static QString settingsPath()
{
    return QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation)
           + QStringLiteral("/settings.conf");
}

QString MatrixBridge::startPage() const
{
    QSettings settings(settingsPath(), QSettings::IniFormat);
    const QString page = settings.value(QStringLiteral("ui/startPage"),
                                        QStringLiteral("rooms")).toString();
    return page == QLatin1String("spaces") ? page : QStringLiteral("rooms");
}

void MatrixBridge::setStartPage(const QString &page)
{
    const QString wanted = page == QLatin1String("spaces") ? QStringLiteral("spaces")
                                                           : QStringLiteral("rooms");
    if (wanted == startPage()) {
        return;
    }
    const QString path = settingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSettings settings(path, QSettings::IniFormat);
    settings.setValue(QStringLiteral("ui/startPage"), wanted);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save the start page (status %d)",
                 static_cast<int>(settings.status()));
    } else {
        qInfo("xmatic: start page set to %s", qPrintable(wanted));
    }
    emit startPageChanged();
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
        qInfo("xmatic: room already open, %d rows kept", m_timeline.count());
        return;
    }

    qInfo("xmatic: opening a room%s", focus.isEmpty() ? "" : " (focused)");
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

void MatrixBridge::checkRecipients(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    send(QStringLiteral("room.checkRecipients"), arguments);
}

bool MatrixBridge::isRecipientTrusted(const QString &userId) const
{
    QSettings settings(settingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("security/trustedRecipients"))
        .toStringList()
        .contains(userId);
}

void MatrixBridge::trustRecipient(const QString &userId)
{
    const QString path = settingsPath();
    QSettings settings(path, QSettings::IniFormat);
    QStringList trusted =
        settings.value(QStringLiteral("security/trustedRecipients")).toStringList();
    if (trusted.contains(userId)) {
        return;
    }
    trusted.append(userId);
    settings.setValue(QStringLiteral("security/trustedRecipients"), trusted);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not persist the trusted recipient");
    }
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
static QString normalizedServerName(const QString &server)
{
    QString name = server.trimmed().toLower();
    if (name.startsWith(QLatin1String("https://"))) {
        name = name.mid(8);
    } else if (name.startsWith(QLatin1String("http://"))) {
        name = name.mid(7);
    }
    while (name.endsWith(QLatin1Char('/'))) {
        name.chop(1);
    }
    return name;
}

QString MatrixBridge::directoryServer() const
{
    QSettings settings(settingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("directory/server")).toString();
}

void MatrixBridge::setDirectoryServer(const QString &server)
{
    const QString wanted = normalizedServerName(server);
    if (wanted == directoryServer()) {
        return;
    }
    const QString path = settingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSettings settings(path, QSettings::IniFormat);
    settings.setValue(QStringLiteral("directory/server"), wanted);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save the directory server (status %d)",
                 static_cast<int>(settings.status()));
    }
    emit directoryServerChanged();
}

QStringList MatrixBridge::directoryServers() const
{
    QSettings settings(settingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("directory/servers")).toStringList();
}

void MatrixBridge::addDirectoryServer(const QString &server)
{
    const QString name = normalizedServerName(server);
    if (name.isEmpty()) {
        return;
    }
    QStringList servers = directoryServers();
    if (servers.contains(name)) {
        return;
    }
    servers.append(name);
    const QString path = settingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSettings settings(path, QSettings::IniFormat);
    settings.setValue(QStringLiteral("directory/servers"), servers);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save the directory servers (status %d)",
                 static_cast<int>(settings.status()));
    }
    emit directoryServersChanged();
}

void MatrixBridge::removeDirectoryServer(const QString &server)
{
    const QString name = normalizedServerName(server);
    QStringList servers = directoryServers();
    if (servers.removeAll(name) == 0) {
        return;
    }
    QSettings settings(settingsPath(), QSettings::IniFormat);
    settings.setValue(QStringLiteral("directory/servers"), servers);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save the directory servers (status %d)",
                 static_cast<int>(settings.status()));
    }
    emit directoryServersChanged();
}

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
    send(QStringLiteral("timeline.markRead"));
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

void MatrixBridge::createRoom(const QString &name, bool encrypted, bool isPublic)
{
    if (name.trimmed().isEmpty()) {
        return;
    }
    setLastError(QString());
    QJsonObject arguments;
    arguments.insert(QStringLiteral("name"), name.trimmed());
    arguments.insert(QStringLiteral("encrypted"), encrypted);
    arguments.insert(QStringLiteral("public"), isPublic);
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

void MatrixBridge::recoverKeys(const QString &key)
{
    if (key.trimmed().isEmpty()) {
        setLastError(tr("Enter your recovery key first."));
        return;
    }
    setLastError(QString());

    QJsonObject arguments;
    arguments.insert(QStringLiteral("key"), key.trimmed());
    send(QStringLiteral("encryption.recover"), arguments);
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

void MatrixBridge::deleteMessage(const QString &eventId)
{
    if (eventId.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    send(QStringLiteral("timeline.redact"), arguments);
}

void MatrixBridge::sendMedia(const QString &path, const QString &mimeType)
{
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
    send(QStringLiteral("timeline.sendMedia"), arguments);
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

void MatrixBridge::requestMedia(const QString &key, const QVariant &source, bool thumbnail)
{
    if (key.isEmpty() || !source.isValid()) {
        return;
    }

    const QString known = m_media.value(key);
    if (!known.isEmpty()) {
        emit mediaReady(key, known);
        return;
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
        emit mediaReady(url, known);
        return;
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
            m_mediaRequests.remove(id);
            m_hierarchyRequests.remove(id);
            m_removeRequests.remove(id);
            setLastError(error);
            return;
        }

        const QJsonObject data = message.value(QStringLiteral("data")).toObject();

        if (command == QLatin1String("login.registrationUrl")) {
            const QString url = data.value(QStringLiteral("url")).toString();
            if (!url.isEmpty()) {
                emit registrationUrlReady(url);
            }
            return;
        }

        if (command == QLatin1String("account.get")) {
            const QString name = data.value(QStringLiteral("displayName")).toString();
            const QString avatar = data.value(QStringLiteral("avatarUrl")).toString();
            if (name != m_profileName || avatar != m_profileAvatar) {
                m_profileName = name;
                m_profileAvatar = avatar;
                emit profileChanged();
            }
            return;
        }

        if (command == QLatin1String("space.hierarchy")) {
            const QString spaceId = m_hierarchyRequests.take(id);
            emit spaceHierarchyReady(spaceId,
                                     data.value(QStringLiteral("rooms")).toArray().toVariantList());
            return;
        }

        if (command == QLatin1String("room.checkRecipients")) {
            emit recipientsChecked(data.value(QStringLiteral("roomId")).toString(),
                                   data.value(QStringLiteral("users")).toArray().toVariantList());
            return;
        }

        if (command == QLatin1String("login.start")) {
            // A server without OAuth that offers the classic password flow
            // answers with a flag instead of a browser URL; the login page
            // then shows the credentials form. Not busy while the user types.
            if (data.value(QStringLiteral("passwordLogin")).toBool()) {
                setLoginRunning(false);
                emit passwordLoginNeeded();
                return;
            }
            const QString url = data.value(QStringLiteral("url")).toString();
            if (url.isEmpty()) {
                setLoginRunning(false);
                emit loginFailed(tr("The homeserver did not return a login page."));
            } else {
                emit loginUrlReady(url);
            }
            return;
        }

        if (command == QLatin1String("login.password")) {
            // The session.changed event that follows this reply carries the
            // whole outcome; nothing to do here.
            return;
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
            return;
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
            return;
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
            return;
        }

        if (command == QLatin1String("room.directChat")) {
            emit directChatReady(data.value(QStringLiteral("roomId")).toString());
            return;
        }

        if (command == QLatin1String("room.followSuccessor")) {
            emit successorReady(data.value(QStringLiteral("roomId")).toString());
            return;
        }

        if (command == QLatin1String("room.info")) {
            emit roomInfoReady(data.toVariantMap());
            return;
        }

        if (command == QLatin1String("room.create")) {
            emit roomCreated(data.value(QStringLiteral("roomId")).toString(),
                             data.value(QStringLiteral("name")).toString(),
                             data.value(QStringLiteral("encrypted")).toBool());
            return;
        }

        if (command == QLatin1String("member.remove")) {
            m_members.removeUser(m_removeRequests.take(id));
            return;
        }

        if (command == QLatin1String("media.fetch")) {
            const QString key = m_mediaRequests.take(id);
            const QString path = data.value(QStringLiteral("path")).toString();
            if (!key.isEmpty() && !path.isEmpty()) {
                m_media.insert(key, path);
                emit mediaReady(key, path);
            }
            return;
        }

        if (command == QLatin1String("encryption.status")
                || command == QLatin1String("encryption.recover")) {
            m_encryptionStatus = data.toVariantMap();
            emit encryptionChanged();
            return;
        }

        if (command == QLatin1String("encryption.enableBackup")) {
            emit recoveryKeyReady(data.value(QStringLiteral("recoveryKey")).toString());
            return;
        }

        if (command == QLatin1String("timeline.open")) {
            setTimelineReady(true);
            return;
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
            return;
        }

        if (data.contains(QStringLiteral("state"))) {
            applySession(data);
        }
        return;
    }

    if (type != QLatin1String("event")) {
        return;
    }

    const QString name = message.value(QStringLiteral("event")).toString();
    const QJsonObject data = message.value(QStringLiteral("data")).toObject();

    if (name == QLatin1String("call.invite")) {
        m_calls->onRemoteInvite(data.value(QStringLiteral("roomId")).toString(),
                                data.value(QStringLiteral("sender")).toString(),
                                data.value(QStringLiteral("callId")).toString(),
                                data.value(QStringLiteral("sdp")).toString());
    } else if (name == QLatin1String("call.answer")) {
        m_calls->onRemoteAnswer(data.value(QStringLiteral("callId")).toString(),
                                data.value(QStringLiteral("sdp")).toString());
    } else if (name == QLatin1String("call.candidates")) {
        m_calls->onRemoteCandidates(data.value(QStringLiteral("callId")).toString(),
                                    data.value(QStringLiteral("candidates")).toArray().toVariantList());
    } else if (name == QLatin1String("call.hangup")) {
        m_calls->onRemoteHangup(data.value(QStringLiteral("callId")).toString());
    } else if (name == QLatin1String("verification.request")) {
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
    } else if (name == QLatin1String("timeline.diff")) {
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
        }
    } else if (name == QLatin1String("roomlist.diff")) {
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
            if (loaded < ids.count()) {
                const QString loadError =
                    data.value(QStringLiteral("loadError")).toString();
                qWarning("xmatic: %d pinned, only %d readable, first failure: %s",
                         ids.count(), loaded, qPrintable(loadError));
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
        // The count shown is the notifying one too: in a mentions-only room
        // "50 new messages" over one lone mention would point at the wrong
        // thing.
        if (notifications > previous && !onScreen) {
            emit roomActivity(id,
                              room.value(QStringLiteral("name")).toString(),
                              notifications,
                              room.value(QStringLiteral("mentions")).toInt());
        }
    }
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
