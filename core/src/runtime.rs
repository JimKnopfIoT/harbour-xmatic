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
use crate::leftrooms;
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
    cross_signing_setup,
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

/// Who still gets an answer. One lock for both sets, so a reply cannot fall
/// between a stand-in answer and its note.
#[derive(Default)]
struct Debts {
    /// Command ids in flight and not yet answered; a reply takes its id out.
    unanswered: std::collections::HashSet<u64>,
    /// Answered by a stand-in; the late real reply is dropped and the id let go.
    settled: std::collections::HashSet<u64>,
}

/// Delivers JSON messages to the front end.
pub struct Sink {
    slot: std::sync::RwLock<CallbackSlot>,
    debts: std::sync::Mutex<Debts>,
}

impl Sink {
    fn debts(&self) -> std::sync::MutexGuard<'_, Debts> {
        self.debts.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn expect_reply(&self, id: u64) {
        let mut debts = self.debts();
        debts.settled.remove(&id);
        debts.unanswered.insert(id);
    }

    /// Whether `id` is still owed an answer; forgets it either way.
    #[cfg(test)]
    fn owed(&self, id: u64) -> bool {
        self.debts().unanswered.remove(&id)
    }

    /// Answers `id` in the command's place if it is still owed; a reply the
    /// command sends later is then dropped. True if this answered.
    fn stand_in(&self, id: u64, reply: Value) -> bool {
        {
            let mut debts = self.debts();
            if !debts.unanswered.remove(&id) {
                return false;
            }
            debts.settled.insert(id);
        }
        self.deliver(&reply);
        true
    }

    /// Answers `id` only if nothing has yet; for a command that cannot reply later.
    fn answer_if_owed(&self, id: u64, reply: Value) {
        if self.debts().unanswered.remove(&id) {
            self.deliver(&reply);
        }
    }

    pub fn new() -> Self {
        Self {
            slot: std::sync::RwLock::new(CallbackSlot {
                func: None,
                user_data: std::ptr::null_mut(),
            }),
            debts: std::sync::Mutex::new(Debts::default()),
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
        if value.get("type").and_then(Value::as_str) == Some("reply") {
            if let Some(id) = value.get("id").and_then(Value::as_u64) {
                let mut debts = self.debts();
                if debts.settled.remove(&id) {
                    return;
                }
                debts.unanswered.remove(&id);
            }
        }
        self.deliver(&value);
    }

    fn deliver(&self, value: &Value) {
        let Ok(text) = serde_json::to_string(value) else { return };
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
    /// The woken process asked the front end for the store key.
    push_unlock_asked: std::sync::atomic::AtomicBool,
    /// Set once that question has an answer, session or not.
    push_unlocked: tokio::sync::watch::Sender<bool>,
    /// A pusher the homeserver did not get; retried when the sync runs again.
    push_dirty: std::sync::atomic::AtomicBool,
    /// A waiter for the next `Running` is out, to pay that debt.
    push_waiting: std::sync::atomic::AtomicBool,
    /// Push work that may hold the client; `teardown` aborts and awaits it.
    push_tasks: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
    spaces: Mutex<Option<tokio::task::JoinHandle<()>>>,
    open_space: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Behind an `Arc` so a command can clone the handle out and release the
    /// lock before going to the server — see `State::timeline`.
    timeline: Mutex<Option<Arc<TimelineHandle>>>,
    /// The open thread's timeline, next to the room's — same locking rules.
    thread: Mutex<Option<Arc<TimelineHandle>>>,
    /// Serialises `open_timeline` and `close_timeline`. Never held across a
    /// request: a build runs outside it, see `pending_open`.
    opening: Mutex<()>,
    /// The same for the thread, and deliberately not `opening`: a thread
    /// that is slow to build must not hold up a room switch.
    opening_thread: Mutex<()>,
    /// Raised by every open and every close that meant the view building; a
    /// build whose number is no longer current is thrown away.
    open_generation: std::sync::atomic::AtomicU64,
    thread_generation: std::sync::atomic::AtomicU64,
    /// The room view or thread being built, stoppable.
    pending_open: std::sync::Mutex<Option<conversation::PendingOpen>>,
    pending_thread: std::sync::Mutex<Option<conversation::PendingOpen>>,
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
    /// Command tasks in flight. A teardown waits for them briefly, then aborts
    /// the rest: a straggler in the SDK's retry holds a client over the store.
    commands: std::sync::Mutex<std::collections::HashMap<tokio::task::Id, (tokio::task::AbortHandle, u64)>>,
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

    /// Commands other than the caller that are still running.
    fn others_running(&self) -> Vec<(tokio::task::AbortHandle, u64)> {
        let own = tokio::task::try_id();
        let mut commands = self.commands.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        commands.retain(|_, (handle, _)| !handle.is_finished());
        commands
            .iter()
            .filter(|(id, _)| Some(**id) != own)
            .map(|(_, entry)| entry.clone())
            .collect()
    }

    /// Waits for running commands, then aborts what is left. Bounded: a command
    /// in the SDK's retry budget must not hold a sign-out for a quarter of an hour.
    async fn drain_commands(&self, limit: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + limit;
        while !self.others_running().is_empty() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        for (straggler, id) in self.others_running() {
            straggler.abort();
            // Aborted before its first poll it has no guard; whoever takes the debt answers.
            self.sink.stand_in(id, reply_error(id, "signed out"));
        }
        let settle = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while !self.others_running().is_empty() && tokio::time::Instant::now() < settle {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
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

fn new_state(paths: Paths, store_key: Option<session::StoreKey>, sink: Arc<Sink>) -> Arc<State> {
    Arc::new(State {
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
        push_unlock_asked: std::sync::atomic::AtomicBool::new(false),
        push_unlocked: tokio::sync::watch::channel(false).0,
        push_dirty: std::sync::atomic::AtomicBool::new(false),
        push_waiting: std::sync::atomic::AtomicBool::new(false),
        push_tasks: std::sync::Mutex::new(Vec::new()),
        spaces: Mutex::new(None),
        open_space: Mutex::new(None),
        timeline: Mutex::new(None),
        thread: Mutex::new(None),
        opening: Mutex::new(()),
        opening_thread: Mutex::new(()),
        open_generation: std::sync::atomic::AtomicU64::new(0),
        thread_generation: std::sync::atomic::AtomicU64::new(0),
        pending_open: std::sync::Mutex::new(None),
        pending_thread: std::sync::Mutex::new(None),
        opening_direct: Mutex::new(()),
        session_gate: Mutex::new(()),
        subscriptions: Mutex::new(Subscriptions::default()),
        call_subscribed: std::sync::Mutex::new(None),
        directory: Mutex::new(None),
        observers: Mutex::new(Vec::new()),
        verification: Arc::new(Mutex::new(None)),
        commands: std::sync::Mutex::new(std::collections::HashMap::new()),
        sink,
    })
}

/// Starts the dispatcher on `runtime` and returns the channel commands go into.
pub fn spawn(
    runtime: &tokio::runtime::Runtime,
    paths: Paths,
    store_key: Option<session::StoreKey>,
    sink: Arc<Sink>,
) -> mpsc::UnboundedSender<Command> {
    let (sender, mut receiver) = mpsc::unbounded_channel::<Command>();
    let state = new_state(paths, store_key, sink);

    runtime.spawn(async move {
        while let Some(command) = receiver.recv().await {
            let state = state.clone();
            // Boxed: the combined future of all command arms is large, and
            // moving it to the heap keeps the task allocation small.
            let registry = state.clone();
            // The session's own commands are what a teardown runs for: never aborted.
            let abortable = !command.survives_teardown();
            let command_id = command.id();
            state.sink.expect_reply(command_id);
            let task = tokio::spawn(Box::pin(async move {
                let id = command.id();
                // Answers an aborted command and unregisters it, on every way out.
                let mut guard = CommandGuard { state: state.clone(), id, completed: false };
                // A panic here would take the reply with it, and the protocol
                // promises exactly one answer per id.
                let outcome = futures_util::FutureExt::catch_unwind(
                    std::panic::AssertUnwindSafe(handle(state, command)),
                )
                .await;
                if outcome.is_err() {
                    guard
                        .state
                        .sink
                        .answer_if_owed(id, reply_error(id, "the command failed unexpectedly"));
                }
                guard.completed = true;
            }));
            if abortable {
                let mut commands =
                    registry.commands.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if !task.is_finished() {
                    commands.insert(task.id(), (task.abort_handle(), command_id));
                }
            }
        }
    });

    sender
}

/// One answer per id, also for a command a teardown aborted - and only one:
/// a command that answered before it was cut off is not answered again.
struct CommandGuard {
    state: Arc<State>,
    id: u64,
    completed: bool,
}

impl Drop for CommandGuard {
    fn drop(&mut self) {
        // Owed after completion too where a worker answers later; only an abort answers here.
        if !self.completed {
            self.state.sink.stand_in(self.id, reply_error(self.id, "signed out"));
        }
        if let Some(own) = tokio::task::try_id() {
            let mut commands =
                self.state.commands.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            commands.remove(&own);
        }
    }
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
        Command::CrossSigningSetup { password, .. } => {
            cross_signing_setup(&state, id, password).await
        }
        Command::RoomEnableEncryption { room_id, .. } => {
            enable_room_encryption(&state, id, room_id).await
        }
        Command::Logout { .. } => logout(&state, id).await,
        Command::RoomListStart { .. } => start_room_list(&state, id).await,
        Command::RoomListFilter { pattern, .. } => filter_room_list(&state, id, pattern).await,
        Command::RoomListMore { .. } => load_more_rooms(&state, id).await,
        Command::RoomListOrder {
            by_name,
            unread_first,
            ..
        } => {
            roomlist::set_order(roomlist::Order { by_name, unread_first });
            state.sink.emit(reply_ok(id, json!({ "ordered": true })));
        }
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
        Command::RoomResolve { address, local, .. } => {
            resolve_room(&state, id, address, local).await
        }
        Command::TimelineClose { room_id, token, .. } => {
            close_timeline(&state, id, room_id, token).await
        }
        Command::TimelinePaginate { room_id, forwards, .. } => {
            paginate_timeline(&state, id, room_id, forwards).await
        }
        Command::TimelineResync { room_id, token, thread, .. } => {
            resync_timeline(&state, id, room_id, token, thread).await
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
        Command::TimelineEdit { event_id, body, mentions, .. } => {
            edit_message(&state, id, event_id, body, mentions).await
        }
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
        // core/src/leftrooms.rs.
        Command::LeftRoomsList { .. } | Command::LeftRoomsForget { .. } => {
            leftrooms::handle(command, state.client().await, &state.sink).await
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
            mentions,
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
                &state, id, path, mime_type, caption, mentions, reply_to, voice, duration, width,
                height, thumbnail, room_id,
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
        Command::CallReject {
            room_id,
            call_id,
            party_id,
            ..
        } => {
            let ended = call_id.clone();
            call_step(&state, id, move |client| async move {
                call::reject(&client, &room_id, &call_id, &party_id).await
            })
            .await;
            release_call_room(&state, &ended).await;
        }
        Command::CallSelectAnswer {
            room_id,
            call_id,
            party_id,
            selected_party_id,
            ..
        } => {
            call_step(&state, id, move |client| async move {
                call::select_answer(&client, &room_id, &call_id, &party_id, &selected_party_id).await
            })
            .await
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
        Command::PushStatus {
            quiet,
            legacy_wanted,
            ..
        } => push_status(&state, id, quiet, legacy_wanted).await,
        Command::PushDistributor {
            distributor,
            enable,
            ..
        } => push_select(&state, id, distributor, enable).await,
        Command::PushUnlock { store_key, .. } => push_unlock(&state, id, store_key).await,
        Command::PushEnable { .. } => push_enable(&state, id).await,
        Command::PushDisable { .. } => push_disable(&state, id).await,
        Command::PushGateway {
            mode,
            gateway,
            enable,
            ..
        } => push_set_gateway(&state, id, mode, gateway, enable).await,
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

#[cfg(test)]
mod reply_tests {
    use super::*;
    use std::ffi::CStr;
    use std::time::Duration;

    type Heard = std::sync::Mutex<Vec<Value>>;

    extern "C" fn record(user_data: *mut c_void, message: *const c_char) {
        let heard = unsafe { &*(user_data as *const Heard) };
        let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        heard.lock().unwrap().push(serde_json::from_str(&text).unwrap());
    }

    /// A sink that records; the box lives as long as the test.
    fn recording_sink() -> (Arc<Sink>, Box<Heard>) {
        let heard = Box::new(Heard::default());
        let sink = Arc::new(Sink::new());
        sink.set_callback(Some(record), &*heard as *const Heard as *mut c_void);
        (sink, heard)
    }

    fn replies(heard: &Heard, id: u64) -> Vec<Value> {
        heard
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message["type"] == "reply" && message["id"] == id)
            .cloned()
            .collect()
    }

    fn scratch(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("xmatic-reply-{name}-{}", std::process::id()));
        Paths::new(&dir, &dir)
    }

    #[test]
    fn a_reply_settles_the_debt_and_an_event_does_not() {
        let sink = Sink::new();
        sink.expect_reply(1);
        sink.emit(event("roomlist.known", json!({})));
        assert!(sink.owed(1));
        assert!(!sink.owed(1), "owed forgets");
        sink.expect_reply(2);
        sink.emit(reply_ok(2, json!({})));
        assert!(!sink.owed(2));
    }

    #[tokio::test]
    async fn an_aborted_command_is_answered_once() {
        let (sink, heard) = recording_sink();
        let state = new_state(scratch("abort"), None, sink.clone());
        sink.expect_reply(7);
        let (started, running) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = CommandGuard { state, id: 7, completed: false };
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        running.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let answers = replies(&heard, 7);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["ok"], false);
        assert!(!sink.owed(7));
    }

    #[tokio::test]
    async fn an_answered_command_is_not_answered_again() {
        let (sink, heard) = recording_sink();
        let state = new_state(scratch("answered"), None, sink.clone());
        // Completed normally.
        sink.expect_reply(8);
        {
            let mut guard = CommandGuard { state: state.clone(), id: 8, completed: false };
            sink.emit(reply_ok(8, json!({})));
            guard.completed = true;
        }
        // Answered, then cut off before completing.
        sink.expect_reply(9);
        let (started, running) = tokio::sync::oneshot::channel();
        let inner = sink.clone();
        let task = tokio::spawn(async move {
            let _guard = CommandGuard { state, id: 9, completed: false };
            inner.emit(reply_ok(9, json!({})));
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        running.await.unwrap();
        task.abort();
        let _ = task.await;
        assert_eq!(replies(&heard, 8).len(), 1);
        let answers = replies(&heard, 9);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["ok"], true);
    }

    #[test]
    fn a_late_reply_after_a_stand_in_is_dropped() {
        let (sink, heard) = recording_sink();
        sink.expect_reply(21);
        assert!(sink.stand_in(21, reply_error(21, "signed out")));
        // The straggler answers after all: dropped, and the id let go.
        sink.emit(reply_ok(21, json!({})));
        let answers = replies(&heard, 21);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["ok"], false);
        // A later reuse of the id is delivered again.
        sink.emit(reply_ok(21, json!({})));
        assert_eq!(replies(&heard, 21).len(), 2);
    }

    #[test]
    fn a_stand_in_for_an_answered_command_says_nothing() {
        let (sink, heard) = recording_sink();
        sink.expect_reply(22);
        sink.emit(reply_ok(22, json!({})));
        assert!(!sink.stand_in(22, reply_error(22, "signed out")));
        assert_eq!(replies(&heard, 22).len(), 1);
        assert_eq!(replies(&heard, 22)[0]["ok"], true);
    }

    #[test]
    fn a_panic_after_the_reply_adds_no_second() {
        let (sink, heard) = recording_sink();
        sink.expect_reply(25);
        sink.emit(reply_ok(25, json!({})));
        sink.answer_if_owed(25, reply_error(25, "the command failed unexpectedly"));
        assert_eq!(replies(&heard, 25).len(), 1);
        sink.expect_reply(26);
        sink.answer_if_owed(26, reply_error(26, "the command failed unexpectedly"));
        assert_eq!(replies(&heard, 26).len(), 1);
    }

    #[test]
    fn a_worker_reply_never_owed_goes_through() {
        let (sink, heard) = recording_sink();
        sink.emit(reply_ok(23, json!({})));
        sink.emit(reply_ok(23, json!({})));
        assert_eq!(replies(&heard, 23).len(), 2);
    }

    #[tokio::test]
    async fn an_aborted_commands_late_worker_reply_is_dropped() {
        let (sink, heard) = recording_sink();
        let state = new_state(scratch("late"), None, sink.clone());
        sink.expect_reply(24);
        let (started, running) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = CommandGuard { state, id: 24, completed: false };
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        running.await.unwrap();
        task.abort();
        let _ = task.await;
        sink.emit(reply_ok(24, json!({})));
        let answers = replies(&heard, 24);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["ok"], false);
    }

    #[test]
    fn the_dispatcher_answers_every_command_once() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (sink, heard) = recording_sink();
        let commands = spawn(&runtime, scratch("dispatch"), None, sink.clone());
        for text in [
            r#"{"cmd":"roomlist.filter","id":11,"pattern":""}"#,
            r#"{"cmd":"roomlist.order","id":12,"byName":true}"#,
            r#"{"cmd":"roomlist.more","id":13}"#,
        ] {
            commands.send(serde_json::from_str(text).unwrap()).unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while [11, 12, 13].iter().any(|id| replies(&heard, *id).is_empty())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(replies(&heard, 11).len(), 1);
        assert_eq!(replies(&heard, 11)[0]["ok"], false);
        assert_eq!(replies(&heard, 12).len(), 1);
        assert_eq!(replies(&heard, 12)[0]["ok"], true);
        assert_eq!(replies(&heard, 13).len(), 1);
        for id in [11, 12, 13] {
            assert!(!sink.owed(id));
        }
        sink.set_callback(None, std::ptr::null_mut());
        drop(commands);
        runtime.shutdown_timeout(Duration::from_secs(1));
    }
}
