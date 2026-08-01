//! The command dispatcher and the callback sink.
//!
//! Commands arrive from the Qt thread, are queued, and are handled on the
//! tokio runtime. Replies and events travel back through a single C callback.
//! Each command gets its own task, so a login that waits minutes for the user
//! to finish in the browser never blocks anything else.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex as StdMutex};

use matrix_sdk::{
    authentication::oauth::CsrfToken,
    ruma::{OwnedRoomId, RoomId},
    store::RoomLoadSettings,
    utils::local_server::LocalServerShutdownHandle,
    Client, SessionChange,
};
use serde_json::{json, Value};
use tokio::sync::{broadcast::error::RecvError, mpsc, Mutex};

use crate::call;
use crate::directory;
use crate::login;
use crate::profile;
use crate::protocol::{event, reply_error, reply_ok, Command};
use crate::media;
use crate::members;
use crate::recovery;
use crate::roomlist::{self, RoomListHandle};
use crate::session::{self, Paths, StoredSession};
use crate::timeline::{self, TimelineHandle};
use crate::verification;

/// Signature of the function the front end registers to receive messages.
pub type XmCallback = extern "C" fn(*mut c_void, *const c_char);

struct CallbackSlot {
    func: Option<XmCallback>,
    user_data: *mut c_void,
}

// The pointer is opaque to Rust and is only ever handed back to the front end
// together with a message. The C++ side owns it, keeps it alive for as long as
// the core exists, and its trampoline is safe to call from any thread.
unsafe impl Send for CallbackSlot {}
unsafe impl Sync for CallbackSlot {}

/// Delivers JSON messages to the front end.
pub struct Sink {
    slot: StdMutex<CallbackSlot>,
}

impl Sink {
    pub fn new() -> Self {
        Self {
            slot: StdMutex::new(CallbackSlot {
                func: None,
                user_data: std::ptr::null_mut(),
            }),
        }
    }

    pub fn set_callback(&self, func: Option<XmCallback>, user_data: *mut c_void) {
        if let Ok(mut slot) = self.slot.lock() {
            slot.func = func;
            slot.user_data = user_data;
        }
    }

    /// Serialises `value` and hands it to the front end. Messages sent before a
    /// callback is registered are dropped.
    pub fn emit(&self, value: Value) {
        let Ok(slot) = self.slot.lock() else { return };
        let Some(func) = slot.func else { return };
        let Ok(text) = serde_json::to_string(&value) else { return };
        let Ok(message) = CString::new(text) else { return };

        let user_data = slot.user_data;
        let _ = catch_unwind(AssertUnwindSafe(|| func(user_data, message.as_ptr())));
    }
}

/// A login waiting for the user, kept so it can be cancelled.
struct PendingLogin {
    shutdown: LocalServerShutdownHandle,
    state: CsrfToken,
}

struct State {
    paths: Paths,
    client: Mutex<Option<Client>>,
    pending: Mutex<Option<PendingLogin>>,
    /// A device-code login polling for approval, kept so it can be cancelled.
    pending_device: Mutex<Option<tokio::task::JoinHandle<()>>>,
    rooms: Mutex<Option<RoomListHandle>>,
    spaces: Mutex<Option<tokio::task::JoinHandle<()>>>,
    open_space: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Behind an `Arc` so a command can clone the handle out and release the
    /// lock before going to the server — see `State::timeline`.
    timeline: Mutex<Option<Arc<TimelineHandle>>>,
    /// Serialises `open_timeline` against itself. Held for the whole open,
    /// including the network part, which is why it cannot be `timeline`.
    opening: Mutex<()>,
    /// Every room that currently needs a sliding-sync subscription. See
    /// `Subscriptions` — they have to be requested together or not at all.
    subscriptions: Mutex<Subscriptions>,
    directory: Mutex<Option<DirectoryHandle>>,
    verification: verification::Slot,
    sink: Arc<Sink>,
}

/// The rooms that need a sliding-sync subscription, by what wants them.
///
/// They live together because `subscribe_to_rooms` is not additive: "All
/// previous room subscriptions will be forgotten" (`matrix-sdk-ui`,
/// `room_list_service/mod.rs`), and the sliding sync clears its whole
/// subscription map on every call. Requesting one room therefore cancels
/// whatever was requested before, which is why these three used to knock each
/// other out — starting a verification dropped the open room back to one
/// timeline event per sync, and opening a room dropped the verification's
/// direct chat, leaving the flow apparently stalled.
///
/// A subscription is not a nicety: an unsubscribed room delivers a single
/// timeline event per sync response (`DEFAULT_LIST_TIMELINE_LIMIT`), so a burst
/// — a call's ICE candidates, several quick messages — arrives as its last
/// event alone.
#[derive(Default)]
struct Subscriptions {
    /// The room whose conversation is on screen.
    open: Option<OwnedRoomId>,
    /// The direct chat a running verification talks through.
    verification: Option<OwnedRoomId>,
    /// The room of a call being set up or running.
    call: Option<OwnedRoomId>,
}

impl Subscriptions {
    fn wanted(&self) -> Vec<OwnedRoomId> {
        let mut rooms: Vec<OwnedRoomId> = [&self.open, &self.verification, &self.call]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        rooms.sort();
        rooms.dedup();
        rooms
    }
}

/// The directory-search task and the channel its orders go in through.
struct DirectoryHandle {
    task: tokio::task::JoinHandle<()>,
    orders: mpsc::UnboundedSender<directory::Order>,
}

impl State {
    /// The signed-in client, or `None`. Cloned so no lock is held across an
    /// await point.
    async fn client(&self) -> Option<Client> {
        self.client.lock().await.clone()
    }

    /// The open timeline, or `None`. Cloned for the same reason as the client:
    /// every timeline command is a network round trip, and the lock must not
    /// be held across it. It used to be — a pagination that the server left
    /// hanging (matrix.org answers `M_LIMIT_EXCEEDED`, and the SDK then retries
    /// for up to fifteen minutes) blocked not just the next pagination but
    /// `open_timeline` as well, so switching rooms froze until the server
    /// relented.
    async fn timeline(&self) -> Option<Arc<TimelineHandle>> {
        self.timeline.lock().await.clone()
    }

    /// Records what a part of the app needs subscribed and asks the sliding
    /// sync for the whole set. Always the whole set: a call with fewer rooms
    /// silently unsubscribes the rest.
    async fn subscribe(&self, change: impl FnOnce(&mut Subscriptions)) {
        // The lock is held across the request, not just across the bookkeeping.
        // Releasing it first would leave a window in which a second caller
        // records its own change and gets its list out ahead of this one; since
        // the request replaces the entire subscription list rather than adding
        // to it, the older list would then be the one left standing and a room
        // would silently lose its subscription. Nothing waits on the network
        // inside here — the sliding sync only rewrites its map and nudges its
        // loop — so holding it costs nothing.
        let mut subscriptions = self.subscriptions.lock().await;
        change(&mut subscriptions);
        let wanted = subscriptions.wanted();

        let service = self.rooms.lock().await.as_ref().map(|handle| handle.service());
        let Some(service) = service else {
            // No sync service yet; whatever was recorded is applied by the next
            // change once there is one.
            return;
        };

        let borrowed: Vec<&RoomId> = wanted.iter().map(|room| room.as_ref()).collect();
        service.subscribe_to_rooms(&borrowed).await;
    }

    /// Describes the current session for a reply or an event.
    async fn session_data(&self) -> Value {
        match &*self.client.lock().await {
            Some(client) => match (client.user_id(), client.device_id()) {
                (Some(user), Some(device)) => json!({
                    "state": "signed-in",
                    "userId": user.as_str(),
                    "deviceId": device.as_str(),
                }),
                _ => json!({ "state": "none" }),
            },
            None => json!({ "state": "none" }),
        }
    }
}

/// Starts the dispatcher on `runtime` and returns the channel commands go into.
pub fn spawn(
    runtime: &tokio::runtime::Runtime,
    paths: Paths,
    sink: Arc<Sink>,
) -> mpsc::UnboundedSender<Command> {
    let (sender, mut receiver) = mpsc::unbounded_channel::<Command>();

    let state = Arc::new(State {
        paths,
        client: Mutex::new(None),
        pending: Mutex::new(None),
        pending_device: Mutex::new(None),
        rooms: Mutex::new(None),
        spaces: Mutex::new(None),
        open_space: Mutex::new(None),
        timeline: Mutex::new(None),
        opening: Mutex::new(()),
        subscriptions: Mutex::new(Subscriptions::default()),
        directory: Mutex::new(None),
        verification: Arc::new(Mutex::new(None)),
        sink,
    });

    runtime.spawn(async move {
        while let Some(command) = receiver.recv().await {
            let state = state.clone();
            // Boxed: the combined future of all command arms is large, and
            // moving it to the heap keeps the task allocation small.
            tokio::spawn(Box::pin(handle(state, command)));
        }
    });

    sender
}

async fn handle(state: Arc<State>, command: Command) {
    let id = command.id();
    match command {
        Command::SessionRestore { .. } => restore_session(&state, id).await,
        Command::LoginStart { homeserver, .. } => start_login(&state, id, homeserver).await,
        Command::LoginDeviceCode { homeserver, .. } => {
            start_device_login(&state, id, homeserver).await
        }
        Command::LoginRegistrationUrl { homeserver, .. } => {
            registration_url(&state, id, homeserver).await
        }
        Command::LoginAbort { .. } => abort_login(&state, id).await,
        Command::RoomEnableEncryption { room_id, .. } => {
            enable_room_encryption(&state, id, room_id).await
        }
        Command::Logout { .. } => logout(&state, id).await,
        Command::RoomListStart { .. } => start_room_list(&state, id).await,
        Command::RoomListFilter { pattern, .. } => filter_room_list(&state, id, pattern).await,
        Command::RoomListStop { .. } => stop_room_list(&state, id).await,
        Command::SpacesStart { .. } => start_spaces(&state, id).await,
        Command::SpacesStop { .. } => stop_spaces(&state, id).await,
        Command::SpaceOpen { room_id, .. } => open_space(&state, id, room_id).await,
        Command::SpaceClose { .. } => close_space(&state, id).await,
        Command::SpaceCreate { name, .. } => create_space(&state, id, name).await,
        Command::SpaceLeave { room_id, .. } => leave_space(&state, id, room_id).await,
        Command::SpaceAddChild {
            space_id, room_id, ..
        } => add_space_child(&state, id, space_id, room_id).await,
        Command::SpaceRemoveChild {
            space_id, room_id, ..
        } => remove_space_child(&state, id, space_id, room_id).await,
        Command::TimelineOpen { room_id, focus, .. } => {
            open_timeline(&state, id, room_id, focus).await
        }
        Command::TimelineClose { .. } => close_timeline(&state, id).await,
        Command::TimelinePaginate { .. } => paginate_timeline(&state, id).await,
        Command::TimelineSend { body, .. } => send_message(&state, id, body).await,
        Command::TimelineMarkRead { .. } => mark_read(&state, id).await,
        Command::TimelineReply { event_id, body, .. } => {
            reply_message(&state, id, event_id, body).await
        }
        Command::TimelineEdit { event_id, body, .. } => edit_message(&state, id, event_id, body).await,
        Command::TimelineRedact { event_id, .. } => redact_message(&state, id, event_id).await,
        Command::TimelineSendMedia { path, mime_type, .. } => {
            send_media(&state, id, path, mime_type).await
        }
        Command::MediaFetch {
            source, thumbnail, ..
        } => fetch_media(&state, id, source, thumbnail).await,
        Command::RoomForward {
            room_id,
            body,
            path,
            mime_type,
            ..
        } => forward(&state, id, room_id, body, path, mime_type).await,
        Command::RoomJoin { room_id, .. } => join_room(&state, id, room_id).await,
        Command::RoomCreate {
            name,
            encrypted,
            public,
            ..
        } => create_room(&state, id, name, encrypted, public).await,
        Command::RoomLeave { room_id, .. } => leave_room(&state, id, room_id).await,
        Command::RoomInvite {
            room_id, user_id, ..
        } => invite_to_room(&state, id, room_id, user_id).await,
        Command::RoomJoinByAlias { alias, .. } => join_by_alias(&state, id, alias).await,
        Command::RoomDirectChat { user_id, .. } => direct_chat(&state, id, user_id).await,
        Command::CallInvite {
            room_id,
            call_id,
            party_id,
            sdp,
            ..
        } => {
            // A call's signalling rides the room's timeline, and an
            // unsubscribed room hands out one event per sync response — a
            // burst of ICE candidates then arrives as its last one alone,
            // which is a call that connects and stays silent.
            set_call_room(&state, Some(&room_id)).await;
            call_step(&state, id, move |client| async move {
                call::invite(&client, &room_id, &call_id, &party_id, sdp).await
            })
            .await
        }
        Command::CallAnswer {
            room_id,
            call_id,
            party_id,
            sdp,
            ..
        } => {
            set_call_room(&state, Some(&room_id)).await;
            call_step(&state, id, move |client| async move {
                call::answer(&client, &room_id, &call_id, &party_id, sdp).await
            })
            .await
        }
        Command::CallCandidates {
            room_id,
            call_id,
            party_id,
            candidates,
            ..
        } => {
            call_step(&state, id, move |client| async move {
                call::candidates(&client, &room_id, &call_id, &party_id, candidates).await
            })
            .await
        }
        Command::CallHangup {
            room_id,
            call_id,
            party_id,
            ..
        } => {
            call_step(&state, id, move |client| async move {
                call::hangup(&client, &room_id, &call_id, &party_id).await
            })
            .await;
            // Released after the goodbye is on its way, not before, or the
            // hangup itself could go out on an unsubscribed room.
            set_call_room(&state, None).await;
        }
        Command::CallTurnServers { .. } => turn_servers(&state, id).await,
        Command::VerificationRequest { user_id, .. } => {
            request_verification(&state, id, user_id).await
        }
        Command::VerificationAccept { .. } => verification_step(&state, id, Step::Accept).await,
        Command::VerificationConfirm { .. } => verification_step(&state, id, Step::Confirm).await,
        Command::VerificationCancel { .. } => verification_step(&state, id, Step::Cancel).await,
        Command::EncryptionStatus { .. } => encryption_status(&state, id).await,
        Command::EncryptionRecover { key, .. } => encryption_recover(&state, id, key).await,
        Command::EncryptionEnableBackup { .. } => encryption_enable_backup(&state, id).await,
        Command::EncryptionFetchKeys { room_id, .. } => fetch_room_keys(&state, id, room_id).await,
        Command::AccountGet { .. } => account_get(&state, id).await,
        Command::AccountSetDisplayName { name, .. } => {
            account_set_display_name(&state, id, name).await
        }
        Command::AccountSetAvatar { path, .. } => account_set_avatar(&state, id, path).await,
        Command::RoomSetMuted { room_id, muted, .. } => {
            room_set_muted(&state, id, room_id, muted).await
        }
        Command::RoomSetFavourite {
            room_id, favourite, ..
        } => room_set_favourite(&state, id, room_id, favourite).await,
        Command::RoomSetLowPriority {
            room_id,
            low_priority,
            ..
        } => room_set_low_priority(&state, id, room_id, low_priority).await,
        Command::TimelinePin { event_id, pin, .. } => pin_message(&state, id, event_id, pin).await,
        Command::DirectorySearch { pattern, server, .. } => {
            directory_search(&state, id, pattern, server).await
        }
        Command::DirectoryLoadMore { .. } => directory_more(&state, id).await,
        Command::DirectoryStop { .. } => directory_stop(&state, id).await,
        Command::MembersLoad { room_id, .. } => members_load(&state, id, room_id).await,
        Command::RoomCheckRecipients { room_id, .. } => {
            room_check_recipients(&state, id, room_id).await
        }
        Command::MemberRemove { room_id, user_id, .. } => {
            member_remove(&state, id, room_id, user_id).await
        }
        Command::SpaceHierarchy { room_id, .. } => space_hierarchy(&state, id, room_id).await,
    }
}

async fn account_get(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match profile::get(&client).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn account_set_display_name(state: &Arc<State>, id: u64, name: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match profile::set_display_name(&client, &name).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "saved": true })));
            emit_profile(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn account_set_avatar(state: &Arc<State>, id: u64, path: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match profile::set_avatar(&client, &path).await {
        Ok(data) => {
            state.sink.emit(reply_ok(id, data));
            emit_profile(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// Re-reads the profile from the server and pushes it, so every page showing
/// it updates after a change without asking again.
async fn emit_profile(state: &Arc<State>) {
    if let Some(client) = state.client().await {
        if let Ok(data) = profile::get(&client).await {
            state.sink.emit(event("profile.changed", data));
        }
    }
}

async fn room_set_muted(state: &Arc<State>, id: u64, room_id: String, muted: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::set_muted(&client, &room_id, muted).await {
        Ok(()) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id, "muted": muted }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn room_set_favourite(state: &Arc<State>, id: u64, room_id: String, favourite: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::set_favourite(&client, &room_id, favourite).await {
        Ok(()) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id, "favourite": favourite }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn room_set_low_priority(state: &Arc<State>, id: u64, room_id: String, low_priority: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::set_low_priority(&client, &room_id, low_priority).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "lowPriority": low_priority }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn pin_message(state: &Arc<State>, id: u64, event_id: String, pin: bool) {
    let Some(handle) = state.timeline().await else {
        state.sink.emit(reply_error(id, "no open room"));
        return;
    };
    match handle.set_pinned(&event_id, pin).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "pinned": pin })));
            // The banner and the row markers follow this list; the server's
            // answer already contains the change made a moment ago.
            let (ids, preview) = handle.pinned_info().await;
            state.sink.emit(event(
                "timeline.pinned",
                json!({
                    "roomId": handle.room_id(),
                    "eventIds": ids,
                    "preview": preview,
                }),
            ));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn directory_search(state: &Arc<State>, id: u64, pattern: String, server: Option<String>) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let via_server = match server.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        None => None,
        Some(name) => match matrix_sdk::ruma::OwnedServerName::try_from(name) {
            Ok(name) => Some(name),
            Err(_) => {
                state.sink.emit(reply_error(id, "not a valid server name"));
                return;
            }
        },
    };

    let mut slot = state.directory.lock().await;
    if slot.is_none() {
        let (task, orders) = directory::start(client, state.sink.clone());
        *slot = Some(DirectoryHandle { task, orders });
    }
    let sent = slot
        .as_ref()
        .expect("just ensured")
        .orders
        .send(directory::Order::Search(pattern, via_server))
        .is_ok();
    drop(slot);

    if sent {
        state.sink.emit(reply_ok(id, json!({ "searching": true })));
    } else {
        state.sink.emit(reply_error(id, "the search is not running"));
    }
}

async fn directory_more(state: &Arc<State>, id: u64) {
    let slot = state.directory.lock().await;
    let sent = slot
        .as_ref()
        .map(|handle| handle.orders.send(directory::Order::More).is_ok())
        .unwrap_or(false);
    drop(slot);

    if sent {
        state.sink.emit(reply_ok(id, json!({ "searching": true })));
    } else {
        state.sink.emit(reply_error(id, "the search is not running"));
    }
}

async fn directory_stop(state: &Arc<State>, id: u64) {
    if let Some(handle) = state.directory.lock().await.take() {
        handle.task.abort();
    }
    state.sink.emit(reply_ok(id, json!({ "searching": false })));
}

async fn space_hierarchy(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::space_hierarchy(&client, &room_id).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn room_check_recipients(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::unverified_recipients(&client, &room_id).await {
        Ok(users) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id, "users": users }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn members_load(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::load(&client, &room_id).await {
        Ok(rows) => {
            let count = rows.len();
            state.sink.emit(event(
                "members.diff",
                json!({ "ops": [{ "op": "reset", "values": rows }] }),
            ));
            state.sink.emit(reply_ok(id, json!({ "count": count })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn member_remove(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::remove(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "removed": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn encryption_status(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    let status = recovery::status(&client).await;
    state.sink.emit(reply_ok(id, status));
}

async fn encryption_recover(state: &Arc<State>, id: u64, key: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match recovery::recover(&client, &key).await {
        Ok(()) => {
            let status = recovery::status(&client).await;
            state.sink.emit(reply_ok(id, status.clone()));
            state.sink.emit(event("encryption.changed", status));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn encryption_enable_backup(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match recovery::enable(&client).await {
        Ok(key) => {
            // The recovery key is shown once and never stored: writing it down
            // is the user's job, and keeping a copy here would defeat it.
            state.sink.emit(reply_ok(id, json!({ "recoveryKey": key })));
            let status = recovery::status(&client).await;
            state.sink.emit(event("encryption.changed", status));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn fetch_room_keys(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match recovery::fetch_room_keys(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "fetched": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn request_verification(state: &Arc<State>, id: u64, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // An empty user means "my own other devices".
    let target = if user_id.trim().is_empty() {
        match client.user_id() {
            Some(own) => own.as_str().to_owned(),
            None => {
                state.sink.emit(reply_error(id, "own user is unknown"));
                return;
            }
        }
    } else {
        user_id.trim().to_owned()
    };

    // A stale flow has to be taken down before a new one is asked for, or the
    // SDK cancels both and the retry is dead on arrival.
    verification::cancel_active(&state.verification).await;

    match verification::request(
        &client,
        state.sink.clone(),
        state.verification.clone(),
        &target,
    )
    .await
    {
        Ok(room_id) => {
            // Verifying another user runs in-room inside the direct chat, and
            // an unsubscribed room delivers one timeline event per sync — the
            // other side's acceptance would only turn up if the user happened
            // to open that chat, and the flow would look stalled at "created".
            if let Some(room_id) = room_id {
                state
                    .subscribe(move |rooms| rooms.verification = Some(room_id))
                    .await;
            }
            state.sink.emit(reply_ok(id, json!({ "requested": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// The three things the user can do with a verification on screen.
enum Step {
    Accept,
    Confirm,
    Cancel,
}

async fn verification_step(state: &Arc<State>, id: u64, step: Step) {
    let active = state.verification.lock().await.clone();
    let Some(active) = active else {
        state
            .sink
            .emit(reply_error(id, "no verification is in progress"));
        return;
    };

    let outcome = match step {
        Step::Accept => active.accept().await,
        Step::Confirm => active.confirm().await,
        Step::Cancel => active.cancel().await,
    };

    match outcome {
        Ok(()) => state
            .sink
            .emit(reply_ok(id, json!({ "flowId": active.flow_id() }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn open_timeline(state: &Arc<State>, id: u64, room_id: String, focus: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // One open at a time. Every command runs in its own task, so leaving a room
    // and stepping straight back into it puts a close and an open in flight
    // together; without this, both could find an empty slot, neither would shut
    // the other's stream down, and whichever finished last would decide which
    // room the next message is sent to. A lock of its own rather than the
    // timeline's, which must stay free for the commands that are running.
    let _opening = state.opening.lock().await;

    // Opening a different room replaces the previous timeline: a phone shows
    // one room at a time, and keeping stale streams alive only costs memory.
    // A focused open always rebuilds, even for the same room — the view is a
    // different one.
    //
    // The guard is bound to a name on purpose. Written as
    // `if let Some(previous) = state.timeline.lock().await.take()`, the
    // temporary guard lives until the end of the `if let` body in edition 2021,
    // so locking again inside it deadlocks a mutex that is not reentrant — and
    // that mutex is the one every other timeline command needs.
    {
        let mut open = state.timeline.lock().await;
        if let Some(previous) = open.take() {
            if previous.room_id() == room_id && focus.is_empty() && previous.is_live() {
                open.replace(previous);
                state.sink.emit(reply_ok(id, json!({ "open": true })));
                return;
            }
            previous.close();
        }
    }

    // Sliding sync only sends a minimal timeline for rooms in the list — one
    // event per response. A room that is being read has to be subscribed
    // explicitly, or its newer messages never arrive and the view shows
    // whatever the event cache happened to hold.
    if let Ok(parsed) = matrix_sdk::ruma::RoomId::parse(&room_id) {
        state
            .subscribe(move |rooms| rooms.open = Some(parsed))
            .await;
    }

    match timeline::open(&client, &room_id, &focus, state.sink.clone()).await {
        Ok(handle) => {
            *state.timeline.lock().await = Some(Arc::new(handle));
            state.sink.emit(reply_ok(id, json!({ "open": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn close_timeline(state: &Arc<State>, id: u64) {
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close();
    }
    state.sink.emit(reply_ok(id, json!({ "open": false })));
}

async fn paginate_timeline(state: &Arc<State>, id: u64) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.paginate().await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(reached_start) => state
            .sink
            .emit(reply_ok(id, json!({ "reachedStart": reached_start }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn send_message(state: &Arc<State>, id: u64, body: String) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "nothing to send"));
        return;
    }

    let outcome = match state.timeline().await {
        Some(handle) => handle.send_text(body).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn reply_message(state: &Arc<State>, id: u64, event_id: String, body: String) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "a reply cannot be empty"));
        return;
    }

    let outcome = match state.timeline().await {
        Some(handle) => handle.reply(&event_id, body).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn edit_message(state: &Arc<State>, id: u64, event_id: String, body: String) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "an edit cannot be empty"));
        return;
    }

    let outcome = match state.timeline().await {
        Some(handle) => handle.edit(&event_id, body).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "edited": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn redact_message(state: &Arc<State>, id: u64, event_id: String) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.redact(&event_id).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "deleted": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn send_media(state: &Arc<State>, id: u64, path: String, mime_type: String) {
    // Cloned out of the guard: the attachment upload takes a while and must
    // not hold the lock.
    let timeline = state.timeline().await.map(|handle| handle.timeline());

    let Some(timeline) = timeline else {
        state.sink.emit(reply_error(id, "no timeline is open"));
        return;
    };

    match media::send(&timeline, &path, &mime_type).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// Forwards either a picture or a piece of text to another room, without
/// disturbing the timeline that is currently open.
async fn forward(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    body: String,
    path: String,
    mime_type: String,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let outcome = if !path.is_empty() {
        let mime = if mime_type.is_empty() {
            "application/octet-stream".to_owned()
        } else {
            mime_type
        };
        media::forward_file(&client, &room_id, &path, &mime).await
    } else if !body.trim().is_empty() {
        media::forward_text(&client, &room_id, body).await
    } else {
        Err("nothing to forward".to_owned())
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "forwarded": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn fetch_media(state: &Arc<State>, id: u64, source: Value, thumbnail: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match media::fetch(&client, &state.paths.media_cache, source, thumbnail).await {
        Ok(path) => state.sink.emit(reply_ok(id, json!({ "path": path }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn mark_read(state: &Arc<State>, id: u64) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.mark_read().await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "read": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// Shared shape of the call commands: they all need a signed-in client and
/// answer with either ok or the reason.
/// Keeps the room of a call being set up or running in the subscription set.
///
/// Only the outgoing side and the moment of answering pass through here, so an
/// invitation that arrives while the app is idle is still read from the
/// unsubscribed stream until the user picks up. Handling that would mean giving
/// `call::install`'s event handlers a way back into this state.
async fn set_call_room(state: &Arc<State>, room_id: Option<&str>) {
    let parsed = room_id.and_then(|room| RoomId::parse(room).ok());
    if room_id.is_some() && parsed.is_none() {
        return;
    }
    state.subscribe(move |rooms| rooms.call = parsed).await;
}

async fn call_step<F, Fut>(state: &Arc<State>, id: u64, action: F)
where
    F: FnOnce(Client) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match action(client).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn turn_servers(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match call::turn_servers(&client).await {
        Ok(servers) => state.sink.emit(reply_ok(id, servers)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn direct_chat(state: &Arc<State>, id: u64, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::direct_chat(&client, &user_id).await {
        Ok(room_id) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn join_by_alias(state: &Arc<State>, id: u64, alias: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::join_by_alias(&client, &alias).await {
        Ok(room_id) => state
            .sink
            .emit(reply_ok(id, json!({ "joined": true, "roomId": room_id }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn join_room(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::join(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "joined": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// The running room list's service, starting the list first if needed.
///
/// The space list and a space's rooms ride on the room list's sync service,
/// and which page runs first depends on the user's start-page choice — so
/// every entry point ensures the sync is up instead of assuming an order.
/// The lock is held across the start so two racing commands cannot start two
/// sync services.
async fn ensure_room_list(
    state: &Arc<State>,
) -> Result<std::sync::Arc<matrix_sdk_ui::room_list_service::RoomListService>, String> {
    let mut rooms = state.rooms.lock().await;
    if rooms.is_none() {
        let client = state.client().await.ok_or_else(|| "not signed in".to_owned())?;
        let handle = roomlist::start(&client, state.sink.clone()).await?;
        *rooms = Some(handle);
    }
    Ok(rooms.as_ref().expect("just ensured").service())
}

async fn start_room_list(state: &Arc<State>, id: u64) {
    match ensure_room_list(state).await {
        Ok(_) => state.sink.emit(reply_ok(id, json!({ "running": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn filter_room_list(state: &Arc<State>, id: u64, pattern: String) {
    let applied = match &*state.rooms.lock().await {
        Some(handle) => handle.set_filter(pattern),
        None => false,
    };

    if applied {
        state.sink.emit(reply_ok(id, json!({ "filtered": true })));
    } else {
        state.sink.emit(reply_error(id, "the room list is not running"));
    }
}

async fn stop_room_list(state: &Arc<State>, id: u64) {
    // The space streams borrow the same room list service, so they go first.
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }
    if let Some(task) = state.spaces.lock().await.take() {
        task.abort();
    }
    if let Some(handle) = state.rooms.lock().await.take() {
        handle.stop().await;
    }
    state.sink.emit(reply_ok(id, json!({ "running": false })));
}

async fn start_spaces(state: &Arc<State>, id: u64) {
    if state.spaces.lock().await.is_some() {
        state.sink.emit(reply_ok(id, json!({ "running": true })));
        return;
    }

    let service = match ensure_room_list(state).await {
        Ok(service) => service,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };

    match roomlist::spawn_spaces(service, state.sink.clone()).await {
        Ok(task) => {
            *state.spaces.lock().await = Some(task);
            state.sink.emit(reply_ok(id, json!({ "running": true })));
            emit_space_children(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// Emits the space child structure now and once more after a short delay, to
/// catch the change once it has synced back — a state event written with
/// `send_state_event` only lands in the local store on the next sync.
fn emit_space_children_soon(state: &Arc<State>) {
    let state = state.clone();
    tokio::spawn(async move {
        emit_space_children(&state).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        emit_space_children(&state).await;
    });
}

async fn stop_spaces(state: &Arc<State>, id: u64) {
    if let Some(task) = state.spaces.lock().await.take() {
        task.abort();
    }
    state.sink.emit(reply_ok(id, json!({ "running": false })));
}

/// Recomputes the per-space child structure and pushes it to the front end,
/// which turns it into the count badge on each space. Called whenever that
/// structure can have changed — the space list started, a space created, a
/// child added or removed.
async fn emit_space_children(state: &Arc<State>) {
    if let Some(client) = state.client().await {
        let data = roomlist::space_children_map(&client).await;
        state.sink.emit(event("spaces.children", data));
    }
}

async fn open_space(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let service = match ensure_room_list(state).await {
        Ok(service) => service,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };

    // Opening a different space replaces the previous one: the UI shows one
    // space's rooms at a time, mirroring how the timeline is handled.
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }

    match roomlist::spawn_space_children(&client, &room_id, service, state.sink.clone()).await {
        Ok(task) => {
            *state.open_space.lock().await = Some(task);
            state.sink.emit(reply_ok(id, json!({ "open": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn close_space(state: &Arc<State>, id: u64) {
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }
    state.sink.emit(reply_ok(id, json!({ "open": false })));
}

async fn create_space(state: &Arc<State>, id: u64, name: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // The new space arrives in the overview through the running sync, so the
    // reply only has to report the id.
    match roomlist::create_space(&client, &name).await {
        Ok(room_id) => {
            state.sink.emit(reply_ok(id, json!({ "roomId": room_id })));
            emit_space_children(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn create_room(state: &Arc<State>, id: u64, name: String, encrypted: bool, public: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // The room reaches the list through the running sync. The reply carries the
    // name and the encryption state back so the front end can open the room
    // under its title, and with the right state, before the first diff.
    match roomlist::create_room(&client, &name, encrypted, public).await {
        Ok(room_id) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "name": name.trim(), "encrypted": encrypted }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn leave_room(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // The row disappears on its own: leaving turns the room's state to Left,
    // which the "non left" filter drops on the next diff. A space that held the
    // room has one child less, so its badge is recomputed.
    match roomlist::leave_room(&client, &room_id).await {
        Ok(()) => {
            state
                .sink
                .emit(reply_ok(id, json!({ "left": true, "roomId": room_id })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn invite_to_room(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::invite_user(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "invited": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn enable_room_encryption(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::enable_encryption(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "encrypted": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn leave_space(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // The space list updates on its own: leaving turns the room's state to
    // Left, which the "non left" filter drops on the next diff.
    match roomlist::leave_space(&client, &room_id).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "left": true })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn add_space_child(state: &Arc<State>, id: u64, space_id: String, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match roomlist::add_child(&client, &space_id, &room_id).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "added": true })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn remove_space_child(state: &Arc<State>, id: u64, space_id: String, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match roomlist::remove_child(&client, &space_id, &room_id).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "removed": true })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn restore_session(state: &Arc<State>, id: u64) {
    let Some(stored) = session::load(&state.paths.session_file) else {
        state.sink.emit(reply_ok(id, json!({ "state": "none" })));
        return;
    };

    let homeserver = stored.homeserver.clone();
    let client = match session::build_client(&homeserver, &state.paths).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("could not open the session: {error}")));
            return;
        }
    };

    if let Err(error) = client
        .oauth()
        .restore_session(stored.into_oauth(), RoomLoadSettings::default())
        .await
    {
        state
            .sink
            .emit(reply_error(id, format!("session no longer valid: {error}")));
        return;
    }

    verification::install(&client, state.sink.clone(), state.verification.clone());
    call::install(&client, state.sink.clone());
    // The backup unlocks itself once a verification hands over the key; only
    // this stream says so.
    recovery::watch(&client, state.sink.clone());
    watch_session(state, &client, homeserver);

    *state.client.lock().await = Some(client);
    let data = state.session_data().await;
    state.sink.emit(reply_ok(id, data.clone()));
    state.sink.emit(event("session.changed", data));
}

async fn start_login(state: &Arc<State>, id: u64, homeserver: String) {
    // A login without a stored session starts a new device, so anything the
    // previous one left behind has to go first — otherwise the crypto store
    // still describes a device this session is not.
    if session::load(&state.paths.session_file).is_none() {
        if let Err(error) = session::reset_store(&state.paths) {
            state
                .sink
                .emit(reply_error(id, format!("could not clear old data: {error}")));
            return;
        }
    }

    if let Err(error) = state.paths.prepare() {
        state
            .sink
            .emit(reply_error(id, format!("could not prepare storage: {error}")));
        return;
    }

    let client = match session::build_client(&homeserver, &state.paths).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("homeserver unreachable: {error}")));
            return;
        }
    };

    let pending = match login::start(&client, homeserver).await {
        Ok(pending) => pending,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };

    let url = pending.url.to_string();
    *state.client.lock().await = Some(client.clone());
    *state.pending.lock().await = Some(PendingLogin {
        shutdown: pending.redirect.shutdown_handle(),
        state: pending.state,
    });

    // The URL goes back right away; the flow itself finishes whenever the user
    // is done in the browser.
    state.sink.emit(reply_ok(id, json!({ "url": url })));

    let waiter = state.clone();
    let homeserver = pending.homeserver;
    tokio::spawn(async move {
        let outcome = login::finish(&client, pending.redirect).await;
        waiter.pending.lock().await.take();

        match outcome {
            Ok(true) => {
                verification::install(&client, waiter.sink.clone(), waiter.verification.clone());
                call::install(&client, waiter.sink.clone());
                recovery::watch(&client, waiter.sink.clone());
                persist(&waiter, &client, homeserver.clone()).await;
                watch_session(&waiter, &client, homeserver);
                let data = waiter.session_data().await;
                waiter.sink.emit(event("session.changed", data));
            }
            Ok(false) => {
                waiter
                    .sink
                    .emit(event("login.aborted", json!({ "state": "none" })));
            }
            Err(message) => {
                waiter
                    .sink
                    .emit(event("login.failed", json!({ "message": message })));
            }
        }
    });
}

/// Begins the device-code login: like `start_login`, but instead of a browser
/// URL the reply carries a short verification URL and a code, and completion
/// means the user approved the login on some other device.
async fn start_device_login(state: &Arc<State>, id: u64, homeserver: String) {
    // Same rule as the browser flow: a fresh login is a fresh device, so a
    // leftover crypto store from a previous device has to go.
    if session::load(&state.paths.session_file).is_none() {
        if let Err(error) = session::reset_store(&state.paths) {
            state
                .sink
                .emit(reply_error(id, format!("could not clear old data: {error}")));
            return;
        }
    }

    if let Err(error) = state.paths.prepare() {
        state
            .sink
            .emit(reply_error(id, format!("could not prepare storage: {error}")));
        return;
    }

    let client = match session::build_client(&homeserver, &state.paths).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("homeserver unreachable: {error}")));
            return;
        }
    };

    let pending = match login::start_device(&client).await {
        Ok(pending) => pending,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };

    *state.client.lock().await = Some(client.clone());

    // URL and code go back right away; the polling task finishes whenever the
    // user approves on the other device.
    state.sink.emit(reply_ok(
        id,
        json!({
            "verificationUri": pending.verification_uri,
            "verificationUriComplete": pending.verification_uri_complete,
            "userCode": pending.user_code,
        }),
    ));

    let waiter = state.clone();
    let task = tokio::spawn(async move {
        let outcome = login::finish_device(&client, pending).await;
        waiter.pending_device.lock().await.take();

        match outcome {
            Ok(()) => {
                verification::install(&client, waiter.sink.clone(), waiter.verification.clone());
                call::install(&client, waiter.sink.clone());
                recovery::watch(&client, waiter.sink.clone());
                persist(&waiter, &client, homeserver.clone()).await;
                watch_session(&waiter, &client, homeserver);
                let data = waiter.session_data().await;
                waiter.sink.emit(event("session.changed", data));
            }
            Err(message) => {
                waiter
                    .sink
                    .emit(event("login.failed", json!({ "message": message })));
            }
        }
    });
    *state.pending_device.lock().await = Some(task);
}

/// Writes the freshly obtained session to disk. A failure here is not fatal for
/// the running session, only for surviving a restart, so it is reported as an
/// event rather than aborting the login.
async fn persist(state: &Arc<State>, client: &Client, homeserver: String) {
    let Some(oauth_session) = client.oauth().full_session() else {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": "session could not be persisted" }),
        ));
        return;
    };

    let stored = StoredSession::from_oauth(homeserver, &oauth_session);
    if let Err(error) = session::store(&stored, &state.paths.session_file) {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": format!("session could not be saved: {error}") }),
        ));
    }
}

/// Keeps `session.json` in step with the SDK's tokens for the life of the
/// client, started once after login and once after a restore.
///
/// OAuth refresh tokens rotate: `handle_refresh_tokens` trades the old one for a
/// new pair in the background, and the server invalidates the old refresh token
/// the instant it is used. Persisting only at login leaves the file holding a
/// spent refresh token; the next start replays it and the server answers
/// `invalid_grant`, which looks like a forced re-login on every launch — made
/// worse here because there is no background process, so the client restarts
/// constantly. Writing the refreshed session back is what stops that.
///
/// This is a broadcast subscription, not a `Client::add_event_handler` handler,
/// so listening in a spawned task does not wedge the sync loop.
fn watch_session(state: &Arc<State>, client: &Client, homeserver: String) {
    let state = state.clone();
    let client = client.clone();
    let mut changes = client.subscribe_to_session_changes();
    tokio::spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SessionChange::TokensRefreshed) => {
                    persist(&state, &client, homeserver.clone()).await;
                }
                Ok(SessionChange::UnknownToken(_)) => {
                    // The refresh token is gone for good and only a new login can
                    // recover. Say so, rather than leaving the UI to show empty
                    // rooms that look like a bug.
                    state.sink.emit(event(
                        "session.expired",
                        json!({ "message": "the session has expired, please sign in again" }),
                    ));
                }
                // Missing a refresh notification only costs a redundant write
                // next time; keep listening.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
}

async fn registration_url(state: &Arc<State>, id: u64, homeserver: String) {
    if let Err(error) = state.paths.prepare() {
        state
            .sink
            .emit(reply_error(id, format!("could not prepare storage: {error}")));
        return;
    }

    let client = match session::build_client(&homeserver, &state.paths).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("homeserver unreachable: {error}")));
            return;
        }
    };

    match login::registration_url(&client).await {
        Ok(url) => state.sink.emit(reply_ok(id, json!({ "url": url }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn abort_login(state: &Arc<State>, id: u64) {
    if let Some(pending) = state.pending.lock().await.take() {
        pending.shutdown.shutdown();
        if let Some(client) = &*state.client.lock().await {
            client.oauth().abort_login(&pending.state).await;
        }
    }
    // A device-code login is just a polling task; dropping it is the abort.
    if let Some(task) = state.pending_device.lock().await.take() {
        task.abort();
    }
    state.sink.emit(reply_ok(id, json!({ "state": "none" })));
}

async fn logout(state: &Arc<State>, id: u64) {
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close();
    }
    if let Some(handle) = state.directory.lock().await.take() {
        handle.task.abort();
    }
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }
    if let Some(task) = state.spaces.lock().await.take() {
        task.abort();
    }
    if let Some(handle) = state.rooms.lock().await.take() {
        handle.stop().await;
    }
    let client = state.client.lock().await.take();
    if let Some(client) = client {
        // Best effort: even if the server cannot be reached, the local session
        // must go away.
        let _ = client.oauth().logout().await;
    }
    session::forget(&state.paths.session_file);
    if let Err(error) = session::reset_store(&state.paths) {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": format!("local data could not be cleared: {error}") }),
        ));
    }

    state.sink.emit(reply_ok(id, json!({ "state": "none" })));
    state
        .sink
        .emit(event("session.changed", json!({ "state": "none" })));
}
