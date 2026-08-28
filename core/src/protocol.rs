//! The wire format between the Qt front end and this core.
//!
//! Commands are JSON objects carrying a caller-chosen `id` and a `cmd`
//! discriminator. Every command is answered by exactly one `reply` message
//! with the same `id`; anything the core reports on its own initiative is an
//! `event`. Keeping this in one place means the C ABI never has to grow a new
//! symbol when a feature is added.

use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

/// A secret on its way out of the process — a login password, a recovery key.
/// Exists so that the two easy leaks are impossible by construction: the heap
/// copy is wiped on drop (`Zeroizing`), and `{:?}` — `Command` derives `Debug`
/// — prints a placeholder instead of the value.
///
/// The recovery key belongs in here for the same reason the password does: it
/// unlocks the entire key backup, so it is the same class of secret even
/// though it travels in the opposite direction on its way back.
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Secret(Zeroizing::new(String::deserialize(deserializer)?)))
    }
}

/// A command sent from the UI to the core.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd")]
pub enum Command {
    /// Restore a previously persisted session, if there is one.
    ///
    /// `storeKey` (base64, optional) hands the core a store key it did not
    /// have at start — the front end obtains it again when the user retries
    /// after a locked secrets collection. Absent, the key from start applies.
    #[serde(rename = "session.restore")]
    SessionRestore {
        id: u64,
        #[serde(rename = "storeKey", default)]
        store_key: Option<String>,
    },

    /// Begin the OAuth 2.0 authorization code flow against `homeserver`.
    ///
    /// The reply carries the URL to open in the user's browser. Completion is
    /// reported later as a `session.changed` or `login.failed` event, because
    /// it depends on the user finishing the flow in the browser.
    #[serde(rename = "login.start")]
    LoginStart { id: u64, homeserver: String },

    /// Begin the OAuth 2.0 device authorization flow against `homeserver` —
    /// the sign-in for devices whose own browser cannot handle the
    /// authentication service's pages.
    ///
    /// The reply carries the verification URL and the code to display.
    /// Completion arrives as `session.changed` or `login.failed`, whenever
    /// the user has approved the login on another device.
    #[serde(rename = "login.deviceCode")]
    LoginDeviceCode { id: u64, homeserver: String },

    /// Sign in with username and password (`m.login.password`), for
    /// homeservers without OAuth. Only sent after `login.start` answered
    /// `passwordLogin: true`; the core verifies that again before it sends
    /// the password anywhere. The reply is the complete outcome — there is
    /// no browser to wait for — and echoes nothing back.
    #[serde(rename = "login.password")]
    LoginPassword {
        id: u64,
        homeserver: String,
        user: String,
        password: Secret,
    },

    /// Ask where accounts can be created on `homeserver`.
    #[serde(rename = "login.registrationUrl")]
    LoginRegistrationUrl { id: u64, homeserver: String },

    /// Cancel a login that was started but not completed.
    #[serde(rename = "login.abort")]
    LoginAbort { id: u64 },

    /// Turn on end-to-end encryption in a room. One-way: Matrix has no way
    /// back to an unencrypted room, which is why the front end asks first.
    #[serde(rename = "room.enableEncryption")]
    RoomEnableEncryption {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// End the session and forget the persisted credentials.
    #[serde(rename = "logout")]
    Logout { id: u64 },

    /// Start the sync service and stream the room list.
    #[serde(rename = "roomlist.start")]
    RoomListStart { id: u64 },

    /// Narrow the room list to rooms matching `pattern`; empty shows all.
    #[serde(rename = "roomlist.filter")]
    RoomListFilter { id: u64, pattern: String },

    /// One more page of rooms. The list starts at one page and grows only on
    /// request; the front end asks when it has been scrolled to its end.
    #[serde(rename = "roomlist.more")]
    RoomListMore { id: u64 },

    /// Stop syncing and drop the room list stream.
    #[serde(rename = "roomlist.stop")]
    RoomListStop { id: u64 },

    /// Stream the joined spaces as their own list. The main room list stays
    /// running; this is a second filtered view over the same rooms.
    #[serde(rename = "spaces.start")]
    SpacesStart { id: u64 },

    /// Stop streaming the space list.
    #[serde(rename = "spaces.stop")]
    SpacesStop { id: u64 },

    /// Stream the rooms that belong to one space. Only one space is open at a
    /// time; opening another one replaces it.
    #[serde(rename = "space.open")]
    SpaceOpen {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Stop streaming the open space's rooms.
    #[serde(rename = "space.close")]
    SpaceClose { id: u64 },

    /// Create a new, empty space with the given name.
    #[serde(rename = "space.create")]
    SpaceCreate { id: u64, name: String },

    /// Leave a space and forget it — the client-side equivalent of deleting.
    #[serde(rename = "space.leave")]
    SpaceLeave {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Add a room to a space (writes an `m.space.child` state event).
    #[serde(rename = "space.addChild")]
    SpaceAddChild {
        id: u64,
        #[serde(rename = "spaceId")]
        space_id: String,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Remove a room from a space.
    #[serde(rename = "space.removeChild")]
    SpaceRemoveChild {
        id: u64,
        #[serde(rename = "spaceId")]
        space_id: String,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Open a room's timeline and stream its updates. Only one timeline is
    /// open at a time; opening another one replaces it. `focus` selects what
    /// the timeline shows: absent or empty for live events, `"pinned"` for the
    /// room's pinned messages, or an event id to open the history around one
    /// event (a permalink jump).
    #[serde(rename = "timeline.open")]
    TimelineOpen {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(default)]
        focus: String,
        /// Track other people's read receipts. Off by default: every receipt
        /// that moves updates a timeline item, and each of those redraws a row
        /// on the Qt side. Only worth it where the status is actually shown.
        #[serde(default)]
        receipts: bool,
    },

    /// Resolve a room address to a room id, and report whether this account
    /// is in that room. Answering only - joining stays a separate command.
    #[serde(rename = "room.resolve")]
    RoomResolve {
        id: u64,
        address: String,
    },

    /// Mark a room read from the list, without opening it.
    #[serde(rename = "room.markRead")]
    RoomMarkRead {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        /// Whether the others may see it. Defaults to *not* telling them: a
        /// privacy flag that goes missing must fall silent, not talk.
        #[serde(default)]
        receipt: bool,
    },

    /// Close the open timeline.
    #[serde(rename = "timeline.close")]
    TimelineClose { id: u64 },

    /// Load a page of older events.
    #[serde(rename = "timeline.paginate")]
    TimelinePaginate { id: u64 },

    /// Send a plain text message to the open room.
    #[serde(rename = "timeline.send")]
    TimelineSend { id: u64, body: String },

    /// Replace the body of a message that was already sent.
    #[serde(rename = "timeline.edit")]
    TimelineEdit {
        id: u64,
        #[serde(rename = "eventId")]
        event_id: String,
        body: String,
    },

    /// Reply to an earlier message.
    #[serde(rename = "timeline.reply")]
    TimelineReply {
        id: u64,
        #[serde(rename = "eventId")]
        event_id: String,
        body: String,
    },

    /// Delete a message.
    /// Add or take back our reaction to a message.
    #[serde(rename = "timeline.react")]
    TimelineReact {
        id: u64,
        #[serde(rename = "eventId")]
        event_id: String,
        key: String,
    },

    /// Put a message the send queue parked back in line.
    #[serde(rename = "timeline.retry")]
    TimelineRetry {
        id: u64,
        #[serde(rename = "txnId")]
        txn_id: String,
    },

    #[serde(rename = "timeline.redact")]
    TimelineRedact {
        id: u64,
        /// Empty for a message that never reached the server; `txnId` names it
        /// then.
        #[serde(default, rename = "eventId")]
        event_id: String,
        #[serde(default, rename = "txnId")]
        txn_id: String,
    },

    /// Send a file from disk as an attachment. `caption` is the text shown
    /// with it and `replyTo` the event it answers — both optional, and both
    /// only decidable here: an attachment that went out bare cannot be turned
    /// into a reply afterwards, and its caption would have to be an edit.
    #[serde(rename = "timeline.sendMedia")]
    TimelineSendMedia {
        id: u64,
        path: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(default)]
        caption: String,
        #[serde(rename = "replyTo", default)]
        reply_to: String,
        /// A recording of one's own rather than an audio file that was picked.
        /// It goes out marked as a voice message (MSC3245), which is what
        /// other clients draw as one and what the bridges to other networks
        /// need to make a native voice note of it.
        #[serde(default)]
        voice: bool,
        /// Its length in milliseconds. Part of the same marking: a voice
        /// message whose length is unknown is treated as a plain audio file.
        #[serde(default)]
        duration: u64,
        /// The picture's own measurements, read by the bridge before the file
        /// is handed over; zero where it is not a picture or could not be
        /// read. They go into the event so the receiving client knows how much
        /// room to leave before a byte is fetched.
        #[serde(default)]
        width: u64,
        #[serde(default)]
        height: u64,
    },

    /// Send a copy of something to another room.
    #[serde(rename = "room.forward")]
    RoomForward {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        path: String,
        #[serde(rename = "mimeType", default)]
        mime_type: String,
        /// As in `timeline.sendMedia`: a forward re-uploads, so it writes the
        /// same description of the file that a first send does.
        #[serde(default)]
        width: u64,
        #[serde(default)]
        height: u64,
    },

    /// Download an attachment and report where it was stored.
    #[serde(rename = "media.fetch")]
    MediaFetch {
        id: u64,
        source: serde_json::Value,
        #[serde(default)]
        thumbnail: bool,
        /// What the event says the file weighs, zero where it says nothing.
        /// Refusing on this costs no download at all; the sender writes the
        /// number, so it is a first gate and not the guarantee - the size of
        /// what actually arrived is checked as well.
        #[serde(default)]
        size: u64,
    },

    /// Mark the open room read: the fully-read marker always, the receipt
    /// others see only where the user allows it. The marker is private account
    /// data - holding it back tells nobody anything and only costs this device
    /// the line in the conversation and the position the room opens at.
    #[serde(rename = "timeline.markRead")]
    TimelineMarkRead {
        id: u64,
        /// Same rule as above: absent means "do not tell them".
        #[serde(default)]
        receipt: bool,
    },

    /// The encrypted lists that name people. `get` answers with all of them;
    /// `set` replaces one.
    #[serde(rename = "private.get")]
    PrivateGet { id: u64 },

    /// Replaces every list at once. One write, not one per list: two writes
    /// would be two read-modify-writes racing each other.
    #[serde(rename = "private.set")]
    PrivateSet {
        id: u64,
        #[serde(default)]
        lists: std::collections::BTreeMap<String, Vec<String>>,
    },

    /// Who may make this phone ring. Sent at start and on every change of the
    /// privacy page; the core refuses everything else before it rings.
    #[serde(rename = "calls.setPolicy")]
    CallsSetPolicy {
        id: u64,
        #[serde(default)]
        policy: String,
        #[serde(default)]
        groups: bool,
        #[serde(default)]
        video: bool,
        #[serde(default)]
        flood: bool,
        #[serde(default)]
        allowed: Vec<String>,
    },

    /// A matrix.to link to the room: its address where it has one, otherwise
    /// its id plus the servers a stranger can join through.
    #[serde(rename = "room.permalink")]
    RoomPermalink {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Who has read up to the given event, with their names. Asked only when
    /// the "read by" mark is tapped, so the rows themselves stay small.
    #[serde(rename = "timeline.readers")]
    TimelineReaders {
        id: u64,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    /// Who reacted to the given event with the given key, with their names.
    /// Asked only when a reaction is held down, for the same reason the readers
    /// are: the rows carry a count, never a list of people.
    #[serde(rename = "timeline.reactors")]
    TimelineReactors {
        id: u64,
        #[serde(rename = "eventId")]
        event_id: String,
        key: String,
    },

    /// Report the state of key backup and recovery.
    #[serde(rename = "encryption.status")]
    EncryptionStatus { id: u64 },

    /// Report whether the local files on this device are encrypted.
    ///
    /// Needs no client: it looks at what is on disk. The front end asks at
    /// start so the answer is available signed out as well.
    #[serde(rename = "storage.status")]
    StorageStatus { id: u64 },

    /// Unlock the key backup with a recovery key or passphrase.
    #[serde(rename = "encryption.recover")]
    EncryptionRecover { id: u64, key: Secret },

    /// Set up key backup and return the new recovery key.
    #[serde(rename = "encryption.enableBackup")]
    EncryptionEnableBackup { id: u64 },

    /// Fetch the room keys of one room from the backup.
    #[serde(rename = "encryption.fetchKeys")]
    EncryptionFetchKeys {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Ask another user — or one's own other devices — to verify.
    #[serde(rename = "verification.request")]
    VerificationRequest {
        id: u64,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// Agree to a verification request that is on screen.
    #[serde(rename = "verification.accept")]
    VerificationAccept { id: u64 },

    /// Confirm that the emoji match on both devices.
    #[serde(rename = "verification.confirm")]
    VerificationConfirm { id: u64 },

    /// Reject a request or abort a running comparison.
    #[serde(rename = "verification.cancel")]
    VerificationCancel { id: u64 },

    /// The emoji did not match: sent as `m.mismatched_sas`, not as an ordinary
    /// cancel.
    #[serde(rename = "verification.mismatch")]
    VerificationMismatch { id: u64 },

    /// Ring the other side of a room with a local session description.
    #[serde(rename = "call.invite")]
    CallInvite {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "callId")]
        call_id: String,
        #[serde(rename = "partyId")]
        party_id: String,
        sdp: String,
    },

    /// Accept a call.
    #[serde(rename = "call.answer")]
    CallAnswer {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "callId")]
        call_id: String,
        #[serde(rename = "partyId")]
        party_id: String,
        sdp: String,
    },

    /// Forward locally gathered ICE candidates.
    #[serde(rename = "call.candidates")]
    CallCandidates {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "callId")]
        call_id: String,
        #[serde(rename = "partyId")]
        party_id: String,
        candidates: Vec<serde_json::Value>,
    },

    /// End a call.
    #[serde(rename = "call.hangup")]
    CallHangup {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "callId")]
        call_id: String,
        #[serde(rename = "partyId")]
        party_id: String,
    },

    /// Fetch relay credentials for media that cannot travel directly.
    #[serde(rename = "call.turnServers")]
    CallTurnServers { id: u64 },

    /// Open, or create, a direct chat with a user.
    #[serde(rename = "room.directChat")]
    RoomDirectChat {
        id: u64,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// Join a room by its address, for example `#room:server`.
    #[serde(rename = "room.joinByAlias")]
    RoomJoinByAlias { id: u64, alias: String },

    /// Accept an invitation or join a known room.
    #[serde(rename = "room.join")]
    RoomJoin {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Everything the room-info page shows, read from local state.
    #[serde(rename = "room.info")]
    RoomInfo {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Join the room that replaced a tombstoned one and answer with its id.
    /// `roomId` is the old room's.
    #[serde(rename = "room.followSuccessor")]
    RoomFollowSuccessor {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Create a room. Everything here is decided once, at creation, because
    /// that is what the server accepts: encryption cannot be switched off
    /// again, a room's federation is fixed for its lifetime, and the rest —
    /// alias, topic, history visibility, power levels — would otherwise have
    /// to be a settings page whose writes depend on a power level the creator
    /// happens to have. `public` lists the room in the server's directory
    /// instead of making it invite-only.
    #[serde(rename = "room.create")]
    RoomCreate {
        id: u64,
        name: String,
        #[serde(default)]
        topic: String,
        /// Local part of the published address, without `#` and without the
        /// server part — the server appends its own. Empty means no address.
        #[serde(default)]
        alias: String,
        #[serde(default)]
        encrypted: bool,
        #[serde(default)]
        public: bool,
        /// `world_readable`, `shared`, `invited` or `joined`; empty leaves the
        /// preset's own default in place.
        #[serde(rename = "historyVisibility", default)]
        history_visibility: String,
        /// Matrix IDs invited as the room is created.
        #[serde(default)]
        invite: Vec<String>,
        /// False keeps the room on this server (`m.federate`), for good.
        #[serde(default = "default_true")]
        federate: bool,
        /// Only moderators may send messages — an announcement room.
        #[serde(rename = "readOnly", default)]
        read_only: bool,
        /// Everyone invited starts at the creator's power level.
        #[serde(rename = "equalPower", default)]
        equal_power: bool,
    },

    /// Leave a room and forget it. On a room that is only invited, this
    /// declines the invitation.
    #[serde(rename = "room.leave")]
    RoomLeave {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Invite a user into a room.
    #[serde(rename = "room.invite")]
    RoomInvite {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// Report the user's own display name and avatar.
    #[serde(rename = "account.get")]
    AccountGet { id: u64 },

    /// Change the user's display name. An empty name removes it.
    #[serde(rename = "account.setDisplayName")]
    AccountSetDisplayName { id: u64, name: String },

    /// Upload a picture from disk and make it the user's avatar.
    #[serde(rename = "account.setAvatar")]
    AccountSetAvatar { id: u64, path: String },

    /// Set how a room may notify: the account default ("default"),
    /// everything ("all"), mentions and keywords only ("mentions"), or
    /// nothing ("mute").
    #[serde(rename = "room.setNotifyMode")]
    RoomSetNotifyMode {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        mode: String,
    },

    /// Mark a room as a favourite, or clear the tag. Clears low priority.
    #[serde(rename = "room.setFavourite")]
    RoomSetFavourite {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        favourite: bool,
    },

    /// Mark a room as low priority, or clear the tag. Clears favourite.
    #[serde(rename = "room.setLowPriority")]
    RoomSetLowPriority {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "lowPriority")]
        low_priority: bool,
    },

    /// Pin a message to the room, or unpin it.
    #[serde(rename = "timeline.pin")]
    TimelinePin {
        id: u64,
        #[serde(rename = "eventId")]
        event_id: String,
        pin: bool,
    },

    /// Search a public room directory. An empty pattern lists the most
    /// popular rooms. Results stream in as `directory.diff`. Without `server`
    /// the own homeserver's directory is searched; with it, that server's
    /// directory is fetched over federation.
    #[serde(rename = "directory.search")]
    DirectorySearch {
        id: u64,
        pattern: String,
        #[serde(default)]
        server: Option<String>,
    },

    /// Load the next page of the current directory search.
    #[serde(rename = "directory.loadMore")]
    DirectoryLoadMore { id: u64 },

    /// Drop the directory search and its stream.
    #[serde(rename = "directory.stop")]
    DirectoryStop { id: u64 },

    /// Load a room's active members (joined and invited). Results arrive as a
    /// single `members.diff` reset.
    #[serde(rename = "members.load")]
    MembersLoad {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Check which joined recipients of an encrypted room still have unverified
    /// devices, so the UI can warn before sending. Replies with `{ roomId,
    /// users: [{ userId, name, devices }] }`.
    #[serde(rename = "room.checkRecipients")]
    RoomCheckRecipients {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Remove (kick) a member from a room.
    #[serde(rename = "member.remove")]
    MemberRemove {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// Everything the member-profile page shows about one user in one room,
    /// answered as a single object.
    #[serde(rename = "member.profile")]
    MemberProfile {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// Ban a member from a room.
    #[serde(rename = "member.ban")]
    MemberBan {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// Lift a member's ban.
    #[serde(rename = "member.unban")]
    MemberUnban {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// Set a member's power level: 0 member, 50 moderator, 100 admin.
    #[serde(rename = "member.setPower")]
    MemberSetPower {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "userId")]
        user_id: String,
        power: i64,
    },

    /// Ignore or unignore a user, account-wide.
    #[serde(rename = "member.setIgnored")]
    MemberSetIgnored {
        id: u64,
        #[serde(rename = "userId")]
        user_id: String,
        ignored: bool,
    },

    /// Withdraw a user's verification after their identity changed.
    #[serde(rename = "member.withdrawVerification")]
    MemberWithdrawVerification {
        id: u64,
        #[serde(rename = "userId")]
        user_id: String,
    },

    /// List the account's ignored users.
    #[serde(rename = "account.ignoredUsers")]
    AccountIgnoredUsers { id: u64 },

    /// Discard the room's outbound group session, so the next message starts
    /// a fresh one and re-shares its key.
    #[serde(rename = "room.resetKeys")]
    RoomResetKeys {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// List a space's linked children from the server's `/hierarchy` API,
    /// including rooms the user has not joined.
    #[serde(rename = "space.hierarchy")]
    SpaceHierarchy {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },

    /// Open one thread's timeline next to the room's; updates stream as
    /// `thread.diff`.
    #[serde(rename = "thread.open")]
    ThreadOpen {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        #[serde(rename = "rootEventId")]
        root_event_id: String,
        /// Echoed in every `thread.diff` so the UI can drop the diffs of an
        /// earlier open of the same thread.
        #[serde(default)]
        token: String,
    },

    /// Close the open thread. `rootEventId` names which one: commands run as
    /// independent tasks, so a close and an open issued together can arrive
    /// in either order.
    #[serde(rename = "thread.close")]
    ThreadClose {
        id: u64,
        #[serde(rename = "rootEventId", default)]
        root_event_id: String,
    },

    /// Send a text message into the open thread.
    #[serde(rename = "thread.send")]
    ThreadSend { id: u64, body: String },

    /// Load older events of the open thread.
    #[serde(rename = "thread.paginate")]
    ThreadPaginate { id: u64 },
}

impl Command {
    /// The `id` this command must be answered with.
    pub fn id(&self) -> u64 {
        match self {
            Command::SessionRestore { id, .. }
            | Command::LoginStart { id, .. }
            | Command::LoginPassword { id, .. }
            | Command::LoginDeviceCode { id, .. }
            | Command::LoginRegistrationUrl { id, .. }
            | Command::LoginAbort { id }
            | Command::RoomEnableEncryption { id, .. }
            | Command::RoomMarkRead { id, .. }
            | Command::RoomResolve { id, .. }
            | Command::Logout { id }
            | Command::RoomListStart { id }
            | Command::RoomListFilter { id, .. }
            | Command::RoomListMore { id }
            | Command::RoomListStop { id }
            | Command::SpacesStart { id }
            | Command::SpacesStop { id }
            | Command::SpaceOpen { id, .. }
            | Command::SpaceClose { id }
            | Command::SpaceCreate { id, .. }
            | Command::SpaceLeave { id, .. }
            | Command::SpaceAddChild { id, .. }
            | Command::SpaceRemoveChild { id, .. }
            | Command::TimelineOpen { id, .. }
            | Command::TimelineClose { id }
            | Command::TimelinePaginate { id }
            | Command::TimelineSend { id, .. }
            | Command::TimelineMarkRead { id, .. }
            | Command::TimelineReaders { id, .. }
            | Command::TimelineReactors { id, .. }
            | Command::RoomPermalink { id, .. }
            | Command::CallsSetPolicy { id, .. }
            | Command::PrivateGet { id }
            | Command::PrivateSet { id, .. }
            | Command::TimelineReply { id, .. }
            | Command::TimelineEdit { id, .. }
            | Command::TimelineRedact { id, .. }
            | Command::TimelineRetry { id, .. }
            | Command::TimelineReact { id, .. }
            | Command::TimelineSendMedia { id, .. }
            | Command::MediaFetch { id, .. }
            | Command::RoomForward { id, .. }
            | Command::RoomJoin { id, .. }
            | Command::RoomFollowSuccessor { id, .. }
            | Command::RoomInfo { id, .. }
            | Command::RoomJoinByAlias { id, .. }
            | Command::RoomDirectChat { id, .. }
            | Command::CallInvite { id, .. }
            | Command::CallAnswer { id, .. }
            | Command::CallCandidates { id, .. }
            | Command::CallHangup { id, .. }
            | Command::CallTurnServers { id }
            | Command::VerificationRequest { id, .. }
            | Command::VerificationAccept { id }
            | Command::VerificationConfirm { id }
            | Command::VerificationCancel { id }
            | Command::VerificationMismatch { id }
            | Command::EncryptionStatus { id }
            | Command::StorageStatus { id }
            | Command::EncryptionRecover { id, .. }
            | Command::EncryptionEnableBackup { id }
            | Command::EncryptionFetchKeys { id, .. }
            | Command::AccountGet { id }
            | Command::AccountSetDisplayName { id, .. }
            | Command::AccountSetAvatar { id, .. }
            | Command::RoomSetNotifyMode { id, .. }
            | Command::RoomSetFavourite { id, .. }
            | Command::RoomSetLowPriority { id, .. }
            | Command::TimelinePin { id, .. }
            | Command::DirectorySearch { id, .. }
            | Command::DirectoryLoadMore { id }
            | Command::DirectoryStop { id }
            | Command::MembersLoad { id, .. }
            | Command::RoomCheckRecipients { id, .. }
            | Command::MemberRemove { id, .. }
            | Command::MemberProfile { id, .. }
            | Command::MemberBan { id, .. }
            | Command::MemberUnban { id, .. }
            | Command::MemberSetPower { id, .. }
            | Command::MemberSetIgnored { id, .. }
            | Command::MemberWithdrawVerification { id, .. }
            | Command::AccountIgnoredUsers { id }
            | Command::RoomResetKeys { id, .. }
            | Command::RoomCreate { id, .. }
            | Command::RoomLeave { id, .. }
            | Command::RoomInvite { id, .. }
            | Command::SpaceHierarchy { id, .. }
            | Command::ThreadOpen { id, .. }
            | Command::ThreadClose { id, .. }
            | Command::ThreadSend { id, .. }
            | Command::ThreadPaginate { id } => *id,
        }
    }
}

/// `#[serde(default)]` for a flag whose absence means yes.
fn default_true() -> bool {
    true
}

/// A successful reply carrying arbitrary payload data.
pub fn reply_ok(id: u64, data: Value) -> Value {
    json!({ "type": "reply", "id": id, "ok": true, "data": data })
}

/// A failed reply. `message` is shown to the user, so it must never contain a
/// token, a full user ID or anything else worth keeping out of a screenshot.
pub fn reply_error(id: u64, message: impl Into<String>) -> Value {
    // The sink, not the source: an SDK error carries the request URL, and that
    // URL carries the room and the user. Ninety-odd places build such a
    // message; this is the one they all pass through.
    json!({
        "type": "reply",
        "id": id,
        "ok": false,
        "error": crate::text::scrub_ids(&message.into()),
    })
}

/// An unsolicited message from the core.
pub fn event(name: &str, data: Value) -> Value {
    json!({ "type": "event", "event": name, "data": data })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message in a script other than Latin has to survive the way in - the
    /// Qt side writes UTF-8 JSON, the C ABI hands over a NUL-terminated string,
    /// and serde reads it back. Asked because a Cyrillic message was reported
    /// as stuck; this end of it is not where it sticks.
    #[test]
    fn a_message_keeps_its_script() {
        for body in [
            "Привет, как дела?",
            "Καλημέρα",
            "こんにちは",
            "emoji 🙂 and a zero-width\u{200b}space",
        ] {
            let raw = json!({ "cmd": "timeline.send", "id": 1, "body": body }).to_string();
            let command: Command = serde_json::from_str(&raw).expect("parses");
            match command {
                Command::TimelineSend { body: parsed, .. } => assert_eq!(parsed, body),
                other => panic!("wrong command: {other:?}"),
            }
        }
    }

    /// The same for the way out: a reply carries the text back unchanged.
    #[test]
    fn a_reply_keeps_its_script() {
        let value = reply_ok(7, json!({ "body": "Привет" }));
        let encoded = value.to_string();
        let decoded: Value = serde_json::from_str(&encoded).expect("parses");
        assert_eq!(decoded["data"]["body"], "Привет");
    }
}
