//! The timeline of a single room. The SDK hands out `VectorDiff`s and the Qt
//! model applies them; encryption sits below and needs nothing here.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use matrix_sdk::{
    room::edit::EditedContent,
    room::Receipts,
    ruma::{
        api::client::room::create_room,
        events::{
            // Two different `ReceiptType`s exist: the one above is what a
            // receipt is sent as, this one is what a stored receipt reads as.
            receipt::{ReceiptThread, ReceiptType as StoredReceiptType},
            room::{
                encryption::RoomEncryptionEventContent,
                message::{FormattedBody, MessageFormat, MessageType, RoomMessageEventContent},
            },
            InitialStateEvent,
        },
        EventId, OwnedEventId, OwnedServerName, OwnedUserId, RoomId, RoomOrAliasId, ServerName,
        UserId,
    },
    Client,
};
use matrix_sdk_base::crypto::types::events::UtdCause;
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    timeline::{
        EncryptedMessage, EventSendState, EventTimelineItem, MembershipChange, MsgLikeKind,
        RoomExt, TimelineDetails, TimelineEventItemId, TimelineItem, TimelineItemContent,
        TimelineReadReceiptTracking, VirtualTimelineItem,
    },
};
use serde_json::{json, Value};

use crate::compose::to_formatted_body;
use crate::protocol::event;
use crate::runtime::Sink;
use crate::text::{safe_file_name, scrub_ids, strip_bidi};

/// How many events one backwards pagination asks for.
const PAGE_SIZE: u16 = 30;

/// Below this many items, "complete" is not believed - see `paginate`. One
/// page: fewer events than a single request returns, yet "that is all".
const SUSPICIOUSLY_SHORT: usize = PAGE_SIZE as usize;

/// An open timeline and the task streaming its updates.
pub struct TimelineHandle {
    pub room_id: String,
    live: bool,
    /// The pinned-messages view, which cannot be paginated at all.
    pinned: bool,
    timeline: Arc<matrix_sdk_ui::timeline::Timeline>,
    task: tokio::task::JoinHandle<()>,
    /// Whether the one permitted cache reset was already used. Without it the room
    /// is cleared on every pagination that legitimately reports a short room.
    retried_from_end: AtomicBool,
    /// The thread this view is focused on, or empty for a room view.
    thread_root: String,
    /// Whether this timeline was built tracking other people's receipts. Part
    /// of the handle because the setting can only be chosen at build time.
    receipts: bool,
    /// Everything this timeline started: the quote-fetch lanes and a handle on
    /// every task. Closed with it, so nothing still holds the client or the store.
    tasks: TimelineTasks,
}

impl TimelineHandle {
    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    /// Which thread this view shows, or empty for a room view.
    pub fn thread_root(&self) -> &str {
        &self.thread_root
    }

    /// Whether this is the live view; focused views (pinned, permalink) are
    /// always rebuilt on open.
    pub fn is_live(&self) -> bool {
        self.live
    }

    /// Whether it was built with receipt tracking on.
    pub fn tracks_receipts(&self) -> bool {
        self.receipts
    }

    /// Pins a message to the room, or unpins it. Idempotent on the server —
    /// pinning something already pinned is not an error.
    pub async fn set_pinned(&self, event_id: &str, pin: bool) -> Result<(), String> {
        let parsed =
            EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;
        let room = self.timeline.room();
        let result = if pin {
            room.pin_event(&parsed).await
        } else {
            room.unpin_event(&parsed).await
        };
        result
            .map(|_| ())
            .map_err(|error| format!("could not change the pin: {error}"))
    }

    /// The room's pinned ids and the newest pin's body, from the server where
    /// possible so a pin made a moment ago is already in the answer.
    pub async fn pinned_info(&self) -> (Vec<String>, String) {
        let room = self.timeline.room();
        let ids = pinned_ids(room).await;
        let preview = pinned_preview(room, &ids).await;
        (ids.iter().map(|id| id.to_string()).collect(), preview)
    }

    /// Loads older events, answers whether the room's start was reached. Never how
    /// many rows: `paginate_backwards` returns before they arrive as diffs.
    pub async fn paginate(&self) -> Result<bool, String> {
        // The pinned view has no history behind it and the SDK answers every
        // pagination on it with `NotSupported`. That is "all of them", not an error.
        if self.pinned {
            return Ok(true);
        }

        let reached_start = self
            .timeline
            .paginate_backwards(PAGE_SIZE)
            .await
            .map_err(|error| format!("could not load older messages: {error}"))?;

        if !reached_start || !self.live {
            return Ok(reached_start);
        }

        // The SDK reports "start of the timeline" whenever its cache has events but
        // no gap - a room joined while running. Drop the cache once and retry.
        if self.timeline.items().await.len() >= SUSPICIOUSLY_SHORT
            || self.retried_from_end.swap(true, Ordering::SeqCst)
        {
            return Ok(true);
        }

        let (cache, _handles) = self
            .timeline
            .room()
            .event_cache()
            .await
            .map_err(|error| format!("could not reach the room cache: {error}"))?;
        cache
            .clear()
            .await
            .map_err(|error| format!("could not clear the room cache: {error}"))?;

        self.timeline
            .paginate_backwards(PAGE_SIZE)
            .await
            .map_err(|error| format!("could not load older messages: {error}"))
    }

    /// Sends a text message. It shows up as a local echo immediately. Where the
    /// composer's markers said so, a `formatted_body` travels beside the text.
    pub async fn send_text(&self, body: String) -> Result<(), String> {
        self.timeline
            .send(text_content(body).into())
            .await
            .map(|_| ())
            .map_err(|error| format!("message could not be sent: {error}"))
    }

    /// A shared handle on the underlying timeline, for the media module.
    /// `Timeline` itself is not `Clone`, hence the `Arc`.
    pub fn timeline(&self) -> Arc<matrix_sdk_ui::timeline::Timeline> {
        self.timeline.clone()
    }

    /// Sends a reply to an earlier message.
    pub async fn reply(&self, event_id: &str, body: String) -> Result<(), String> {
        let id = EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;
        self.timeline
            .send_reply(text_content(body).into(), id)
            .await
            .map_err(|error| format!("could not reply: {error}"))
    }

    /// An attachment is edited as a *caption*: `EditedContent::RoomMessage` would
    /// turn an `m.image` into an `m.text` and the picture would be gone for good.
    pub async fn edit(&self, event_id: &str, body: String) -> Result<(), String> {
        let id = parse_event_id(event_id)?;
        let content = match self.is_media_event(&id).await {
            Some(true) => EditedContent::MediaCaption {
                formatted_caption: to_formatted_body(&body).map(FormattedBody::html),
                caption: Some(body).filter(|text| !text.is_empty()),
                mentions: None,
            },
            Some(false) => EditedContent::RoomMessage(text_content(body).into()),
            // Unknown kind: refusing costs an edit the UI could not have
            // offered anyway, while guessing "text" on a picture destroys it.
            None => return Err("this message is no longer loaded".to_owned()),
        };
        self.timeline
            .edit(&id, content)
            .await
            .map_err(|error| format!("could not edit: {error}"))
    }

    /// Whether that event carries an attachment. `None` when the timeline does
    /// not hold the event, so the caller can refuse rather than guess.
    async fn is_media_event(&self, id: &TimelineEventItemId) -> Option<bool> {
        // A local echo has no event id yet, and cannot be edited either.
        let TimelineEventItemId::EventId(event_id) = id else {
            return None;
        };
        let item = self.timeline.item_by_event_id(event_id).await?;
        match item.content() {
            TimelineItemContent::MsgLike(content) => match &content.kind {
                MsgLikeKind::Message(message) => Some(matches!(
                    message.msgtype(),
                    MessageType::Image(_)
                        | MessageType::Video(_)
                        | MessageType::Audio(_)
                        | MessageType::File(_)
                )),
                _ => Some(false),
            },
            _ => Some(false),
        }
    }

    /// Adds our reaction or takes it back. One command: the SDK decides from what
    /// it holds, and two would be two chances to disagree with it.
    pub async fn toggle_reaction(&self, event_id: &str, key: &str) -> Result<(), String> {
        if key.is_empty() {
            return Err("no reaction given".to_owned());
        }
        let id = parse_event_id(event_id)?;
        self.timeline
            .toggle_reaction(&id, key)
            .await
            .map(|_| ())
            .map_err(|error| format!("could not react: {error}"))
    }

    /// Puts a parked message back in the queue - the SDK stops after an error it
    /// cannot recover from. Only for a message that never reached the server.
    pub async fn retry(&self, txn_id: &str) -> Result<(), String> {
        if txn_id.is_empty() {
            return Err("not a queued message".to_owned());
        }
        let wanted = TimelineEventItemId::TransactionId(txn_id.into());
        // Walked rather than looked up: the SDK offers `item_by_event_id`, and
        // a queued message is precisely the one without an event id.
        let items = self.timeline.items().await;
        let handle = items
            .iter()
            .filter_map(|item| item.as_event())
            .find(|event| event.identifier() == wanted)
            .and_then(|event| event.local_echo_send_handle())
            .ok_or_else(|| "this message is no longer queued".to_owned())?;
        handle
            .unwedge()
            .await
            .map_err(|error| format!("could not send it again: {}", scrub_ids(&error.to_string())))
    }

    /// Redacts a sent message, or aborts a queued one: the first leaves a deleted
    /// row, the second makes the row disappear.
    pub async fn redact(&self, event_id: &str, txn_id: &str) -> Result<(), String> {
        let id = if event_id.is_empty() {
            if txn_id.is_empty() {
                return Err("not an event identifier".to_owned());
            }
            TimelineEventItemId::TransactionId(txn_id.into())
        } else {
            parse_event_id(event_id)?
        };
        self.timeline
            .redact(&id, None)
            .await
            .map_err(|error| format!("could not delete: {error}"))
    }

    /// Marks the room read: the receipt others see and the fully-read marker, in
    /// one request. Answers whether anything was sent at all.
    pub async fn mark_read(&self, receipt: bool) -> Result<bool, String> {
        let Some(event_id) = self.timeline.latest_event_id().await else {
            return Ok(false);
        };
        let mut receipts = Receipts::new().fully_read_marker(event_id.clone());
        if receipt {
            receipts = receipts.public_read_receipt(event_id);
        } else {
            // Nobody is told and the room still counts as read: the counters anchor on
            // the receipts, not on the marker, so the marker alone left the badge standing.
            receipts = receipts.private_read_receipt(event_id);
        }
        self.timeline
            .send_multiple_receipts(receipts)
            .await
            .map_err(|error| format!("read receipt could not be sent: {error}"))?;
        Ok(true)
    }

    /// Everybody who has read up to `event_id` *or past it*: a receipt marks the
    /// newest event read, so asking only this one answers "nobody".
    pub async fn readers(&self, event_id: &str) -> Result<Vec<Value>, String> {
        let target = EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;
        let items = self.timeline.items().await;
        let room = self.timeline.room();
        let own = room.own_user_id().to_owned();

        let mut from = None;
        for (index, item) in items.iter().enumerate() {
            if let Some(event) = item.as_event() {
                if event.event_id() == Some(target.as_ref()) {
                    from = Some(index);
                    break;
                }
            }
        }
        let from = from.ok_or_else(|| "that message is not in the timeline".to_owned())?;

        let mut users: Vec<OwnedUserId> = Vec::new();
        for item in items.iter().skip(from) {
            let Some(event) = item.as_event() else {
                continue;
            };
            for user in event.read_receipts().keys() {
                if user == &own {
                    continue;
                }
                if !users.iter().any(|known| known == user) {
                    users.push(user.clone());
                }
            }
        }

        let mut readers = Vec::with_capacity(users.len());
        for user in users {
            let member = room.get_member_no_sync(&user).await.ok().flatten();
            let name = member
                .as_ref()
                .and_then(|member| member.display_name())
                .map(strip_bidi)
                .unwrap_or_else(|| user.as_str().to_owned());
            let avatar = member
                .as_ref()
                .and_then(|member| member.avatar_url())
                .map(|url| url.to_string());
            readers.push(json!({
                "userId": user.as_str(),
                "name": name,
                "avatar": avatar,
            }));
        }
        Ok(readers)
    }

    /// Who reacted with `key`. Asked on a long press, since rows carry counts only.
    /// The key is the one the row sends back, variation selectors included.
    pub async fn reactors(&self, event_id: &str, key: &str) -> Result<Vec<Value>, String> {
        let target = EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;
        let items = self.timeline.items().await;
        let room = self.timeline.room();

        let mut users: Vec<OwnedUserId> = Vec::new();
        let mut found = false;
        for item in items.iter() {
            let Some(event) = item.as_event() else {
                continue;
            };
            if event.event_id() != Some(target.as_ref()) {
                continue;
            }
            found = true;
            if let TimelineItemContent::MsgLike(content) = event.content() {
                for (reaction_key, senders) in content.reactions.iter() {
                    if reaction_key.as_str() != key {
                        continue;
                    }
                    for user in senders.keys() {
                        users.push(user.clone());
                    }
                }
            }
            break;
        }
        if !found {
            return Err("that message is not in the timeline".to_owned());
        }

        let mut reactors = Vec::with_capacity(users.len());
        for user in users {
            let member = room.get_member_no_sync(&user).await.ok().flatten();
            let name = member
                .as_ref()
                .and_then(|member| member.display_name())
                .map(strip_bidi)
                .unwrap_or_else(|| user.as_str().to_owned());
            reactors.push(json!({
                "userId": user.as_str(),
                "name": name,
            }));
        }
        Ok(reactors)
    }

    /// Takes `&self` rather than consuming: the runtime keeps this behind an `Arc`
    /// so a command can work on it without the state lock.
    pub async fn close(&self) {
        self.task.abort();
        // Not only the diff stream: every quote fetch holds the timeline, the room and
        // the open store, and a `reset_store` must have no client on the directory.
        self.tasks.close().await;
    }
}

fn parse_event_id(event_id: &str) -> Result<TimelineEventItemId, String> {
    EventId::parse(event_id)
        .map(TimelineEventItemId::EventId)
        .map_err(|_| "not an event identifier".to_owned())
}

/// The HTML variant rewritten into the markup Qt can draw. Only the textual
/// types have one - elsewhere `body` is a file name.
fn formatted_body(message_type: &MessageType) -> Option<String> {
    let formatted: &FormattedBody = match message_type {
        MessageType::Text(content) => content.formatted.as_ref()?,
        MessageType::Notice(content) => content.formatted.as_ref()?,
        MessageType::Emote(content) => content.formatted.as_ref()?,
        _ => return None,
    };
    // `format` is an open enum in the spec; HTML is the only one defined, and
    // anything else is markup this client has never seen.
    if formatted.format != MessageFormat::Html {
        return None;
    }
    crate::markup::to_styled_text(&formatted.body)
}

/// Describes an attachment for showing and fetching. The source travels as
/// opaque JSON: in encrypted rooms it carries the decryption keys.
fn media_info(message_type: &MessageType) -> Option<Value> {
    // Encrypted rooms have no server-made thumbnails - the sender ships one as a
    // separate source, and it is the only way to avoid full-size downloads.
    let thumbnail = match message_type {
        MessageType::Image(content) => content
            .info
            .as_ref()
            .and_then(|i| i.thumbnail_source.as_ref())
            .and_then(|source| serde_json::to_value(source).ok()),
        MessageType::Video(content) => content
            .info
            .as_ref()
            .and_then(|i| i.thumbnail_source.as_ref())
            .and_then(|source| serde_json::to_value(source).ok()),
        _ => None,
    };

    // `filename()`, never `body`: a captioned attachment carries the caption in
    // `body` (MSC2530), and reading it saved pictures under their caption.
    let (source, mimetype, size, filename) = match message_type {
        MessageType::Image(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.filename().to_owned(),
        ),
        MessageType::Video(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.filename().to_owned(),
        ),
        MessageType::Audio(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.filename().to_owned(),
        ),
        MessageType::File(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.filename().to_owned(),
        ),
        _ => return None,
    };

    let (width, height) = match message_type {
        MessageType::Image(content) => (
            content.info.as_ref().and_then(|i| i.width),
            content.info.as_ref().and_then(|i| i.height),
        ),
        MessageType::Video(content) => (
            content.info.as_ref().and_then(|i| i.width),
            content.info.as_ref().and_then(|i| i.height),
        ),
        _ => (None, None),
    };

    Some(json!({
        "source": serde_json::to_value(source).ok(),
        "thumbnailSource": thumbnail,
        "mimetype": mimetype,
        "size": size.map(u64::from),
        // Sanitised here, not where it is saved: a name out of an event can hold a
        // path, and the rule belongs where it can be tested (`core/src/text.rs`).
        "filename": safe_file_name(&filename),
        // Passed through as declared. Nulling an absurd value made the row's guard
        // read it as "not declared" and let it through.
        "width": width.map(u64::from),
        "height": height.map(u64::from),
    }))
}

/// The event id if this is a reply whose quote still has to be asked for:
/// `Unavailable` was never requested, `Error` failed and is worth one more try.
fn reply_needing_details(item: &TimelineItem) -> Option<OwnedEventId> {
    let event = item.as_event()?;
    let TimelineItemContent::MsgLike(content) = event.content() else {
        return None;
    };
    // The SDK marks every threaded event a reply to the previous one, for clients
    // without threads. In the room that is one fetch per reply for nothing.
    if content.thread_root.is_some() {
        return None;
    }
    let in_reply_to = content.in_reply_to.as_ref()?;
    if !matches!(
        in_reply_to.event,
        TimelineDetails::Unavailable | TimelineDetails::Error(_)
    ) {
        return None;
    }
    event.event_id().map(|id| id.to_owned())
}

/// Event ids of the reply rows in `diff` whose quoted message was never loaded,
/// so their details can be asked for.
fn replies_needing_details(diff: &VectorDiff<Arc<TimelineItem>>) -> Vec<OwnedEventId> {
    match diff {
        VectorDiff::Append { values } | VectorDiff::Reset { values } => {
            values.iter().filter_map(|item| reply_needing_details(item)).collect()
        }
        VectorDiff::PushBack { value }
        | VectorDiff::PushFront { value }
        | VectorDiff::Insert { value, .. }
        | VectorDiff::Set { value, .. } => reply_needing_details(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// How often one reply's details are asked for. A failure returns as a `Set`
/// diff that qualifies again - unbounded, that is a loop against a 429.
const DETAIL_ATTEMPTS: u8 = 2;

/// How many pinned events the diagnostic may look at. "N pinned, only M
/// readable" gets no truer past twenty, and each miss is a request.
const PINNED_REPORT_LIMIT: usize = 20;

/// Quote fetches in flight per timeline. Unbounded, a room of quoted messages
/// fired a hundred requests at once, each 429 nursed for fifteen minutes.
const DETAIL_LANES: usize = 4;

/// Everything one timeline set running, and the switch that ends it. A
/// `tokio::spawn` here that is not registered is the next lost task.
#[derive(Clone)]
struct TimelineTasks {
    permits: Arc<tokio::sync::Semaphore>,
    open: Arc<AtomicBool>,
    /// Every fetch that may still be running: closing the queue is not enough, a
    /// task holding a permit sits inside a request the SDK nurses for minutes.
    running: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl TimelineTasks {
    fn new() -> Self {
        Self {
            permits: Arc::new(tokio::sync::Semaphore::new(DETAIL_LANES)),
            open: Arc::new(AtomicBool::new(true)),
            running: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    /// Spawns a task that dies with this timeline; pruning happens here because a
    /// task being aborted cannot tidy up. A closed timeline starts nothing.
    async fn spawn_tracked<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        if !self.open.load(Ordering::SeqCst) {
            return;
        }
        let handle = tokio::spawn(future);
        let mut running = self.running.lock().await;
        running.retain(|existing| !existing.is_finished());
        running.push(handle);
    }

    /// Ends everything and waits: `abort()` only marks a task, and the caller
    /// deletes the store next. The SDK's own pagination task is out of reach.
    async fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
        // Wakes everything waiting; `acquire_owned` then fails and the task drops its
        // handle instead of holding it for the next quarter of an hour.
        self.permits.close();
        let handles: Vec<_> = {
            let mut running = self.running.lock().await;
            running.drain(..).collect()
        };
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            // The abort is what ends it; this only waits for the drop to have
            // happened, so the `Arc` on the client is really gone.
            let _ = handle.await;
        }
    }
}

/// Spawns a detail fetch per reply target with attempts left. Failures go out
/// as `timeline.detailError`, truncated and scrubbed.
async fn request_reply_details(
    // A vector, not an iterator: this is async, and a borrowed iterator becomes a
    // lifetime the spawned task cannot satisfy.
    ids: Vec<OwnedEventId>,
    requested: &mut HashMap<OwnedEventId, u8>,
    timeline: &Arc<matrix_sdk_ui::timeline::Timeline>,
    sink: &Arc<Sink>,
    tasks: &TimelineTasks,
) {
    for id in ids {
        let attempts = requested.entry(id.clone()).or_insert(0);
        if *attempts >= DETAIL_ATTEMPTS {
            continue;
        }
        *attempts += 1;
        let source = timeline.clone();
        let sink = sink.clone();
        let inner = tasks.clone();
        tasks.spawn_tracked(async move {
            // Queued, not dropped: every quote gets its turn, four at a time. A closed
            // timeline fails the acquire and what the task holds is dropped.
            let Ok(_permit) = inner.permits.clone().acquire_owned().await else {
                return;
            };
            if !inner.open.load(Ordering::SeqCst) {
                return;
            }
            if let Err(error) = source.fetch_details_for_event(&id).await {
                // On a char boundary: `truncate` counts bytes and would panic
                // on an id whose server part is not ASCII.
                let full = id.as_str();
                let cut = full
                    .char_indices()
                    .nth(9)
                    .map(|(index, _)| index)
                    .unwrap_or(full.len());
                let short = full[..cut].to_owned();
                // A class, not the message: the SDK's error carries the full id in its
                // `Debug` output, and that is the common case here.
                let class = match error {
                    matrix_sdk_ui::timeline::Error::EventNotInTimeline(_) => "not-in-timeline",
                    _ => "fetch-failed",
                };
                sink.emit(event(
                    "timeline.detailError",
                    json!({ "eventId": short, "error": class }),
                ));
            }
        })
        .await;
    }
}

/// Why an event could not be decrypted, as a stable key. Throwing it away
/// turns every "he cannot read me" into an afternoon of measuring.
fn utd_cause(event: &EventTimelineItem) -> &'static str {
    let TimelineItemContent::MsgLike(content) = event.content() else {
        return "";
    };
    let MsgLikeKind::UnableToDecrypt(message) = &content.kind else {
        return "";
    };
    let EncryptedMessage::MegolmV1AesSha2 { cause, .. } = message else {
        return "";
    };
    match cause {
        UtdCause::SentBeforeWeJoined => "sentBeforeJoin",
        UtdCause::VerificationViolation => "verificationViolation",
        UtdCause::UnsignedDevice => "unsignedDevice",
        UtdCause::UnknownDevice => "unknownDevice",
        UtdCause::HistoricalMessageAndBackupIsDisabled => "historicalNoBackup",
        UtdCause::HistoricalMessageAndDeviceIsUnverified => "historicalUnverifiedDevice",
        UtdCause::WithheldForUnverifiedOrInsecureDevice => "withheldInsecure",
        UtdCause::WithheldBySender => "withheldBySender",
        // The SDK has no explanation either; the row keeps its plain text.
        UtdCause::Unknown => "",
    }
}

/// Describes the quoted message as far as it is known. `state`: "ready",
/// "error" (not there or not permitted), or "loading".
fn reply_info(content: &matrix_sdk_ui::timeline::MsgLikeContent) -> Option<Value> {
    let in_reply_to = content.in_reply_to.as_ref()?;

    let state = match &in_reply_to.event {
        TimelineDetails::Ready(_) => "ready",
        TimelineDetails::Error(_) => "error",
        _ => "loading",
    };

    // What the quoted message is, beyond its text: a picture quoted by its file
    // name says nothing, and the file name is all a media message has as a body.
    let mut msgtype = String::new();
    let mut media = None;

    let (sender, body) = match &in_reply_to.event {
        TimelineDetails::Ready(embedded) => {
            let body = match &embedded.content {
                TimelineItemContent::MsgLike(msg) => match &msg.kind {
                    MsgLikeKind::Message(message) => {
                        msgtype = message.msgtype().msgtype().to_owned();
                        media = media_info(message.msgtype());
                        strip_bidi(message.body())
                    }
                    MsgLikeKind::Redacted => String::new(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            let sender = match &embedded.sender_profile {
                TimelineDetails::Ready(profile) => profile
                    .display_name
                    .clone()
                    .unwrap_or_else(|| embedded.sender.as_str().to_owned()),
                _ => embedded.sender.as_str().to_owned(),
            };
            (sender, body)
        }
        // Details are fetched lazily; the quote fills in once they arrive.
        _ => (String::new(), String::new()),
    };

    Some(json!({
        "eventId": in_reply_to.event_id.as_str(),
        "sender": sender,
        "body": body,
        "state": state,
        "msgtype": msgtype,
        "media": media,
    }))
}

/// What was last known about a sender. `fetch_members` marks every profile
/// pending before it asks, so without this every name falls back to an address.
static KNOWN_SENDERS: Mutex<Option<HashMap<String, (String, Option<String>)>>> =
    Mutex::new(None);

/// The key a sender is remembered under: room and user, because a display
/// name is a per-room member event.
fn sender_key(room_id: &str, user: &str) -> String {
    format!("{room_id}\u{1}{user}")
}

/// Wipes the remembered names. A sign-out must not carry one account's
/// contacts into the next.
pub fn forget_senders() {
    if let Ok(mut guard) = KNOWN_SENDERS.lock() {
        *guard = None;
    }
}

fn remember_sender(room_id: &str, user: &str, name: Option<&str>, avatar: Option<&str>) {
    let Ok(mut guard) = KNOWN_SENDERS.lock() else {
        return;
    };
    let known = guard.get_or_insert_with(HashMap::new);
    let entry = known
        .entry(sender_key(room_id, user))
        .or_insert_with(|| (user.to_owned(), None));
    if let Some(name) = name {
        entry.0 = name.to_owned();
    }
    if avatar.is_some() {
        entry.1 = avatar.map(|url| url.to_owned());
    }
}

fn recall_sender(room_id: &str, user: &str) -> Option<(String, Option<String>)> {
    let guard = KNOWN_SENDERS.lock().ok()?;
    guard.as_ref()?.get(&sender_key(room_id, user)).cloned()
}

/// The authenticity marker for one event: `"red"`, `"grey"` or nothing, with
/// the SDK's own machine-readable reason.
fn shield_state(event: &EventTimelineItem) -> Option<Value> {
    use matrix_sdk_ui::timeline::{TimelineEventShieldState, TimelineEventShieldStateCode};

    let reason = |code: TimelineEventShieldStateCode| match code {
        TimelineEventShieldStateCode::AuthenticityNotGuaranteed => "authenticity",
        TimelineEventShieldStateCode::UnknownDevice => "unknownDevice",
        TimelineEventShieldStateCode::UnsignedDevice => "unsignedDevice",
        TimelineEventShieldStateCode::UnverifiedIdentity => "unverifiedIdentity",
        TimelineEventShieldStateCode::VerificationViolation => "identityChanged",
        TimelineEventShieldStateCode::MismatchedSender => "mismatchedSender",
        TimelineEventShieldStateCode::SentInClear => "sentInClear",
    };

    match event.get_shield(false) {
        TimelineEventShieldState::Red { code } => {
            Some(json!({ "level": "red", "reason": reason(code) }))
        }
        TimelineEventShieldState::Grey { code } => {
            Some(json!({ "level": "grey", "reason": reason(code) }))
        }
        TimelineEventShieldState::None => None,
    }
}

/// What a reaction may look like on screen: no bidi overrides, and short
/// enough for a chip.
fn reaction_key(key: &str) -> String {
    strip_bidi(key).chars().take(32).collect()
}

/// The sender's display name, falling back to the last one seen and only then
/// to the bare user ID.
fn sender_name(room_id: &str, item: &EventTimelineItem) -> String {
    let user = item.sender().as_str();
    match item.sender_profile() {
        TimelineDetails::Ready(profile) => {
            let name = profile.display_name.as_deref().map(strip_bidi);
            let avatar = profile.avatar_url.as_ref().map(|url| url.to_string());
            remember_sender(room_id, user, name.as_deref(), avatar.as_deref());
            name.unwrap_or_else(|| user.to_owned())
        }
        _ => recall_sender(room_id, user)
            .map(|(name, _)| name)
            .unwrap_or_else(|| user.to_owned()),
    }
}

/// Where the sender's picture lives. The same URL for every message of one
/// sender, so the UI fetches it once per person.
fn sender_avatar(room_id: &str, item: &EventTimelineItem) -> Option<String> {
    match item.sender_profile() {
        TimelineDetails::Ready(profile) => {
            profile.avatar_url.as_ref().map(|url| url.to_string())
        }
        // Same reason as in sender_name: pending is not "has none".
        _ => recall_sender(room_id, item.sender().as_str()).and_then(|(_, avatar)| avatar),
    }
}

/// One timeline item as the UI needs it. Items that are neither messages nor
/// dividers stay as kind "other": dropping them shifts the indices.
fn encode_item(room_id: &str, item: &TimelineItem, own: Option<&UserId>) -> Value {
    let id = item.unique_id().0.clone();

    if let Some(virtual_item) = item.as_virtual() {
        return match virtual_item {
            VirtualTimelineItem::DateDivider(timestamp) => json!({
                "id": id,
                "kind": "date",
                "timestamp": u64::from(timestamp.get()),
            }),
            VirtualTimelineItem::ReadMarker => json!({ "id": id, "kind": "marker" }),
            VirtualTimelineItem::TimelineStart => json!({ "id": id, "kind": "start" }),
        };
    }

    let Some(event) = item.as_event() else {
        return json!({ "id": id, "kind": "other" });
    };

    // An undecryptable event keeps its row - hiding it swallows part of the
    // conversation. Calls and membership changes become "system" rows.
    let (thread_root, thread_count) = match event.content() {
        TimelineItemContent::MsgLike(content) => (
            content
                .thread_root
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            content
                .thread_summary
                .as_ref()
                .map(|summary| summary.num_replies)
                .unwrap_or(0),
        ),
        _ => (String::new(), 0),
    };

    let (kind, body, msgtype, edited, media, reply, system, name) = match event.content() {
        TimelineItemContent::MsgLike(content) => match &content.kind {
            MsgLikeKind::Message(message) => (
                "message",
                strip_bidi(message.body()),
                message.msgtype().msgtype().to_owned(),
                message.is_edited(),
                media_info(message.msgtype()),
                reply_info(content),
                "",
                String::new(),
            ),
            MsgLikeKind::UnableToDecrypt(_) => {
                ("undecryptable", String::new(), String::new(), false, None, None, "", String::new())
            }
            MsgLikeKind::Redacted => {
                ("redacted", String::new(), String::new(), false, None, None, "", String::new())
            }
            // Stickers and polls are ordinary items but used to fall into "other", which
            // the UI draws at zero height - the same silent hole.
            MsgLikeKind::Sticker(sticker) => (
                "message",
                strip_bidi(&sticker.content().body),
                "m.sticker".to_owned(),
                false,
                None,
                reply_info(content),
                "",
                String::new(),
            ),
            MsgLikeKind::Poll(poll) => (
                "message",
                poll.fallback_text().as_deref().map(strip_bidi).unwrap_or_default(),
                "m.poll".to_owned(),
                false,
                None,
                reply_info(content),
                "",
                String::new(),
            ),
            _ => ("other", String::new(), String::new(), false, None, None, "", String::new()),
        },
        // Only m.call.invite / m.rtc.notification surface as timeline items;
        // answer, candidates and hangup never become items of their own.
        TimelineItemContent::CallInvite | TimelineItemContent::RtcNotification { .. } => {
            ("system", String::new(), String::new(), false, None, None, "call", String::new())
        }
        TimelineItemContent::MembershipChange(change) => {
            let token = match change.change() {
                Some(MembershipChange::Joined) | Some(MembershipChange::InvitationAccepted) => {
                    "member.joined"
                }
                Some(MembershipChange::Left) => "member.left",
                Some(MembershipChange::Invited) => "member.invited",
                Some(MembershipChange::Kicked) => "member.kicked",
                Some(MembershipChange::Banned) | Some(MembershipChange::KickedAndBanned) => {
                    "member.banned"
                }
                Some(MembershipChange::InvitationRejected)
                | Some(MembershipChange::InvitationRevoked) => "member.declined",
                Some(MembershipChange::Knocked) => "member.knocked",
                _ => "member",
            };
            (
                "system",
                String::new(),
                String::new(),
                false,
                None,
                None,
                token,
                strip_bidi(&change.display_name().unwrap_or_default()),
            )
        }
        TimelineItemContent::ProfileChange(_) => {
            ("system", String::new(), String::new(), false, None, None, "profile", String::new())
        }
        _ => ("other", String::new(), String::new(), false, None, None, "", String::new()),
    };

    // Reactions grouped as they are drawn: key, count, and whether we are in it.
    // Keys stay as they arrived - a variation selector makes a different key.
    let reactions = match event.content() {
        TimelineItemContent::MsgLike(content) => content
            .reactions
            .iter()
            .map(|(key, senders)| {
                json!({
                    // Filtered like every string a stranger wrote: a reaction is free text in a
                    // chip. The round trip uses the original from `reactionKeys`.
                    "key": reaction_key(key),
                    "sendKey": key,
                    "count": senders.len(),
                    "mine": own.map(|user| senders.contains_key(user)).unwrap_or(false),
                })
            })
            .collect::<Vec<Value>>(),
        _ => Vec::new(),
    };

    // The caption an attachment was sent with. `body` alone cannot say: without a
    // caption it holds the file name, which would show as a message.
    let caption = match event.content() {
        TimelineItemContent::MsgLike(content) => match &content.kind {
            MsgLikeKind::Message(message) => match message.msgtype() {
                // Through the same filter as a message body: a bidi override in a caption
                // reverses what the reader sees.
                MessageType::Image(inner) => inner.caption().map(strip_bidi),
                MessageType::Video(inner) => inner.caption().map(strip_bidi),
                MessageType::Audio(inner) => inner.caption().map(strip_bidi),
                MessageType::File(inner) => inner.caption().map(strip_bidi),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    };

    // Separate from the tuple above: it concerns exactly one of its arms, and
    // threading a ninth element through every other arm would say otherwise.
    let formatted = match event.content() {
        TimelineItemContent::MsgLike(content) => match &content.kind {
            MsgLikeKind::Message(message) => formatted_body(message.msgtype()),
            _ => None,
        },
        _ => None,
    };

    json!({
        "id": id,
        "eventId": event.event_id().map(|id| id.as_str()),
        // A message in the send queue has no event id. The transaction id is the only
        // handle by which it can be discarded.
        "txnId": match event.identifier() {
            TimelineEventItemId::TransactionId(id) => Some(id.to_string()),
            TimelineEventItemId::EventId(_) => None,
        },
        // The SDK calls a local echo editable, but every edit path here goes by event
        // id, and a queued message has none.
        "editable": event.is_editable() && event.event_id().is_some(),
        "kind": kind,
        "body": body,
        // The same message as `body`, as markup, where it adds something. Null
        // for most messages — see core/src/markup.rs.
        "formatted": formatted,
        "msgtype": msgtype,
        "media": media,
        "caption": caption,
        "replyTo": reply,
        "utdCause": utd_cause(event),
        // Whether the message is what it claims to be - the SDK's recommended
        // decoration. Without it, a message sent in the clear looks like any other.
        "shield": shield_state(event),
        "edited": edited,
        "system": system,
        "name": name,
        "sender": event.sender().as_str(),
        "senderName": sender_name(room_id, event),
        "senderAvatar": sender_avatar(room_id, event),
        "own": event.is_own(),
        "timestamp": u64::from(event.timestamp().get()),
        "pending": event.send_state().is_some(),
        "sendState": send_state(event),
        "sendError": send_error(event),
        "threadRoot": thread_root,
        "threadCount": thread_count,
        // How many other people have read up to here. Always 0 while receipt
        // tracking is off, which is the default — the map is then empty.
        "reactions": reactions,
        "readBy": event
            .read_receipts()
            .keys()
            .filter(|user| Some(user.as_ref()) != own)
            .count(),
        // Who read this far. One receipt per person in the whole timeline, so the
        // rows do not grow; names are fetched when the mark is tapped.
        "readByUsers": event
            .read_receipts()
            .keys()
            .filter(|user| Some(user.as_ref()) != own)
            .map(|user| user.to_string())
            .collect::<Vec<_>>(),
    })
}

/// How far a message of ours got: "", "sending", "failed". "failed" covers an
/// ordinary network gap too - without showing it, a lost message looks sent.
fn send_error(event: &EventTimelineItem) -> Option<Value> {
    match event.send_state() {
        Some(EventSendState::SendingFailed {
            error,
            is_recoverable,
        }) => Some(json!({
            "reason": scrub_ids(&error.to_string()),
            "recoverable": is_recoverable,
        })),
        _ => None,
    }
}

fn send_state(event: &EventTimelineItem) -> &'static str {
    match event.send_state() {
        None | Some(EventSendState::Sent { .. }) => "",
        Some(EventSendState::NotSentYet { .. }) => "sending",
        Some(EventSendState::SendingFailed { .. }) => "failed",
    }
}

fn encode_items<'a>(
    room_id: &str,
    items: impl IntoIterator<Item = &'a Arc<TimelineItem>>,
    own: Option<&UserId>,
) -> Vec<Value> {
    items
        .into_iter()
        .map(|item| encode_item(room_id, item.as_ref(), own))
        .collect()
}

fn encode(room_id: &str, diff: &VectorDiff<Arc<TimelineItem>>, own: Option<&UserId>) -> Value {
    crate::protocol::encode_diff(diff, |item| encode_item(room_id, item.as_ref(), own))
}

/// A stable stand-in for room and view, in front of every item id. Without it
/// the SDK numbers from zero per timeline and attachment caches collide.
fn id_prefix(room_id: &str, focus: &str) -> String {
    let mut hasher = DefaultHasher::new();
    room_id.hash(&mut hasher);
    focus.hash(&mut hasher);
    format!("{:x}/", hasher.finish())
}

/// Where this account stopped reading: the fully-read marker, which the line
/// follows, plus the receipt as fallback - a marker can name a row-less event.
pub async fn own_read_marker(client: &Client, room_id: &str) -> (Option<String>, Option<String>) {
    use matrix_sdk::ruma::events::fully_read::FullyReadEventContent;

    let Ok(parsed) = RoomId::parse(room_id) else {
        return (None, None);
    };
    let Some(room) = client.get_room(&parsed) else {
        return (None, None);
    };

    let mut marker = None;
    if let Ok(Some(raw)) = room.account_data_static::<FullyReadEventContent>().await {
        if let Ok(content) = raw.deserialize() {
            marker = Some(content.content.event_id.to_string());
        }
    }

    let mut receipt = None;
    if let Some(user) = client.user_id() {
        if let Ok(Some((event_id, _))) = room
            .load_user_receipt(StoredReceiptType::Read, ReceiptThread::Unthreaded, user)
            .await
        {
            receipt = Some(event_id.to_string());
        }
    }

    // Where only one of the two exists it is the marker, so a caller that
    // knows nothing of the distinction still gets the one usable answer.
    match (marker, receipt) {
        (None, second) => (second, None),
        (first, second) if first == second => (first, None),
        (first, second) => (first, second),
    }
}

/// Opens the timeline of `room_id` and streams updates. `focus`: empty for
/// live, `"pinned"`, or an event id for the permalink jump.
pub async fn open(
    client: &Client,
    room_id: &str,
    focus: &str,
    receipts: bool,
    token: &str,
    sink: Arc<Sink>,
) -> Result<TimelineHandle, String> {
    use matrix_sdk_ui::timeline::{TimelineEventFocusThreadMode, TimelineFocus};

    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;

    let prefix = id_prefix(room_id, focus);

    let live = focus.is_empty();
    let tracking = if receipts {
        // Message-like only: a receipt on a membership change would update a
        // row nobody reads a status off.
        TimelineReadReceiptTracking::MessageLikeEvents
    } else {
        TimelineReadReceiptTracking::Disabled
    };

    let timeline = if live {
        room.timeline_builder()
            // Thread replies stay visible in the room, duplication included: the SDK
            // counts no threaded event towards the badge, so hiding them hides mentions.
            .with_focus(TimelineFocus::Live {
                hide_threaded_events: false,
            })
            .track_read_marker_and_receipts(tracking)
            .with_internal_id_prefix(prefix)
            .build()
            .await
            .map_err(|error| format!("timeline unavailable: {error}"))?
    } else {
        let focus = if focus == "pinned" {
            TimelineFocus::PinnedEvents
        } else {
            let target = EventId::parse(focus)
                .map_err(|_| "not an event identifier".to_owned())?;
            TimelineFocus::Event {
                target,
                num_context_events: 20,
                thread_mode: TimelineEventFocusThreadMode::Automatic {
                    hide_threaded_events: false,
                },
            }
        };
        // A focused view - pinned messages, a permalink - shows a slice, not
        // the conversation; a read status has nothing to say in it.
        room.timeline_builder()
            .with_focus(focus)
            .with_internal_id_prefix(prefix)
            .build()
            .await
            .map_err(|error| format!("timeline unavailable: {error}"))?
    };
    let timeline = Arc::new(timeline);
    // Everything spawned here is registered and dies with the timeline. Declared
    // before the first spawn: a task above this line is one nobody can stop.
    let detail_tasks = TimelineTasks::new();

    // Owned: the stream task outlives this call, and every row it encodes
    // needs to know which receipt is ours so it is not counted as a reader.
    let own = client.user_id().map(|user| user.to_owned());

    let (initial, mut stream) = timeline.subscribe().await;

    // A reset, so the model starts from a known state - with the token, because
    // the diffs of the view that is going away are still in the queue.
    sink.emit(event(
        "timeline.diff",
        json!({
            "roomId": room_id,
            "token": token,
            "ops": [{ "op": "reset", "values": encode_items(room_id, &initial, own.as_deref()) }],
        }),
    ));

    // Sender profiles need the member list, which the SDK does not fetch by
    // itself. Only where it says the list is unsynced - asking blanks live names.
    if !room.are_members_synced() {
        let timeline = timeline.clone();
        detail_tasks.spawn_tracked(async move {
            timeline.fetch_members().await;
        })
        .await;
    }

    // The room's thread roots from the server: the SDK's summary is missing for
    // every root cached before threading. Failure is silent, and that is the old state.
    if live {
        let thread_room = room.clone();
        let thread_sink = sink.clone();
        let thread_room_id = room_id.to_owned();
        detail_tasks.spawn_tracked(async move {
            let roots = match thread_room
                .list_threads(matrix_sdk::room::ListThreadsOptions::default())
                .await
            {
                Ok(roots) => roots,
                Err(_) => return,
            };
            let entries: Vec<Value> = roots
                .chunk
                .iter()
                .filter_map(|event| {
                    let raw = event.raw().json();
                    let value: Value = serde_json::from_str(raw.get()).ok()?;
                    let id = value.get("event_id")?.as_str()?.to_owned();
                    // The reply count travels in the bundled aggregation. Where the server sends
                    // none, one is the truthful floor for an event it lists as a root.
                    let count = value
                        .pointer("/unsigned/m.relations/m.thread/count")
                        .and_then(|count| count.as_u64())
                        .unwrap_or(1);
                    Some(json!({ "eventId": id, "count": count }))
                })
                .collect();
            if entries.is_empty() {
                return;
            }
            thread_sink.emit(event(
                "timeline.threads",
                json!({ "roomId": thread_room_id, "roots": entries }),
            ));
        })
        .await;
    }

    // The room's pinned messages for banner and markers. Spawned: the newest
    // pin's body may have to come from the server.
    if live {
        let banner_room = room.clone();
        let banner_sink = sink.clone();
        let banner_room_id = room_id.to_owned();
        detail_tasks.spawn_tracked(async move {
            let ids = pinned_ids(&banner_room).await;
            let preview = pinned_preview(&banner_room, &ids).await;
            let (loaded, checked, error) = pinned_load_report(&banner_room, &ids).await;
            let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
            banner_sink.emit(event(
                "timeline.pinned",
                json!({
                    "roomId": banner_room_id,
                    "eventIds": ids,
                    "preview": preview,
                    "loaded": loaded,
                    // How many were looked at, which past the cap is not how many there are.
                    // Comparing against the full list reported a failure in every large room.
                    "checked": checked,
                    "loadError": error,
                }),
            ));
        })
        .await;
    }

    // Whether this room was replaced. From local state, so it costs nothing - and
    // always sent live, the negative included, to clear the previous room's banner.
    if live {
        sink.emit(event(
            "timeline.tombstone",
            json!({
                "roomId": room_id,
                "successor": successor(&room).await,
            }),
        ));
    }

    let room_id_owned = room_id.to_owned();
    let token_owned = token.to_owned();
    let task_tasks = detail_tasks.clone();
    let detail_source = timeline.clone();
    let initial_empty = initial.is_empty();
    let task = tokio::spawn(async move {
        // How often each reply target was asked for, so a row that is
        // rewritten several times does not send the same request again.
        let mut requested: HashMap<OwnedEventId, u8> = HashMap::new();

        // The initial batch arrived as a reset before this task started and needs the
        // same treatment: a reopened room's reply rows never pass the stream.
        request_reply_details(
            initial.iter().filter_map(|item| reply_needing_details(item)).collect(),
            &mut requested,
            &detail_source,
            &sink,
            &task_tasks,
        )
        .await;

        while let Some(diffs) = stream.next().await {
            let ops: Vec<Value> = diffs.iter().map(|diff| encode(&room_id_owned, diff, own.as_deref())).collect();
            sink.emit(event(
                "timeline.diff",
                json!({ "roomId": room_id_owned, "token": token_owned, "ops": ops }),
            ));

            // `Unavailable` means "not requested", and nothing fetches them by itself -
            // without this the quote box stayed empty for good.
            request_reply_details(
                diffs.iter().flat_map(replies_needing_details).collect(),
                &mut requested,
                &detail_source,
                &sink,
                &task_tasks,
            )
            .await;
        }
    });

    // A room never in the sync window arrives empty; one page here saves the
    // detour over the UI. Focused views load their own content.
    if live && initial_empty {
        let timeline = timeline.clone();
        detail_tasks.spawn_tracked(async move {
            let _ = timeline.paginate_backwards(PAGE_SIZE).await;
        })
        .await;
    }

    Ok(TimelineHandle {
        room_id: room_id.to_owned(),
        live,
        pinned: focus == "pinned",
        timeline,
        task,
        retried_from_end: AtomicBool::new(false),
        thread_root: String::new(),
        receipts,
        tasks: detail_tasks,
    })
}

/// Opens one thread (`TimelineFocus::Thread`) as `thread.diff`, next to the
/// room's live timeline. Sending on this handle threads automatically.
pub async fn open_thread(
    client: &Client,
    room_id: &str,
    root: &str,
    token: &str,
    sink: Arc<Sink>,
) -> Result<TimelineHandle, String> {
    use matrix_sdk_ui::timeline::TimelineFocus;

    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;
    let root_id = EventId::parse(root).map_err(|_| "not an event identifier".to_owned())?;

    let timeline = room
        .timeline_builder()
        .with_focus(TimelineFocus::Thread { root_event_id: root_id })
        .with_internal_id_prefix(id_prefix(room_id, root))
        .build()
        .await
        .map_err(|error| format!("thread unavailable: {error}"))?;
    let timeline = Arc::new(timeline);
    // Everything spawned here is registered and dies with the timeline. Declared
    // before the first spawn: a task above this line is one nobody can stop.
    let detail_tasks = TimelineTasks::new();

    // Owned: the stream task outlives this call, and every row it encodes
    // needs to know which receipt is ours so it is not counted as a reader.
    let own = client.user_id().map(|user| user.to_owned());

    let (initial, mut stream) = timeline.subscribe().await;

    sink.emit(event(
        "thread.diff",
        json!({
            "roomId": room_id,
            "root": root,
            "token": token,
            "ops": [{ "op": "reset", "values": encode_items(room_id, &initial, own.as_deref()) }],
        }),
    ));

    let room_id_owned = room_id.to_owned();
    let root_owned = root.to_owned();
    let token_owned = token.to_owned();
    let task_tasks = detail_tasks.clone();
    let detail_source = timeline.clone();
    let initial_empty = initial.is_empty();
    let error_sink = sink.clone();
    let task = tokio::spawn(async move {
        let mut requested: HashMap<OwnedEventId, u8> = HashMap::new();
        request_reply_details(
            initial.iter().filter_map(|item| reply_needing_details(item)).collect(),
            &mut requested,
            &detail_source,
            &sink,
            &task_tasks,
        )
        .await;
        while let Some(diffs) = stream.next().await {
            let ops: Vec<Value> = diffs.iter().map(|diff| encode(&room_id_owned, diff, own.as_deref())).collect();
            sink.emit(event(
                "thread.diff",
                json!({
                    "roomId": room_id_owned,
                    "root": root_owned,
                    "token": token_owned,
                    "ops": ops,
                }),
            ));
            request_reply_details(
                diffs.iter().flat_map(replies_needing_details).collect(),
                &mut requested,
                &detail_source,
                &sink,
                &task_tasks,
            )
            .await;
        }
    });

    // A thread not in the cache arrives empty; one page fills it. A failure has
    // to be reported or the view sits on "loading" for good.
    if initial_empty {
        let timeline = timeline.clone();
        let root_owned = root.to_owned();
        detail_tasks.spawn_tracked(async move {
            if let Err(error) = timeline.paginate_backwards(PAGE_SIZE).await {
                error_sink.emit(event(
                    "thread.error",
                    json!({
                        "root": root_owned,
                        "message": scrub_ids(&error.to_string()),
                    }),
                ));
            }
        })
        .await;
    }

    Ok(TimelineHandle {
        room_id: room_id.to_owned(),
        live: false,
        pinned: false,
        timeline,
        task,
        retried_from_end: AtomicBool::new(true),
        thread_root: root.to_owned(),
        receipts: false,
        tasks: detail_tasks,
    })
}


/// The room's pinned ids: the server's answer, with the room's own state as
/// fallback. Local state alone is empty on a store that was just created.
async fn pinned_ids(room: &matrix_sdk::Room) -> Vec<matrix_sdk::ruma::OwnedEventId> {
    match room.load_pinned_events().await {
        Ok(Some(ids)) => ids,
        // `Ok(None)` is "no pinned state", an error is a failed request. Neither may
        // claim there are none - local state is the better answer in both.
        _ => room.pinned_event_ids().unwrap_or_default(),
    }
}

/// Fetches every pinned event and reports how many could be read: counts and an
/// error class only, so an empty pinned view can be told from a broken one.
async fn pinned_load_report(
    room: &matrix_sdk::Room,
    ids: &[matrix_sdk::ruma::OwnedEventId],
) -> (usize, usize, String) {
    let mut loaded = 0;
    let mut checked = 0;
    let mut first_error = String::new();
    // Bounded, and out of the cache: `Room::event` always asks the server, so a
    // room with twenty pins made twenty requests on every entry.
    for id in ids.iter().take(PINNED_REPORT_LIMIT) {
        checked += 1;
        match room.load_or_fetch_event(id, None).await {
            Ok(_) => loaded += 1,
            Err(error) if first_error.is_empty() => {
                first_error = scrub_ids(&error.to_string());
            }
            Err(_) => {}
        }
    }
    (loaded, checked, first_error)
}

/// The newest pinned body for the banner, fetched and decrypted where possible.
/// Empty where there is nothing usable; the UI falls back to a count.
async fn pinned_preview(
    room: &matrix_sdk::Room,
    ids: &[matrix_sdk::ruma::OwnedEventId],
) -> String {
    let Some(last) = ids.last() else {
        return String::new();
    };
    // Cache first, for the same reason the report above uses it: the banner is
    // rebuilt on every opening of the room.
    let Ok(fetched) = room.load_or_fetch_event(last, None).await else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(fetched.raw().json().get()) else {
        return String::new();
    };
    let body = value
        .get("content")
        .and_then(|content| content.get("body"))
        .and_then(|body| body.as_str())
        .unwrap_or_default();

    // The banner is one line: whitespace collapses and the rest is cut, or a
    // pinned changelog paints itself across the page.
    let mut preview = String::new();
    for word in body.split_whitespace() {
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(word);
        if preview.chars().count() >= PINNED_PREVIEW_CHARS {
            break;
        }
    }
    match preview.char_indices().nth(PINNED_PREVIEW_CHARS) {
        Some((cut, _)) => {
            preview.truncate(cut);
            preview.push('…');
            preview
        }
        None => preview,
    }
}

/// How much of a pinned message the one-line banner can ever show.
const PINNED_PREVIEW_CHARS: usize = 120;

/// Opens a direct chat with another user, reusing an existing one if there is
/// already a private room with exactly that person.
pub async fn direct_chat(client: &Client, user_id: &str) -> Result<String, String> {
    let user = UserId::parse(user_id.trim())
        .map_err(|_| "not a user identifier — expected @name:server".to_owned())?;

    // A second direct room with the same person would split the conversation,
    // so an existing one wins.
    if let Some(room) = client.get_dm_room(&user) {
        return Ok(room.room_id().as_str().to_owned());
    }

    let mut request = create_room::v3::Request::new();
    request.is_direct = true;
    request.invite = vec![user.clone()];
    request.preset = Some(create_room::v3::RoomPreset::TrustedPrivateChat);
    // Encrypted from the first message: no preset does it, and enabling it later
    // cannot protect what was already sent.
    request.initial_state = vec![InitialStateEvent::with_empty_state_key(
        RoomEncryptionEventContent::with_recommended_defaults(),
    )
    .to_raw_any()];

    let room = client
        .create_room(request)
        .await
        .map_err(|error| format!("could not start the chat: {error}"))?;

    Ok(room.room_id().as_str().to_owned())
}

/// Throws away the outbound group session so the next message re-shares its
/// key. Received keys stay; this fixes the next message, not the last one.
pub async fn discard_room_key(client: &Client, room_id: &str) -> Result<(), String> {
    let parsed = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;
    room.discard_room_key()
        .await
        .map_err(|error| format!("could not reset the room key: {}", scrub_ids(&error.to_string())))
}

/// Turns on end-to-end encryption in a room. Irreversible by design of the
/// protocol, so the front end asks the user before sending this.
pub async fn enable_encryption(client: &Client, room_id: &str) -> Result<(), String> {
    let parsed = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;
    room.enable_encryption()
        .await
        .map_err(|error| format!("could not enable encryption: {error}"))
}

/// Invites a user into a room. Nothing changes on this side until the other
/// one accepts; the member list shows them as invited in the meantime.
pub async fn invite_user(client: &Client, room_id: &str, user_id: &str) -> Result<(), String> {
    let parsed = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| "not a room identifier".to_owned())?;
    let user = UserId::parse(user_id.trim())
        .map_err(|_| "not a user identifier — expected @name:server".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;
    room.invite_user_by_id(&user)
        .await
        .map_err(|error| format!("could not invite: {error}"))
}

/// Joins by address or id. The alias' server travels as a routing hint, which
/// is what lets a join succeed for a room this homeserver never saw.
pub async fn join_by_alias(client: &Client, alias: &str) -> Result<String, String> {
    let trimmed = alias.trim();
    let parsed = RoomOrAliasId::parse(trimmed)
        .map_err(|_| "not a room address — expected #room:server".to_owned())?;

    let servers: Vec<OwnedServerName> = match trimmed.rsplit_once(':') {
        Some((_, server)) => ServerName::parse(server).map(|s| vec![s]).unwrap_or_default(),
        None => Vec::new(),
    };

    let room = client
        .join_room_by_id_or_alias(&parsed, &servers)
        .await
        .map_err(|error| format!("could not join: {error}"))?;

    Ok(room.room_id().as_str().to_owned())
}

/// Accepts an invitation, or joins a room that is already known.
pub async fn join(client: &Client, room_id: &str) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;

    room.join()
        .await
        .map_err(|error| format!("could not join: {error}"))
}

/// Everything the room-info page shows, all from local state - no request, so
/// the page fills at once. Counts are the sync summary, not a member list.
pub async fn room_info(client: &Client, room_id: &str) -> Result<Value, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;

    let predecessor = room.predecessor_room().map(|predecessor| {
        let joined = matches!(
            client_room_state(client.clone(), &predecessor.room_id),
            Some(matrix_sdk::RoomState::Joined)
        );
        json!({ "roomId": predecessor.room_id.as_str(), "joined": joined })
    });

    Ok(json!({
        "roomId": room_id,
        "name": strip_bidi(
            &room.cached_display_name().map(|name| name.to_string()).unwrap_or_default(),
        ),
        "topic": strip_bidi(&room.topic().unwrap_or_default()),
        "avatar": room.avatar_url().map(|url| url.to_string()),
        "alias": room.canonical_alias().map(|alias| alias.to_string()).unwrap_or_default(),
        "altAliases": room
            .alt_aliases()
            .iter()
            .map(|alias| alias.to_string())
            .collect::<Vec<_>>(),
        "joinedMembers": room.joined_members_count(),
        "invitedMembers": room.invited_members_count(),
        "encrypted": room.encryption_state().is_encrypted(),
        // `None` means the join rule says nothing; the UI then keeps quiet. Not named
        // "public": that is reserved in QML and the page would not parse.
        "isPublic": room.is_public(),
        "direct": room.is_direct().await.unwrap_or(false),
        "space": room.is_space(),
        "version": room
            .clone_info()
            .room_version()
            .map(|version| version.to_string())
            .unwrap_or_default(),
        "favourite": room.is_favourite(),
        "lowPriority": room.is_low_priority(),
        "muted": matches!(
            room.cached_user_defined_notification_mode(),
            Some(matrix_sdk::notification_settings::RoomNotificationMode::Mute)
        ),
        "notifyMode": crate::roomlist::notify_mode_string(
            room.cached_user_defined_notification_mode()
        ),
        "successor": successor(&room).await,
        "predecessor": predecessor,
    }))
}

/// What a tombstoned room says about its replacement. Such a room keeps its
/// history but takes no messages, and nothing else says why.
async fn successor(room: &matrix_sdk::Room) -> Option<Value> {
    let successor = room.successor_room()?;
    let joined = matches!(
        client_room_state(room.client(), &successor.room_id),
        Some(matrix_sdk::RoomState::Joined)
    );
    Some(json!({
        "roomId": successor.room_id.as_str(),
        "reason": successor.reason.unwrap_or_default(),
        "joined": joined,
    }))
}

/// The membership of a room the client may or may not know at all.
fn client_room_state(
    client: Client,
    room_id: &matrix_sdk::ruma::RoomId,
) -> Option<matrix_sdk::RoomState> {
    client.get_room(room_id).map(|room| room.state())
}

/// Joins the room that replaced `room_id`. Routing hints come from the
/// tombstone's sender: from room version 12 a room id carries no server part.
pub async fn follow_successor(client: &Client, room_id: &str) -> Result<String, String> {
    use matrix_sdk::ruma::events::room::tombstone::RoomTombstoneEventContent;
    use matrix_sdk::ruma::OwnedUserId;

    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;
    let replacement = room
        .successor_room()
        .ok_or_else(|| "this room has no successor".to_owned())?
        .room_id;

    // Already a member — the upgrade was followed before, or on another
    // device. Nothing to join, only somewhere to go.
    if matches!(
        client_room_state(client.clone(), &replacement),
        Some(matrix_sdk::RoomState::Joined)
    ) {
        return Ok(replacement.as_str().to_owned());
    }

    let mut servers: Vec<OwnedServerName> = Vec::new();
    let mut add = |server: Option<OwnedServerName>| {
        if let Some(server) = server {
            if !servers.contains(&server) {
                servers.push(server);
            }
        }
    };
    add(replacement.server_name().map(|name| name.to_owned()));
    let sender = room
        .get_state_event_static::<RoomTombstoneEventContent>()
        .await
        .ok()
        .flatten()
        .and_then(|raw| match raw {
            matrix_sdk::deserialized_responses::RawSyncOrStrippedState::Sync(raw) => {
                raw.get_field::<OwnedUserId>("sender").ok().flatten()
            }
            matrix_sdk::deserialized_responses::RawSyncOrStrippedState::Stripped(raw) => {
                raw.get_field::<OwnedUserId>("sender").ok().flatten()
            }
        });
    add(sender.map(|user| user.server_name().to_owned()));
    add(parsed.server_name().map(|name| name.to_owned()));

    let target = RoomOrAliasId::parse(replacement.as_str())
        .map_err(|_| "the successor is not a room identifier".to_owned())?;
    let joined = client
        .join_room_by_id_or_alias(&target, &servers)
        .await
        .map_err(|error| format!("could not join the new room: {error}"))?;

    Ok(joined.room_id().as_str().to_owned())
}

/// One place decides whether a message ships a formatted copy: the markers in
/// the text. The plain body keeps them, which is what a client without HTML
/// shows and what an edit reads back.
fn text_content(body: String) -> RoomMessageEventContent {
    match to_formatted_body(&body) {
        Some(html) => RoomMessageEventContent::text_html(body, html),
        None => RoomMessageEventContent::text_plain(body),
    }
}
