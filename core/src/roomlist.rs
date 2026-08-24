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
use crate::timeline::scrub_ids;
use crate::runtime::Sink;
use crate::text::strip_bidi;

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

/// The room's latest event as one banner line: what kind of thing it is, and
/// for text the text itself, cut to a length a notification can show. The
/// kind lets the UI word the non-text cases in its own language ("picture",
/// "voice message") instead of the core guessing at one.
///
/// Only remote events count — a preview exists so a notification can say what
/// arrived, and what arrived is never the user's own unsent message. Anything
/// that is not a message (state, reactions, redacted) yields no preview, and
/// an event this device could not decrypt says so as its kind, so the banner
/// can still be honest about it.
/// Resolves a room address - `#alias:server` or `!id:server` - to a room id,
/// and says whether this account is already in it.
///
/// Used by a tapped Matrix link. It answers, it never joins: a link in a
/// message must not be able to put anybody into a room, so the front end asks
/// first where the answer says "not a member".
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

/// Marks a room read without opening it: a read receipt on its latest event,
/// plus clearing the manual unread flag. Used by the chat list, where the point
/// is not having to read the room at all.
///
/// A room whose latest event this device has not seen as a remote event - an
/// empty room, or one that only holds a local echo - has nothing to point a
/// receipt at, and answers that it did nothing rather than failing.
pub async fn mark_read(client: &Client, room_id: &str) -> Result<bool, String> {
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

    // The receipt and the fully-read marker together, in one request: the
    // marker is what the room opens at next time.
    room.send_multiple_receipts(
        Receipts::new()
            .public_read_receipt(event_id.clone())
            .fully_read_marker(event_id),
    )
    .await
    .map_err(|error| format!("could not mark it read: {}", scrub_ids(&error.to_string())))?;
    // The flag is what a manual "unread" sets; a receipt alone would leave it
    // standing and the room would look unread with nothing in it to read.
    let _ = room.set_unread_flag(false).await;
    Ok(true)
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
            // An edit carries its readable text in m.new_content; the event's
            // own body is only the "* …" fallback. For an edited reply that
            // fallback starts with "* > <@…>", which the quote stripper below
            // does not recognize — the banner then shows the quoted text, not
            // the message (reported as "the notification shows my own text").
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

/// A body reduced to what a banner can hold: the quoted-reply fallback that
/// older clients still prepend ("> <@…> …" lines and the blank line after
/// them) dropped, line breaks folded to spaces, and the whole cut to a
/// length that fits two lines of a notification.
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

/// One room as the UI needs it. Deliberately flat and small — this crosses the
/// FFI on every change.
fn summarize(item: &RoomListItem) -> Value {
    let (preview_kind, preview_text, preview_sender) = latest_preview(item);
    json!({
        "id": item.room_id().as_str(),
        "name": strip_bidi(
            &item
                .cached_display_name()
                .map(|name| name.to_string())
                .unwrap_or_default(),
        ),
        "unread": item.num_unread_messages(),
        // Counted client-side against the account's push rules: only events
        // whose rules say "notify". This is the number a banner may follow —
        // `unread` counts everything and would ignore a room set to
        // mentions-only.
        "notifications": item.num_unread_notifications(),
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
/// Asks the homeserver whether it speaks the sync this app is built on.
///
/// Everything here rides on `SyncService`, which is simplified sliding sync
/// (MSC4186). The client is built with `VersionBuilder::Native`, so it
/// assumes support rather than discovering it — and where the assumption is
/// wrong every sync fails, the offline mode turns that into `Offline`, and
/// the restart loop below makes the banner flash while the room list stays
/// empty. That looks like a network problem and is not one, which is exactly
/// what a field report could not tell us.
///
/// The flag is the same one the SDK's own discovery reads. Answering `false`
/// does not stop anything: a server that supports it without advertising
/// would otherwise be locked out over a missing header.
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
    tokio::spawn(async move {
        let data = match sliding_sync_supported(&client).await {
            Ok(supported) => json!({ "supported": supported }),
            // Could not ask — no verdict. The usual reason is that there is
            // no network yet, which the offline banner already covers.
            Err(message) => json!({ "supported": true, "error": message }),
        };
        sink.emit(event("sync.support", data));
    });
}

pub async fn start(client: &Client, sink: Arc<Sink>) -> Result<RoomListHandle, String> {
    // Asked once per start, next to the sync service rather than before it:
    // the answer is a diagnosis, not a gate.
    spawn_support_check(client.clone(), sink.clone());

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

/// Everything a new room can be given at creation. It is a struct and not a
/// row of booleans because the list grew past what a call site can read, and
/// because every field here shares one property: the server takes it only now.
/// Encryption cannot be switched off again, `m.federate` is fixed for the
/// room's lifetime, and an alias or a power level added later depends on a
/// right the creator may have lost by then. Asking once beats a settings page
/// whose writes fail quietly.
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

/// Translates the front end's history setting into the state event's value.
/// An unknown name is refused rather than silently ignored: the difference
/// between `world_readable` and `joined` is who can read the room forever.
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

/// The local part the server should publish the room under. The whole address
/// is accepted too and reduced to its local part — a user who types what they
/// see elsewhere (`#name:server`) would otherwise have the server escape the
/// `#` and the `:` into an address nobody can reach.
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

/// Creates a room and returns its id. A private room is invite-only and
/// unlisted; a public one is published in the server's room directory, which is
/// what makes it findable through the directory search.
///
/// Encryption has to be asked for at creation time — no preset turns it on, and
/// switching it on later cannot protect what was already sent. It is offered
/// for public rooms too, even though that is unusual: everyone who joins later
/// can still read from their join onwards.
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

    // The invitees are parsed before anything is created: a typo in the third
    // address should not leave a half-furnished room behind. The address is
    // never echoed back — an error message ends up on screen and in
    // screenshots, and a full Matrix ID does not belong in either, so the
    // position in the list is what names the offending entry.
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
        // The one preset that hands every invitee the creator's level. It is
        // offered for private rooms only: in a public room it would promote
        // whoever the creator happened to invite and nobody else, which reads
        // as a bug rather than as a decision.
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

    // Only sent when it is false: `m.federate` defaults to true, and a request
    // that spells out the default would still pin the room to this server on a
    // server that reads the field rather than its absence.
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
/// A room's per-room notification override in the UI's vocabulary,
/// `"default"` when the room follows the account.
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

/// Sets how a room may notify: `"default"` removes the per-room override and
/// returns the room to the account default; `"all"`, `"mentions"` and
/// `"mute"` set an explicit per-room rule.
///
/// Leaving "mute" for "default" goes through the SDK's `unmute_room` rather
/// than just deleting the per-room rules: when the account default is itself
/// "mute", deleting the override leaves the room muted, and the room has to
/// be given an explicit "all messages" rule instead.
///
/// The room's cached notification mode is written back afterwards. The SDK
/// keeps that cache — which is what the room list reads — but only refreshes
/// it while processing a sync response, so without this the list would keep
/// showing the old state until the next restart.
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
                // From the push rules' point of view a "one to one" room is
                // one with exactly two members, encrypted and unencrypted
                // rooms have separate defaults, so both have to be passed to
                // find the right default.
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
