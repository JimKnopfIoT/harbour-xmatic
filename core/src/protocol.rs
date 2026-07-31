//! The wire format between the Qt front end and this core.
//!
//! Commands are JSON objects carrying a caller-chosen `id` and a `cmd`
//! discriminator. Every command is answered by exactly one `reply` message
//! with the same `id`; anything the core reports on its own initiative is an
//! `event`. Keeping this in one place means the C ABI never has to grow a new
//! symbol when a feature is added.

use serde::Deserialize;
use serde_json::{json, Value};

/// A command sent from the UI to the core.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd")]
pub enum Command {
    /// Restore a previously persisted session, if there is one.
    #[serde(rename = "session.restore")]
    SessionRestore { id: u64 },

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
    #[serde(rename = "timeline.redact")]
    TimelineRedact {
        id: u64,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    /// Send a file from disk as an attachment.
    #[serde(rename = "timeline.sendMedia")]
    TimelineSendMedia {
        id: u64,
        path: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
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
    },

    /// Download an attachment and report where it was stored.
    #[serde(rename = "media.fetch")]
    MediaFetch {
        id: u64,
        source: serde_json::Value,
        #[serde(default)]
        thumbnail: bool,
    },

    /// Send a read receipt for the open room.
    #[serde(rename = "timeline.markRead")]
    TimelineMarkRead { id: u64 },

    /// Report the state of key backup and recovery.
    #[serde(rename = "encryption.status")]
    EncryptionStatus { id: u64 },

    /// Unlock the key backup with a recovery key or passphrase.
    #[serde(rename = "encryption.recover")]
    EncryptionRecover { id: u64, key: String },

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

    /// Create a room with the given name. `public` lists it in the server's
    /// room directory instead of making it invite-only, `encrypted` turns on
    /// end-to-end encryption from the first message.
    #[serde(rename = "room.create")]
    RoomCreate {
        id: u64,
        name: String,
        #[serde(default)]
        encrypted: bool,
        #[serde(default)]
        public: bool,
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

    /// Mute a room's notifications, or return it to the account default.
    #[serde(rename = "room.setMuted")]
    RoomSetMuted {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
        muted: bool,
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

    /// List a space's linked children from the server's `/hierarchy` API,
    /// including rooms the user has not joined.
    #[serde(rename = "space.hierarchy")]
    SpaceHierarchy {
        id: u64,
        #[serde(rename = "roomId")]
        room_id: String,
    },
}

impl Command {
    /// The `id` this command must be answered with.
    pub fn id(&self) -> u64 {
        match self {
            Command::SessionRestore { id }
            | Command::LoginStart { id, .. }
            | Command::LoginDeviceCode { id, .. }
            | Command::LoginRegistrationUrl { id, .. }
            | Command::LoginAbort { id }
            | Command::RoomEnableEncryption { id, .. }
            | Command::Logout { id }
            | Command::RoomListStart { id }
            | Command::RoomListFilter { id, .. }
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
            | Command::TimelineMarkRead { id }
            | Command::TimelineReply { id, .. }
            | Command::TimelineEdit { id, .. }
            | Command::TimelineRedact { id, .. }
            | Command::TimelineSendMedia { id, .. }
            | Command::MediaFetch { id, .. }
            | Command::RoomForward { id, .. }
            | Command::RoomJoin { id, .. }
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
            | Command::EncryptionStatus { id }
            | Command::EncryptionRecover { id, .. }
            | Command::EncryptionEnableBackup { id }
            | Command::EncryptionFetchKeys { id, .. }
            | Command::AccountGet { id }
            | Command::AccountSetDisplayName { id, .. }
            | Command::AccountSetAvatar { id, .. }
            | Command::RoomSetMuted { id, .. }
            | Command::RoomSetFavourite { id, .. }
            | Command::RoomSetLowPriority { id, .. }
            | Command::TimelinePin { id, .. }
            | Command::DirectorySearch { id, .. }
            | Command::DirectoryLoadMore { id }
            | Command::DirectoryStop { id }
            | Command::MembersLoad { id, .. }
            | Command::RoomCheckRecipients { id, .. }
            | Command::MemberRemove { id, .. }
            | Command::RoomCreate { id, .. }
            | Command::RoomLeave { id, .. }
            | Command::RoomInvite { id, .. }
            | Command::SpaceHierarchy { id, .. } => *id,
        }
    }
}

/// A successful reply carrying arbitrary payload data.
pub fn reply_ok(id: u64, data: Value) -> Value {
    json!({ "type": "reply", "id": id, "ok": true, "data": data })
}

/// A failed reply. `message` is shown to the user, so it must never contain a
/// token, a full user ID or anything else worth keeping out of a screenshot.
pub fn reply_error(id: u64, message: impl Into<String>) -> Value {
    json!({ "type": "reply", "id": id, "ok": false, "error": message.into() })
}

/// An unsolicited message from the core.
pub fn event(name: &str, data: Value) -> Value {
    json!({ "type": "event", "event": name, "data": data })
}
