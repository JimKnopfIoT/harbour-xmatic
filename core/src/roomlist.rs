//! The room list, driven by the SDK's sync service: `VectorDiff`s in, the same
//! diffs out to the Qt model. Nothing here reimplements sync or ordering.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use matrix_sdk::deserialized_responses::{SyncOrStrippedState, TimelineEventKind};
use matrix_sdk::latest_events::LatestEventValue;
use matrix_sdk::room::Receipts;
use matrix_sdk::ruma::events::room::message::{MessageType, Relation};
use matrix_sdk::ruma::events::{
    AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
};
use matrix_sdk::ruma::api::client::room::{create_room, Visibility};
use matrix_sdk::ruma::events::{
    room::encryption::RoomEncryptionEventContent,
    room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
    space::child::SpaceChildEventContent, InitialStateEvent, SyncStateEvent,
};
use matrix_sdk::ruma::room::RoomType;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{Int, OwnedRoomId, OwnedServerName, RoomAliasId, RoomId, UserId};
use matrix_sdk::{Client, RoomState};
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    room_list_service::{
        filters::{
            new_filter_all, new_filter_fuzzy_match_room_name, new_filter_identifiers,
            new_filter_non_left, new_filter_not, new_filter_space, BoxedFilterFn,
        },
        RoomList, RoomListItem, RoomListLoadingState, RoomListService,
    },
    sync_service::{State as SyncState, SyncService},
};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant};

use crate::protocol::event;
use crate::text::scrub_ids;
use crate::runtime::Sink;
use crate::text::strip_bidi;

/// How many rooms the dynamic adapter loads per page. A phone screen shows a
/// handful; this is generous enough that scrolling rarely has to grow it.
const PAGE_SIZE: usize = 50;

/// Tasks the room list started that hold a client. Same reason as
/// `TimelineTasks`: a held client keeps the pool open under a `reset_store`.
static SIDE_TASKS: std::sync::Mutex<Vec<tokio::task::AbortHandle>> =
    std::sync::Mutex::new(Vec::new());

/// Registers such a task and forgets the ones that have finished.
fn track_side_task(handle: tokio::task::AbortHandle) {
    let mut running = SIDE_TASKS.lock().unwrap_or_else(|error| error.into_inner());
    running.retain(|existing| !existing.is_finished());
    running.push(handle);
}

/// Ends them. Called from `stop()`, which the sign-out runs before the store is
/// cleared.
fn stop_side_tasks() {
    let mut running = SIDE_TASKS.lock().unwrap_or_else(|error| error.into_inner());
    for handle in running.drain(..) {
        handle.abort();
    }
}

pub struct RoomListHandle {
    pub sync: Arc<SyncService>,
    filters: mpsc::UnboundedSender<String>,
    /// Asks for one more page of rooms. The dynamic list starts at `PAGE_SIZE`
    /// and grows only when told to.
    more: mpsc::UnboundedSender<()>,
    task: tokio::task::JoinHandle<()>,
    states: tokio::task::JoinHandle<()>,
    queue: tokio::task::JoinHandle<()>,
    loading: tokio::task::JoinHandle<()>,
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

    /// One more page of rooms, for a list that has been scrolled to its end.
    /// A no-op once every room is loaded.
    pub fn load_more(&self) -> bool {
        self.more.send(()).is_ok()
    }

    pub async fn stop(self) {
        self.task.abort();
        self.states.abort();
        self.queue.abort();
        self.loading.abort();
        // The two loose ones as well - the name lookup holds a `Room`, the sync probe
        // a `Client` - and both outlived every handle this struct knows.
        stop_side_tasks();
        self.sync.stop().await;
    }
}

/// The room's own avatar, or the single hero's picture for a direct chat.
/// With two or more heroes the room is a group and any face would be arbitrary.
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

/// Resolves `#alias:server` or `!id:server` to a room id and says whether this
/// account is in it. It answers, it never joins.
pub async fn resolve(client: &Client, address: &str) -> Result<Value, String> {
    let address = address.trim();
    let room_id = if address.starts_with('!') {
        OwnedRoomId::try_from(address).map_err(|_| "not a room identifier".to_owned())?
    } else {
        let alias = RoomAliasId::parse(address).map_err(|_| "not a room address".to_owned())?;
        client
            .resolve_room_alias(&alias)
            .await
            .map_err(|error| format!("this address is not known: {}", scrub_ids(&error.to_string())))?
            .room_id
    };

    let joined = client
        .get_room(&room_id)
        .map(|room| room.state() == RoomState::Joined)
        .unwrap_or(false);

    Ok(json!({ "roomId": room_id.as_str(), "joined": joined }))
}

/// Marks a room read from the chat list: receipt on the latest event plus the
/// manual flag. Answers that it did nothing where there is no remote event.
pub async fn mark_read(client: &Client, room_id: &str, receipt: bool) -> Result<bool, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;

    let LatestEventValue::Remote(event) = room.latest_event() else {
        return Ok(false);
    };
    let Ok(parsed_event) = event.raw().deserialize() else {
        return Ok(false);
    };
    let event_id = parsed_event.event_id().to_owned();

    // Marker always, public receipt only where allowed - the same rule the room
    // follows. Sending it unconditionally ignored the privacy switch here.
    let mut receipts = Receipts::new().fully_read_marker(event_id.clone());
    if receipt {
        receipts = receipts.public_read_receipt(event_id);
    } else {
        // The private receipt for the same reason as in the room: the counters hang
        // off receipts, not off the marker, so the badge would come back.
        receipts = receipts.private_read_receipt(event_id);
    }
    room.send_multiple_receipts(receipts)
    .await
    .map_err(|error| format!("could not mark it read: {}", scrub_ids(&error.to_string())))?;
    // The flag is what a manual "unread" sets; a receipt alone would leave it
    // standing and the room would look unread with nothing in it to read.
    let _ = room.set_unread_flag(false).await;
    Ok(true)
}

/// A matrix.to link: the address where the room has one, else the id plus the
/// servers it can be found through. An id alone is not joinable.
pub async fn permalink(client: &Client, room_id: &str) -> Result<String, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;
    room.matrix_to_permalink()
        .await
        .map(|uri| uri.to_string())
        .map_err(|error| format!("no link for this room: {}", scrub_ids(&error.to_string())))
}

fn latest_preview(item: &RoomListItem) -> (Option<&'static str>, Option<String>, Option<String>) {
    let LatestEventValue::Remote(event) = item.latest_event() else {
        return (None, None, None);
    };
    if matches!(event.kind, TimelineEventKind::UnableToDecrypt { .. }) {
        return (Some("encrypted"), None, None);
    }
    let Ok(AnySyncTimelineEvent::MessageLike(message_like)) = event.raw().deserialize() else {
        return (None, None, None);
    };
    let sender = Some(message_like.sender().to_string());
    match message_like {
        AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(message)) => {
            // An edit's readable text is in `m.new_content`; the body is the "* …"
            // fallback, which for a reply made the banner show the quoted text.
            let msgtype = match &message.content.relates_to {
                Some(Relation::Replacement(replacement)) => &replacement.new_content.msgtype,
                _ => &message.content.msgtype,
            };
            match msgtype {
                MessageType::Text(content) => {
                    (Some("text"), Some(preview_text(&content.body)), sender)
                }
                MessageType::Notice(content) => {
                    (Some("text"), Some(preview_text(&content.body)), sender)
                }
                MessageType::Emote(content) => {
                    (Some("emote"), Some(preview_text(&content.body)), sender)
                }
                MessageType::Image(_) => (Some("image"), None, sender),
                MessageType::Video(_) => (Some("video"), None, sender),
                MessageType::Audio(_) => (Some("audio"), None, sender),
                MessageType::File(_) => (Some("file"), None, sender),
                MessageType::Location(_) => (Some("location"), None, sender),
                _ => (None, None, None),
            }
        }
        AnySyncMessageLikeEvent::Sticker(SyncMessageLikeEvent::Original(_)) => {
            (Some("image"), None, sender)
        }
        _ => (None, None, None),
    }
}

/// A body reduced to what a banner holds: the quoted-reply fallback dropped,
/// line breaks folded, cut to two notification lines.
fn preview_text(body: &str) -> String {
    const PREVIEW_CHARS: usize = 160;
    // The notification banner is drawn from this, and a line that reverses
    // itself there is the same trick as in a message.
    let body = &strip_bidi(body);
    let mut lines = body.lines().peekable();
    if lines.peek().map_or(false, |line| line.starts_with('>')) {
        while lines.peek().map_or(false, |line| line.starts_with('>')) {
            lines.next();
        }
        if lines.peek().map_or(false, |line| line.trim().is_empty()) {
            lines.next();
        }
    }
    let folded = lines.collect::<Vec<_>>().join(" ");
    let folded = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    if folded.chars().count() > PREVIEW_CHARS {
        let cut: String = folded.chars().take(PREVIEW_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        folded
    }
}

/// Rooms whose display name this process has already asked to be computed.
static NAMES_REQUESTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Forgets which rooms were asked for a name, or one asked for under the
/// previous account keeps its empty name until the process restarts.
pub fn forget_name_requests() {
    if let Ok(mut guard) = NAMES_REQUESTED.lock() {
        *guard = None;
    }
}

/// The room's name, and a request for one where the SDK has none: the cache is
/// filled by a sync alone, so an unseen room has no name at all, not a wrong one.
fn display_name(item: &RoomListItem) -> String {
    if let Some(name) = item.cached_display_name() {
        return strip_bidi(&name.to_string());
    }

    let room_id = item.room_id().to_string();
    let asked = match NAMES_REQUESTED.lock() {
        Ok(mut guard) => !guard
            .get_or_insert_with(HashSet::new)
            .insert(room_id),
        Err(_) => true,
    };
    if !asked {
        let room = std::ops::Deref::deref(item).clone();
        let room_id = item.room_id().to_string();
        let handle = tokio::spawn(async move {
            if room.display_name().await.is_err() {
                // Marked before the attempt so two syncs do not both ask; on failure the mark
                // goes, or one bad moment costs the room its name for the process.
                if let Ok(mut guard) = NAMES_REQUESTED.lock() {
                    if let Some(rooms) = guard.as_mut() {
                        rooms.remove(&room_id);
                    }
                }
            }
        });
        track_side_task(handle.abort_handle());
    }
    String::new()
}

/// Events per room in a sync, and therefore how far the badge can see. Its
/// cost is this number times the rooms in the request - see the builder below.
pub const LIST_TIMELINE_LIMIT: u32 = 20;

/// One room as the UI needs it. Deliberately flat and small — this crosses the
/// FFI on every change.
fn summarize(item: &RoomListItem) -> Value {
    let (preview_kind, preview_text, preview_sender) = latest_preview(item);
    // Held to what the window can show: at the ceiling the count is a floor, and
    // the UI adds the "+". Above it the number would be arbitrary anyway.
    let cap = u64::from(LIST_TIMELINE_LIMIT);
    let unread = item.num_unread_messages().max(item.num_unread_notifications());
    let mentions = item.num_unread_mentions();
    json!({
        "id": item.room_id().as_str(),
        "name": display_name(item),
        // A floor, not a repair - the window above is what makes it right. The larger
        // of the two counters, because the badge is meant to count every message.
        "unread": unread.min(cap),
        // Whether that number is the whole truth or the edge of the window.
        "unreadCapped": unread >= cap || mentions >= cap,
        // Counted client-side against the push rules: the number a banner may follow.
        // `unread` counts everything and would ignore a mentions-only room.
        "notifications": item.num_unread_notifications(),
        "mentions": mentions.min(cap),
        "encrypted": item.encryption_state().is_encrypted(),
        "space": item.is_space(),
        // The user's tags (m.favourite / m.lowpriority): the list groups on them, and
        // low priority also suppresses notifications.
        "favourite": item.is_favourite(),
        "lowPriority": item.is_low_priority(),
        // Upgraded away: still listed, still opens, nothing new arrives. Said out
        // loud, because a dead room looks like a quiet one.
        "tombstoned": item.is_tombstoned(),
        "muted": matches!(
            item.cached_user_defined_notification_mode(),
            Some(matrix_sdk::notification_settings::RoomNotificationMode::Mute)
        ),
        "notifyMode": notify_mode_string(item.cached_user_defined_notification_mode()),
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
        // What the latest event is, for a notification that wants to say
        // more than a count. Absent when there is nothing to say.
        "previewKind": preview_kind,
        "previewText": preview_text,
        "previewSender": preview_sender,
    })
}

/// The diff stream carries immutable vectors, not slices, so this takes
/// anything iterable over items.
fn summarize_all<'a>(items: impl IntoIterator<Item = &'a RoomListItem>) -> Vec<Value> {
    items.into_iter().map(summarize).collect()
}

/// Translates one diff into the JSON the Qt model applies.
fn encode(diff: &VectorDiff<RoomListItem>) -> Value {
    crate::protocol::encode_diff(diff, summarize)
}

/// Asks whether the homeserver speaks simplified sliding sync (MSC4186). The
/// client assumes it, and where that is wrong every sync fails as "offline".
async fn sliding_sync_supported(client: &Client) -> Result<bool, String> {
    const FEATURE: &str = "org.matrix.simplified_msc3575";

    let versions = client
        .supported_versions()
        .await
        .map_err(|error| format!("{}", scrub_ids(&error.to_string())))?;

    Ok(versions
        .features
        .iter()
        .any(|feature| feature.as_str() == FEATURE))
}

/// Reports the outcome of that question as `sync.support`, so the UI can say
/// "this server cannot do it" instead of showing a network error forever.
fn spawn_support_check(client: Client, sink: Arc<Sink>) {
    let handle = tokio::spawn(async move {
        let data = match sliding_sync_supported(&client).await {
            Ok(supported) => json!({ "supported": supported }),
            // Could not ask — no verdict. The usual reason is that there is
            // no network yet, which the offline banner already covers.
            Err(message) => json!({ "supported": true, "error": message }),
        };
        sink.emit(event("sync.support", data));
    });
    track_side_task(handle.abort_handle());
}

/// The server's own room count, so a short list can say "20 of 412". The list
/// range only grows past twenty once the service reaches `Running`.
fn spawn_loading_state(room_list: &RoomList, sink: Arc<Sink>) -> JoinHandle<()> {
    let mut states = room_list.loading_state();
    tokio::spawn(async move {
        while let Some(state) = states.next().await {
            // `NotLoaded` is "no sync has answered", not zero - the front end must be
            // able to tell those apart or it reports "0 of 0" for a young list.
            let total = match state {
                RoomListLoadingState::NotLoaded => None,
                RoomListLoadingState::Loaded {
                    maximum_number_of_rooms,
                } => maximum_number_of_rooms,
            };
            sink.emit(event("roomlist.total", json!({ "total": total })));
        }
    })
}

pub async fn start(client: &Client, sink: Arc<Sink>) -> Result<RoomListHandle, String> {
    // Asked once per start, next to the sync service rather than before it:
    // the answer is a diagnosis, not a gate.
    spawn_support_check(client.clone(), sink.clone());

    // Without the offline mode one network loss parks the service in `State::Error`
    // until the app restarts.
    let sync = SyncService::builder(client.clone())
        .with_offline_mode()
        // Twenty, and the badge says "20+" from there. This multiplies by the rooms in
        // one request, and 200 shipped in 0.27.0 stopped large accounts updating.
        .with_room_list_timeline_limit(LIST_TIMELINE_LIMIT)
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

    // Subscribed before the list is moved into the task below; the subscriber
    // keeps receiving as long as that task holds the list alive.
    let loading = spawn_loading_state(&room_list, sink.clone());

    let (filters, mut filter_updates) = mpsc::unbounded_channel::<String>();
    // The dynamic adapter starts at one page and grows only when asked, and only
    // the front end knows the list was scrolled to its end.
    let (more, mut more_requests) = mpsc::unbounded_channel::<()>();

    let task = tokio::spawn(async move {
        let (stream, controller) = room_list.entries_with_dynamic_adapters(PAGE_SIZE);

        // Nothing is emitted until a filter is set. Spaces are excluded: they have no
        // timeline and live on their own level.
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
                request = more_requests.recv() => {
                    let Some(()) = request else { break };
                    controller.add_one_page();
                }
            }
        }
    });

    Ok(RoomListHandle {
        sync,
        filters,
        more,
        task,
        states,
        queue,
        loading,
    })
}

/// Puts a room's send queue back to work: the SDK disables it after any failed
/// send and leaves recoverable messages sitting. Announced by `subscribe_errors`.
fn spawn_send_queue_recovery(client: Client, sink: Arc<Sink>) -> JoinHandle<()> {
    /// Long enough not to wake into a network that is still gone, short enough to
    /// be over before anyone reaches for the app menu.
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
                // Lagged means failures were missed while this task waited. Nothing changes -
                // the queue still needs switching back on.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };

            let now = Instant::now();
            if previous.map_or(true, |last| now.duration_since(last) > SETTLED) {
                delay = FIRST_DELAY;
            }
            previous = Some(now);

            // Unrecoverable failures too: the SDK wedges the message but disables the
            // room's queue just the same, stranding every other message behind it.
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

/// Forwards the sync state as `sync.state` so the user sees a recovery happen.
/// Reaching `Running` also re-enables the send queue.
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
        // Whether the connection has been down since it was last up. Not set before
        // the first `Running`, so a normal start asks the server nothing extra.
        let mut was_down = false;

        loop {
            sink.emit(event("sync.state", json!({ "state": name(&current) })));

            match current {
                SyncState::Running => {
                    restart_in = FIRST_RESTART;
                    client.send_queue().set_enabled(true).await;

                    // A key backup is a question over the network and had no answer while the
                    // connection was down. Spawned: this loop must stay free for state changes.
                    if was_down {
                        was_down = false;
                        let client = client.clone();
                        let sink = sink.clone();
                        let handle = tokio::spawn(async move {
                            sink.emit(event(
                                "encryption.changed",
                                crate::recovery::status(&client).await,
                            ));
                        });
                        track_side_task(handle.abort_handle());
                    }
                }

                // The offline mode does not cover these: a termination without an error leaves
                // the supervisor for good. `start()` is safe to call repeatedly.
                SyncState::Idle | SyncState::Terminated | SyncState::Error(_) => {
                    was_down = true;
                    sleep(restart_in).await;
                    restart_in = (restart_in * 2).min(MAX_RESTART);
                    sync.start().await;
                }

                // Recovers on its own; the state is forwarded only so the UI
                // can say that reconnecting is in progress.
                SyncState::Offline => was_down = true,
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

/// A filtered view over the same rooms under `event_name`. Spaces and a single
/// space's rooms are the room list with another filter.
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

/// Streams a space's rooms as `space.diff`, from its `m.space.child` state.
/// Only children this device knows as rooms appear.
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

/// Creates an empty space - a room with `room_type` `m.space` - and returns its
/// id. It reaches the overview through the ordinary sync.
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

/// Everything a new room can be given at creation, as a struct: every field
/// here is one the server takes only now.
pub struct NewRoom {
    pub name: String,
    pub topic: String,
    /// Local part of the published address, without `#` and server part.
    pub alias: String,
    pub encrypted: bool,
    pub public: bool,
    /// One of `world_readable`, `shared`, `invited`, `joined`; empty keeps the
    /// preset's default.
    pub history_visibility: String,
    pub invite: Vec<String>,
    /// False confines the room to this homeserver.
    pub federate: bool,
    /// Only moderators may send messages.
    pub read_only: bool,
    /// Everyone invited starts at the creator's power level.
    pub equal_power: bool,
}

/// The front end's history setting as the state event's value. An unknown name
/// is refused: `world_readable` against `joined` is who can read forever.
fn history_visibility(name: &str) -> Result<Option<HistoryVisibility>, String> {
    Ok(match name.trim() {
        "" => None,
        "world_readable" => Some(HistoryVisibility::WorldReadable),
        "shared" => Some(HistoryVisibility::Shared),
        "invited" => Some(HistoryVisibility::Invited),
        "joined" => Some(HistoryVisibility::Joined),
        _ => return Err("unknown setting for the history".to_owned()),
    })
}

/// The local part to publish under. A whole address is reduced to it - the
/// server would otherwise escape `#` and `:` into an unreachable name.
fn alias_localpart(input: &str) -> Result<Option<String>, String> {
    let trimmed = input.trim().trim_start_matches('#');
    if trimmed.is_empty() {
        return Ok(None);
    }
    let localpart = match trimmed.split_once(':') {
        Some((local, _server)) => local,
        None => trimmed,
    };
    if localpart.is_empty() {
        return Err("the address needs a name in front of the server".to_owned());
    }
    if localpart.chars().any(char::is_whitespace) {
        return Err("the address must not contain spaces".to_owned());
    }
    Ok(Some(localpart.to_owned()))
}

/// Creates a room and returns its id. Encryption has to be asked for now: no
/// preset turns it on, and later cannot protect what was already sent.
pub async fn create_room(client: &Client, room: NewRoom) -> Result<String, String> {
    let name = room.name.trim();
    if name.is_empty() {
        return Err("a room needs a name".to_owned());
    }

    let mut request = create_room::v3::Request::new();
    request.name = Some(name.to_owned());

    let topic = room.topic.trim();
    if !topic.is_empty() {
        request.topic = Some(topic.to_owned());
    }

    request.room_alias_name = alias_localpart(&room.alias)?;

    // Invitees are parsed before anything is created, and never echoed back: an
    // error ends up on screen and a full Matrix ID does not belong there.
    let mut invites = Vec::new();
    for (position, entry) in room.invite.iter().enumerate() {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let user = UserId::parse(entry)
            .map_err(|_| format!("entry {} is not a valid Matrix address", position + 1))?;
        invites.push(user);
    }
    request.invite = invites;

    if room.public {
        request.visibility = Visibility::Public;
        request.preset = Some(create_room::v3::RoomPreset::PublicChat);
    } else if room.equal_power {
        // The one preset that hands every invitee the creator's level. Private rooms
        // only: in a public one it would promote whoever happened to be invited.
        request.preset = Some(create_room::v3::RoomPreset::TrustedPrivateChat);
    } else {
        request.preset = Some(create_room::v3::RoomPreset::PrivateChat);
    }

    let mut initial = Vec::new();
    if room.encrypted {
        initial.push(
            InitialStateEvent::with_empty_state_key(
                RoomEncryptionEventContent::with_recommended_defaults(),
            )
            .to_raw_any(),
        );
    }
    if let Some(visibility) = history_visibility(&room.history_visibility)? {
        initial.push(
            InitialStateEvent::with_empty_state_key(RoomHistoryVisibilityEventContent::new(
                visibility,
            ))
            .to_raw_any(),
        );
    }
    request.initial_state = initial;

    // Only sent when false: `m.federate` defaults to true, and spelling out the
    // default still pins the room on a server that reads the field.
    if !room.federate {
        let mut creation = create_room::v3::CreationContent::new();
        creation.federate = false;
        request.creation_content = Some(
            Raw::new(&creation).map_err(|error| format!("could not describe the room: {error}"))?,
        );
    }

    if room.read_only {
        // Moderator level to speak. Nothing else is overridden, so the server's
        // own defaults keep applying to kicking, banning and state.
        let mut levels = create_room::RoomPowerLevelsContentOverride::new();
        levels.events_default = Some(Int::from(50u8));
        request.power_level_content_override = Some(
            Raw::new(&levels).map_err(|error| format!("could not describe the room: {error}"))?,
        );
    }

    let room = client
        .create_room(request)
        .await
        .map_err(|error| format!("could not create the room: {error}"))?;

    Ok(room.room_id().as_str().to_owned())
}

/// The child ids a space names through `m.space.child`. An empty `via` is how a
/// child is removed; a store error yields an empty list, not a failure.
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

/// Each joined space with its member rooms and how many children are spaces.
/// The front end sums the unread counts, so the badge stays live.
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

/// Leave, then forget. Matrix has no "delete room", and for a space nobody
/// else joined the pair is equivalent to deleting it.
pub async fn leave_space(client: &Client, space_id: &str) -> Result<(), String> {
    leave_and_forget(client, space_id, "space").await
}

/// Leaves and forgets a room; on an invited room the same request declines it.
/// Forgetting drops the row instead of leaving it as a left room.
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

/// Adds a room to a space through `m.space.child`, with a `via` list so the
/// child can be joined. It appears once the change syncs back.
pub async fn add_child(client: &Client, space_id: &str, child_id: &str) -> Result<(), String> {
    let (space, child) = space_and_child(client, space_id, child_id).await?;

    // `via` must name at least one server that can join the child: its own
    // server, the user's as fallback - an empty list means "removed".
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

/// A room's notification override in the UI's vocabulary, `"default"` where the
/// room follows the account.
pub fn notify_mode_string(
    mode: Option<matrix_sdk::notification_settings::RoomNotificationMode>,
) -> &'static str {
    use matrix_sdk::notification_settings::RoomNotificationMode;
    match mode {
        None => "default",
        Some(RoomNotificationMode::AllMessages) => "all",
        Some(RoomNotificationMode::MentionsAndKeywordsOnly) => "mentions",
        Some(RoomNotificationMode::Mute) => "mute",
    }
}

/// Sets how a room may notify. Leaving "mute" goes through `unmute_room`: where
/// the account default is mute, deleting the override changes nothing.
pub async fn set_notification_mode(
    client: &Client,
    room_id: &str,
    mode: &str,
) -> Result<(), String> {
    use matrix_sdk::notification_settings::{IsEncrypted, IsOneToOne, RoomNotificationMode};

    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room not found".to_owned())?;
    let settings = client.notification_settings().await;

    let explicit = match mode {
        "all" => Some(RoomNotificationMode::AllMessages),
        "mentions" => Some(RoomNotificationMode::MentionsAndKeywordsOnly),
        "mute" => Some(RoomNotificationMode::Mute),
        "default" => None,
        _ => return Err("unknown notification mode".to_owned()),
    };

    match explicit {
        Some(wanted) => settings
            .set_room_notification_mode(&parsed, wanted)
            .await
            .map_err(|error| format!("could not change the room's notifications: {error}"))?,
        None => {
            let was_muted = matches!(
                settings
                    .get_user_defined_room_notification_mode(&parsed)
                    .await,
                Some(RoomNotificationMode::Mute)
            );
            if was_muted {
                // Push rules call a room with exactly two members "one to one", and encrypted
                // and unencrypted rooms have separate defaults.
                settings
                    .unmute_room(
                        &parsed,
                        IsEncrypted::from(room.encryption_state().is_encrypted()),
                        IsOneToOne::from(room.active_members_count() == 2),
                    )
                    .await
                    .map_err(|error| format!("could not unmute the room: {error}"))?;
            } else {
                settings
                    .delete_user_defined_room_rules(&parsed)
                    .await
                    .map_err(|error| {
                        format!("could not restore the room's notifications: {error}")
                    })?;
            }
        }
    }

    match settings
        .get_user_defined_room_notification_mode(&parsed)
        .await
    {
        Some(current) => room.update_cached_user_defined_notification_mode(current),
        None => room.clear_user_defined_notification_mode(),
    }
    Ok(())
}

/// Marks a room favourite or clears the tag. Favourite and low priority are
/// mutually exclusive, so a room never sits in two groups.
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

/// A space's linked children from `/hierarchy` - the only way to see rooms in
/// it that are not joined. The space itself is dropped from the answer.
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
                "name": strip_bidi(summary.name.as_deref().unwrap_or_default()),
                "topic": strip_bidi(summary.topic.as_deref().unwrap_or_default()),
                "members": u64::from(summary.num_joined_members),
                "avatar": summary.avatar_url.as_ref().map(|url| url.to_string()),
                "space": summary.room_type == Some(RoomType::Space),
                "joined": joined,
            })
        })
        .collect();

    Ok(json!({ "rooms": rooms }))
}
