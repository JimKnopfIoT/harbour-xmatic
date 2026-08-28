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
use crate::login;
use crate::profile;
use crate::private;
use crate::protocol::{event, reply_error, reply_ok, Command, Secret};
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
    /// The store key from Sailfish Secrets; `None` degrades to unencrypted.
    /// Behind a lock because the front end may hand it in later: after a
    /// locked secrets collection the user retries, and `session.restore`
    /// then carries the key the start did not have.
    store_key: std::sync::RwLock<Option<session::StoreKey>>,
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
    /// The open thread's timeline, next to the room's — same locking rules.
    thread: Mutex<Option<Arc<TimelineHandle>>>,
    /// Serialises `open_timeline` against itself. Held for the whole open,
    /// including the network part, which is why it cannot be `timeline`.
    opening: Mutex<()>,
    /// The same for `open_thread`, and deliberately not `opening`: a thread
    /// that is slow to build must not hold up a room switch.
    opening_thread: Mutex<()>,
    /// Every room that currently needs a sliding-sync subscription. See
    /// `Subscriptions` — they have to be requested together or not at all.
    subscriptions: Mutex<Subscriptions>,
    directory: Mutex<Option<DirectoryHandle>>,
    /// The observers that outlive a command: the recovery/backup state watcher
    /// and the session watcher. Both hold a client clone, and a task holding
    /// one keeps the SQLite pool open - `reset_store` on sign-out would then
    /// delete the directory under a live connection, which is the one mistake
    /// this project has already paid for. Kept here so they can be stopped
    /// before the client goes.
    observers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    verification: verification::Slot,
    /// How many command tasks are in flight.
    ///
    /// Each of them runs on its own and most hold a clone of the client, so a
    /// sign-out can delete the store directory while one is still inside a
    /// request - the same fault the timeline handles were taught to avoid, on
    /// the one path that has no handle to close. They cannot be aborted
    /// (a command that is half-way through a send has to finish), so they are
    /// counted, and the sign-out waits briefly for the count to fall.
    running: std::sync::atomic::AtomicUsize,
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

    async fn thread(&self) -> Option<Arc<TimelineHandle>> {
        self.thread.lock().await.clone()
    }

    /// Waits, briefly, for the running commands to finish.
    ///
    /// Bounded on purpose: a command stuck in the SDK's retry budget must not
    /// hold a sign-out for a quarter of an hour, and what one of them can still
    /// do after this wait is a single request against a server that is about to
    /// forget the session anyway. The sign-out itself is one of the commands,
    /// hence the comparison against one rather than zero.
    async fn drain_commands(&self, limit: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + limit;
        while self.running.load(std::sync::atomic::Ordering::SeqCst) > 1
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
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
        pending_device: Mutex::new(None),
        rooms: Mutex::new(None),
        spaces: Mutex::new(None),
        open_space: Mutex::new(None),
        timeline: Mutex::new(None),
        thread: Mutex::new(None),
        opening: Mutex::new(()),
        opening_thread: Mutex::new(()),
        subscriptions: Mutex::new(Subscriptions::default()),
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
                handle(state, command).await;
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
            ..
        } => open_timeline(&state, id, room_id, focus, receipts).await,
        Command::RoomMarkRead { room_id, receipt, .. } => {
            mark_room_read(&state, id, room_id, receipt).await
        }
        Command::RoomResolve { address, .. } => resolve_room(&state, id, address).await,
        Command::TimelineClose { .. } => close_timeline(&state, id).await,
        Command::TimelinePaginate { .. } => paginate_timeline(&state, id).await,
        Command::TimelineSend { body, .. } => send_message(&state, id, body).await,
        Command::TimelineMarkRead { receipt, .. } => mark_read(&state, id, receipt).await,
        Command::PrivateGet { .. } => {
            match private::load(&state.paths.private_file, state.store_key().as_ref()) {
                private::Loaded::Lists(lists) => {
                    state.sink.emit(reply_ok(id, json!({ "lists": lists, "readable": true })))
                }
                // Empty *and* writable is only true where a key exists. Without
                // one every write is refused, and answering "readable" sent the
                // app into the legacy migration, whose failure asks again -
                // measured as an endless command loop on a device whose secrets
                // collection was still locked after a reboot.
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
            // The front end sends the whole state, so nothing has to be read
            // first and no two writes can overwrite each other. The file is
            // only consulted to refuse a write over an unreadable one.
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
        Command::TimelineReply { event_id, body, .. } => {
            reply_message(&state, id, event_id, body).await
        }
        Command::TimelineEdit { event_id, body, .. } => edit_message(&state, id, event_id, body).await,
        Command::TimelineRetry { txn_id, .. } => retry_message(&state, id, txn_id).await,
        Command::TimelineReact { event_id, key, .. } => react(&state, id, event_id, key).await,
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
            ..
        } => {
            send_media(&state, id, path, mime_type, caption, reply_to, voice, duration).await
        }
        Command::MediaFetch {
            source,
            thumbnail,
            size,
            ..
        } => fetch_media(&state, id, source, thumbnail, size).await,
        Command::RoomForward {
            room_id,
            body,
            path,
            mime_type,
            ..
        } => forward(&state, id, room_id, body, path, mime_type).await,
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
            // A call's signalling rides the room's timeline, and an
            // unsubscribed room hands out one event per sync response — a
            // burst of ICE candidates then arrives as its last one alone,
            // which is a call that connects and stays silent.
            set_call_room(&state, Some(&room_id)).await;
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
        Command::VerificationMismatch { .. } => {
            verification_step(&state, id, Step::Mismatch).await
        }
        Command::EncryptionStatus { .. } => encryption_status(&state, id).await,
        Command::StorageStatus { .. } => storage_status(&state, id),
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
        Command::MembersLoad { room_id, .. } => members_load(&state, id, room_id).await,
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
        Command::ThreadSend { body, .. } => send_thread_message(&state, id, body).await,
        Command::ThreadPaginate { .. } => paginate_thread(&state, id).await,
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

async fn room_set_notify_mode(state: &Arc<State>, id: u64, room_id: String, mode: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::set_notification_mode(&client, &room_id, &mode).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "mode": mode, "muted": mode == "mute" }),
        )),
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

async fn member_profile(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::profile(&client, &room_id, &user_id).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn member_ban(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::ban(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "userId": user_id }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn member_unban(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::unban(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "userId": user_id }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn member_set_power(state: &Arc<State>, id: u64, room_id: String, user_id: String, power: i64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::set_power(&client, &room_id, &user_id, power).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "userId": user_id, "power": power }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn member_set_ignored(state: &Arc<State>, id: u64, user_id: String, ignored: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::set_ignored(&client, &user_id, ignored).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "userId": user_id, "ignored": ignored }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn room_reset_keys(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match timeline::discard_room_key(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "reset": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn account_ignored_users(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::ignored(&client).await {
        Ok(users) => state.sink.emit(reply_ok(id, json!({ "users": users }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn member_withdraw_verification(state: &Arc<State>, id: u64, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::withdraw_verification(&client, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "userId": user_id }))),
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

/// Answers what the local storage on this device amounts to.
///
/// Synchronous and client-free on purpose: the answer is about files, and the
/// UI needs it while signed out too — an install that runs unencrypted should
/// say so before the first login, not only after it.
fn storage_status(state: &Arc<State>, id: u64) {
    let key = state.store_key();
    let storage = session::storage_state(&state.paths, key.as_ref());
    state.sink.emit(reply_ok(
        id,
        json!({
            "encrypted": storage.fully_encrypted(),
            "storeEncrypted": storage.store_encrypted,
            "sessionPresent": storage.session_present,
            "sessionEncrypted": storage.session_encrypted,
            "keyAvailable": storage.key_available,
            // True where the stores could be encrypted but are not, and a key
            // exists to do it with — the only case in which offering the
            // re-encryption is honest. `store_key_applies` can never upgrade an
            // existing store in place, so this is a sign-out away, not a switch.
            "canEncrypt": storage.key_available && !storage.store_encrypted,
        }),
    ));
}

async fn encryption_recover(state: &Arc<State>, id: u64, key: Secret) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match recovery::recover(&client, key.as_str()).await {
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
    Mismatch,
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
        Step::Mismatch => active.mismatch().await,
    };

    match outcome {
        Ok(()) => state
            .sink
            .emit(reply_ok(id, json!({ "flowId": active.flow_id() }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn open_timeline(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    focus: String,
    receipts: bool,
) {
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
    // Where this device's own reading stopped, so the view can open there
    // instead of at the newest message. Read before the timeline lock is taken:
    // it is a store read, and this project already froze every room switch once
    // by awaiting inside that lock. It also has to be the state *before* this
    // visit marks anything read.
    let marker = timeline::own_read_marker(&client, &room_id).await;
    // What this account may do in this room, for the menus that would
    // otherwise offer an action the server refuses. Read here for the same
    // reason as the marker: it is a store read, and it must not happen under
    // the timeline lock.
    let permissions = match matrix_sdk::ruma::RoomId::parse(&room_id)
        .ok()
        .and_then(|parsed| client.get_room(&parsed))
    {
        Some(room) => members::room_permissions(&client, &room).await,
        None => json!({}),
    };

    {
        let mut open = state.timeline.lock().await;
        if let Some(previous) = open.take() {
            // The receipt setting is chosen when the timeline is built, so a
            // changed one has to rebuild even for the room already open.
            if previous.room_id() == room_id
                && focus.is_empty()
                && previous.is_live()
                && previous.tracks_receipts() == receipts
            {
                open.replace(previous);
                state.sink.emit(reply_ok(
                    id,
                    json!({
                        "open": true,
                        "readMarker": marker,
                        "rebuilt": false,
                        "can": permissions,
                    }),
                ));
                return;
            }
            previous.close().await;
        }
    }

    // Past the early return, so re-entering the same room keeps its thread:
    // a thread belongs to the view it was opened from, and a stream left
    // running would keep emitting `thread.diff` for the room just left.
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
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

    match timeline::open(&client, &room_id, &focus, receipts, state.sink.clone()).await {
        Ok(handle) => {
            *state.timeline.lock().await = Some(Arc::new(handle));
            // Rebuilt, so the view starts from nothing and may open where
            // reading stopped; the branch above kept the rows it had.
            state.sink.emit(reply_ok(
                id,
                json!({
                    "open": true,
                    "readMarker": marker,
                    "rebuilt": true,
                    "can": permissions,
                }),
            ));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// A tapped Matrix link: which room is meant, and are we in it.
async fn resolve_room(state: &Arc<State>, id: u64, address: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::resolve(&client, &address).await {
        Ok(mut data) => {
            if let Some(object) = data.as_object_mut() {
                object.insert("address".to_owned(), json!(address));
            }
            state.sink.emit(reply_ok(id, data));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// The chat list's "mark as read": no timeline is opened for it, so it cannot
/// go through the open handle the way the room's own does.
async fn mark_room_read(state: &Arc<State>, id: u64, room_id: String, receipt: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::mark_read(&client, &room_id, receipt).await {
        Ok(marked) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id, "marked": marked }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn close_timeline(state: &Arc<State>, id: u64) {
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close().await;
    }
    // A thread never outlives its room's view.
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }
    state.sink.emit(reply_ok(id, json!({ "open": false })));
}

async fn open_thread(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    root_event_id: String,
    token: String,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let _opening = state.opening_thread.lock().await;

    if let Some(previous) = state.thread.lock().await.take() {
        previous.close().await;
    }

    match timeline::open_thread(&client, &room_id, &root_event_id, &token, state.sink.clone()).await
    {
        Ok(handle) => {
            *state.thread.lock().await = Some(Arc::new(handle));
            state.sink.emit(reply_ok(id, json!({ "open": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// Closes the open thread. A close that names a different thread than the one
/// currently open is ignored: commands run as independent tasks, so the close
/// of the thread just left can reach the core after the open of the next one.
async fn close_thread(state: &Arc<State>, id: u64, root_event_id: String) {
    let mut open = state.thread.lock().await;
    let matches = open
        .as_ref()
        .map(|handle| root_event_id.is_empty() || handle.thread_root() == root_event_id)
        .unwrap_or(false);
    if matches {
        if let Some(handle) = open.take() {
            handle.close().await;
        }
    }
    drop(open);
    state.sink.emit(reply_ok(id, json!({ "open": false })));
}

async fn send_thread_message(state: &Arc<State>, id: u64, body: String) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "nothing to send"));
        return;
    }

    let outcome = match state.thread().await {
        Some(handle) => handle.send_text(body).await,
        None => Err("no thread is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn paginate_thread(state: &Arc<State>, id: u64) {
    let outcome = match state.thread().await {
        Some(handle) => handle.paginate().await,
        None => Err("no thread is open".to_owned()),
    };

    match outcome {
        Ok(reached_start) => state
            .sink
            .emit(reply_ok(id, json!({ "reachedStart": reached_start }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
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

async fn react(state: &Arc<State>, id: u64, event_id: String, key: String) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.toggle_reaction(&event_id, &key).await,
        None => Err("no timeline is open".to_owned()),
    };
    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "reacted": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn retry_message(state: &Arc<State>, id: u64, txn_id: String) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.retry(&txn_id).await,
        None => Err("no timeline is open".to_owned()),
    };
    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "queued": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn redact_message(state: &Arc<State>, id: u64, event_id: String, txn_id: String) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.redact(&event_id, &txn_id).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "deleted": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_media(
    state: &Arc<State>,
    id: u64,
    path: String,
    mime_type: String,
    caption: String,
    reply_to: String,
    voice: bool,
    duration: u64,
) {
    // Cloned out of the guard: the attachment upload takes a while and must
    // not hold the lock.
    let timeline = state.timeline().await.map(|handle| handle.timeline());

    let Some(timeline) = timeline else {
        state.sink.emit(reply_error(id, "no timeline is open"));
        return;
    };

    let voice = if voice { Some(duration) } else { None };
    match media::send(&timeline, &path, &mime_type, &caption, &reply_to, voice).await {
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

async fn fetch_media(state: &Arc<State>, id: u64, source: Value, thumbnail: bool, size: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match media::fetch(&client, &state.paths.media_cache, source, thumbnail, size).await {
        Ok(path) => state.sink.emit(reply_ok(id, json!({ "path": path }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

async fn mark_read(state: &Arc<State>, id: u64, receipt: bool) {
    // The room goes back with the answer: a receipt does not reliably come
    // back as a room-list diff, so the app clears the badge itself - and it
    // may only do that for the room this actually marked.
    let handle = state.timeline().await;
    let room_id = handle
        .as_ref()
        .map(|handle| handle.room_id().to_owned())
        .unwrap_or_default();
    let outcome = match handle {
        Some(handle) => handle.mark_read(receipt).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(read) => state
            .sink
            .emit(reply_ok(id, json!({ "read": read, "roomId": room_id }))),
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

/// Answers with everything the room-info page shows.
async fn room_info(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::room_info(&client, &room_id).await {
        Ok(info) => state.sink.emit(reply_ok(id, info)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// Joins the room a tombstoned one points at, and answers with its id so the
/// front end can open it.
async fn follow_successor(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::follow_successor(&client, &room_id).await {
        Ok(new_room_id) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": new_room_id }))),
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

/// One more page of rooms for a list that has reached its end.
///
/// The dynamic adapter behind the room list holds one page and grows only when
/// told to (`add_one_page`), so an account with more rooms than a page simply
/// had no others - and nothing said so. Asking again once everything is loaded
/// does nothing, which is why this needs no "is there more" of its own.
async fn load_more_rooms(state: &Arc<State>, id: u64) {
    let asked = match &*state.rooms.lock().await {
        Some(handle) => handle.load_more(),
        None => false,
    };

    if asked {
        state.sink.emit(reply_ok(id, json!({ "asked": true })));
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

async fn create_room(state: &Arc<State>, id: u64, room: roomlist::NewRoom) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // Kept for the reply: the request is consumed by the call below, and the
    // front end opens the room under this name before the first diff arrives.
    let name = room.name.trim().to_owned();
    let encrypted = room.encrypted;

    // The room reaches the list through the running sync. The reply carries the
    // name and the encryption state back so the front end can open the room
    // under its title, and with the right state, before the first diff.
    match roomlist::create_room(&client, room).await {
        Ok(room_id) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "name": name, "encrypted": encrypted }),
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

async fn restore_session(state: &Arc<State>, id: u64, store_key: Option<String>) {
    // A key handed in with the command replaces the one from start. Only a
    // well-formed key does — a garbled one must not silently turn into "no
    // key" and open an unencrypted path.
    if let Some(mut encoded) = store_key {
        use zeroize::Zeroize;
        let decoded = session::decode_key(&encoded);
        encoded.zeroize();
        if decoded.is_some() {
            *state
                .store_key
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = decoded;
        }
    }
    let key = state.store_key();

    let stored = match session::load(&state.paths.session_file, key.as_ref()) {
        session::LoadOutcome::Session(stored) => stored,
        session::LoadOutcome::None => {
            state.sink.emit(reply_ok(id, json!({ "state": "none" })));
            return;
        }
        // The session is there, the key is not: a state of its own, never
        // "no session". The UI shows a retry, not the login page, and no
        // login may reset the store while the file is on disk.
        session::LoadOutcome::Locked => {
            let data = json!({ "state": "locked" });
            state.sink.emit(reply_ok(id, data.clone()));
            state.sink.emit(event("session.changed", data));
            return;
        }
    };

    let homeserver = stored.homeserver().to_owned();

    // An encrypted store without its key is the same locked state — opening
    // it anyway would surface as decryption garbage three layers further down.
    if session::store_marked_encrypted(&state.paths) && key.is_none() {
        let data = json!({ "state": "locked" });
        state.sink.emit(reply_ok(id, data.clone()));
        state.sink.emit(event("session.changed", data));
        return;
    }

    let client = match session::build_client(&homeserver, &state.paths, key.as_ref()).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("could not open the session: {error}")));
            return;
        }
    };

    // Before the token is handed to the client, not after: the stored server
    // string goes through discovery again on every start, and a `.well-known`
    // that changed to http would put the access token in the clear from then
    // on. The rule that guards the first sign-in has to guard every one after.
    if let Err(message) = require_https(&client) {
        state.sink.emit(reply_error(id, message));
        return;
    }

    if let Err(error) = client
        .restore_session_with(stored.into_auth_session(), RoomLoadSettings::default())
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
    // this stream says so. Kept, not dropped: it holds a client clone and
    // would otherwise still be holding one after a sign-out.
    let recovery_task = recovery::watch(&client, state.sink.clone());
    // Rewritten once per restore so a plaintext session.json from before the
    // store key existed becomes encrypted now — token refreshes would do it
    // eventually for OAuth, but a classic session without refresh tokens
    // would otherwise stay plaintext forever.
    persist(state, &client, homeserver.clone()).await;
    let session_task = watch_session(state, &client, homeserver);
    state
        .observers
        .lock()
        .await
        .extend([recovery_task, session_task]);

    *state.client.lock().await = Some(client);
    let data = state.session_data().await;
    state.sink.emit(reply_ok(id, data.clone()));
    state.sink.emit(event("session.changed", data));
}

/// Clears the ground for a login that starts a new device: without a stored
/// session, whatever the previous device left in the store has to go first —
/// otherwise the crypto store still describes a device this session is not.
///
/// A client an earlier `login.start` cached has its SQLite stores open on that
/// very directory, so it is taken out of the state and dropped *before* the
/// reset. Resetting under it does not fail the login: the open files keep
/// serving their existing connections, but every connection the store pool
/// opens afterwards finds an empty database, and the sync service that starts
/// after the sign-in fails and retries with nothing reaching the room list
/// until a restart. That was the 0.18.0 password-login report; the rule since
/// is that `reset_store` runs only when no client has the directory open.
async fn prepare_fresh_login(state: &Arc<State>) -> Result<(), String> {
    match session::load(&state.paths.session_file, state.store_key().as_ref()) {
        session::LoadOutcome::None => {
            drop(state.client.lock().await.take());
            session::reset_store(&state.paths)
                .map_err(|error| format!("could not clear old data: {error}"))?;
        }
        session::LoadOutcome::Session(_) => {}
        // A locked session is a session. Logging in over it would reset the
        // store its key still protects; the way out of a lost key is an
        // explicit sign-out, not a login.
        session::LoadOutcome::Locked => {
            return Err(
                "a session is stored but its key is not available; unlock or sign out first"
                    .to_owned(),
            );
        }
    }
    state
        .paths
        .prepare()
        .map_err(|error| format!("could not prepare storage: {error}"))
}

/// Refuses a homeserver that is not reached over https.
///
/// Asked of the client, not of what was typed: a homeserver may be found
/// through `.well-known`, and that document is free to name an http URL.
/// Without this the SDK notices the plain scheme itself and switches OAuth
/// into its insecure mode - after which the client registration, the token
/// exchange and every request carrying the token go over the wire in the
/// clear. An access token opens the same account a password does.
///
/// One function rather than the same three lines at each entrance: the rule
/// was written four times and still missed the two paths that matter most -
/// restoring a stored session, which runs at every single start, and the
/// registration page, which sends the user off to create an account.
fn require_https(client: &Client) -> Result<(), String> {
    if client.homeserver().scheme() == "https" {
        return Ok(());
    }
    Err("this homeserver is not reached over https".to_owned())
}

async fn start_login(state: &Arc<State>, id: u64, homeserver: String) {
    if let Err(message) = prepare_fresh_login(state).await {
        state.sink.emit(reply_error(id, message));
        return;
    }

    let client = match session::build_client(&homeserver, &state.paths, state.store_key().as_ref()).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("homeserver unreachable: {}", crate::text::scrub_ids(&error.to_string()))));
            return;
        }
    };

    if let Err(message) = require_https(&client) {
        state.sink.emit(reply_error(id, message));
        return;
    }

    // Which sign-in does this server speak? OAuth discovery decides. The
    // password form is only ever offered on the affirmative `NotSupported`
    // answer — a transport error, a timeout or a broken authentication
    // service is an error, never a downgrade to asking for a password.
    match client.oauth().server_metadata().await {
        Ok(_) => {}
        Err(OAuthDiscoveryError::NotSupported) => {
            let flows = login::login_flows(&client).await;
            match flows.as_ref().map(|list| list.iter().any(|flow| flow == "password")) {
                Ok(true) => {
                    // No scheme check here: this function refused anything but
                    // https before it got this far.
                    // Kept so `login.password` reuses this client — and with
                    // it this discovery result — instead of trusting the UI.
                    *state.client.lock().await = Some(client);
                    state
                        .sink
                        .emit(reply_ok(id, json!({ "passwordLogin": true })));
                }
                // Naming the method the server does want is the whole point:
                // SSO is a redirect to the server's own web page, and a user
                // told only "sign-in failed" will retype a password that was
                // never wrong.
                Ok(false) => {
                    let sso = flows
                        .as_ref()
                        .map(|list| list.iter().any(|flow| flow == "sso"))
                        .unwrap_or(false);
                    state.sink.emit(reply_error(
                        id,
                        if sso {
                            "this server signs in through its own web page (SSO), which this app cannot do yet"
                        } else {
                            "this server offers no sign-in method this app supports"
                        },
                    ));
                }
                Err(_) => state.sink.emit(reply_error(
                    id,
                    flows.err().unwrap_or_else(|| "sign-in methods unknown".to_owned()),
                )),
            }
            return;
        }
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("sign-in discovery failed: {}", crate::text::scrub_ids(&error.to_string()))));
            return;
        }
    }

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
                let recovery_task = recovery::watch(&client, waiter.sink.clone());
                persist(&waiter, &client, homeserver.clone()).await;
                let session_task = watch_session(&waiter, &client, homeserver);
                waiter
                    .observers
                    .lock()
                    .await
                    .extend([recovery_task, session_task]);
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
                    .emit(event("login.failed", json!({ "message": crate::text::scrub_ids(&message) })));
            }
        }
    });
}

/// Signs in with `m.login.password`. Unlike the browser and device-code
/// flows there is nothing external to wait for, so the reply is the complete
/// outcome: an error for a wrong password, `session.changed` on success.
///
/// The password arrives as a `Secret` — wiped on drop, unprintable — and is
/// only borrowed onwards. The UI is not trusted with the flow decision: the
/// checks that put the password form on screen are repeated here, so a UI
/// bug cannot send a password to an OAuth server or over plain http.
async fn password_login(
    state: &Arc<State>,
    id: u64,
    homeserver: String,
    user: String,
    password: Secret,
) {
    // Reuse the client `login.start` built and vetted; build one only if the
    // UI skipped that step, and then vet it the same way.
    //
    // The store reset every fresh login begins with belongs to whoever builds
    // the client: `login.start` ran it before building the cached client,
    // whose SQLite stores have been open on that directory ever since.
    // Running it again here — as 0.18.0 did — deleted those files under the
    // live client (see `prepare_fresh_login`).
    let cached = state.client.lock().await.clone();
    let client = match cached {
        Some(client) => client,
        None => {
            if let Err(message) = prepare_fresh_login(state).await {
                state.sink.emit(reply_error(id, message));
                return;
            }
            match session::build_client(&homeserver, &state.paths, state.store_key().as_ref()).await {
                Ok(client) => client,
                Err(error) => {
                    state
                        .sink
                        .emit(reply_error(id, format!("homeserver unreachable: {}", crate::text::scrub_ids(&error.to_string()))));
                    return;
                }
            }
        }
    };

    if let Err(message) = require_https(&client) {
        state.sink.emit(reply_error(id, message));
        return;
    }

    match client.oauth().server_metadata().await {
        Err(OAuthDiscoveryError::NotSupported) => {}
        Ok(_) => {
            state.sink.emit(reply_error(
                id,
                "this server signs in through its own page, not with a password here",
            ));
            return;
        }
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("sign-in discovery failed: {}", crate::text::scrub_ids(&error.to_string()))));
            return;
        }
    }

    if let Err(message) = login::password(&client, &user, password.as_str()).await {
        state.sink.emit(reply_error(id, message));
        return;
    }

    verification::install(&client, state.sink.clone(), state.verification.clone());
    call::install(&client, state.sink.clone());
    let recovery_task = recovery::watch(&client, state.sink.clone());
    persist(state, &client, homeserver.clone()).await;
    let session_task = watch_session(state, &client, homeserver);
    state
        .observers
        .lock()
        .await
        .extend([recovery_task, session_task]);
    *state.client.lock().await = Some(client);

    let data = state.session_data().await;
    state.sink.emit(reply_ok(id, data.clone()));
    state.sink.emit(event("session.changed", data));
}

/// Begins the device-code login: like `start_login`, but instead of a browser
/// URL the reply carries a short verification URL and a code, and completion
/// means the user approved the login on some other device.
async fn start_device_login(state: &Arc<State>, id: u64, homeserver: String) {
    if let Err(message) = prepare_fresh_login(state).await {
        state.sink.emit(reply_error(id, message));
        return;
    }

    let client = match session::build_client(&homeserver, &state.paths, state.store_key().as_ref()).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("homeserver unreachable: {}", crate::text::scrub_ids(&error.to_string()))));
            return;
        }
    };

    if let Err(message) = require_https(&client) {
        state.sink.emit(reply_error(id, message));
        return;
    }

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
                let recovery_task = recovery::watch(&client, waiter.sink.clone());
                persist(&waiter, &client, homeserver.clone()).await;
                let session_task = watch_session(&waiter, &client, homeserver);
                waiter
                    .observers
                    .lock()
                    .await
                    .extend([recovery_task, session_task]);
                let data = waiter.session_data().await;
                waiter.sink.emit(event("session.changed", data));
            }
            Err(message) => {
                waiter
                    .sink
                    .emit(event("login.failed", json!({ "message": crate::text::scrub_ids(&message) })));
            }
        }
    });
    *state.pending_device.lock().await = Some(task);
}

/// Writes the freshly obtained session to disk. A failure here is not fatal for
/// the running session, only for surviving a restart, so it is reported as an
/// event rather than aborting the login.
async fn persist(state: &Arc<State>, client: &Client, homeserver: String) {
    // Whichever auth API owns the session: OAuth for the browser and
    // device-code logins, the Matrix API for the password login. Only the
    // tokens are stored either way — a password never reaches this function.
    let stored = if let Some(oauth_session) = client.oauth().full_session() {
        StoredSession::from_oauth(homeserver, &oauth_session)
    } else if let Some(matrix_session) = client.matrix_auth().session() {
        StoredSession::from_matrix(homeserver, matrix_session)
    } else {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": "session could not be persisted" }),
        ));
        return;
    };
    if let Err(error) = session::store(&stored, &state.paths.session_file, state.store_key().as_ref()) {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": crate::text::scrub_ids(&format!("session could not be saved: {error}")) }),
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
fn watch_session(
    state: &Arc<State>,
    client: &Client,
    homeserver: String,
) -> tokio::task::JoinHandle<()> {
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
                    // rooms that look like a bug - and stop everything that is
                    // still talking to a server which has thrown this session
                    // away, instead of leaving a sync service to retry against
                    // it for as long as the app runs.
                    session_expired(&state).await;
                    state.sink.emit(event(
                        "session.expired",
                        json!({ "message": "the session has expired, please sign in again" }),
                    ));
                    // Nothing more can arrive on this subscription, and the
                    // clone this task holds is the last thing keeping the old
                    // client alive.
                    break;
                }
                // Missing a refresh notification only costs a redundant write
                // next time; keep listening.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    })
}

async fn registration_url(state: &Arc<State>, id: u64, homeserver: String) {
    if let Err(error) = state.paths.prepare() {
        state
            .sink
            .emit(reply_error(id, format!("could not prepare storage: {error}")));
        return;
    }

    let client = match session::build_client(&homeserver, &state.paths, state.store_key().as_ref()).await {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, format!("homeserver unreachable: {}", crate::text::scrub_ids(&error.to_string()))));
            return;
        }
    };

    if let Err(message) = require_https(&client) {
        state.sink.emit(reply_error(id, message));
        return;
    }

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

/// Stops the observers that hold a client clone. Called before the client is
/// dropped, by both the deliberate sign-out and an expired session.
async fn stop_observers(state: &Arc<State>) {
    for task in state.observers.lock().await.drain(..) {
        task.abort();
    }
}

/// What is done when the server says this session no longer exists.
///
/// The same teardown as signing out, minus two things: there is no point
/// telling the server about a token it has already thrown away, and the stores
/// stay where they are - the account is unchanged, a fresh login resets the
/// store itself, and wiping the crypto store here would take the device's keys
/// with it for a session that may only have been revoked by mistake.
///
/// The session file does go: left on disk it would be restored at the next
/// start, fail every sync, and show as "offline" over an empty room list
/// instead of as the login page.
async fn session_expired(state: &Arc<State>) {
    stop_observers(state).await;
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close().await;
    }
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }
    // Same as in `logout`: the verification request holds a client.
    verification::cancel_active(&state.verification).await;
    drop(state.verification.lock().await.take());
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
    state.client.lock().await.take();
    session::forget(&state.paths.session_file);
}

async fn logout(state: &Arc<State>, id: u64) {
    // First, because everything below assumes nothing else is holding the
    // client - the store is deleted at the end of this function.
    stop_observers(state).await;
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close().await;
    }
    // A thread handle holds the timeline, and through it the client and the
    // open SQLite pool — `reset_store` below would delete the directory from
    // under it.
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }
    // And a verification in progress holds one too: the SDK's request type
    // carries a `Client` of its own, so a sign-out during a verification left
    // the store open under the very `reset_store` at the end of this function -
    // the fault that cost this project its users' device identities once
    // already, in a different module.
    verification::cancel_active(&state.verification).await;
    drop(state.verification.lock().await.take());
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
    // The handles above are ours to close; the command tasks are not, so the
    // sign-out gives them a moment to finish before the store goes. Bounded -
    // see `drain_commands`.
    state.drain_commands(std::time::Duration::from_secs(2)).await;
    let client = state.client.lock().await.take();
    if let Some(client) = client {
        // Best effort, and bounded. Even if the server cannot be reached, the
        // local session must go away - but the SDK's retry policy has no
        // attempt limit and a fifteen-minute budget, so an unanswered logout
        // used to hold up everything below it: the session file, the crypto
        // store and, because the app clears the media on the state change that
        // comes last, every downloaded picture. Signing out in a dead spot
        // looked like nothing happening, and the next start restored the
        // session. Ten seconds is a server saying yes; anything longer is the
        // local wipe's business alone.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), client.logout()).await;
        // Explicit: `reset_store` below must not run while a client still has
        // the store directory open.
        drop(client);
    }
    session::forget(&state.paths.session_file);
    // The lists that name people belong to the account that is leaving.
    session::forget(&state.paths.private_file);
    // Same for what is only in memory: remembered display names, the call
    // policy with its allow list, and who rang when.
    timeline::forget_senders();
    call::forget_state();
    if let Err(error) = session::reset_store(&state.paths) {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": crate::text::scrub_ids(&format!("local data could not be cleared: {error}")) }),
        ));
    }

    state.sink.emit(reply_ok(id, json!({ "state": "none" })));
    state
        .sink
        .emit(event("session.changed", json!({ "state": "none" })));
}
