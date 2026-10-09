//! The command dispatcher and the callback sink. Each command is its own
//! task, so a login that waits minutes for the browser blocks nothing.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use matrix_sdk::{
    authentication::oauth::{error::OAuthDiscoveryError, CsrfToken},
    ruma::{OwnedRoomId, RoomId},
    store::RoomLoadSettings,
    utils::local_server::LocalServerShutdownHandle,
    Client, SessionChange,
};
use serde_json::{json, Value};
use tokio::sync::{broadcast::error::RecvError, mpsc, Mutex};

use crate::call;
use crate::directory;
use crate::gate::{self, Entry, LoginKind, Phase, Slot};
use crate::login;
use crate::profile;
use crate::linkpreview;
use crate::location;
use crate::poll;
use crate::roomsettings;
use crate::sendqueue;
use crate::private;
use crate::protocol::{event, reply_error, reply_ok, Command, MediaStill, Secret};
use crate::media;
use crate::members;
use crate::mention;
use crate::recovery;
use crate::roomlist::{self, RoomListHandle};
use crate::session::{self, Paths, StoredSession};
use crate::search;
use crate::storehealth;
use crate::timeline::{self, TimelineHandle};
use crate::verification;
use leghorn::Leghorn;

mod lifecycle;
mod account;
use account::*;
mod roomactions;
use roomactions::*;
mod roomdirectory;
use roomdirectory::*;
mod searching;
use searching::*;
mod membership;
use membership::*;
mod pushcmd;
use pushcmd::*;
mod storage;
use storage::*;
mod encryption;
use encryption::*;
mod conversation;
use conversation::*;
mod calls;
use calls::*;
mod lists;
use lists::*;
use lifecycle::{
    abort_login, logout, password_login, rebuild_store, registration_url, restore_session,
    start_device_login, start_login,
};

/// Signature of the function the front end registers to receive messages.
pub type XmCallback = extern "C" fn(*mut c_void, *const c_char);

/// The most results one search command may ask for: a page fills a screen and
/// is asked for again. Every row costs an event load.
const SEARCH_PAGE_MAX: usize = 50;

struct CallbackSlot {
    func: Option<XmCallback>,
    user_data: *mut c_void,
}

// The pointer is opaque to Rust and only handed back with a message. C++ owns
// it; `set_callback` waits out every call in flight before it returns.
unsafe impl Send for CallbackSlot {}
unsafe impl Sync for CallbackSlot {}

/// Delivers JSON messages to the front end.
pub struct Sink {
    slot: std::sync::RwLock<CallbackSlot>,
}

impl Sink {
    pub fn new() -> Self {
        Self {
            slot: std::sync::RwLock::new(CallbackSlot {
                func: None,
                user_data: std::ptr::null_mut(),
            }),
        }
    }

    /// Blocks until no delivery is running: afterwards the old `user_data` is
    /// never touched again, so the front end may free it.
    pub fn set_callback(&self, func: Option<XmCallback>, user_data: *mut c_void) {
        let mut slot = self.slot.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        slot.func = func;
        slot.user_data = user_data;
    }

    /// Serialises `value` and hands it to the front end. Messages sent before a
    /// callback is registered are dropped.
    pub fn emit(&self, value: Value) {
        let Ok(text) = serde_json::to_string(&value) else { return };
        let Ok(message) = CString::new(text) else { return };

        // Read guard across the call: emitters run side by side, `set_callback`
        // waits. The callback must never call back into the core.
        let slot = self.slot.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(func) = slot.func else { return };
        let _ = catch_unwind(AssertUnwindSafe(|| func(slot.user_data, message.as_ptr())));
    }
}

type Gate<'a> = tokio::sync::MutexGuard<'a, ()>;

/// How long discovery may hold the gate. A server that never answers must not
/// keep a sign-out waiting.
const BUILD_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// A sign-in's network part, and the command it may still owe an answer.
struct LoginTask {
    handle: tokio::task::JoinHandle<()>,
    reply_to: u64,
    answered: Arc<std::sync::atomic::AtomicBool>,
}

/// Answers a sign-in command once, whoever gets there first. Set and sent with
/// no await between: an abort cannot fall in the middle.
fn answer_once(sink: &Sink, answered: &std::sync::atomic::AtomicBool, reply: Value) {
    if !answered.swap(true, std::sync::atomic::Ordering::SeqCst) {
        sink.emit(reply);
    }
}

/// Spawns a sign-in's whole network part as the task `cancel_login` stops.
/// Under the gate, so it cannot reach its own gate before it is findable.
async fn spawn_login<F>(
    state: &Arc<State>,
    _gate: &Gate<'_>,
    id: u64,
    work: impl FnOnce(Arc<std::sync::atomic::AtomicBool>) -> F,
) where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let answered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let work = work(answered.clone());
    let sink = state.sink.clone();
    let flag = answered.clone();
    let handle = tokio::spawn(async move {
        let outcome = futures_util::FutureExt::catch_unwind(AssertUnwindSafe(work)).await;
        if outcome.is_err() {
            answer_once(&sink, &flag, reply_error(id, "the sign-in failed unexpectedly"));
        }
    });
    *state.login_task.lock().await = Some(LoginTask { handle, reply_to: id, answered });
}

/// A login waiting for the user, kept so it can be cancelled.
struct PendingLogin {
    shutdown: LocalServerShutdownHandle,
    state: CsrfToken,
}

struct State {
    paths: Paths,
    /// The store key; `None` opens nothing new. Behind a lock because the front
    /// end may hand it in later, after the user retried a locked collection.
    store_key: std::sync::RwLock<Option<session::StoreKey>>,
    client: Mutex<Option<Client>>,
    pending: Mutex<Option<PendingLogin>>,
    /// What `client` is for; see `gate`. Changed only under `session_gate`.
    slot: std::sync::Mutex<Slot>,
    /// The network part of a sign-in, kept so a sign-out or the next sign-in
    /// can stop it - and wait until its client clone is gone.
    login_task: Mutex<Option<LoginTask>>,
    rooms: Mutex<Option<RoomListHandle>>,
    /// The UnifiedPush connector. Runs only while push is on or the push page asks.
    push: Mutex<Option<Arc<Leghorn>>>,
    push_listener: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Serialises pusher changes with each other and with the sign-out.
    push_sync: Mutex<()>,
    /// Pushkey and gateway last registered with the homeserver.
    push_registered: Mutex<Option<(String, String)>>,
    /// The gateway the user picked.
    push_gateway: std::sync::Mutex<crate::push::Gateway>,
    /// The woken process's connector, for `push.yield`.
    push_wake: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
    push_yielded: std::sync::atomic::AtomicBool,
    spaces: Mutex<Option<tokio::task::JoinHandle<()>>>,
    open_space: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Behind an `Arc` so a command can clone the handle out and release the
    /// lock before going to the server — see `State::timeline`.
    timeline: Mutex<Option<Arc<TimelineHandle>>>,
    /// The open thread's timeline, next to the room's — same locking rules.
    thread: Mutex<Option<Arc<TimelineHandle>>>,
    /// Serialises `open_timeline` against itself. Held for the whole open,
    /// including the network part, which is why it cannot be `timeline`.
    opening: Mutex<()>,
    /// The same for `open_thread`, and deliberately not `opening`: a thread
    /// that is slow to build must not hold up a room switch.
    opening_thread: Mutex<()>,
    /// Serialises `direct_chat`: a second tap waits and finds the first room.
    opening_direct: Mutex<()>,
    /// Held for every change to the client, the store, the store key and
    /// `session.json`. Never across a sign-in's network wait.
    session_gate: Mutex<()>,
    /// Every room that currently needs a sliding-sync subscription. See
    /// `Subscriptions` — they have to be requested together or not at all.
    subscriptions: Mutex<Subscriptions>,
    /// The call whose room is subscribed: only its own hangup releases it.
    call_subscribed: std::sync::Mutex<Option<String>>,
    directory: Mutex<Option<DirectoryHandle>>,
    /// Observers that outlive a command. Each holds a client clone and keeps the
    /// SQLite pool open, so they are stopped before `reset_store` runs.
    observers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    verification: verification::Slot,
    /// How many command tasks are in flight. They cannot be aborted mid-send, so
    /// they are counted and the sign-out waits briefly for the count to fall.
    running: std::sync::atomic::AtomicUsize,
    sink: Arc<Sink>,
}

/// The rooms needing a sliding-sync subscription, by what wants them.
/// `set_room_subscriptions` is not additive - a call forgets every earlier one.
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
    /// The signed-in client. A sign-in's client is not one to sync or send
    /// with; the sign-in code reads the slot itself.
    async fn client(&self) -> Option<Client> {
        if self.slot().phase != Phase::Session {
            return None;
        }
        self.client.lock().await.clone()
    }

    /// The open timeline, cloned like the client: every timeline command is a
    /// round trip, and holding the lock across one froze every room switch.
    async fn timeline(&self) -> Option<Arc<TimelineHandle>> {
        self.timeline.lock().await.clone()
    }

    /// Live only: a send on a slice takes the slice's thread.
    async fn sendable_timeline(&self) -> Result<Arc<TimelineHandle>, String> {
        match self.timeline().await {
            Some(handle) if handle.is_live() => Ok(handle),
            Some(_) => Err("not in the live conversation; nothing is sent from here".to_owned()),
            None => Err("no timeline is open".to_owned()),
        }
    }

    async fn thread(&self) -> Option<Arc<TimelineHandle>> {
        self.thread.lock().await.clone()
    }

    fn slot(&self) -> Slot {
        *self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn with_slot<T>(&self, _gate: &Gate<'_>, change: impl FnOnce(&mut Slot) -> T) -> T {
        change(&mut self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    /// Puts `client` in place; the generation returned names it from now on.
    async fn assign(&self, gate: &Gate<'_>, client: Client, phase: Phase) -> u64 {
        *self.client.lock().await = Some(client);
        self.with_slot(gate, |slot| slot.assigned(phase))
    }

    /// Takes the client out, whoever held it.
    async fn unassign(&self, gate: &Gate<'_>) -> Option<Client> {
        let client = self.client.lock().await.take();
        self.with_slot(gate, |slot| slot.cleared());
        client
    }

    /// Takes the client out only if it is still the one `generation` names.
    async fn unassign_if(&self, gate: &Gate<'_>, generation: u64) {
        if self.slot().holds(generation) {
            drop(self.unassign(gate).await);
        }
    }

    /// Waits briefly for running commands. Bounded: a command in the SDK's retry
    /// budget must not hold a sign-out for a quarter of an hour.
    async fn drain_commands(&self, limit: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + limit;
        while self.running.load(std::sync::atomic::Ordering::SeqCst) > 1
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// Records what a part of the app needs and asks for the whole set. Always the
    /// whole set: a call with fewer rooms unsubscribes the rest.
    async fn subscribe(&self, change: impl FnOnce(&mut Subscriptions)) {
        // The lock is held across the request: the call replaces the whole list, so a
        // second caller getting ahead would leave the older list standing.
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
        service.set_room_subscriptions(&borrowed).await;
    }

    /// Describes the current session for a reply or an event.
    /// A copy of the store key, if there is one; the copy is zeroized on drop.
    fn store_key(&self) -> Option<session::StoreKey> {
        self.store_key
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

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
    store_key: Option<session::StoreKey>,
    sink: Arc<Sink>,
) -> mpsc::UnboundedSender<Command> {
    let (sender, mut receiver) = mpsc::unbounded_channel::<Command>();

    let state = Arc::new(State {
        paths,
        store_key: std::sync::RwLock::new(store_key),
        client: Mutex::new(None),
        pending: Mutex::new(None),
        slot: std::sync::Mutex::new(Slot::default()),
        login_task: Mutex::new(None),
        rooms: Mutex::new(None),
        push: Mutex::new(None),
        push_listener: Mutex::new(None),
        push_sync: Mutex::new(()),
        push_registered: Mutex::new(None),
        push_gateway: std::sync::Mutex::new(crate::push::Gateway::Unset),
        push_wake: std::sync::Mutex::new(None),
        push_yielded: std::sync::atomic::AtomicBool::new(false),
        spaces: Mutex::new(None),
        open_space: Mutex::new(None),
        timeline: Mutex::new(None),
        thread: Mutex::new(None),
        opening: Mutex::new(()),
        opening_thread: Mutex::new(()),
        opening_direct: Mutex::new(()),
        session_gate: Mutex::new(()),
        subscriptions: Mutex::new(Subscriptions::default()),
        call_subscribed: std::sync::Mutex::new(None),
        directory: Mutex::new(None),
        observers: Mutex::new(Vec::new()),
        verification: Arc::new(Mutex::new(None)),
        running: std::sync::atomic::AtomicUsize::new(0),
        sink,
    });

    runtime.spawn(async move {
        while let Some(command) = receiver.recv().await {
            let state = state.clone();
            // Boxed: the combined future of all command arms is large, and
            // moving it to the heap keeps the task allocation small.
            state.running.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::spawn(Box::pin(async move {
                let counted = state.clone();
                let id = command.id();
                // A panic here would take the reply and the counter with it, and
                // the protocol promises exactly one answer per id.
                let outcome = futures_util::FutureExt::catch_unwind(
                    std::panic::AssertUnwindSafe(handle(state, command)),
                )
                .await;
                if outcome.is_err() {
                    counted
                        .sink
                        .emit(reply_error(id, "the command failed unexpectedly"));
                }
                counted.running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }));
        }
    });

    sender
}

async fn handle(state: Arc<State>, command: Command) {
    let id = command.id();
    match command {
        Command::SessionRestore { store_key, .. } => restore_session(&state, id, store_key).await,
        Command::SessionRebuildStore { .. } => rebuild_store(&state, id).await,
        Command::LoginStart { homeserver, .. } => start_login(&state, id, homeserver).await,
        Command::LoginPassword {
            homeserver,
            user,
            password,
            ..
        } => password_login(&state, id, homeserver, user, password).await,
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
        Command::RoomListMore { .. } => load_more_rooms(&state, id).await,
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
        Command::TimelineOpen {
            room_id,
            focus,
            receipts,
            token,
            rebuild,
            ..
        } => open_timeline(&state, id, room_id, focus, receipts, token, rebuild).await,
        Command::RoomMarkRead { room_id, receipt, .. } => {
            mark_room_read(&state, id, room_id, receipt).await
        }
        Command::RoomResolve { address, .. } => resolve_room(&state, id, address).await,
        Command::TimelineClose { room_id, .. } => close_timeline(&state, id, room_id).await,
        Command::TimelinePaginate { room_id, .. } => {
            paginate_timeline(&state, id, room_id).await
        }
        Command::TimelineSend { body, mentions, .. } => {
            send_message(&state, id, body, mentions).await
        }
        Command::TimelineMarkRead { receipt, .. } => mark_read(&state, id, receipt).await,
        Command::PrivateGet { .. } => {
            match private::load(&state.paths.private_file, state.store_key().as_ref()) {
                private::Loaded::Lists(lists) => {
                    state.sink.emit(reply_ok(id, json!({ "lists": lists, "readable": true })))
                }
                // Empty *and* writable is only true where a key exists. Otherwise the app
                // went into the migration, whose failure asks again - an endless loop.
                private::Loaded::Empty => {
                    let readable = state.store_key().is_some();
                    state
                        .sink
                        .emit(reply_ok(id, json!({ "lists": {}, "readable": readable })))
                }
                // Locked or damaged: say so rather than answering "there is
                // nothing", which is what a write would then destroy.
                private::Loaded::Unreadable => state
                    .sink
                    .emit(reply_ok(id, json!({ "lists": {}, "readable": false }))),
            }
        }

        Command::PrivateSet { lists, .. } => {
            // Under the gate and only signed in: a write landing after a sign-out
            // would bring back the leaving account's lists.
            let _gate = state.session_gate.lock().await;
            if state.slot().phase != Phase::Session {
                state.sink.emit(reply_error(id, "not signed in"));
                return;
            }
            // The front end sends the whole state, so nothing is read first and no two
            // writes collide. The file is only consulted to refuse writing over it.
            match private::load(&state.paths.private_file, state.store_key().as_ref()) {
                private::Loaded::Unreadable => {
                    state.sink.emit(reply_error(
                        id,
                        "the stored lists cannot be read; nothing was changed".to_owned(),
                    ));
                    return;
                }
                _ => {}
            }
            match private::save(&state.paths.private_file, state.store_key().as_ref(), &lists) {
                Ok(()) => state
                    .sink
                    .emit(reply_ok(id, json!({ "lists": lists, "readable": true }))),
                Err(error) => state
                    .sink
                    .emit(reply_error(id, format!("list could not be saved: {error}"))),
            }
        }

        Command::CallsSetPolicy {
            policy,
            groups,
            video,
            flood,
            allowed,
            ..
        } => {
            call::set_policy(call::CallPolicy {
                who: policy,
                groups,
                video,
                flood,
                allowed,
            });
            state.sink.emit(reply_ok(id, json!({ "set": true })));
        }

        Command::RoomPermalink { room_id, .. } => {
            let client = match state.client().await {
                Some(client) => client,
                None => {
                    state.sink.emit(reply_error(id, "not signed in".to_owned()));
                    return;
                }
            };
            match roomlist::permalink(&client, &room_id).await {
                Ok(link) => state.sink.emit(reply_ok(id, json!({ "link": link }))),
                Err(message) => state.sink.emit(reply_error(id, message)),
            }
        }

        Command::TimelineReaders { event_id, .. } => {
            let outcome = match state.timeline().await {
                Some(handle) => handle.readers(&event_id).await,
                None => Err("no timeline is open".to_owned()),
            };
            match outcome {
                Ok(readers) => state
                    .sink
                    .emit(reply_ok(id, json!({ "readers": readers }))),
                Err(message) => state.sink.emit(reply_error(id, message)),
            }
        }
        Command::TimelineReactors { event_id, key, .. } => {
            let outcome = match state.timeline().await {
                Some(handle) => handle.reactors(&event_id, &key).await,
                None => Err("no timeline is open".to_owned()),
            };
            match outcome {
                Ok(reactors) => state.sink.emit(reply_ok(
                    id,
                    json!({ "eventId": event_id, "key": key, "reactors": reactors }),
                )),
                Err(message) => state.sink.emit(reply_error(id, message)),
            }
        }
        Command::TimelineReply { event_id, body, mentions, .. } => {
            reply_message(&state, id, event_id, body, mentions).await
        }
        Command::TimelineEdit { event_id, body, .. } => edit_message(&state, id, event_id, body).await,
        Command::TimelineReact { event_id, key, .. } => react(&state, id, event_id, key).await,
        Command::LinkPreview { url, .. } => {
            linkpreview::handle(state.client().await, &state.sink, id, url).await
        }
        // core/src/roomsettings.rs.
        Command::RoomSettingsLoad { .. }
        | Command::RoomSettingsSetName { .. }
        | Command::RoomSettingsSetTopic { .. }
        | Command::RoomSettingsSetAvatar { .. }
        | Command::RoomSettingsRemoveAvatar { .. } => {
            roomsettings::handle(command, state.client().await, &state.sink).await
        }
        // Routed, not handled: the poll rules live in core/src/poll.rs.
        // A new poll could take a slice's thread; vote and end carry their relation.
        Command::PollStart { .. } => match state.sendable_timeline().await {
            Ok(handle) => poll::handle(command, Some(handle.timeline()), &state.sink).await,
            Err(message) => state.sink.emit(reply_error(id, message)),
        },
        Command::PollVote { .. } | Command::PollEnd { .. } => {
            let timeline = state.timeline().await.map(|handle| handle.timeline());
            poll::handle(command, timeline, &state.sink).await
        }
        // Routed, not handled: core/src/sendqueue.rs.
        Command::QueueStuck { .. } | Command::QueueRetry { .. } | Command::QueueDiscard { .. } => {
            sendqueue::handle(command, state.client().await, &state.sink).await
        }
        // Routed, not handled: core/src/location.rs.
        Command::LocationSend { .. }
        | Command::LocationLiveStart { .. }
        | Command::LocationBeacon { .. }
        | Command::LocationLiveStop { .. }
        | Command::LocationTiles { .. } => {
            // A one-off location goes through the timeline: refused in a slice.
            let sendable = state.sendable_timeline().await;
            if let (Command::LocationSend { .. }, Err(message)) = (&command, &sendable) {
                state.sink.emit(reply_error(id, message.clone()));
                return;
            }
            let timeline = sendable.ok().map(|handle| handle.timeline());
            let tiles = state.paths.media_cache.join("tiles");
            location::handle(command, state.client().await, timeline, tiles, &state.sink).await
        }
        Command::TimelineRedact {
            event_id, txn_id, ..
        } => redact_message(&state, id, event_id, txn_id).await,
        Command::TimelineSendMedia {
            path,
            mime_type,
            caption,
            reply_to,
            voice,
            duration,
            width,
            height,
            thumbnail,
            room_id,
            ..
        } => {
            send_media(
                &state, id, path, mime_type, caption, reply_to, voice, duration, width, height,
                thumbnail, room_id,
            )
            .await
        }
        Command::MediaFetch {
            source,
            thumbnail,
            size,
            limit,
            ..
        } => fetch_media(&state, id, source, thumbnail, size, limit).await,
        Command::RoomForward {
            room_id,
            body,
            path,
            mime_type,
            width,
            height,
            thumbnail,
            ..
        } => forward(&state, id, room_id, body, path, mime_type, width, height, thumbnail).await,
        Command::RoomJoin { room_id, .. } => join_room(&state, id, room_id).await,
        Command::RoomFollowSuccessor { room_id, .. } => {
            follow_successor(&state, id, room_id).await
        }
        Command::RoomInfo { room_id, .. } => room_info(&state, id, room_id).await,
        Command::RoomCreate {
            name,
            topic,
            alias,
            encrypted,
            public,
            history_visibility,
            invite,
            federate,
            read_only,
            equal_power,
            ..
        } => {
            create_room(
                &state,
                id,
                roomlist::NewRoom {
                    name,
                    topic,
                    alias,
                    encrypted,
                    public,
                    history_visibility,
                    invite,
                    federate,
                    read_only,
                    equal_power,
                },
            )
            .await
        }
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
            // A call's signalling rides the room timeline, and an unsubscribed room hands
            // out one event per sync - a burst of ICE candidates arrives as its last.
            hold_call_room(&state, &room_id, &call_id).await;
            let client = match state.client().await {
                Some(client) => client,
                None => {
                    state.sink.emit(reply_error(id, "not signed in".to_owned()));
                    return;
                }
            };
            match call::invite(&client, &room_id, &call_id, &party_id, sdp).await {
                // `peer` is who may answer: in a two-person room the one other
                // member, bound before the call rings.
                Ok(peer) => state
                    .sink
                    .emit(reply_ok(id, json!({ "sent": true, "peer": peer }))),
                Err(message) => state.sink.emit(reply_error(id, message)),
            }
        }
        Command::CallAnswer {
            room_id,
            call_id,
            party_id,
            sdp,
            ..
        } => {
            hold_call_room(&state, &room_id, &call_id).await;
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
            let ended = call_id.clone();
            call_step(&state, id, move |client| async move {
                call::hangup(&client, &room_id, &call_id, &party_id).await
            })
            .await;
            // Released after the goodbye is on its way, and only for its own call:
            // a busy reply or a replaced call must not unsubscribe the live one.
            release_call_room(&state, &ended).await;
        }
        Command::CallTurnServers { .. } => turn_servers(&state, id).await,
        Command::VerificationRequest { user_id, .. } => {
            request_verification(&state, id, user_id).await
        }
        Command::VerificationAccept { .. } => verification_step(&state, id, Step::Accept).await,
        Command::VerificationConfirm { .. } => verification_step(&state, id, Step::Confirm).await,
        Command::VerificationCancel { .. } => verification_step(&state, id, Step::Cancel).await,
        Command::VerificationMismatch { .. } => {
            verification_step(&state, id, Step::Mismatch).await
        }
        Command::EncryptionStatus { .. } => encryption_status(&state, id).await,
        Command::StorageStatus { .. } => storage_status(&state, id),
        Command::StorageRepair { .. } => repair_storage(&state, id).await,
        Command::PushStatus { .. } => push_status(&state, id).await,
        Command::PushEnable { .. } => push_enable(&state, id).await,
        Command::PushDisable { .. } => push_disable(&state, id).await,
        Command::PushGateway { mode, gateway, .. } => {
            push_set_gateway(&state, id, mode, gateway).await
        }
        Command::PushWake { .. } => push_wake(&state, id).await,
        Command::PushYield { .. } => push_yield(&state, id),
        Command::PushNotify {
            room_id, event_id, ..
        } => push_notify(&state, id, room_id, event_id).await,
        Command::EncryptionRecover { key, .. } => encryption_recover(&state, id, key).await,
        Command::EncryptionEnableBackup { .. } => encryption_enable_backup(&state, id).await,
        Command::EncryptionFetchKeys { room_id, .. } => fetch_room_keys(&state, id, room_id).await,
        Command::AccountGet { .. } => account_get(&state, id).await,
        Command::AccountSetDisplayName { name, .. } => {
            account_set_display_name(&state, id, name).await
        }
        Command::AccountSetAvatar { path, .. } => account_set_avatar(&state, id, path).await,
        Command::RoomSetNotifyMode { room_id, mode, .. } => {
            room_set_notify_mode(&state, id, room_id, mode).await
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
        Command::SearchIndex { room_id, .. } => search_index(&state, id, room_id).await,
        Command::SearchRoom { room_id, query, offset, limit, .. } => {
            search_room(&state, id, room_id, query, offset, limit).await
        }
        Command::MembersLoad { room_id, .. } => members_load(&state, id, room_id).await,
        // Routed, not handled: what a mention is lives in core/src/mention.rs.
        Command::MentionCandidates { room_id, query, .. } => {
            mention_candidates(&state, id, room_id, query).await
        }
        Command::RoomCheckRecipients { room_id, .. } => {
            room_check_recipients(&state, id, room_id).await
        }
        Command::MemberRemove { room_id, user_id, .. } => {
            member_remove(&state, id, room_id, user_id).await
        }
        Command::MemberProfile { room_id, user_id, .. } => {
            member_profile(&state, id, room_id, user_id).await
        }
        Command::MemberBan { room_id, user_id, .. } => {
            member_ban(&state, id, room_id, user_id).await
        }
        Command::MemberUnban { room_id, user_id, .. } => {
            member_unban(&state, id, room_id, user_id).await
        }
        Command::MemberSetPower {
            room_id,
            user_id,
            power,
            ..
        } => member_set_power(&state, id, room_id, user_id, power).await,
        Command::MemberSetIgnored {
            user_id, ignored, ..
        } => member_set_ignored(&state, id, user_id, ignored).await,
        Command::MemberWithdrawVerification { user_id, .. } => {
            member_withdraw_verification(&state, id, user_id).await
        }
        Command::AccountIgnoredUsers { .. } => account_ignored_users(&state, id).await,
        Command::RoomResetKeys { room_id, .. } => room_reset_keys(&state, id, room_id).await,
        Command::SpaceHierarchy { room_id, .. } => space_hierarchy(&state, id, room_id).await,
        Command::ThreadOpen {
            room_id,
            root_event_id,
            token,
            ..
        } => open_thread(&state, id, room_id, root_event_id, token).await,
        Command::ThreadClose { root_event_id, .. } => {
            close_thread(&state, id, root_event_id).await
        }
        Command::ThreadSend { body, mentions, .. } => {
            send_thread_message(&state, id, body, mentions).await
        }
        Command::ThreadPaginate { .. } => paginate_thread(&state, id).await,
    }
}

/// A line of this core's own into the app's error log, scrubbed like the SDK's.
fn log(state: &Arc<State>, level: &str, message: String) {
    state.sink.emit(event(
        "core.log",
        json!({
            "level": level,
            "target": "xmatic",
            "message": crate::text::scrub_ids(&message),
        }),
    ));
}

#[cfg(test)]
mod sink_tests {
    use super::*;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    // 0 idle, 1 inside the callback, 2 returned from it.
    static PHASE: AtomicU8 = AtomicU8::new(0);

    extern "C" fn slow(_: *mut c_void, _: *const c_char) {
        PHASE.store(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        PHASE.store(2, Ordering::SeqCst);
    }

    #[test]
    fn clearing_the_callback_waits_for_a_delivery_in_flight() {
        let sink = Arc::new(Sink::new());
        sink.set_callback(Some(slow), 1 as *mut c_void);
        let emitter = {
            let sink = sink.clone();
            std::thread::spawn(move || sink.emit(serde_json::json!({})))
        };
        while PHASE.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        sink.set_callback(None, std::ptr::null_mut());
        assert_eq!(PHASE.load(Ordering::SeqCst), 2);
        emitter.join().unwrap();
    }
}
