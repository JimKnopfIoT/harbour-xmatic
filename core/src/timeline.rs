//! The timeline of a single room.
//!
//! Like the room list, the SDK hands out `VectorDiff`s over timeline items, so
//! the Qt model applies them rather than assembling a view of its own. Edits,
//! reactions and local echoes all arrive as updates to existing items — that
//! bookkeeping lives upstream, not here.
//!
//! Encrypted rooms need no special handling at this level: the crypto machine
//! sits below the timeline and hands decrypted content out the same way.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::{
    room::edit::EditedContent,
    ruma::{
        api::client::{receipt::create_receipt::v3::ReceiptType, room::create_room},
        events::{
            room::{
                encryption::RoomEncryptionEventContent,
                message::{MessageType, RoomMessageEventContent},
            },
            InitialStateEvent,
        },
        EventId, OwnedEventId, OwnedServerName, RoomId, RoomOrAliasId, ServerName, UserId,
    },
    Client,
};
use matrix_sdk_base::crypto::types::events::UtdCause;
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    timeline::{
        EncryptedMessage, EventSendState, EventTimelineItem, MembershipChange, MsgLikeKind,
        RoomExt, TimelineDetails, TimelineEventItemId, TimelineItem, TimelineItemContent,
        VirtualTimelineItem,
    },
};
use serde_json::{json, Value};

use crate::protocol::event;
use crate::runtime::Sink;

/// How many events one backwards pagination asks for.
const PAGE_SIZE: u16 = 30;

/// Below this many items a timeline claiming to be complete is not believed —
/// see `paginate`. One page is the natural threshold: fewer events than a
/// single request would have returned, yet "that is all there is".
const SUSPICIOUSLY_SHORT: usize = PAGE_SIZE as usize;

/// An open timeline and the task streaming its updates.
pub struct TimelineHandle {
    room_id: String,
    live: bool,
    /// The pinned-messages view, which cannot be paginated at all.
    pinned: bool,
    timeline: Arc<matrix_sdk_ui::timeline::Timeline>,
    task: tokio::task::JoinHandle<()>,
    /// Whether the one permitted cache reset (see `paginate`) was already used.
    /// Without this the room would be cleared on every single pagination that
    /// legitimately reports the start of a short room.
    retried_from_end: AtomicBool,
    /// The thread this view is focused on, or empty for a room view.
    thread_root: String,
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

    /// The room's pinned event ids and the newest pin's body, fresh from the
    /// server when possible so a pin made a moment ago is already in the
    /// answer.
    pub async fn pinned_info(&self) -> (Vec<String>, String) {
        let room = self.timeline.room();
        let ids = match room.load_pinned_events().await {
            Ok(Some(ids)) => ids,
            _ => room.pinned_event_ids().unwrap_or_default(),
        };
        let preview = pinned_preview(room, &ids).await;
        (ids.iter().map(|id| id.to_string()).collect(), preview)
    }

    /// Loads older events. Returns whether the start of the room was reached.
    ///
    /// It deliberately does not report how many rows this added, although the
    /// UI would like to know: `paginate_backwards` hands the new items to the
    /// subscriber stream rather than having them in place when it returns — for
    /// a lazy pagination it only lowers a skip count and returns at once
    /// (`matrix-sdk-ui`, `timeline/pagination.rs`). Counting items around the
    /// call therefore reports zero for a page that did arrive, a moment later,
    /// through the diff stream. The growth is judged where the diffs have
    /// landed, which is the model on the Qt side.
    pub async fn paginate(&self) -> Result<bool, String> {
        // The pinned view holds exactly the pinned events and has no history
        // behind it; the SDK answers any pagination on it with
        // `PaginationError::NotSupported` unconditionally
        // (`matrix-sdk-ui`, `timeline/pagination.rs`). Passed on, that became a
        // user-facing error banner every time the pinned page was opened with
        // too few pins to fill the screen — an error where the honest answer is
        // simply "that is all of them".
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

        // The SDK reports "start of the timeline" as soon as its event cache
        // holds any event but knows neither a gap nor an older chunk — it never
        // asks the server. A room joined while the app was running lands in
        // exactly that state: sliding sync delivered the join event without a
        // `prev_batch` token, so there is no gap, and three rows are declared
        // the whole room. An empty cache, by contrast, makes the SDK paginate
        // from the end of the room, which is what actually fetches history.
        //
        // So for a timeline far too short to be believable, the room's cache is
        // dropped once and the pagination retried. That restores the only state
        // in which the SDK asks the server. Discarding the cache costs nothing
        // here: what is being thrown away is the handful of events that just
        // proved insufficient.
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

    /// Sends a plain text message. It shows up as a local echo immediately.
    pub async fn send_text(&self, body: String) -> Result<(), String> {
        self.timeline
            .send(RoomMessageEventContent::text_plain(body).into())
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
            .send_reply(RoomMessageEventContent::text_plain(body).into(), id)
            .await
            .map_err(|error| format!("could not reply: {error}"))
    }

    /// Replaces the body of a message that was already sent.
    pub async fn edit(&self, event_id: &str, body: String) -> Result<(), String> {
        let id = parse_event_id(event_id)?;
        self.timeline
            .edit(
                &id,
                EditedContent::RoomMessage(RoomMessageEventContent::text_plain(body).into()),
            )
            .await
            .map_err(|error| format!("could not edit: {error}"))
    }

    /// Redacts a message. The event stays in the timeline as a deleted one.
    pub async fn redact(&self, event_id: &str) -> Result<(), String> {
        let id = parse_event_id(event_id)?;
        self.timeline
            .redact(&id, None)
            .await
            .map_err(|error| format!("could not delete: {error}"))
    }

    /// Marks the room as read up to the latest event.
    pub async fn mark_read(&self) -> Result<(), String> {
        self.timeline
            .mark_as_read(ReceiptType::Read)
            .await
            .map(|_| ())
            .map_err(|error| format!("read receipt could not be sent: {error}"))
    }

    /// Takes `&self` rather than consuming the handle: the runtime keeps it
    /// behind an `Arc` so a command can work on it without holding the state
    /// lock, and an `Arc` cannot hand out ownership.
    pub fn close(&self) {
        self.task.abort();
    }
}

fn parse_event_id(event_id: &str) -> Result<TimelineEventItemId, String> {
    EventId::parse(event_id)
        .map(TimelineEventItemId::EventId)
        .map_err(|_| "not an event identifier".to_owned())
}

/// Describes an attachment so the front end can show and fetch it.
///
/// The media source travels as opaque JSON: in encrypted rooms it carries the
/// decryption keys, so a bare MXC URI would not be enough.
fn media_info(message_type: &MessageType) -> Option<Value> {
    // In encrypted rooms the server cannot generate thumbnails — it only ever
    // sees ciphertext — so the sender ships one as a separate media source.
    // Using it is the only way to avoid downloading full-size images.
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

    let (source, mimetype, size, filename) = match message_type {
        MessageType::Image(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.body.clone(),
        ),
        MessageType::Video(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.body.clone(),
        ),
        MessageType::Audio(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.body.clone(),
        ),
        MessageType::File(content) => (
            &content.source,
            content.info.as_ref().and_then(|i| i.mimetype.clone()),
            content.info.as_ref().and_then(|i| i.size),
            content.body.clone(),
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
        "filename": filename,
        "width": width.map(u64::from),
        "height": height.map(u64::from),
    }))
}

/// The event id of `item` if it is a reply whose quoted message still has to
/// be asked for: `Unavailable` was never requested, `Error` was requested and
/// failed — a failure is worth another try, not a final answer.
fn reply_needing_details(item: &TimelineItem) -> Option<OwnedEventId> {
    let event = item.as_event()?;
    let TimelineItemContent::MsgLike(content) = event.content() else {
        return None;
    };
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

/// How often one reply's details are asked for over a timeline's lifetime.
///
/// A failed fetch comes back through the stream as a `Set` diff carrying
/// `Error`, which qualifies for another request — unbounded, that is a
/// request loop against a homeserver that answers 429, so the second attempt
/// is also the last.
const DETAIL_ATTEMPTS: u8 = 2;

/// Spawns a detail fetch for each reply target that has attempts left.
/// Failures go out as `timeline.detailError` (truncated id, scrubbed error)
/// so they land in the journal instead of vanishing.
fn request_reply_details(
    ids: impl IntoIterator<Item = OwnedEventId>,
    requested: &mut HashMap<OwnedEventId, u8>,
    timeline: &Arc<matrix_sdk_ui::timeline::Timeline>,
    sink: &Arc<Sink>,
) {
    for id in ids {
        let attempts = requested.entry(id.clone()).or_insert(0);
        if *attempts >= DETAIL_ATTEMPTS {
            continue;
        }
        *attempts += 1;
        let source = timeline.clone();
        let sink = sink.clone();
        tokio::spawn(async move {
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
                // A class, not the message: the SDK's own error carries the
                // full id in its `Debug` output (`EventId("$…")`), and the
                // most common case here is exactly that variant.
                let class = match error {
                    matrix_sdk_ui::timeline::Error::EventNotInTimeline(_) => "not-in-timeline",
                    _ => "fetch-failed",
                };
                sink.emit(event(
                    "timeline.detailError",
                    json!({ "eventId": short, "error": class }),
                ));
            }
        });
    }
}

/// Why an event could not be decrypted, as a stable key the front end turns
/// into a sentence.
///
/// The SDK works this out and hands it over; throwing it away is what turns
/// every report of "he cannot read me" into an afternoon of measuring. The
/// distinction that matters most in practice is between a key that never
/// arrived (the sender withheld it, or could not deliver it) and a message
/// that is simply older than this device — the first is the sender's setting,
/// the second is nobody's fault and cannot be repaired from here.
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

/// Describes the message a reply refers to, as far as it is already known.
/// `state`: "ready", "error" (fetch failed — not there or not permitted),
/// or "loading".
fn reply_info(content: &matrix_sdk_ui::timeline::MsgLikeContent) -> Option<Value> {
    let in_reply_to = content.in_reply_to.as_ref()?;

    let state = match &in_reply_to.event {
        TimelineDetails::Ready(_) => "ready",
        TimelineDetails::Error(_) => "error",
        _ => "loading",
    };

    let (sender, body) = match &in_reply_to.event {
        TimelineDetails::Ready(embedded) => {
            let body = match &embedded.content {
                TimelineItemContent::MsgLike(msg) => match &msg.kind {
                    MsgLikeKind::Message(message) => message.body().to_owned(),
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
    }))
}

/// The sender's display name, falling back to the bare user ID while the
/// profile is still being resolved.
fn sender_name(item: &EventTimelineItem) -> String {
    match item.sender_profile() {
        TimelineDetails::Ready(profile) => profile
            .display_name
            .clone()
            .unwrap_or_else(|| item.sender().as_str().to_owned()),
        _ => item.sender().as_str().to_owned(),
    }
}

/// Where the sender's picture lives, if the profile is known.
///
/// This is the same URL for every message of one sender, which is what lets
/// the UI fetch it once per person instead of once per message.
fn sender_avatar(item: &EventTimelineItem) -> Option<String> {
    match item.sender_profile() {
        TimelineDetails::Ready(profile) => {
            profile.avatar_url.as_ref().map(|url| url.to_string())
        }
        _ => None,
    }
}

/// One timeline item as the UI needs it.
///
/// Items that are neither messages nor date dividers still appear, with kind
/// "other" — dropping them here would put the model's indices out of step with
/// the diffs the SDK sends.
fn encode_item(item: &TimelineItem) -> Value {
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

    // An event that could not be decrypted still gets a row of its own. It
    // means a key is missing — usually because the message predates this
    // device — and hiding it would silently swallow part of the conversation.
    //
    // Calls and membership changes are not messages, but a room that consists
    // only of them (a call test room, say) must not look empty: they become a
    // "system" row. The `system` token is machine-readable so the QML side can
    // localise it; `name` carries the affected member for membership changes.
    // Thread membership and summary, for the marker row and the inline tag.
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
                message.body().to_owned(),
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
            // Stickers and polls are ordinary timeline items — the SDK's
            // default filter admits both — but they used to fall into "other",
            // which the UI draws with a height of zero. They were therefore
            // invisible, the same silent hole in the conversation that the
            // handling of undecryptable and redacted events exists to avoid.
            // Their own message type is passed on; the UI does not draw either
            // specially yet and falls back to text, so at least the sticker's
            // name and the poll's question are there.
            MsgLikeKind::Sticker(sticker) => (
                "message",
                sticker.content().body.clone(),
                "m.sticker".to_owned(),
                false,
                None,
                reply_info(content),
                "",
                String::new(),
            ),
            MsgLikeKind::Poll(poll) => (
                "message",
                poll.fallback_text().unwrap_or_default(),
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
                change.display_name().unwrap_or_default(),
            )
        }
        TimelineItemContent::ProfileChange(_) => {
            ("system", String::new(), String::new(), false, None, None, "profile", String::new())
        }
        _ => ("other", String::new(), String::new(), false, None, None, "", String::new()),
    };

    json!({
        "id": id,
        "eventId": event.event_id().map(|id| id.as_str()),
        "editable": event.is_editable(),
        "kind": kind,
        "body": body,
        "msgtype": msgtype,
        "media": media,
        "replyTo": reply,
        "utdCause": utd_cause(event),
        "edited": edited,
        "system": system,
        "name": name,
        "sender": event.sender().as_str(),
        "senderName": sender_name(event),
        "senderAvatar": sender_avatar(event),
        "own": event.is_own(),
        "timestamp": u64::from(event.timestamp().get()),
        "pending": event.send_state().is_some(),
        "sendState": send_state(event),
        "threadRoot": thread_root,
        "threadCount": thread_count,
    })
}

/// How far a message of our own has got: "" once the server has it, "sending"
/// while it waits, "failed" when the attempt did not work.
///
/// "failed" is not the exception it sounds like. The SDK reports it for an
/// ordinary network gap as well — a recoverable failure sets exactly this state
/// (`timeline/controller/mod.rs`, `RoomSendQueueUpdate::SendError`) — and the
/// message stays in the queue waiting for another try. Without showing it, a
/// message that never left looks exactly like one that arrived, which is how a
/// tester came to send the same text twice.
fn send_state(event: &EventTimelineItem) -> &'static str {
    match event.send_state() {
        None | Some(EventSendState::Sent { .. }) => "",
        Some(EventSendState::NotSentYet { .. }) => "sending",
        Some(EventSendState::SendingFailed { .. }) => "failed",
    }
}

fn encode_items<'a>(items: impl IntoIterator<Item = &'a Arc<TimelineItem>>) -> Vec<Value> {
    items
        .into_iter()
        .map(|item| encode_item(item.as_ref()))
        .collect()
}

fn encode(diff: &VectorDiff<Arc<TimelineItem>>) -> Value {
    match diff {
        VectorDiff::Append { values } => json!({ "op": "append", "values": encode_items(values) }),
        VectorDiff::Clear => json!({ "op": "clear" }),
        VectorDiff::PushFront { value } => {
            json!({ "op": "insert", "index": 0, "value": encode_item(value) })
        }
        VectorDiff::PushBack { value } => {
            json!({ "op": "append", "values": [encode_item(value)] })
        }
        VectorDiff::PopFront => json!({ "op": "remove", "index": 0 }),
        VectorDiff::PopBack => json!({ "op": "popBack" }),
        VectorDiff::Insert { index, value } => {
            json!({ "op": "insert", "index": index, "value": encode_item(value) })
        }
        VectorDiff::Set { index, value } => {
            json!({ "op": "set", "index": index, "value": encode_item(value) })
        }
        VectorDiff::Remove { index } => json!({ "op": "remove", "index": index }),
        VectorDiff::Truncate { length } => json!({ "op": "truncate", "length": length }),
        VectorDiff::Reset { values } => json!({ "op": "reset", "values": encode_items(values) }),
    }
}

/// A short, stable stand-in for the room and the view an item belongs to, put
/// in front of every item id of that timeline.
///
/// Without it the SDK numbers items from zero for every timeline it builds —
/// `internal_id_prefix` defaults to `None` and the id is `"{prefix}{counter}"`
/// (`matrix-sdk-ui`, `timeline/builder.rs` and `controller/metadata.rs`) — so
/// the first picture of one room and the first picture of the next both get the
/// id "0". The front end keys its cache of downloaded attachments on that id,
/// which made a picture from the room just left show up in the room just
/// opened, without so much as a request going out. The pinned view of a room
/// collides with its live view the same way, hence `focus` in the hash.
///
/// Hashed rather than spelled out, so no full room identifier travels in a
/// field that might one day be logged.
fn id_prefix(room_id: &str, focus: &str) -> String {
    let mut hasher = DefaultHasher::new();
    room_id.hash(&mut hasher);
    focus.hash(&mut hasher);
    format!("{:x}/", hasher.finish())
}

/// Opens the timeline of `room_id` and starts streaming updates.
///
/// `focus` selects the view: empty for the live timeline, `"pinned"` for the
/// room's pinned messages, or an event id to show the history around one event
/// — the permalink jump.
pub async fn open(
    client: &Client,
    room_id: &str,
    focus: &str,
    sink: Arc<Sink>,
) -> Result<TimelineHandle, String> {
    use matrix_sdk_ui::timeline::{TimelineEventFocusThreadMode, TimelineFocus};

    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;

    let prefix = id_prefix(room_id, focus);

    let live = focus.is_empty();
    let timeline = if live {
        room.timeline_builder()
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
        room.timeline_builder()
            .with_focus(focus)
            .with_internal_id_prefix(prefix)
            .build()
            .await
            .map_err(|error| format!("timeline unavailable: {error}"))?
    };
    let timeline = Arc::new(timeline);

    let (initial, mut stream) = timeline.subscribe().await;

    // The first batch goes out as a reset so the model starts from a known
    // state instead of replaying the room's history as insertions.
    sink.emit(event(
        "timeline.diff",
        json!({
            "roomId": room_id,
            "ops": [{ "op": "reset", "values": encode_items(&initial) }],
        }),
    ));

    // Sender profiles — the display name and the picture — are only filled in
    // once the room's member list is known, and the SDK does not fetch it on
    // its own: "If the full member list is not known, sender profiles are
    // currently likely not going to be available" (Timeline::fetch_members).
    // Without this some senders have a picture and others do not, depending on
    // what a sync happened to carry along.
    //
    // Spawned, not awaited: the conversation appears at once and the profiles
    // arrive through the diff stream that is already running.
    {
        let timeline = timeline.clone();
        tokio::spawn(async move {
            timeline.fetch_members().await;
        });
    }

    // The room's pinned messages, so the UI can show a banner and mark the
    // pinned rows. Spawned: the newest pin's body may have to be fetched from
    // the server, and opening the room must not wait for that.
    if live {
        let banner_room = room.clone();
        let banner_sink = sink.clone();
        let banner_room_id = room_id.to_owned();
        tokio::spawn(async move {
            let ids = banner_room.pinned_event_ids().unwrap_or_default();
            let preview = pinned_preview(&banner_room, &ids).await;
            let (loaded, error) = pinned_load_report(&banner_room, &ids).await;
            let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
            banner_sink.emit(event(
                "timeline.pinned",
                json!({
                    "roomId": banner_room_id,
                    "eventIds": ids,
                    "preview": preview,
                    "loaded": loaded,
                    "loadError": error,
                }),
            ));
        });
    }

    // Whether this room was replaced by a newer one. Read from the room's own
    // state, so it costs nothing and is known before the first message renders.
    // Always sent for a live view, the negative answer included: it is what
    // clears the banner of the room visited before this one.
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
    let detail_source = timeline.clone();
    let initial_empty = initial.is_empty();
    let task = tokio::spawn(async move {
        // How often each reply target was asked for, so a row that is
        // rewritten several times does not send the same request again.
        let mut requested: HashMap<OwnedEventId, u8> = HashMap::new();

        // The initial batch went out as a reset before this task started, and
        // it needs the same treatment as the diffs below: a room reopened
        // during the session arrives entirely out of the event cache, so its
        // reply rows never pass through the stream — their quotes stayed
        // empty for good.
        request_reply_details(
            initial.iter().filter_map(|item| reply_needing_details(item)),
            &mut requested,
            &detail_source,
            &sink,
        );

        while let Some(diffs) = stream.next().await {
            let ops: Vec<Value> = diffs.iter().map(encode).collect();
            sink.emit(event(
                "timeline.diff",
                json!({ "roomId": room_id_owned, "ops": ops }),
            ));

            // A reply whose quoted message is not in the loaded window comes
            // with its details `Unavailable`, and the SDK means that literally:
            // "not available yet, and have not been requested". Nothing fetches
            // them by itself. Without this the quote box stayed on screen with
            // both its lines empty — for good, not "until it arrives", which is
            // what the comment on `reply_info` used to promise.
            request_reply_details(
                diffs.iter().flat_map(replies_needing_details),
                &mut requested,
                &detail_source,
                &sink,
            );
        }
    });

    // A room that was never in the sliding-sync window arrives with an empty
    // timeline; fetching the first page here saves it the detour over the UI.
    // That side keeps asking as long as the list is too short to scroll — a
    // freshly joined room does not arrive empty, it arrives holding its own
    // join event, which this check would miss. Spawned, not awaited — the
    // events come back through the stream task above as ordinary diffs.
    // Focused views load their own content and do not paginate this way.
    if live && initial_empty {
        let timeline = timeline.clone();
        tokio::spawn(async move {
            let _ = timeline.paginate_backwards(PAGE_SIZE).await;
        });
    }

    Ok(TimelineHandle {
        room_id: room_id.to_owned(),
        live,
        pinned: focus == "pinned",
        timeline,
        task,
        retried_from_end: AtomicBool::new(false),
        thread_root: String::new(),
    })
}

/// Opens the timeline of one thread (`TimelineFocus::Thread`) and streams its
/// updates as `thread.diff`. Runs next to the room's live timeline, not
/// instead of it. Sending on this handle threads automatically — the SDK
/// derives the relation from the focus.
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

    let (initial, mut stream) = timeline.subscribe().await;

    sink.emit(event(
        "thread.diff",
        json!({
            "roomId": room_id,
            "root": root,
            "token": token,
            "ops": [{ "op": "reset", "values": encode_items(&initial) }],
        }),
    ));

    let room_id_owned = room_id.to_owned();
    let root_owned = root.to_owned();
    let token_owned = token.to_owned();
    let detail_source = timeline.clone();
    let initial_empty = initial.is_empty();
    let error_sink = sink.clone();
    let task = tokio::spawn(async move {
        let mut requested: HashMap<OwnedEventId, u8> = HashMap::new();
        request_reply_details(
            initial.iter().filter_map(|item| reply_needing_details(item)),
            &mut requested,
            &detail_source,
            &sink,
        );
        while let Some(diffs) = stream.next().await {
            let ops: Vec<Value> = diffs.iter().map(encode).collect();
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
                diffs.iter().flat_map(replies_needing_details),
                &mut requested,
                &detail_source,
                &sink,
            );
        }
    });

    // A thread that is not in the cache arrives empty; one page fills it.
    // A failure here has to be reported, or the view sits on "loading" for
    // good — the rows it would have delivered come through the diff stream,
    // so nothing else would ever say that they are not coming.
    if initial_empty {
        let timeline = timeline.clone();
        let root_owned = root.to_owned();
        tokio::spawn(async move {
            if let Err(error) = timeline.paginate_backwards(PAGE_SIZE).await {
                error_sink.emit(event(
                    "thread.error",
                    json!({
                        "root": root_owned,
                        "message": scrub_ids(&error.to_string()),
                    }),
                ));
            }
        });
    }

    Ok(TimelineHandle {
        room_id: room_id.to_owned(),
        live: false,
        pinned: false,
        timeline,
        task,
        retried_from_end: AtomicBool::new(true),
        thread_root: root.to_owned(),
    })
}

/// Strips anything that could identify a room, user or event from a message
/// meant for the log or the error line.
///
/// Testing only a word's first character was not enough, and neither is a
/// sigil test on its own. Measured against what the SDK actually produces:
/// ids come wrapped in their own syntax (`EventId("$id")`), reqwest appends
/// `for url (…)` with the id percent-encoded inside the path (`%21`, `%40`,
/// `%23`, `%24`), and a server's JSON body is printed whole. So a word goes
/// as soon as it looks like it could carry an identifier at all: any sigil,
/// any percent escape, anything URL-shaped, anything quoted.
pub fn scrub_ids(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let suspicious = word.contains(['$', '!', '@', '#', '"'])
                || word.contains('%')
                || word.contains("://");
            if suspicious {
                "<id>"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tries to fetch every pinned event and reports how many could be read.
/// The pinned view is built by the SDK from exactly these fetches, so when it
/// stays empty this says whether the events are unreachable or the view is
/// wired up wrongly. Counts and an error class only — never an identifier.
async fn pinned_load_report(
    room: &matrix_sdk::Room,
    ids: &[matrix_sdk::ruma::OwnedEventId],
) -> (usize, String) {
    let mut loaded = 0;
    let mut first_error = String::new();
    for id in ids {
        match room.event(id, None).await {
            Ok(_) => loaded += 1,
            Err(error) if first_error.is_empty() => {
                first_error = scrub_ids(&error.to_string());
            }
            Err(_) => {}
        }
    }
    (loaded, first_error)
}

/// The body of the newest pinned message, for the banner. Fetched from the
/// server (and decrypted where possible); empty when there is nothing usable
/// to show, in which case the UI falls back to a count.
async fn pinned_preview(
    room: &matrix_sdk::Room,
    ids: &[matrix_sdk::ruma::OwnedEventId],
) -> String {
    let Some(last) = ids.last() else {
        return String::new();
    };
    let Ok(fetched) = room.event(last, None).await else {
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

    // The banner is one line high. A pinned announcement can be a whole
    // changelog, so every run of whitespace — newlines included — collapses to
    // a single space and the rest is cut off; an unfolded body would otherwise
    // paint itself across the entire page.
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
    // Encrypted from the first message: no preset does this by itself, it is
    // the client's job — and enabling it later cannot protect what was
    // already sent.
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

/// Throws away the room's outbound group session, so the next message starts
/// a fresh one and shares its key with every device known at that moment.
///
/// The remedy for "the other side cannot read what I send": a Megolm key
/// reaches a device through an Olm session, and once that pairing is out of
/// step the recipient stays locked out for the whole session's lifetime. Only
/// the sending session goes — received keys stay, so nothing of the own
/// history becomes unreadable. Messages already sent stay unreadable for
/// whoever never got that key; this fixes the next one, not the last one.
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

/// Joins a room by its address, for example `#room:server`. Also accepts a
/// room id. The server part of the alias is offered as a routing hint, which
/// is what lets a join succeed for a room this homeserver has never seen.
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

/// Everything the room-info page shows, in one reply.
///
/// All of it is read from the room's local state — no request goes out, so the
/// page is filled the moment it opens. The counts are the server's summary
/// (the "heroes" summary of sliding sync), not a member list this would have to
/// fetch: opening the info page must not pull several hundred members.
///
/// The room id and the room version are in here because they are what tells a
/// user that a room is old — the reporter of the upgrade case worked that out
/// from those two lines in another client before any client said it outright.
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
        "name": room.cached_display_name().map(|name| name.to_string()).unwrap_or_default(),
        "topic": room.topic().unwrap_or_default(),
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
        // `None` means the join rule says nothing either way; the UI then keeps
        // quiet rather than claiming a room is private.
        //
        // Not named "public": a JSON field becomes a property name in QML, and
        // `public` is a reserved word there — `info.public` does not fail at
        // run time, it refuses to parse the whole page.
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

/// What a tombstoned room says about its replacement, as the UI needs it.
///
/// A room that was upgraded keeps its history but takes no new messages: the
/// upgrade raises the power level for sending, so the conversation simply stops
/// while the room still looks alive. Without this the user sits in a room where
/// nothing arrives any more and nothing says why.
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

/// Follows a room upgrade: joins the room that replaced `room_id` and answers
/// with its id, so the caller can open it.
///
/// Joining by id alone only works for a room the own homeserver already knows.
/// The replacement usually lives elsewhere, so this collects routing hints the
/// way a permalink would carry them — and it cannot lean on the room id for
/// that: from room version 12 on a room id is a plain hash with no server part
/// left in it (`!hash`, not `!local:server`). The server of whoever wrote the
/// tombstone is the reliable hint; it is by definition a server in both rooms.
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
