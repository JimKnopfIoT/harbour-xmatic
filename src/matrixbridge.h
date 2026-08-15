#ifndef MATRIXBRIDGE_H
#define MATRIXBRIDGE_H

#include <QElapsedTimer>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QStandardPaths>
#include <QObject>
#include <QTimer>
#include <QVariantList>
#include <QVariant>
#include <QVariantMap>
#include <QString>

#include "roomlistmodel.h"
#include "roomsortmodel.h"
#include "directorymodel.h"
#include "membermodel.h"
#include "callengine.h"
#include "voicerecorder.h"
#include "timelinemodel.h"
#include "xmatic_core.h"

/// The single point of contact between QML and the Rust core.
///
/// Commands are JSON objects with a running id; replies and events come back
/// through one callback that fires on a core worker thread. The trampoline
/// therefore does nothing but re-post the message into the Qt event loop, so
/// everything below runs on the UI thread.
class MatrixBridge : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QString coreVersion READ coreVersion CONSTANT)
    Q_PROPERTY(QString sessionState READ sessionState NOTIFY sessionChanged)
    Q_PROPERTY(QString syncState READ syncState NOTIFY syncStateChanged)
    Q_PROPERTY(QString userId READ userId NOTIFY sessionChanged)
    Q_PROPERTY(QString deviceId READ deviceId NOTIFY sessionChanged)
    Q_PROPERTY(bool ready READ ready CONSTANT)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    Q_PROPERTY(bool paginating READ paginating NOTIFY paginatingChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
    Q_PROPERTY(QObject *rooms READ rooms CONSTANT)
    Q_PROPERTY(QObject *spaces READ spaces CONSTANT)
    Q_PROPERTY(QObject *spaceRooms READ spaceRooms CONSTANT)
    Q_PROPERTY(QString startPage READ startPage WRITE setStartPage NOTIFY startPageChanged)
    Q_PROPERTY(int spaceCounts READ spaceCounts NOTIFY spaceCountsChanged)
    Q_PROPERTY(QObject *timeline READ timeline CONSTANT)
    Q_PROPERTY(QObject *recorder READ recorder CONSTANT)
    Q_PROPERTY(QObject *calls READ calls CONSTANT)
    Q_PROPERTY(QString openRoomId READ openRoomId NOTIFY openRoomChanged)
    Q_PROPERTY(QStringList pinnedEventIds READ pinnedEventIds NOTIFY pinnedChanged)
    Q_PROPERTY(QString pinnedPreview READ pinnedPreview NOTIFY pinnedChanged)
    Q_PROPERTY(bool roomReplaced READ roomReplaced NOTIFY tombstoneChanged)
    Q_PROPERTY(QString replacementReason READ replacementReason NOTIFY tombstoneChanged)
    Q_PROPERTY(bool replacementJoined READ replacementJoined NOTIFY tombstoneChanged)
    Q_PROPERTY(bool timelineAtStart READ timelineAtStart NOTIFY timelineAtStartChanged)
    Q_PROPERTY(bool timelineReady READ timelineReady NOTIFY timelineReadyChanged)
    Q_PROPERTY(QString verificationState READ verificationState NOTIFY verificationChanged)
    Q_PROPERTY(QString verificationUser READ verificationUser NOTIFY verificationChanged)
    Q_PROPERTY(bool verificationIsSelf READ verificationIsSelf NOTIFY verificationChanged)
    Q_PROPERTY(bool verificationWeStarted READ verificationWeStarted NOTIFY verificationChanged)
    Q_PROPERTY(QVariantList verificationEmoji READ verificationEmoji NOTIFY verificationChanged)
    Q_PROPERTY(QVariantMap encryptionStatus READ encryptionStatus NOTIFY encryptionChanged)
    Q_PROPERTY(QString profileName READ profileName NOTIFY profileChanged)
    Q_PROPERTY(QString profileAvatar READ profileAvatar NOTIFY profileChanged)
    Q_PROPERTY(QObject *directory READ directory CONSTANT)
    Q_PROPERTY(bool directoryAtEnd READ directoryAtEnd NOTIFY directoryStateChanged)
    Q_PROPERTY(QString directoryServer READ directoryServer WRITE setDirectoryServer
               NOTIFY directoryServerChanged)
    Q_PROPERTY(QStringList directoryServers READ directoryServers NOTIFY directoryServersChanged)
    Q_PROPERTY(QObject *members READ members CONSTANT)

public:
    /// `storeKey` is the base64 store key from Sailfish Secrets, or empty
    /// when the secrets service is unavailable — the core then runs with
    /// unencrypted stores. It goes into the core's config and nowhere else.
    explicit MatrixBridge(const QString &dataDirectory,
                          const QString &cacheDirectory,
                          const QString &storeKey = QString(),
                          QObject *parent = nullptr);
    ~MatrixBridge() override;

    QString coreVersion() const { return m_coreVersion; }
    QString sessionState() const { return m_sessionState; }

    /// One of "idle", "running", "offline", "terminated", "error". "offline"
    /// means the core noticed the network is gone and reconnects on its own.
    QString syncState() const { return m_syncState; }
    QString userId() const { return m_userId; }
    QString deviceId() const { return m_deviceId; }
    bool ready() const { return m_core != nullptr; }
    bool busy() const { return !m_pending.isEmpty() || m_loginRunning; }

    /// Whether a request for older messages is in flight.
    ///
    /// Deliberately separate from `busy`. `busy` means "some command, any
    /// command, is waiting for an answer" — an avatar thumbnail counts — and
    /// hanging the timeline's controls off that made them dead whenever the
    /// homeserver was slow with something entirely unrelated.
    bool paginating() const { return m_paginateId != 0; }
    QString lastError() const { return m_lastError; }
    // The UI sees the grouped view (favourites up, low priority down); the
    // core still drives the flat source underneath.
    QObject *rooms() { return &m_roomsSorted; }
    QObject *spaces() { return &m_spaces; }
    QObject *spaceRooms() { return &m_spaceRooms; }
    int spaceCounts() const { return m_spaceCountsRevision; }
    QObject *timeline() { return &m_timeline; }
    QObject *recorder() { return m_recorder; }
    QObject *calls() { return m_calls; }
    QString openRoomId() const { return m_openRoomId; }

    /// Event ids of the open room's pinned messages, for the banner and the
    /// per-row markers.
    QStringList pinnedEventIds() const { return m_pinnedEventIds; }

    /// Body of the newest pinned message; empty when there is nothing usable.
    QString pinnedPreview() const { return m_pinnedPreview; }

    /// Whether the open room was upgraded away — replaced by a newer room that
    /// carries the conversation on. Such a room keeps its history but takes no
    /// new messages, which is invisible without saying so.
    bool roomReplaced() const { return !m_successorRoomId.isEmpty(); }

    /// The reason the tombstone gives, if any. Free text written by whoever
    /// upgraded the room.
    QString replacementReason() const { return m_replacementReason; }

    /// Whether the replacement room is one this account already joined — then
    /// following the upgrade is navigation, not a join.
    bool replacementJoined() const { return m_replacementJoined; }
    bool timelineAtStart() const { return m_timelineAtStart; }

    /// False between asking for a room's timeline and its first batch.
    bool timelineReady() const { return m_timelineReady; }

    /// One of "none", "requested", "comparing", "done", "cancelled".
    QString verificationState() const { return m_verificationState; }
    QString verificationUser() const { return m_verificationUser; }
    bool verificationIsSelf() const { return m_verificationIsSelf; }

    /// True when this side asked for the verification — it then waits for the
    /// other side to accept instead of offering accept/decline itself.
    bool verificationWeStarted() const { return m_verificationWeStarted; }
    QVariantList verificationEmoji() const { return m_verificationEmoji; }

    /// Members: recovery, backup, backupEnabled, backupOnServer, crossSigned.
    QVariantMap encryptionStatus() const { return m_encryptionStatus; }

    QString profileName() const { return m_profileName; }
    QString profileAvatar() const { return m_profileAvatar; }
    QObject *directory() { return &m_directory; }
    bool directoryAtEnd() const { return m_directoryAtEnd; }
    QObject *members() { return &m_members; }

    /// Looks for a persisted session and signs in with it if there is one.
    Q_INVOKABLE void restoreSession();

    /// Begins the browser login against `homeserver`, which may be a server
    /// name such as "matrix.org" or a full URL. On a server without OAuth
    /// that offers the classic password sign-in, the answer arrives as
    /// passwordLoginNeeded() instead of a browser URL.
    Q_INVOKABLE void startLogin(const QString &homeserver);

    /// Signs in with username and password, after startLogin() answered
    /// passwordLoginNeeded(). The password is passed straight through to the
    /// core and the send buffer is wiped; it is never stored or logged.
    Q_INVOKABLE void startPasswordLogin(const QString &homeserver,
                                        const QString &user,
                                        const QString &password);

    /// Begins the device-code login against `homeserver`: the answer arrives
    /// as deviceCodeReady() with a URL and a code to show, and the sign-in
    /// itself happens in a browser on any other device.
    Q_INVOKABLE void startDeviceCodeLogin(const QString &homeserver);

    /// Asks where an account can be created on `homeserver`; the answer
    /// arrives as registrationUrlReady().
    Q_INVOKABLE void requestRegistrationUrl(const QString &homeserver);

    /// Cancels a login that is waiting for the browser.
    Q_INVOKABLE void abortLogin();

    /// Ends the session and forgets the stored credentials.
    Q_INVOKABLE void logout();

    /// Starts syncing and streaming the room list.
    Q_INVOKABLE void startRoomList();

    /// Stops syncing.
    Q_INVOKABLE void stopRoomList();

    /// Narrows the room list; an empty pattern shows everything again.
    Q_INVOKABLE void setRoomFilter(const QString &pattern);

    /// Starts streaming the joined spaces into the `spaces` model.
    Q_INVOKABLE void startSpaces();

    /// Stops streaming the space list.
    Q_INVOKABLE void stopSpaces();

    /// Streams one space's rooms into the `spaceRooms` model. Only one space
    /// is open at a time; opening another replaces it.
    Q_INVOKABLE void openSpace(const QString &roomId);

    /// Stops streaming the open space's rooms.
    Q_INVOKABLE void closeSpace();

    /// Creates a new, empty space with the given name.
    Q_INVOKABLE void createSpace(const QString &name);

    /// Leaves a space and forgets it — how a space is "deleted" client-side.
    Q_INVOKABLE void leaveSpace(const QString &roomId);

    /// Adds a room to a space.
    Q_INVOKABLE void addRoomToSpace(const QString &spaceId, const QString &roomId);

    /// Moves a room from one space to another: added to the target first, so a
    /// failure cannot leave the room in neither space.
    Q_INVOKABLE void moveRoomToSpace(const QString &fromSpaceId,
                                     const QString &toSpaceId,
                                     const QString &roomId);

    /// Removes a room from a space.
    Q_INVOKABLE void removeRoomFromSpace(const QString &spaceId, const QString &roomId);

    /// The badge text for a space in the overview: "(messages)", or
    /// "(subspaces/messages)" when the space has sub-spaces, or empty when
    /// there is nothing to show. Recomputed live as unread counts change.
    Q_INVOKABLE QString spaceBadge(const QString &spaceId) const;

    /// Opens a room's timeline. Only one is open at a time. `focus` selects
    /// the view: empty for the live timeline, "pinned" for the room's pinned
    /// messages, or an event id to show the history around one event.
    Q_INVOKABLE void openRoom(const QString &roomId, const QString &focus = QString());

    /// Pins a message of the open room, or unpins it.
    Q_INVOKABLE void pinMessage(const QString &eventId, bool pin);

    /// Follows a room upgrade: joins the room that replaced this one, if
    /// needed, and answers with `successorReady` so the UI can open it.
    Q_INVOKABLE void followSuccessor(const QString &roomId);

    /// Asks for everything the room-info page shows; the answer arrives as
    /// `roomInfoReady`. Read from local state, so it comes back at once.
    Q_INVOKABLE void loadRoomInfo(const QString &roomId);

    /// Mutes a room's notifications, or returns it to the default.
    Q_INVOKABLE void setRoomMuted(const QString &roomId, bool muted);

    /// Marks a room as a favourite, or clears the tag. Clears low priority.
    Q_INVOKABLE void setRoomFavourite(const QString &roomId, bool favourite);

    /// Marks a room as low priority, or clears the tag. Clears favourite. Low
    /// priority also stops the room from raising notifications.
    Q_INVOKABLE void setRoomLowPriority(const QString &roomId, bool lowPriority);

    /// Asks the core which recipients of an encrypted room still have
    /// unverified devices. The answer comes back as `recipientsChecked`.
    Q_INVOKABLE void checkRecipients(const QString &roomId);

    /// Whether the user chose to stop being warned about this recipient's
    /// unverified devices (the "remember for this user" tick).
    Q_INVOKABLE bool isRecipientTrusted(const QString &userId) const;

    /// Remembers that unverified devices of this recipient should no longer
    /// raise the pre-send warning. Persisted in the app config.
    Q_INVOKABLE void trustRecipient(const QString &userId);

    /// Turns on end-to-end encryption in a room; there is no way back, so the
    /// UI asks first.
    Q_INVOKABLE void enableEncryption(const QString &roomId);

    /// Asks for the account's display name and avatar; the answer lands in
    /// profileName / profileAvatar.
    Q_INVOKABLE void fetchProfile();

    /// Changes the display name. An empty name removes it.
    Q_INVOKABLE void setDisplayName(const QString &name);

    /// Uploads a picture and makes it the account's avatar.
    Q_INVOKABLE void setAvatarFile(const QString &path);

    /// Searches a public room directory; results stream into `directory`.
    /// An empty pattern lists the most popular rooms. An empty server means
    /// the own homeserver; any other is fetched over federation.
    Q_INVOKABLE void searchDirectory(const QString &pattern,
                                     const QString &server = QString());

    /// Loads the next page of the current directory search.
    Q_INVOKABLE void directoryLoadMore();

    /// Drops the directory search and empties the model.
    Q_INVOKABLE void stopDirectory();

    /// Loads a room's members into `members`. The list arrives as one reset.
    Q_INVOKABLE void loadMembers(const QString &roomId);

    /// Removes (kicks) a member from a room; the row disappears on success.
    Q_INVOKABLE void removeMember(const QString &roomId, const QString &userId);

    /// Asks the server for a space's linked children, including rooms the
    /// user has not joined; the answer arrives as spaceHierarchyReady().
    Q_INVOKABLE void fetchSpaceHierarchy(const QString &spaceId);

    /// Closes the open timeline.
    Q_INVOKABLE void closeRoom();

    /// Which room the user is actually looking at, or empty.
    ///
    /// Deliberately not the same as the open room. The core keeps one timeline
    /// subscription alive after leaving a room so stepping back into it is
    /// instant, so `openRoomId` names the room last visited — not the one on
    /// screen. Using it to suppress notifications made the last room visited
    /// stay silent for good, which is exactly the room someone tests with.
    Q_INVOKABLE void setVisibleRoom(const QString &roomId) { m_visibleRoomId = roomId; }

    /// Loads a page of older messages.
    Q_INVOKABLE void loadOlder();

    /// Sends a plain text message to the open room.
    Q_INVOKABLE void sendMessage(const QString &body);

    /// Sends a read receipt for the open room.
    Q_INVOKABLE void markRead();

    /// Accepts an invitation or joins a known room.
    Q_INVOKABLE void joinRoom(const QString &roomId);

    /// Joins a room by its address, for example "#room:server".
    Q_INVOKABLE void joinRoomByAlias(const QString &alias);

    /// Creates a room. A public room is listed in the server's directory, a
    /// private one is invite-only; encryption can only be decided here.
    Q_INVOKABLE void createRoom(const QString &name, bool encrypted, bool isPublic);

    /// Leaves a room and forgets it. On an invitation this declines it.
    Q_INVOKABLE void leaveRoom(const QString &roomId);

    /// Invites a user into a room.
    Q_INVOKABLE void inviteToRoom(const QString &roomId, const QString &userId);

    /// Opens, or creates, a direct chat with a user.
    Q_INVOKABLE void startDirectChat(const QString &userId);

    /// Asks a user to verify. An empty id means one's own other devices.
    Q_INVOKABLE void requestVerification(const QString &userId);

    /// Agrees to the verification request on screen.
    Q_INVOKABLE void acceptVerification();

    /// Confirms that the emoji match on both devices.
    Q_INVOKABLE void confirmVerification();

    /// Rejects the request or aborts the comparison.
    Q_INVOKABLE void cancelVerification();

    /// Forgets a finished verification so the page can close.
    Q_INVOKABLE void clearVerification();

    /// Asks the core for the state of key backup and recovery.
    Q_INVOKABLE void refreshEncryptionStatus();

    /// Unlocks the key backup with a recovery key or passphrase.
    Q_INVOKABLE void recoverKeys(const QString &key);

    /// Sets up key backup; the new recovery key arrives via recoveryKeyReady().
    Q_INVOKABLE void enableKeyBackup();

    /// Pulls one room's keys out of the backup.
    Q_INVOKABLE void fetchRoomKeys(const QString &roomId);

    /// Sends a reply to an earlier message.
    Q_INVOKABLE void replyToMessage(const QString &eventId, const QString &body);

    /// Replaces the body of a message that was already sent.
    Q_INVOKABLE void editMessage(const QString &eventId, const QString &body);

    /// Deletes a message.
    Q_INVOKABLE void deleteMessage(const QString &eventId);

    /// Sends a file from disk as an attachment.
    Q_INVOKABLE void sendMedia(const QString &path, const QString &mimeType);

    /// Downloads an attachment. The result arrives as mediaReady(key, path);
    /// an already downloaded file is reported immediately.
    Q_INVOKABLE void requestMedia(const QString &key, const QVariant &source, bool thumbnail);

    /// The local path of an attachment that was already downloaded, or empty.
    Q_INVOKABLE QString mediaPath(const QString &key) const { return m_media.value(key); }

    /// Downloads a profile picture, keyed by its own address.
    ///
    /// Separate from requestMedia for two reasons: an avatar is a bare MXC
    /// address rather than the media object an event carries, and it is always
    /// wanted as a thumbnail — a picture uploaded at full camera resolution
    /// would otherwise be decoded in every row of a list. Keying by the
    /// address is what makes one download serve every message of a sender.
    Q_INVOKABLE void requestAvatar(const QString &url);

    /// Sends a copy of a picture or a text to another room.
    Q_INVOKABLE void forwardToRoom(const QString &roomId,
                                   const QString &body,
                                   const QString &path,
                                   const QString &mimeType);

    /// The type of a file on disk, guessed from its name and content.
    ///
    /// A file picked in the app comes with its type from the picker, but one
    /// handed over by another application through the share dialog does not,
    /// and an attachment sent without a type is not shown as a picture by the
    /// receiving client.
    Q_INVOKABLE QString mimeTypeForPath(const QString &path) const;

    /// Copies a downloaded attachment into the user's picture folder.
    /// Returns the new path, or an empty string on failure.
    Q_INVOKABLE QString saveToPictures(const QString &path, const QString &suggestedName);

    /// Same, but into the download folder — for anything that is not a picture.
    Q_INVOKABLE QString saveToDownloads(const QString &path, const QString &suggestedName);

    /// Which page opens first after sign-in: "rooms" or "spaces". Persisted so
    /// the choice survives a restart.
    QString startPage() const;
    void setStartPage(const QString &page);

    /// The directory server the search page last used; empty means the own
    /// homeserver. Persisted so the choice survives a restart.
    QString directoryServer() const;
    void setDirectoryServer(const QString &server);

    /// Directory servers the user added, next to the built-in suggestions.
    QStringList directoryServers() const;
    Q_INVOKABLE void addDirectoryServer(const QString &server);
    Q_INVOKABLE void removeDirectoryServer(const QString &server);

signals:
    void startPageChanged();
    void spaceCountsChanged();

    void sessionChanged();
    void syncStateChanged();
    void busyChanged();
    void paginatingChanged();
    void lastErrorChanged();
    void openRoomChanged();
    void pinnedChanged();
    void tombstoneChanged();
    void timelineAtStartChanged();
    void timelineReadyChanged();
    void verificationChanged();
    void encryptionChanged();
    void profileChanged();
    void directoryStateChanged();
    void directoryServerChanged();
    void directoryServersChanged();

    /// A space's linked children as maps with id, name, topic, members,
    /// avatar, space and joined.
    void spaceHierarchyReady(const QString &spaceId, const QVariantList &rooms);

    /// The result of a `checkRecipients` call: the joined recipients of the
    /// room that still have unverified devices, as maps with userId, name and
    /// devices. Empty when there is nothing to warn about.
    void recipientsChecked(const QString &roomId, const QVariantList &users);

    /// The freshly created recovery key. Shown once, never stored.
    void recoveryKeyReady(const QString &key);

    /// A request for older messages came back. Carries no row count on
    /// purpose: the rows it fetched reach the model through the diff stream,
    /// which is a separate path from this reply and regularly arrives after it.
    /// Whether the timeline grew can only be judged from the model, one round
    /// later.
    void paginated();

    /// An attachment finished downloading.
    void mediaReady(const QString &key, const QString &path);

    /// A direct chat is ready to be opened.
    void directChatReady(const QString &roomId);

    /// The room that replaced an upgraded one is joined and can be opened.
    void successorReady(const QString &roomId);

    /// A room's details, for the room-info page: name, topic, alias, member
    /// counts, encryption, version, tags, and the predecessor / successor of a
    /// room upgrade.
    void roomInfoReady(const QVariantMap &info);

    /// A room was created and can be opened. Name and encryption state come
    /// back with it, so the room opens correctly before the first sync diff.
    void roomCreated(const QString &roomId, const QString &name, bool encrypted);

    /// Unread messages arrived in a room that is not on screen.
    void roomActivity(const QString &roomId,
                      const QString &roomName,
                      int unread,
                      int mentions);

    /// The URL that has to be opened in the browser to continue a login.
    void loginUrlReady(const QString &url);

    /// The homeserver has no OAuth and signs in with the classic password
    /// flow; the login page has to show the username and password form.
    void passwordLoginNeeded();

    /// A device-code login started: show `url` and `code`; the core keeps
    /// waiting until the user approves on another device.
    void deviceCodeReady(const QString &url, const QString &code);

    /// The sign-up page of the chosen homeserver.
    void registrationUrlReady(const QString &url);

    /// A login did not complete. `message` is safe to show.
    void loginFailed(const QString &message);

private slots:
    /// Receives one JSON message from the core, already back on the UI thread.
    void handleMessage(const QString &json);

    /// Names commands that have been waiting far too long, once each. The
    /// point is the journal: a stalled request otherwise leaves no trace at
    /// all, and "the app does nothing" is not something a tester can report.
    void checkStalledCommands();

private:
    static void deliver(void *userData, const char *json);

    /// Runs the stall watch exactly while there is something to watch.
    void updateStallWatch();

    /// `wipePayload` overwrites the serialized command buffer after the core
    /// has taken its copy — for the one command that carries a password.
    quint64 send(const QString &command,
                 const QJsonObject &arguments = QJsonObject(),
                 bool wipePayload = false);
    void applySession(const QJsonObject &data);
    void reportNewMessages(const QJsonArray &operations);
    void updateSpaceChildren(const QJsonObject &spaces);
    void bumpSpaceCounts();
    QString saveInto(QStandardPaths::StandardLocation location,
                     const QString &path,
                     const QString &suggestedName);
    void setLastError(const QString &message);
    void setLoginRunning(bool running);
    void setTimelineAtStart(bool atStart);
    void setTimelineReady(bool ready);

    XmCore *m_core = nullptr;
    QString m_coreVersion;
    QString m_sessionState = QStringLiteral("none");
    QString m_syncState = QStringLiteral("idle");
    QString m_userId;
    QString m_deviceId;
    QString m_lastError;
    bool m_loginRunning = false;

    RoomListModel m_rooms;
    RoomSortModel m_roomsSorted;
    RoomListModel m_spaces;
    RoomListModel m_spaceRooms;
    VoiceRecorder *m_recorder = nullptr;
    CallEngine *m_calls = nullptr;
    TimelineModel m_timeline;
    QString m_openRoomId;
    /// The room actually on screen; see setVisibleRoom.
    QString m_visibleRoomId;
    /// Empty for the live view; "pinned" or an event id for a focused one.
    QString m_timelineFocus;
    QStringList m_pinnedEventIds;
    QString m_pinnedPreview;
    QString m_successorRoomId;
    QString m_replacementReason;
    bool m_replacementJoined = false;
    bool m_timelineAtStart = false;
    bool m_timelineReady = false;
    QString m_verificationState = QStringLiteral("none");
    QString m_verificationUser;
    bool m_verificationIsSelf = false;
    bool m_verificationWeStarted = false;
    QVariantList m_verificationEmoji;
    QVariantMap m_encryptionStatus;

    /// Downloaded attachments by request key, and the requests in flight.
    QHash<QString, QString> m_media;
    QHash<quint64, QString> m_mediaRequests;

    /// Last seen unread count per room, for spotting new activity.
    QHash<QString, int> m_unread;

    /// Per-space child structure from the core: the member rooms whose unread
    /// counts make up a space's badge, and how many children are sub-spaces.
    QHash<QString, QStringList> m_spaceChildRooms;
    QHash<QString, int> m_spaceSubspaces;
    /// Bumped whenever a space badge may have changed, so QML re-evaluates.
    int m_spaceCountsRevision = 0;

    QString m_profileName;
    QString m_profileAvatar;
    /// Which space a pending `space.hierarchy` request was made for.
    QHash<quint64, QString> m_hierarchyRequests;
    DirectoryModel m_directory;
    bool m_directoryAtEnd = true;
    MemberModel m_members;
    /// Which user a pending `member.remove` request is for.
    QHash<quint64, QString> m_removeRequests;

    quint64 m_nextId = 1;

    /// A command waiting for its answer. The timestamp is the diagnostic part:
    /// a request the homeserver leaves hanging is invisible otherwise — the
    /// SDK retries a rate-limited request for up to fifteen minutes without
    /// ever failing, and all the user sees is that the app went quiet.
    struct PendingCommand {
        QString command;
        qint64 sentAt = 0;
        /// Whether the stall was already logged, so it is said once and not
        /// on every tick of the watch.
        bool reported = false;
    };

    QHash<quint64, PendingCommand> m_pending;
    /// Id of the pagination in flight, 0 when there is none.
    quint64 m_paginateId = 0;
    /// Runs only while something is pending; see checkStalledCommands.
    QTimer *m_stallWatch = nullptr;
    QElapsedTimer m_uptime;
};

#endif // MATRIXBRIDGE_H
