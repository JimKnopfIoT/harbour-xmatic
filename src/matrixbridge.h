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
#include <QSet>
#include <QString>

#include "roomlistmodel.h"
#include "roomsortmodel.h"
#include "directorymodel.h"
#include "membermodel.h"
#include "searchmodel.h"
#include "callengine.h"
#include "voicerecorder.h"
#include "timelinemodel.h"
#include "secretskeeper.h"
#include "xmatic_core.h"

/// The single point of contact between QML and the Rust core.
///
/// Commands are JSON objects with a running id; replies and events come back
/// through one callback that fires on a core worker thread. The trampoline
/// therefore does nothing but re-post the message into the Qt event loop, so
/// everything below runs on the UI thread.
class AppSettings;
class EmojiStore;

class MatrixBridge : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QString coreVersion READ coreVersion CONSTANT)
    Q_PROPERTY(QString emojiDirectory READ emojiDirectory CONSTANT)
    Q_PROPERTY(int emojiRevision READ emojiRevision NOTIFY emojiRevisionChanged)
    Q_PROPERTY(QStringList allowedCallers READ allowedCallers NOTIFY privateListsChanged)
    /// Whether the encrypted lists could be read at all. False means the key is
    /// not available - after a reboot, until the device is unlocked once - and
    /// an empty list then says nothing about what is in the file. A page that
    /// prints "nobody yet" has to know the difference.
    Q_PROPERTY(bool privateListsReadable READ privateListsReadable NOTIFY privateListsChanged)
    Q_PROPERTY(QString sessionState READ sessionState NOTIFY sessionChanged)
    Q_PROPERTY(QString syncState READ syncState NOTIFY syncStateChanged)
    /// False when the homeserver does not advertise the sync this app is
    /// built on. Then "offline" means "cannot work with this server", not
    /// "no network", and the UI has to say so instead of flashing a banner.
    Q_PROPERTY(bool serverSupported READ serverSupported NOTIFY serverSupportedChanged)
    /// How many rooms the *server* counts for this account, or -1 while no
    /// sync has answered yet. The list on screen holds one page at a time and
    /// the sliding sync window starts at twenty rooms, so "fewer rooms than I
    /// have" has two very different causes; this is the number that tells them
    /// apart in a field report.
    Q_PROPERTY(int roomTotal READ roomTotal NOTIFY roomTotalChanged)
    Q_PROPERTY(QString userId READ userId NOTIFY sessionChanged)
    Q_PROPERTY(QString deviceId READ deviceId NOTIFY sessionChanged)
    Q_PROPERTY(bool ready READ ready CONSTANT)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    /// Whether a command of this app's encryption half is in flight.
    ///
    /// Deliberately separate from `busy`, and for the reason `paginating` is:
    /// `busy` means "some command, any command", and a media download that ran
    /// into a 404 and then sat in the retry budget for a minute greyed out
    /// "Verify my other devices" and "Verify this user" for that whole minute.
    /// Reported from the field as "verification does not work"; it worked, and
    /// an unrelated picture was holding the button down.
    Q_PROPERTY(bool encryptionBusy READ encryptionBusy NOTIFY busyChanged)
    Q_PROPERTY(bool paginating READ paginating NOTIFY paginatingChanged)
    Q_PROPERTY(QObject *searchResults READ searchResults CONSTANT)
    /// A search of its own, for the reason `paginating` is one: gating it on
    /// the global `busy` would grey out the search box whenever anything else
    /// was waiting on a slow homeserver.
    Q_PROPERTY(bool searching READ searching NOTIFY searchingChanged)
    /// Whether the last page came back full, so there may be another one.
    Q_PROPERTY(bool searchHasMore READ searchHasMore NOTIFY searchHasMoreChanged)
    /// True while the room's stored messages are being folded into the index.
    Q_PROPERTY(bool indexing READ indexing NOTIFY indexingChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
    Q_PROPERTY(QObject *rooms READ rooms CONSTANT)
    // Forwarded from the room list so the cover, which only sees the bridge,
    // can show "rooms with news / messages" without touching a model.
    Q_PROPERTY(int unreadRooms READ unreadRooms NOTIFY unreadTotalsChanged)
    Q_PROPERTY(int unreadMessages READ unreadMessages NOTIFY unreadTotalsChanged)
    Q_PROPERTY(QObject *spaces READ spaces CONSTANT)
    Q_PROPERTY(QObject *spaceRooms READ spaceRooms CONSTANT)
    /// Whether a notification may carry the message itself, not just a count.
    /// Off by default: the banner also lands on the lock screen.
    /// Whether web links in message bodies are tappable. Off by default: a
    /// tapped link opens the browser, and that is attack surface the user has
    /// to opt into.
    Q_PROPERTY(int spaceCounts READ spaceCounts NOTIFY spaceCountsChanged)
    Q_PROPERTY(QObject *timeline READ timeline CONSTANT)
    Q_PROPERTY(QObject *threadTimeline READ threadTimeline CONSTANT)
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
    /// What the app's own files on this device amount to: `encrypted`,
    /// `storeEncrypted`, `sessionEncrypted`, `keyAvailable`, `canEncrypt`.
    /// Filled from `storage.status`, which needs no session — the answer is
    /// available before the first login.
    Q_PROPERTY(QVariantMap storageStatus READ storageStatus NOTIFY storageChanged)
    /// Whether this system has the Sailfish Secrets daemon installed at all.
    /// False means no retry can help and a package has to be installed — the
    /// difference between "the system cannot do this" and "not right now".
    Q_PROPERTY(bool secretsDaemonPresent READ secretsDaemonPresent NOTIFY storageChanged)
    /// True while a store must not be created: nothing lies on this device
    /// yet, no key can be had, and the user has not said the app may run
    /// unencrypted. The UI shows what to install instead of a login — the one
    /// moment at which a plaintext store would come into existence.
    Q_PROPERTY(bool storageBlocked READ storageBlocked NOTIFY storageChanged)
    /// secretsd's own reason for the last failed attempt, for the page that
    /// explains it. Empty where nothing failed; never carries key material.
    Q_PROPERTY(QString storeKeyReason READ storeKeyReason NOTIFY storageChanged)
    /// Whether this device's own browser can be expected to finish an OAuth
    /// sign-in. False on Sailfish 4, whose Gecko cannot render the MAS pages:
    /// the sign-in button loops back to the form, measured on hardware. The
    /// login page then leads with the device-code route instead of offering it
    /// third and unexplained.
    Q_PROPERTY(bool browserLoginReliable READ browserLoginReliable CONSTANT)
    /// True between a recovery key being accepted and the recovery state
    /// actually settling.
    ///
    /// `encryption.recover` waits for the import and only then answers, but the
    /// state itself arrives through the SDK's own stream and catches up in
    /// stages - measured on one device as three steps inside a second, and on a
    /// slower one against a rate-limiting server as long enough that a tester
    /// entered the key a second time because the line simply stayed orange.
    /// Saying "still finishing" is what stops the app from teaching people to
    /// paste a recovery key twice.
    Q_PROPERTY(bool recoverySettling READ recoverySettling NOTIFY encryptionChanged)
    /// What the signed-in user may do in the room that is open: `pin`,
    /// `invite`, `redactOthers`, `topic`, `name`. Empty until a room has been
    /// opened, and a missing entry means "not answered yet" - a menu should
    /// then show rather than hide, so a slow answer never takes an action away
    /// from somebody who has it.
    Q_PROPERTY(QVariantMap roomPermissions READ roomPermissions NOTIFY roomPermissionsChanged)
    Q_PROPERTY(QString profileName READ profileName NOTIFY profileChanged)
    Q_PROPERTY(QString profileAvatar READ profileAvatar NOTIFY profileChanged)
    Q_PROPERTY(QObject *directory READ directory CONSTANT)
    Q_PROPERTY(bool directoryAtEnd READ directoryAtEnd NOTIFY directoryStateChanged)
    Q_PROPERTY(QObject *members READ members CONSTANT)

public:
    /// `storeKey` carries the base64 store key from Sailfish Secrets and how
    /// the attempt to get it ended. The key goes into the core's config and
    /// nowhere else; the state decides what the UI may offer.
    explicit MatrixBridge(const QString &dataDirectory,
                          const QString &cacheDirectory,
                          const StoreKeyResult &storeKey,
                          AppSettings *settings,
                          QObject *parent = nullptr);
    ~MatrixBridge() override;

    QString coreVersion() const { return m_coreVersion; }

    /// Where a user may put emoji images. Named on the appearance page so it
    /// can be found without guessing.
    QString emojiDirectory() const;

    /// The image for a reaction, or empty where there is none. The file name is
    /// derived from the reaction's own code points and never taken from the
    /// directory, so nothing in there can name itself into a path; only .svg
    /// and .png are looked for, and a file past 64 KB is refused - an emoji is
    /// a few hundred bytes, and the decoder is the app's main untrusted-input
    /// surface (docs/SECURITY.md).
    Q_INVOKABLE QString emojiSource(const QString &key) const;
    QString sessionState() const { return m_sessionState; }

    /// One of "idle", "running", "offline", "terminated", "error". "offline"
    /// means the core noticed the network is gone and reconnects on its own.
    QString syncState() const { return m_syncState; }
    bool serverSupported() const { return m_serverSupported; }
    int roomTotal() const { return m_roomTotal; }
    QString userId() const { return m_userId; }
    QString deviceId() const { return m_deviceId; }
    bool ready() const { return m_core != nullptr; }
    bool busy() const { return !m_pending.isEmpty() || m_loginRunning; }
    bool encryptionBusy() const;

    /// Whether a request for older messages is in flight.
    ///
    /// Deliberately separate from `busy`. `busy` means "some command, any
    /// command, is waiting for an answer" — an avatar thumbnail counts — and
    /// hanging the timeline's controls off that made them dead whenever the
    /// homeserver was slow with something entirely unrelated.
    bool paginating() const { return m_paginateId != 0; }
    QObject *searchResults() { return &m_searchResults; }
    bool searching() const { return m_searchRequest != 0; }
    bool searchHasMore() const { return m_searchHasMore; }
    bool indexing() const { return m_indexRequest != 0; }
    QString lastError() const { return m_lastError; }
    // The UI sees the grouped view (favourites up, low priority down); the
    // core still drives the flat source underneath.
    QObject *rooms() { return &m_roomsSorted; }
    int unreadRooms() const { return m_rooms.unreadRooms(); }
    int unreadMessages() const { return m_rooms.unreadMessages(); }
    QObject *spaces() { return &m_spaces; }
    QObject *spaceRooms() { return &m_spaceRooms; }
    int spaceCounts() const { return m_spaceCountsRevision; }
    QObject *timeline() { return &m_timeline; }
    QObject *threadTimeline() { return &m_threadTimeline; }
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
    QVariantMap storageStatus() const { return m_storageStatus; }
    QVariantMap roomPermissions() const { return m_roomPermissions; }

    QString profileName() const { return m_profileName; }
    QString profileAvatar() const { return m_profileAvatar; }
    QObject *directory() { return &m_directory; }
    bool directoryAtEnd() const { return m_directoryAtEnd; }
    QObject *members() { return &m_members; }

    /// Looks for a persisted session and signs in with it if there is one.
    Q_INVOKABLE void restoreSession();

    /// The session is stored encrypted but its key was not available at
    /// start (sessionState "locked"): asks the device's secrets storage for
    /// the key again — the system may show its unlock dialog — and retries
    /// the restore with it. Never touches the stored data.
    Q_INVOKABLE void retryUnlock();

    /// Asks the secrets storage again from the blocked page. Unlike
    /// `retryUnlock` there is no session to restore yet — this only updates
    /// what the app knows about the key, so the gate can open.
    Q_INVOKABLE void retryStoreKey();

    bool secretsDaemonPresent() const { return m_secretsDaemonPresent; }
    bool browserLoginReliable() const;
    bool recoverySettling() const { return m_recoverySettling; }
    bool storageBlocked() const;
    QString storeKeyReason() const { return m_storeKeyReason; }

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

    /// Asks for one more page of rooms. The list holds one page and grows only
    /// on request, so an account with more rooms than that had no others at
    /// all. Harmless once everything is loaded.
    Q_INVOKABLE void loadMoreRooms();

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

    /// What was typed into a room's message field and not sent. Kept for as
    /// long as the app runs and never written to disk: a draft is message text,
    /// and the only unencrypted place this app has is its settings file. So it
    /// survives the way back to the chat list and the trip through another room
    /// - which is what it was reported for - and not a restart.
    Q_INVOKABLE void setDraft(const QString &roomId, const QString &text);
    Q_INVOKABLE QString draft(const QString &roomId) const;

    /// Asks for everything the room-info page shows; the answer arrives as
    /// `roomInfoReady`. Read from local state, so it comes back at once.
    Q_INVOKABLE void loadRoomInfo(const QString &roomId);

    /// Mutes a room's notifications, or returns it to the default.
    Q_INVOKABLE void setRoomMuted(const QString &roomId, bool muted);
    /// "default", "all", "mentions" or "mute" — the per-room override on the
    /// account's push rules, so it holds in every client.
    Q_INVOKABLE void setRoomNotifyMode(const QString &roomId, const QString &mode);

    /// Marks a room as a favourite, or clears the tag. Clears low priority.
    Q_INVOKABLE void setRoomFavourite(const QString &roomId, bool favourite);

    /// Marks a room as low priority, or clears the tag. Clears favourite. Low
    /// priority also stops the room from raising notifications.
    Q_INVOKABLE void setRoomLowPriority(const QString &roomId, bool lowPriority);

    /// Asks the core which recipients of an encrypted room still have
    /// unverified devices. The answer comes back as `recipientsChecked`.
    Q_INVOKABLE void checkRecipients(const QString &roomId);

    /// Marks a room read from the chat list, without opening it.
    Q_INVOKABLE void markRoomRead(const QString &roomId);

    /// Asks which room an address means and whether this account is in it.
    /// Answers with roomResolved. Resolving only - a tapped link must never
    /// join anything by itself.
    Q_INVOKABLE void resolveRoom(const QString &address);

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

    /// Folds everything already stored for a room into its search index.
    /// The SDK indexes an event when it saves it, so history that was on the
    /// device before the index existed is invisible until this has run.
    Q_INVOKABLE void indexRoom(const QString &roomId);

    /// Searches one room's messages. Replaces whatever `searchResults` held;
    /// an empty query only clears it. Nothing leaves the device.
    Q_INVOKABLE void searchRoom(const QString &roomId, const QString &query);

    /// Appends the next page of the running search. Does nothing while one
    /// page is still in flight or when the last one was already short.
    Q_INVOKABLE void searchMore();

    /// Drops the results and forgets the query, for a page that is closing.
    Q_INVOKABLE void clearSearch();

    /// Loads a room's members into `members`. The list arrives as one reset.
    Q_INVOKABLE void loadMembers(const QString &roomId);

    /// Removes (kicks) a member from a room; the row disappears on success.
    Q_INVOKABLE void removeMember(const QString &roomId, const QString &userId);

    /// Asks for everything the member-profile page shows about one user in
    /// one room; the answer arrives as `memberProfileReady`.
    Q_INVOKABLE void loadMemberProfile(const QString &roomId, const QString &userId);

    /// Bans a member from a room — a kick that also blocks rejoining.
    Q_INVOKABLE void banMember(const QString &roomId, const QString &userId);

    /// Lifts a member's ban.
    Q_INVOKABLE void unbanMember(const QString &roomId, const QString &userId);

    /// Sets a member's power level: 0 member, 50 moderator, 100 admin.
    Q_INVOKABLE void setMemberPower(const QString &roomId, const QString &userId, int power);

    /// Ignores or unignores a user, account-wide.
    Q_INVOKABLE void setMemberIgnored(const QString &userId, bool ignored);

    /// Withdraws a verification after the other side's identity changed.
    Q_INVOKABLE void withdrawMemberVerification(const QString &userId);

    /// Asks for the account's ignored users; the answer arrives as
    /// `ignoredUsersReady`.
    Q_INVOKABLE void loadIgnoredUsers();

    /// Discards the room's outbound group session: the next message starts a
    /// fresh one and re-shares its key. The remedy when the other side cannot
    /// read what this device sends.
    Q_INVOKABLE void resetRoomKeys(const QString &roomId);

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

    /// Opens one thread's timeline next to the room's; rows stream into
    /// `threadTimeline`.
    Q_INVOKABLE void openThread(const QString &roomId, const QString &rootEventId);

    /// Closes the open thread and empties its model.
    Q_INVOKABLE void closeThread();

    /// Sends a text message into the open thread.
    Q_INVOKABLE void sendThreadMessage(const QString &body);

    /// Loads older events of the open thread.
    Q_INVOKABLE void threadLoadOlder();

    /// Sends a plain text message to the open room.
    Q_INVOKABLE void sendMessage(const QString &body);

    /// Sends a read receipt for the open room.
    Q_INVOKABLE void markRead();

    /// The checked picture set, once it exists. Without one the app keeps
    /// looking pictures up as plain files, which is how a hand-copied set
    /// works - unchecked, and openly so.
    void setEmojiStore(EmojiStore *store);

    /// Bumped whenever the picture set changed under the app. QML has to read
    /// this where it asks for a picture: emojiSource() is a function call, not
    /// a property, so nothing else would tell a binding to ask again - and a
    /// set that was just read in would stay invisible until the next start.
    int emojiRevision() const { return m_emojiRevision; }

    /// The lists that name people. They live encrypted in the core; this is
    /// the copy the UI reads, refreshed from every answer.
    QStringList allowedCallers() const { return m_privateLists.value(QStringLiteral("callers")); }
    bool privateListsReadable() const { return m_privateListsReadable; }
    Q_INVOKABLE bool callerAllowed(const QString &userId) const;
    Q_INVOKABLE void allowCaller(const QString &userId);
    Q_INVOKABLE void forbidCaller(const QString &userId);

    /// Recipients the send warning is switched off for.
    Q_INVOKABLE bool recipientTrusted(const QString &userId) const;
    Q_INVOKABLE void trustRecipient(const QString &userId);
    Q_INVOKABLE int resetRecipientWarnings();

    /// Removes the downloaded media and the recordings. On the way out when
    /// the privacy page asks for it, once at start because a killed process
    /// never gets to the way out, and on demand from that page.
    Q_INVOKABLE void clearMediaCache();

    /// Whether a path handed in from outside may be sent. Refuses anything
    /// inside this app's own directories: the share dialog names a file, not a
    /// command, and the session token and the crypto store live there.
    Q_INVOKABLE bool shareableFile(const QString &path) const;

    /// Asks for the room's matrix.to link. Answered by roomLinkReady().
    Q_INVOKABLE void loadRoomLink(const QString &roomId);

    /// Asks who has read up to this message. Answered by readersReady(); the
    /// rows carry only the count, names are fetched when they are wanted.
    Q_INVOKABLE void loadReaders(const QString &eventId);

    /// Empties the last error. A page that shows the field wants what happened
    /// *there*, not what some other command left behind - the field is global,
    /// and pages that care clear it when they open.
    Q_INVOKABLE void clearLastError();

    /// Asks who reacted to this message with this key. Answered by
    /// reactorsReady(); the rows carry a count and nothing else, so the people
    /// behind it are fetched only when one reaction is held down.
    Q_INVOKABLE void loadReactors(const QString &eventId, const QString &key);

    /// Accepts an invitation or joins a known room.
    Q_INVOKABLE void joinRoom(const QString &roomId);

    /// Joins a room by its address, for example "#room:server".
    Q_INVOKABLE void joinRoomByAlias(const QString &alias);

    /// Creates a room from the options the create page collected. A map and
    /// not a row of arguments: everything the server accepts only at creation
    /// belongs in here, the list is expected to grow, and a positional call
    /// with ten arguments is unreadable from QML. Known keys are `name`,
    /// `topic`, `alias`, `encrypted`, `public`, `historyVisibility`, `invite`,
    /// `federate`, `readOnly` and `equalPower`; each one may be left out.
    Q_INVOKABLE void createRoom(const QVariantMap &options);

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
    /// The emoji did not match. A different code on the wire from an ordinary
    /// cancel, and the only one that warns the other side.
    Q_INVOKABLE void reportVerificationMismatch();

    /// Forgets a finished verification so the page can close.
    Q_INVOKABLE void clearVerification();

    /// Asks the core for the state of key backup and recovery.
    Q_INVOKABLE void refreshEncryptionStatus();

    /// Unlocks the key backup with a recovery key or passphrase.
    Q_INVOKABLE void recoverKeys(const QString &key);

    /// Asks the core what the local files amount to; the answer lands in
    /// storageStatus().
    Q_INVOKABLE void refreshStorageStatus();

    /// Sets up key backup; the new recovery key arrives via recoveryKeyReady().
    Q_INVOKABLE void enableKeyBackup();

    /// Pulls one room's keys out of the backup.
    Q_INVOKABLE void fetchRoomKeys(const QString &roomId);

    /// Sends a reply to an earlier message.
    Q_INVOKABLE void replyToMessage(const QString &eventId, const QString &body);

    /// Replaces the body of a message that was already sent.
    Q_INVOKABLE void editMessage(const QString &eventId, const QString &body);

    /// Deletes a message.
    /// Deletes a sent message, or discards one that never left - a message
    /// whose send failed for good has no event id and can only be named by its
    /// transaction id.
    Q_INVOKABLE void deleteMessage(const QString &eventId, const QString &txnId = QString());

    /// Puts a message the send queue parked back in line. Only for one that
    /// never reached the server.
    Q_INVOKABLE void retryMessage(const QString &txnId);

    /// Adds a reaction to a message, or takes ours back if it is already
    /// there. The key is the reaction itself - usually one emoji character.
    Q_INVOKABLE void toggleReaction(const QString &eventId, const QString &key);

    /// Sends a file from disk as an attachment. `caption` is the text shown
    /// with it and `replyTo` the event it answers; both may be empty. Neither
    /// can be added afterwards, which is why the send page asks for them
    /// before the upload starts.
    /// A recording of one's own goes out as a voice message rather than as an
    /// audio file: `voiceDuration` above zero marks it as one, which is what
    /// other clients draw a waveform for and what the bridges to other
    /// networks turn into a native voice note.
    Q_INVOKABLE void sendMedia(const QString &path, const QString &mimeType,
                               const QString &caption = QString(),
                               const QString &replyTo = QString(),
                               qint64 voiceDuration = 0);

    /// Downloads an attachment. The result arrives as mediaReady(key, path);
    /// an already downloaded file is reported immediately.
    /// `declaredSize` is what the event says the file weighs, zero where it
    /// says nothing. Passed on so the core can refuse an outsized attachment
    /// before it downloads it - the SDK has no way to stream one, so a file
    /// that is asked for is a file that is held in memory whole.
    Q_INVOKABLE void requestMedia(const QString &key, const QVariant &source, bool thumbnail,
                                  qint64 declaredSize = 0);

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

signals:
    void spaceCountsChanged();
    void unreadTotalsChanged();

    void sessionChanged();
    void syncStateChanged();
    void serverSupportedChanged();
    void roomTotalChanged();
    void busyChanged();
    void paginatingChanged();
    void searchingChanged();
    void searchHasMoreChanged();
    void indexingChanged();
    /// The room's stored messages are in the index; `count` is how many were
    /// handed over. A search started before this is worth running again.
    void indexReady(int count);
    /// The search could not be run. The page says so instead of showing an
    /// empty result, which would read as "nothing found".
    void searchFailed(const QString &error);
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

    /// A space's linked children as maps with id, name, topic, members,
    /// avatar, space and joined.
    void spaceHierarchyReady(const QString &spaceId, const QVariantList &rooms);

    /// The result of a `checkRecipients` call: the joined recipients of the
    /// room that still have unverified devices, as maps with userId, name and
    /// devices. Empty when there is nothing to warn about.
    void recipientsChecked(const QString &roomId, const QVariantList &users);

    /// The freshly created recovery key. Shown once, never stored.
    void recoveryKeyReady(const QString &key);
    void storageChanged();
    /// A check asked for from the blocked page has finished. `available` says
    /// whether a key came out of it. Emitted even when nothing changed, which
    /// is the whole point: a button that runs and reports nothing is a button
    /// that appears broken.
    void storeKeyChecked(bool available);
    void roomPermissionsChanged();

    /// A request for older messages came back. Carries no row count on
    /// purpose: the rows it fetched reach the model through the diff stream,
    /// which is a separate path from this reply and regularly arrives after it.
    /// Whether the timeline grew can only be judged from the model, one round
    /// later.
    void paginated();

    /// The open timeline is ready, and where this device's reading had stopped
    /// - empty when nothing was ever read here. The view uses it to open at the
    /// first unread message instead of at the newest one.
    /// `readReceipt` is the second guess: the marker can name an event this
    /// room has no row for, and the receipt sits on a message that was really
    /// seen. Empty where there is no second source or both name the same
    /// event.
    void timelineOpened(const QString &readMarker,
                        const QString &readReceipt,
                        bool rebuilt);

    /// A room address resolved: the room, and whether we are a member.
    void roomResolved(const QString &address, const QString &roomId, bool joined);
    /// That address could not be resolved; the message says why.
    void roomResolveFailed(const QString &address, const QString &message);

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

    /// A member's profile, for the member-profile page.
    void memberProfileReady(const QVariantMap &profileData);
    /// Answer to loadRoomLink().
    void roomLinkReady(const QString &link);

    /// A media fetch that will not arrive - refused, failed or given up on.
    /// The row that asked stops waiting instead of spinning for good.
    void mediaFailed(const QString &key);

    /// Answer to loadReaders(): who has read up to that message.
    void readersReady(const QString &eventId, const QVariantList &readers);

    /// Answer to loadReactors(): who reacted with that key.
    void reactorsReady(const QString &eventId, const QString &key,
                       const QVariantList &reactors);
    void emojiRevisionChanged();
    void privateListsChanged();

    /// A moderation or ignore action succeeded; no diff follows, so the open
    /// profile page reloads on this.
    void memberChanged(const QString &changedUserId);

    /// The result of one member action, for pages that have to update without
    /// re-reading: the server has confirmed it, but the local store only
    /// learns of the state event on a later sync, so a reload would answer
    /// with the old values. `action` is ban, unban, setPower, setIgnored or
    /// withdrawVerification.
    void memberActionDone(const QString &action, const QVariantMap &result);

    /// A member profile could not be read; the page needs a state of its own
    /// rather than a spinner that never stops.
    void memberProfileFailed(const QString &message);

    /// A thread could not be opened, paginated or posted to. Same reason:
    /// `lastError` is global, and the page must not sit on "loading".
    void threadFailed(const QString &message);

    /// The account's ignored users, for the ignore list page.
    void ignoredUsersReady(const QStringList &users);

    /// A room was created and can be opened. Name and encryption state come
    /// back with it, so the room opens correctly before the first sync diff.
    void roomCreated(const QString &roomId, const QString &name, bool encrypted);

    /// Unread messages arrived in a room that is not on screen. The preview
    /// describes the room's latest event — `previewKind` is one of text,
    /// emote, image, video, audio, file, location, encrypted, or empty when
    /// there is nothing to say; `previewText` is set for text and emote only;
    /// `previewSender` is the sender's Matrix ID. Whether any of it is shown
    /// is the UI's decision (see `AppSettings::notificationPreview`).
    void roomActivity(const QString &roomId,
                      const QString &roomName,
                      int unread,
                      int mentions,
                      const QString &previewKind,
                      const QString &previewText,
                      const QString &previewSender);

    /// A room's notifying events dropped back to zero — it was read, here or
    /// in another client. Read receipts travel between clients, so the server
    /// clears the counter for every device; a banner raised for that room has
    /// nothing left to announce and is taken down.
    void roomRead(const QString &roomId);

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
    /// Says in the journal why a message did not go out, once per message.
    void reportSendFailures(const QJsonArray &operations);
    /// The two halves of an incoming message, each dispatched to the handler
    /// for its domain. A domain handler answers whether the message was its
    /// own, so the chain stops at the first that recognises it.
    void handleReply(const QJsonObject &message);
    void handleEvent(const QJsonObject &message);
    bool replyLogin(const QString &command, const QJsonObject &data);
    bool replyAccount(const QString &command, const QJsonObject &data);
    bool replyRoom(quint64 id, const QString &command, const QJsonObject &data);
    bool replyMember(quint64 id, const QString &command, const QJsonObject &data);
    bool replySearch(quint64 id, const QString &command, const QJsonObject &data);
    void sendSearchPage();
    bool replyTimeline(quint64 id, const QString &command, const QJsonObject &data);
    bool replyEncryption(const QString &command, const QJsonObject &data);
    bool replyCall(const QString &command, const QJsonObject &data);
    bool eventCall(const QString &name, const QJsonObject &data);
    bool eventVerification(const QString &name, const QJsonObject &data);
    bool eventTimeline(const QString &name, const QJsonObject &data);
    bool eventLists(const QString &name, const QJsonObject &data);
    bool eventSession(const QString &name, const QJsonObject &data);

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
    /// Hands the privacy page's call rules to the core, which enforces them
    /// before anything rings.
    void pushCallPolicy();
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
    /// Where the session file and the stores live; the unlock retry asks
    /// the secrets keeper for the key against it.
    QString m_dataDirectory;
    /// What the last attempt at the store key produced. Re-measured by
    /// `retryStoreKey`, so the gate reflects the device as it is now and not
    /// as it was at start.
    StoreKeyState m_storeKeyState = StoreKeyState::Unavailable;
    QString m_storeKeyReason;
    bool m_secretsDaemonPresent = false;
    /// Set when a recovery key was accepted, cleared when the state settles or
    /// the timer gives up. A claim that something is in progress may not
    /// outlive the progress, so it is bounded.
    bool m_recoverySettling = false;
    QTimer m_recoverySettleTimeout;
    QString m_cacheDirectory;
    /// Answers of emojiSource(), which QML asks once per chip per rebuild.
    mutable QHash<QString, QString> m_emojiSources;
    EmojiStore *m_emojiStore = nullptr;
    /// Mirror of the core's encrypted lists, by name.
    QHash<QString, QStringList> m_privateLists;
    /// Whether the encrypted file could be read. False means locked or
    /// damaged - the lists must not be written over in that state.
    bool m_privateListsReadable = false;
    bool m_legacyDropPending = false;
    void sendPrivateList(const QString &list, const QStringList &values);
    void sendPrivateLists();
    int m_emojiRevision = 0;
    QString m_sessionState = QStringLiteral("none");
    QString m_syncState = QStringLiteral("idle");
    bool m_serverSupported = true;
    int m_roomTotal = -1;
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
    TimelineModel m_threadTimeline;
    /// Which room `m_members` was loaded for, so an action in another room
    /// does not rewrite this one's rows.
    QString m_membersRoomId;
    /// Root event of the open thread, to drop late diffs after a switch.
    QString m_openThreadRoot;
    /// Counts thread opens, so diffs of a previous open of the *same* thread
    /// can be told apart from this one's.
    quint64 m_threadGeneration = 0;
    /// The user's preferences. Not owned - main() outlives the bridge. Read
    /// where a command needs one, and listened to where a change has to act.
    AppSettings *m_settings = nullptr;
    /// Transaction ids of messages whose failure was already reported, so a
    /// row that is rewritten does not repeat itself in the journal.
    QSet<QString> m_reportedSendFailures;
    /// Unsent message text per room. Memory only, cleared on sign-out.
    QHash<QString, QString> m_drafts;
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
    QVariantMap m_storageStatus;
    QVariantMap m_roomPermissions;

    /// Downloaded attachments by request key, and the requests in flight.
    QHash<QString, QString> m_media;
    QHash<quint64, QString> m_mediaRequests;

    /// Last seen unread count per room, for spotting new activity.
    QHash<QString, int> m_unread;

    /// Last seen notifying-events count per room (counted against the push
    /// rules). The banner follows this one, not `m_unread` — see
    /// reportNewMessages.
    QHash<QString, int> m_notified;

    /// A banner whose text is not there yet. The SDK fills the room's latest
    /// event in a second pass, after the one that raises the counts, so the
    /// diff that says "something arrived" still carries the *previous*
    /// message — a banner built from it is always one message behind. The
    /// announcement therefore waits for the pass that moves the room's
    /// timestamp, and goes out with a count only if that never comes.
    struct PendingBanner {
        QString name;
        int notifications = 0;
        int mentions = 0;
        /// The latest-event timestamp seen when the counts rose. The preview
        /// belongs to this one; anything newer is the message just announced.
        quint64 timestamp = 0;
    };
    QHash<QString, PendingBanner> m_pendingBanners;
    /// Rooms whose call this device refused, with the moment it happened: the
    /// counter the invitation raised must not ring as a message.
    QHash<QString, qint64> m_blockedCallRooms;
    QTimer m_bannerTimeout;
    void publishBanner(const QString &roomId, const PendingBanner &pending,
                       const QJsonObject &room);

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
    SearchModel m_searchResults;
    /// One screen of hits, asked for again when the list reaches its end.
    static const int SearchPageSize = 20;
    /// The room and query the results belong to, so the next page asks the
    /// same question, and the id of the page in flight - a reply carrying a
    /// different one belongs to a search the user has already moved on from.
    QString m_searchRoomId;
    QString m_searchQuery;
    int m_searchOffset = 0;
    bool m_searchHasMore = false;
    quint64 m_searchRequest = 0;
    quint64 m_indexRequest = 0;
    /// Which user a pending `member.remove` request is for.
    QHash<quint64, QString> m_removeRequests;
    /// Lets go of everything a finished, failed or abandoned command was
    /// remembered by.
    void forgetRequest(quint64 id);

    /// Which message a pending `timeline.readers` request is about.
    QHash<quint64, QString> m_readerRequests;
    /// Which message and key a pending `timeline.reactors` request is about.
    /// The core answers with both, so this only says the request was ours.
    QSet<quint64> m_reactorRequests;
    /// Recordings waiting for their send to come back, so they can go.
    QHash<quint64, QString> m_voiceSends;
    QString m_voiceDirectory;

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
