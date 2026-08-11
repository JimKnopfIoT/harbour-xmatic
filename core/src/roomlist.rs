//! The room list, driven by the SDK's sync service.
//!
//! `RoomListService` hands out a stream of `VectorDiff`s over the rooms —
//! exactly the vocabulary `QAbstractListModel` speaks. Each diff is forwarded
//! to the front end verbatim, so the Qt model only has to translate insert into
//! `beginInsertRows` and so on. This is the payoff of putting the protocol in
//! Rust: nothing here reimplements sync, ordering or unread counting.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use matrix_sdk::deserialized_responses::SyncOrStrippedState;
use matrix_sdk::ruma::api::client::room::{create_room, Visibility};
use matrix_sdk::ruma::events::{
    room::encryption::RoomEncryptionEventContent, space::child::SpaceChildEventContent,
    InitialStateEvent, SyncStateEvent,
};
use matrix_sdk::ruma::room::RoomType;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{OwnedRoomId, OwnedServerName, RoomId};
use matrix_sdk::{Client, RoomState};
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    room_list_service::{
        filters::{
            new_filter_all, new_filter_fuzzy_match_room_name, new_filter_identifiers,
            new_filter_non_left, new_filter_not, new_filter_space, BoxedFilterFn,
        },
        RoomList, RoomListItem, RoomListService,
    },
    sync_service::{State as SyncState, SyncService},
};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant};

use crate::protocol::event;
use crate::runtime::Sink;

/// How many rooms the dynamic adapter loads per page. A phone screen shows a
/// handful; this is generous enough that scrolling rarely has to grow it.
const PAGE_SIZE: usize = 50;

/// Everything needed to keep the list running and to shut it down again.
pub struct RoomListHandle {
    sync: Arc<SyncService>,
    filters: mpsc::UnboundedSender<String>,
    task: tokio::task::JoinHandle<()>,
    states: tokio::task::JoinHandle<()>,
    queue: tokio::task::JoinHandle<()>,
}

impl RoomListHandle {
    /// The service behind the list, needed to subscribe to a single room.
    pub fn service(&self) -> std::sync::Arc<matrix_sdk_ui::room_list_service::RoomListService> {
        self.sync.room_list_service()
    }

    /// Applies a search pattern. An empty pattern shows all rooms again.
    pub fn set_filter(&self, pattern: String) -> bool {
        self.filters.send(pattern).is_ok()
    }

    pub async fn stop(self) {
        self.task.abort();
        self.states.abort();
        self.queue.abort();
        self.sync.stop().await;
    }
}

/// Where a room's picture comes from.
///
/// A room's own avatar if it has one. A one-to-one chat usually has none — the
/// picture people expect there is the other person's, which the server sends
/// along as the room's "hero". Without this fallback exactly the rooms that
/// matter most, the direct chats, would be the ones showing a blank circle.
///
/// Only for a single hero: with two or more the room is a group, and picking
/// one member's face to stand for it would be arbitrary.
fn room_avatar(item: &RoomListItem) -> Option<String> {
    if let Some(url) = item.avatar_url() {
        return Some(url.to_string());
    }
    let heroes = item.heroes();
    match heroes.as_slice() {
        [hero] => hero.avatar_url.as_ref().map(|url| url.to_string()),
        _ => None,
    }
}

/// One room as the UI needs it. Deliberately flat and small — this crosses the
/// FFI on every change.
fn summarize(item: &RoomListItem) -> Value {
    json!({
        "id": item.room_id().as_str(),
        "name": item
            .cached_display_name()
            .map(|name| name.to_string())
            .unwrap_or_default(),
        "unread": item.num_unread_messages(),
        "mentions": item.num_unread_mentions(),
        "encrypted": item.encryption_state().is_encrypted(),
        "space": item.is_space(),
        // Tags the user set (m.favourite / m.lowpriority). The list groups on
        // these — favourites to the top, low priority to the bottom — and low
        // priority additionally suppresses notifications.
        "favourite": item.is_favourite(),
        "lowPriority": item.is_low_priority(),
        // Upgraded away: the room still lists and still opens, but nothing new
        // arrives in it. The list says so, because a dead room is otherwise
        // indistinguishable from a quiet one.
        "tombstoned": item.is_tombstoned(),
        "muted": matches!(
            item.cached_user_defined_notification_mode(),
            Some(matrix_sdk::notification_settings::RoomNotificationMode::Mute)
        ),
        "membership": match item.state() {
            RoomState::Joined => "joined",
            RoomState::Invited => "invited",
            RoomState::Left => "left",
            RoomState::Knocked => "knocked",
            RoomState::Banned => "banned",
        },
        "avatar": room_avatar(item),
        "timestamp": item
            .latest_event_timestamp()
            .map(|ts| u64::from(ts.get())),
    })
}

/// The diff stream carries immutable vectors, not slices, so this takes
/// anything iterable over items.
fn summarize_all<'a>(items: impl IntoIterator<Item = &'a RoomListItem>) -> Vec<Value> {
    items.into_iter().map(summarize).collect()
}

/// Translates one diff into the JSON the Qt model applies.
fn encode(diff: &VectorDiff<RoomListItem>) -> Value {
    match diff {
        VectorDiff::Append { values } => {
            json!({ "op": "append", "values": summarize_all(values) })
        }
        VectorDiff::Clear => json!({ "op": "clear" }),
        VectorDiff::PushFront { value } => {
            json!({ "op": "insert", "index": 0, "value": summarize(value) })
        }
        VectorDiff::PushBack { value } => {
            json!({ "op": "append", "values": [summarize(value)] })
        }
        VectorDiff::PopFront => json!({ "op": "remove", "index": 0 }),
        VectorDiff::PopBack => json!({ "op": "popBack" }),
        VectorDiff::Insert { index, value } => {
            json!({ "op": "insert", "index": index, "value": summarize(value) })
        }
        VectorDiff::Set { index, value } => {
            json!({ "op": "set", "index": index, "value": summarize(value) })
        }
        VectorDiff::Remove { index } => json!({ "op": "remove", "index": index }),
        VectorDiff::Truncate { length } => json!({ "op": "truncate", "length": length }),
        VectorDiff::Reset { values } => {
            json!({ "op": "reset", "values": summarize_all(values) })
        }
    }
}

/// Starts the sync service and the room list stream.
///
/// The stream borrows the room list, so both live inside the spawned task;
/// filter changes are passed in over a channel rather than by handing the
/// controller out.
pub async fn start(client: &Client, sink: Arc<Sink>) -> Result<RoomListHandle, String> {
    // Without the offline mode a network loss (flight mode, dead WLAN) ends in
    // `State::Error` and the service stays down until the app restarts. With
    // it, the service polls the homeserver and restarts both sync loops on its
    // own once the network is back.
    let sync = SyncService::builder(client.clone())
        .with_offline_mode()
        .build()
        .await
        .map_err(|error| format!("sync service could not be built: {error}"))?;
    let sync = Arc::new(sync);
    sync.start().await;

    let states = spawn_sync_state(sync.clone(), client.clone(), sink.clone());
    let queue = spawn_send_queue_recovery(client.clone(), sink.clone());

    let room_list = sync
        .room_list_service()
        .all_rooms()
        .await
        .map_err(|error| format!("room list unavailable: {error}"))?;

    let (filters, mut filter_updates) = mpsc::unbounded_channel::<String>();

    let task = tokio::spawn(async move {
        let (stream, controller) = room_list.entries_with_dynamic_adapters(PAGE_SIZE);

        // Nothing is emitted until a filter is set. Spaces are excluded here:
        // they have no timeline and would open an empty room, so they live on
        // their own level (see `spawn_spaces`) rather than in the chat list.
        controller.set_filter(main_list_filter(""));

        futures_util::pin_mut!(stream);

        loop {
            tokio::select! {
                diffs = stream.next() => {
                    let Some(diffs) = diffs else { break };
                    let ops: Vec<Value> = diffs.iter().map(encode).collect();
                    sink.emit(event("roomlist.diff", json!({ "ops": ops })));
                }
                pattern = filter_updates.recv() => {
                    let Some(pattern) = pattern else { break };
                    controller.set_filter(main_list_filter(pattern.trim()));
                }
            }
        }
    });

    Ok(RoomListHandle {
        sync,
        filters,
        task,
        states,
        queue,
    })
}

/// Puts a room's send queue back to work after a failure it can recover from.
///
/// The SDK switches a room's queue off after *any* failed send — "Disable the
/// queue for this room after any kind of error happened", `send_queue/mod.rs` —
/// and for a recoverable failure it deliberately leaves the message sitting in
/// the queue. Switching the queue back on is the caller's job, and the SDK
/// announces the moment through `subscribe_errors`.
///
/// Doing it only on a `Running` state of the sync service, which is what this
/// used to rely on, misses the ordinary case. The queue goes off when a *send*
/// fails, which has nothing to do with the sync service; the service only
/// enters `Offline` if one of *its* requests fails, and the state stream yields
/// on changes alone. A network gap short enough that no sync request failed
/// therefore produced no state change, no `Running`, and no revival — the queue
/// stayed off for the rest of the session and every message written afterwards
/// silently stayed put. Only restarting the app got them out, which is exactly
/// what a tester reported: two identical messages, both delivered at restart.
fn spawn_send_queue_recovery(client: Client, sink: Arc<Sink>) -> JoinHandle<()> {
    /// How long to wait before the first revival. Long enough that a queue is
    /// not woken into a network that is still gone, short enough to be over
    /// before anyone reaches for the app menu.
    const FIRST_DELAY: Duration = Duration::from_secs(5);

    /// Ceiling for the doubling, so a long outage settles at one attempt a
    /// minute instead of hammering a dead network.
    const MAX_DELAY: Duration = Duration::from_secs(60);

    /// A failure this long after the previous one counts as a new problem
    /// rather than the same one still going, so the waiting starts over.
    const SETTLED: Duration = Duration::from_secs(180);

    tokio::spawn(async move {
        let mut errors = client.send_queue().subscribe_errors();
        let mut delay = FIRST_DELAY;
        let mut previous: Option<Instant> = None;

        loop {
            let failure = match errors.recv().await {
                Ok(failure) => failure,
                // Lagged means some failures were missed while this task was
                // busy waiting. That changes nothing about what has to happen
                // — the queue still needs switching back on.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };

            let now = Instant::now();
            if previous.map_or(true, |last| now.duration_since(last) > SETTLED) {
                delay = FIRST_DELAY;
            }
            previous = Some(now);

            // Unrecoverable failures are revived too. The SDK marks the
            // offending message as wedged and skips it from then on, but it
            // switches the room's queue off just the same — leaving it off
            // would strand every *other* message in that room behind one that
            // will never be sent.
            sink.emit(event(
                "send.queue",
                json!({
                    "state": "retrying",
                    "recoverable": failure.is_recoverable,
                    "inSeconds": delay.as_secs(),
                }),
            ));

            sleep(delay).await;
            client.send_queue().set_enabled(true).await;
            delay = (delay * 2).min(MAX_DELAY);
        }
    })
}

/// Forwards the sync service's state to the UI as `sync.state`. The offline
/// mode recovers on its own; this stream only lets the user see that it is
/// happening instead of a silently stale room list.
///
/// Reaching `Running` also re-enables the send queue: a failed send disables
/// the queue for its room, so messages written while offline would otherwise
/// stay unsent even after the network came back.
fn spawn_sync_state(sync: Arc<SyncService>, client: Client, sink: Arc<Sink>) -> JoinHandle<()> {
    fn name(state: &SyncState) -> &'static str {
        match state {
            SyncState::Idle => "idle",
            SyncState::Running => "running",
            SyncState::Offline => "offline",
            SyncState::Terminated => "terminated",
            SyncState::Error(_) => "error",
        }
    }
    /// Wait before restarting a service that gave up, doubling up to this.
    const FIRST_RESTART: Duration = Duration::from_secs(2);
    const MAX_RESTART: Duration = Duration::from_secs(60);

    tokio::spawn(async move {
        let mut states = sync.state();
        let mut current = states.get();
        let mut restart_in = FIRST_RESTART;

        loop {
            sink.emit(event("sync.state", json!({ "state": name(&current) })));

            match current {
                SyncState::Running => {
                    restart_in = FIRST_RESTART;
                    client.send_queue().set_enabled(true).await;
                }

                // The offline mode does not cover these. Its recovery sits
                // inside the branch for a termination that carries an error;
                // a termination *without* one sets `Idle` or `Terminated` and
                // leaves the supervisor loop for good
                // (`matrix-sdk-ui`, `sync_service.rs`). The service is then
                // simply dead, the app quietly stops receiving, and only a
                // restart of the app brings it back — which is why the SDK
                // says the caller "MUST" watch this state and call `start()`
                // again. Nothing here did.
                //
                // `start()` is safe to call repeatedly: it only does anything
                // when the service is stopped or offline.
                SyncState::Idle | SyncState::Terminated | SyncState::Error(_) => {
                    sleep(restart_in).await;
                    restart_in = (restart_in * 2).min(MAX_RESTART);
                    sync.start().await;
                }

                // Recovers on its own; the state is forwarded only so the UI
                // can say that reconnecting is in progress.
                SyncState::Offline => {}
            }

            match states.next().await {
                Some(next) => current = next,
                None => break,
            }
        }
    })
}

/// The filter for the main chat list: never-left rooms, spaces excluded, with
/// an optional fuzzy name search. Spaces are handled on their own level.
fn main_list_filter(pattern: &str) -> BoxedFilterFn {
    let mut filters: Vec<BoxedFilterFn> = vec![
        Box::new(new_filter_non_left()),
        Box::new(new_filter_not(Box::new(new_filter_space()))),
    ];
    if !pattern.is_empty() {
        filters.push(Box::new(new_filter_fuzzy_match_room_name(pattern)));
    }
    Box::new(new_filter_all(filters))
}

/// Spawns a filtered view over the same rooms and forwards its diffs under
/// `event_name`. Both the space overview and a single space's rooms are just
/// the room list under a different filter, so they reuse the whole machinery —
/// ordering, unread counts and diffing all still come from the SDK.
fn spawn_filtered(
    room_list: RoomList,
    filter: BoxedFilterFn,
    event_name: &'static str,
    sink: Arc<Sink>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (stream, controller) = room_list.entries_with_dynamic_adapters(PAGE_SIZE);
        controller.set_filter(filter);
        futures_util::pin_mut!(stream);
        while let Some(diffs) = stream.next().await {
            let ops: Vec<Value> = diffs.iter().map(encode).collect();
            sink.emit(event(event_name, json!({ "ops": ops })));
        }
    })
}

/// Streams the joined spaces as `spaces.diff`. Reuses the running sync
/// service's room list, so no second sync connection is opened.
pub async fn spawn_spaces(
    service: Arc<RoomListService>,
    sink: Arc<Sink>,
) -> Result<JoinHandle<()>, String> {
    let room_list = service
        .all_rooms()
        .await
        .map_err(|error| format!("room list unavailable: {error}"))?;

    let filter: BoxedFilterFn = Box::new(new_filter_all(vec![
        Box::new(new_filter_non_left()),
        Box::new(new_filter_space()),
    ]));

    Ok(spawn_filtered(room_list, filter, "spaces.diff", sink))
}

/// Streams the rooms that belong to one space as `space.diff`.
///
/// The children come from the space's own `m.space.child` state — the state
/// key of each such event is the child's room id (see the spec's
/// `m.space.child` relationship). Only children this device already knows as
/// rooms appear; ones the user has not joined are silently left out, because
/// the room list only carries rooms the client has.
pub async fn spawn_space_children(
    client: &Client,
    space_room_id: &str,
    service: Arc<RoomListService>,
    sink: Arc<Sink>,
) -> Result<JoinHandle<()>, String> {
    let parsed = RoomId::parse(space_room_id).map_err(|_| "not a room identifier".to_owned())?;
    let space = client
        .get_room(&parsed)
        .ok_or_else(|| "space is not known yet".to_owned())?;

    let children = read_child_ids(&space).await;

    let room_list = service
        .all_rooms()
        .await
        .map_err(|error| format!("room list unavailable: {error}"))?;

    let filter: BoxedFilterFn = Box::new(new_filter_all(vec![
        Box::new(new_filter_non_left()),
        Box::new(new_filter_identifiers(children)),
    ]));

    Ok(spawn_filtered(room_list, filter, "space.diff", sink))
}

/// Creates a new, empty space (a room with a `room_type` of `m.space`) and
/// returns its id. It shows up in the space overview through the normal sync,
/// so nothing else has to be told about it.
pub async fn create_space(client: &Client, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a space needs a name".to_owned());
    }

    // The room type is what makes this a space rather than a chat room.
    let mut creation = create_room::v3::CreationContent::new();
    creation.room_type = Some(RoomType::Space);
    let creation = Raw::new(&creation)
        .map_err(|error| format!("could not describe the space: {error}"))?;

    let mut request = create_room::v3::Request::new();
    request.name = Some(name.to_owned());
    request.creation_content = Some(creation);
    // A private, invite-only space — a personal folder, not a public listing.
    request.preset = Some(create_room::v3::RoomPreset::PrivateChat);

    let room = client
        .create_room(request)
        .await
        .map_err(|error| format!("could not create the space: {error}"))?;

    Ok(room.room_id().as_str().to_owned())
}

/// Creates a room and returns its id. A private room is invite-only and
/// unlisted; a public one is published in the server's room directory, which is
/// what makes it findable through the directory search.
///
/// Encryption has to be asked for at creation time — no preset turns it on, and
/// switching it on later cannot protect what was already sent. It is offered
/// for public rooms too, even though that is unusual: everyone who joins later
/// can still read from their join onwards.
pub async fn create_room(
    client: &Client,
    name: &str,
    encrypted: bool,
    public: bool,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a room needs a name".to_owned());
    }

    let mut request = create_room::v3::Request::new();
    request.name = Some(name.to_owned());
    if public {
        request.visibility = Visibility::Public;
        request.preset = Some(create_room::v3::RoomPreset::PublicChat);
    } else {
        request.preset = Some(create_room::v3::RoomPreset::PrivateChat);
    }
    if encrypted {
        request.initial_state = vec![InitialStateEvent::with_empty_state_key(
            RoomEncryptionEventContent::with_recommended_defaults(),
        )
        .to_raw_any()];
    }

    let room = client
        .create_room(request)
        .await
        .map_err(|error| format!("could not create the room: {error}"))?;

    Ok(room.room_id().as_str().to_owned())
}

/// The room ids a space names as its children through `m.space.child` state.
/// A child with an empty `via` is not a valid child (that is how a room is
/// removed), and a redacted event means the link is gone; both are dropped. A
/// state-store error yields an empty list rather than failing the caller.
async fn read_child_ids(space: &matrix_sdk::Room) -> Vec<OwnedRoomId> {
    space
        .get_state_events_static::<SpaceChildEventContent>()
        .await
        .map(|events| {
            events
                .into_iter()
                .filter_map(|child| match child.deserialize() {
                    Ok(SyncOrStrippedState::Sync(SyncStateEvent::Original(event))) => {
                        if event.content.via.is_empty() {
                            None
                        } else {
                            Some(event.state_key.to_owned())
                        }
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A map from each joined space to its child structure: the ids of its member
/// rooms (excluding sub-spaces) and how many of its children are themselves
/// spaces. The front end sums the unread counts of the member rooms itself, so
/// the badge on a space stays live as messages arrive.
///
/// Shape: `{ "spaces": { "<spaceId>": { "rooms": ["<id>", ...],
/// "subspaces": N } } }`.
pub async fn space_children_map(client: &Client) -> Value {
    let mut spaces = serde_json::Map::new();

    for room in client.rooms() {
        if !room.is_space() {
            continue;
        }

        let mut rooms: Vec<String> = Vec::new();
        let mut subspaces: u64 = 0;
        for child_id in read_child_ids(&room).await {
            match client.get_room(&child_id) {
                Some(child) if child.is_space() => subspaces += 1,
                Some(child) => rooms.push(child.room_id().as_str().to_owned()),
                // A child the client does not know as a room cannot contribute
                // an unread count, so it is left out of the total.
                None => {}
            }
        }

        spaces.insert(
            room.room_id().as_str().to_owned(),
            json!({ "rooms": rooms, "subspaces": subspaces }),
        );
    }

    json!({ "spaces": Value::Object(spaces) })
}

/// Removes a space from the account: leave the room, then forget it so it
/// also vanishes from the store. Matrix has no client-side "delete room" —
/// leaving is what makes a space disappear from one's own list, and for a
/// space nobody else has joined that is equivalent to deleting it.
pub async fn leave_space(client: &Client, space_id: &str) -> Result<(), String> {
    leave_and_forget(client, space_id, "space").await
}

/// Leaves a room and forgets it. On a room that was only invited this declines
/// the invitation, which is the same request. Forgetting drops it from the
/// store as well, so the row goes away instead of lingering as a left room;
/// rejoining later is unaffected, the server hands out the history the room's
/// visibility allows.
pub async fn leave_room(client: &Client, room_id: &str) -> Result<(), String> {
    leave_and_forget(client, room_id, "room").await
}

/// The shared body of both: `what` only names the thing in the error message,
/// because a failed "delete space" must not report a room.
async fn leave_and_forget(client: &Client, room_id: &str, what: &str) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| format!("{what} is not known yet"))?;

    room.leave()
        .await
        .map_err(|error| format!("could not leave the {what}: {error}"))?;
    // Forgetting is cleanup; failing at it must not fail the leave.
    let _ = room.forget().await;
    Ok(())
}

/// Resolves a space and a child room id, returning the space room. Shared by
/// adding and removing a child.
async fn space_and_child(
    client: &Client,
    space_id: &str,
    child_id: &str,
) -> Result<(matrix_sdk::Room, OwnedRoomId), String> {
    let space_parsed =
        RoomId::parse(space_id).map_err(|_| "not a room identifier".to_owned())?;
    let child_parsed =
        RoomId::parse(child_id).map_err(|_| "not a room identifier".to_owned())?;
    let space = client
        .get_room(&space_parsed)
        .ok_or_else(|| "space is not known yet".to_owned())?;
    Ok((space, child_parsed))
}

/// Adds a room to a space by writing an `m.space.child` state event into the
/// space, with a `via` list so the child can be joined. The room shows up under
/// the space once the change syncs back.
pub async fn add_child(client: &Client, space_id: &str, child_id: &str) -> Result<(), String> {
    let (space, child) = space_and_child(client, space_id, child_id).await?;

    // `via` must name at least one server that can be used to join the child.
    // The child's own server is the natural candidate; the user's homeserver is
    // the fallback so the link is never empty (which would mean "removed").
    let mut via: Vec<OwnedServerName> = Vec::new();
    if let Some(server) = child.server_name() {
        via.push(server.to_owned());
    }
    if via.is_empty() {
        if let Some(user) = client.user_id() {
            via.push(user.server_name().to_owned());
        }
    }

    let content = SpaceChildEventContent::new(via);
    space
        .send_state_event_for_key(&child, content)
        .await
        .map_err(|error| format!("could not add the room to the space: {error}"))?;
    Ok(())
}

/// Removes a room from a space. An `m.space.child` with an empty `via` is not a
/// valid child, so this unlinks the room without needing to redact anything.
pub async fn remove_child(client: &Client, space_id: &str, child_id: &str) -> Result<(), String> {
    let (space, child) = space_and_child(client, space_id, child_id).await?;

    let content = SpaceChildEventContent::new(Vec::new());
    space
        .send_state_event_for_key(&child, content)
        .await
        .map_err(|error| format!("could not remove the room from the space: {error}"))?;
    Ok(())
}

/// Mutes a room's notifications, or lifts the mute so the account default
/// applies again. The change lands in the account's push rules and syncs to
/// every client.
///
/// Unmuting goes through the SDK's `unmute_room` rather than just deleting the
/// per-room rules: when the account default is itself "mute", deleting the
/// override leaves the room muted, and the room has to be given an explicit
/// "all messages" rule instead.
///
/// The room's cached notification mode is written back afterwards. The SDK
/// keeps that cache — which is what the room list reads — but only refreshes
/// it while processing a sync response, so without this the list would keep
/// showing the old state until the next restart.
pub async fn set_muted(client: &Client, room_id: &str, muted: bool) -> Result<(), String> {
    use matrix_sdk::notification_settings::{IsEncrypted, IsOneToOne, RoomNotificationMode};

    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room not found".to_owned())?;
    let settings = client.notification_settings().await;

    if muted {
        settings
            .set_room_notification_mode(&parsed, RoomNotificationMode::Mute)
            .await
            .map_err(|error| format!("could not mute the room: {error}"))?;
    } else {
        // From the push rules' point of view a "one to one" room is one with
        // exactly two members, encrypted and unencrypted rooms have separate
        // defaults, so both have to be passed to find the right default.
        settings
            .unmute_room(
                &parsed,
                IsEncrypted::from(room.encryption_state().is_encrypted()),
                IsOneToOne::from(room.active_members_count() == 2),
            )
            .await
            .map_err(|error| format!("could not unmute the room: {error}"))?;
    }

    match settings
        .get_user_defined_room_notification_mode(&parsed)
        .await
    {
        Some(mode) => room.update_cached_user_defined_notification_mode(mode),
        None => room.clear_user_defined_notification_mode(),
    }
    Ok(())
}

/// Marks a room as a favourite, or clears the tag. Favourite and low priority
/// are mutually exclusive — setting one clears the other, the way every other
/// client treats them — so a room never sits in two groups at once.
pub async fn set_favourite(client: &Client, room_id: &str, favourite: bool) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room not found".to_owned())?;
    if favourite {
        room.set_is_low_priority(false, None)
            .await
            .map_err(|error| format!("could not clear the low-priority tag: {error}"))?;
    }
    room.set_is_favourite(favourite, None)
        .await
        .map_err(|error| format!("could not change the favourite tag: {error}"))
}

/// Marks a room as low priority, or clears the tag. Clears the favourite tag
/// when set, mirroring `set_favourite`.
pub async fn set_low_priority(
    client: &Client,
    room_id: &str,
    low_priority: bool,
) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room not found".to_owned())?;
    if low_priority {
        room.set_is_favourite(false, None)
            .await
            .map_err(|error| format!("could not clear the favourite tag: {error}"))?;
    }
    room.set_is_low_priority(low_priority, None)
        .await
        .map_err(|error| format!("could not change the low-priority tag: {error}"))
}

/// Lists a space's linked children from the server's `/hierarchy` endpoint —
/// the only way to see rooms that are in the space but not joined yet. The
/// space itself is part of the server's answer and is dropped here.
pub async fn space_hierarchy(client: &Client, space_id: &str) -> Result<Value, String> {
    use matrix_sdk::ruma::api::client::space::get_hierarchy;

    let parsed = RoomId::parse(space_id).map_err(|_| "not a room identifier".to_owned())?;

    let mut request = get_hierarchy::v1::Request::new(parsed.to_owned());
    request.limit = Some(100u32.into());
    request.max_depth = Some(1u32.into());

    let response = client
        .send(request)
        .await
        .map_err(|error| format!("the space hierarchy is unavailable: {error}"))?;

    let rooms: Vec<Value> = response
        .rooms
        .iter()
        .filter(|chunk| chunk.summary.room_id != parsed)
        .map(|chunk| {
            let summary = &chunk.summary;
            let joined = client
                .get_room(&summary.room_id)
                .map(|room| room.state() == RoomState::Joined)
                .unwrap_or(false);
            json!({
                "id": summary.room_id.as_str(),
                "name": summary.name.clone().unwrap_or_default(),
                "topic": summary.topic.clone().unwrap_or_default(),
                "members": u64::from(summary.num_joined_members),
                "avatar": summary.avatar_url.as_ref().map(|url| url.to_string()),
                "space": summary.room_type == Some(RoomType::Space),
                "joined": joined,
            })
        })
        .collect();

    Ok(json!({ "rooms": rooms }))
}
