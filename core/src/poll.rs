//! Polls (MSC3381): what the UI may see of the votes, the three events, the
//! commands, and the check that an end came from the creator. The timeline
//! module asks for the payload; the runtime routes.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use matrix_sdk::room::{IncludeRelations, RelationsOptions};
use matrix_sdk::ruma::api::Direction;
use matrix_sdk::ruma::events::poll::PollResponseData;
use matrix_sdk::ruma::events::poll::{
    compile_unstable_poll_results,
    start::PollKind,
    unstable_end::UnstablePollEndEventContent,
    unstable_response::UnstablePollResponseEventContent,
    unstable_start::{
        NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollAnswers,
        UnstablePollStartContentBlock, UnstablePollStartEventContent,
    },
};
use matrix_sdk::ruma::events::relation::RelationType;
use matrix_sdk::ruma::events::{
    AnyMessageLikeEventContent, AnySyncMessageLikeEvent, AnySyncTimelineEvent,
    SyncMessageLikeEvent,
};
use matrix_sdk::ruma::{
    EventId, MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedTransactionId, OwnedUserId, UInt,
    UserId,
};
use matrix_sdk::send_queue::{LocalEchoContent, RoomSendQueueUpdate, SendHandle};
use matrix_sdk::Room;
use matrix_sdk_ui::timeline::{
    EventTimelineItem, MsgLikeKind, PollResult, PollState, Timeline, TimelineItem,
    TimelineItemContent,
};
use serde_json::{json, Value};
use tokio::sync::broadcast::{error::RecvError, Receiver};

use crate::protocol::{event, reply_error, reply_ok, Command};
use crate::runtime::Sink;
use crate::text::strip_bidi;

/// What a bubble holds of a stranger's poll; the spec bounds only the count.
const QUESTION_CHARS: usize = 500;
const ANSWER_CHARS: usize = 200;

fn bounded(text: &str, limit: usize) -> String {
    let mut cut: String = text.chars().take(limit).collect();
    if cut.chars().count() < text.chars().count() {
        cut.push('…');
    }
    strip_bidi(&cut)
}

type Votes = Vec<(String, Vec<String>)>;

/// What is known about a poll the SDK reports as ended. The SDK applies any
/// `m.poll.end` whoever sent it; here a poll ends on its creator's word.
enum Verdict {
    /// A check is running; its guard removes this if it never answers.
    Pending,
    /// The creator ended it. `complete`: every response was read.
    Ended { votes: Votes, complete: bool },
    /// Somebody else did: counted from the server instead, refreshed later.
    Rogue { votes: Votes, complete: bool, checked: Instant },
    /// The server could not be asked; retried a bounded number of times.
    Failed { attempts: u8, at: Instant },
}

static VERDICTS: LazyLock<Mutex<HashMap<OwnedEventId, Verdict>>> =
    LazyLock::new(Default::default);

fn verdicts() -> MutexGuard<'static, HashMap<OwnedEventId, Verdict>> {
    VERDICTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Memory bound: past this, every verdict is asked for again.
const VERDICT_LIMIT: usize = 500;

/// How long a rogue-ended poll's re-count stands before a diff asks again.
const RECOUNT_AFTER: Duration = Duration::from_secs(60);

/// Failed checks in a row before the poll is left alone for `RETRY_AFTER`.
const CHECK_ATTEMPTS: u8 = 2;
const RETRY_AFTER: Duration = Duration::from_secs(300);

/// Relation pages per check, a hundred each; the timeout per page is what
/// stands between a 429 and the SDK's retry budget.
const RELATION_PAGES: usize = 5;
const PAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a sent vote or end is watched for the queue giving it up. Long:
/// offline, the queue holds it until the sync is back.
const SEND_WATCH: Duration = Duration::from_secs(900);

pub fn forget() {
    verdicts().clear();
}

fn remember(event_id: OwnedEventId, verdict: Verdict) {
    remember_in(&mut verdicts(), event_id, verdict);
}

fn remember_in(known: &mut HashMap<OwnedEventId, Verdict>, event_id: OwnedEventId, verdict: Verdict) {
    if known.len() >= VERDICT_LIMIT && !known.contains_key(&event_id) {
        known.clear();
    }
    known.insert(event_id, verdict);
}

/// Takes the `Pending` entry back out if the check never answers - dropped
/// with the future, aborted or never spawned.
struct PendingGuard {
    event_id: OwnedEventId,
    armed: bool,
}

impl PendingGuard {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut known = verdicts();
        if matches!(known.get(&self.event_id), Some(Verdict::Pending)) {
            known.remove(&self.event_id);
        }
    }
}

/// What a check needs: the poll as started, who started it, the SDK's count.
pub struct Check {
    event_id: OwnedEventId,
    creator: OwnedUserId,
    block: UnstablePollStartContentBlock,
    sdk_votes: Votes,
    /// Where the SDK stopped counting - the first end it saw, whoever sent it.
    sdk_end: Option<MilliSecondsSinceUnixEpoch>,
    attempt: u8,
    /// A stranger's end is already known: the SDK's count stops there and
    /// cannot be trusted even once the creator's end turns up.
    recount: bool,
    guard: PendingGuard,
}

impl Check {
    fn new(
        event: &EventTimelineItem,
        results: PollResult,
        attempt: u8,
        recount: bool,
        armed: bool,
    ) -> Option<Self> {
        let event_id = event.event_id()?.to_owned();
        let answers: Vec<(String, String)> = results
            .answers
            .iter()
            .map(|answer| (answer.id.clone(), answer.text.clone()))
            .collect();
        Some(Self {
            event_id: event_id.clone(),
            creator: event.sender().to_owned(),
            block: build_block(&results.question, results.kind.clone(), results.max_selections, &answers)?,
            sdk_votes: results.votes.into_iter().collect(),
            sdk_end: results.end_time,
            attempt,
            recount,
            guard: PendingGuard { event_id, armed },
        })
    }
}

/// The poll as started. Not `From<PollState>`: that drops `kind` and
/// `max_selections` and makes every poll single-choice and undisclosed.
fn build_block(
    question: &str,
    kind: PollKind,
    max_selections: u64,
    answers: &[(String, String)],
) -> Option<UnstablePollStartContentBlock> {
    let listed: Vec<UnstablePollAnswer> = answers
        .iter()
        .map(|(id, text)| UnstablePollAnswer::new(id.clone(), text.clone()))
        .collect();
    let listed = UnstablePollAnswers::try_from(listed).ok()?;
    let mut block = UnstablePollStartContentBlock::new(question.to_owned(), listed);
    block.kind = kind;
    block.max_selections = UInt::try_from(max_selections).unwrap_or(UInt::from(1u32));
    Some(block)
}

/// Polls whose end is not yet vouched for. Marked pending here, unmarked by
/// the guard.
pub fn checks_needed<'a>(items: impl Iterator<Item = &'a Arc<TimelineItem>>) -> Vec<Check> {
    // Declared before the lock: a guard dropped while the lock is held would
    // lock again.
    let mut checks = Vec::new();
    let mut known = verdicts();
    for item in items {
        let Some(event) = item.as_event() else { continue };
        let TimelineItemContent::MsgLike(content) = event.content() else { continue };
        let MsgLikeKind::Poll(state) = &content.kind else { continue };
        let Some(event_id) = event.event_id() else { continue };
        let results = state.results();
        if results.end_time.is_none() {
            continue;
        }
        let attempt = match known.get_mut(event_id) {
            None => 0,
            Some(Verdict::Failed { attempts, .. }) if *attempts < CHECK_ATTEMPTS => *attempts,
            Some(Verdict::Failed { at, .. }) if at.elapsed() > RETRY_AFTER => 0,
            Some(Verdict::Rogue { checked, .. }) if checked.elapsed() > RECOUNT_AFTER => {
                // The verdict stays while the re-count runs: unarmed guard.
                *checked = Instant::now();
                checks.extend(Check::new(event, results, 0, true, false));
                continue;
            }
            _ => continue,
        };
        if let Some(check) = Check::new(event, results, attempt, false, true) {
            remember_in(&mut known, event_id.to_owned(), Verdict::Pending);
            checks.push(check);
        }
    }
    checks
}

/// What one page of relations contributed.
struct Page {
    creator_end: Option<MilliSecondsSinceUnixEpoch>,
    foreign_end: bool,
    responses: Vec<(OwnedUserId, MilliSecondsSinceUnixEpoch, Vec<String>)>,
    next: Option<String>,
}

/// One page, newest first, bounded in time: `Room::relations` takes no
/// request config, and the SDK would nurse a 429 for a quarter of an hour.
async fn relations_page(room: &Room, check: &Check, from: Option<String>) -> Option<Page> {
    let options = RelationsOptions {
        from,
        dir: Direction::Backward,
        limit: Some(UInt::from(100u32)),
        include_relations: IncludeRelations::RelationsOfType(RelationType::Reference),
        recurse: false,
    };
    let fetched = tokio::time::timeout(PAGE_TIMEOUT, room.relations(check.event_id.clone(), options))
        .await
        .ok()?
        .ok()?;

    let mut page = Page {
        creator_end: None,
        foreign_end: false,
        responses: Vec::new(),
        next: fetched.next_batch_token,
    };
    for related in &fetched.chunk {
        let Ok(AnySyncTimelineEvent::MessageLike(message_like)) = related.raw().deserialize() else {
            continue;
        };
        match message_like {
            AnySyncMessageLikeEvent::UnstablePollEnd(SyncMessageLikeEvent::Original(end)) => {
                if end.sender != check.creator {
                    page.foreign_end = true;
                } else if page.creator_end.is_none_or(|known| end.origin_server_ts < known) {
                    page.creator_end = Some(end.origin_server_ts);
                }
            }
            AnySyncMessageLikeEvent::UnstablePollResponse(SyncMessageLikeEvent::Original(
                response,
            )) => page.responses.push((
                response.sender,
                response.origin_server_ts,
                response.content.poll_response.answers,
            )),
            _ => {}
        }
    }
    Some(page)
}

/// Reads the poll's relations, newest first, and decides. The creator's end
/// is the newest thing about the poll, so it is usually on the first page and
/// the SDK's own count stands - unless a stranger's end is in play, which
/// stopped that count early: then every page is read and the votes counted
/// up to the creator's end. A read that fails decides nothing.
pub async fn verify(room: Room, check: Check, sink: Arc<Sink>) {
    let mut from: Option<String> = None;
    let mut creator_end: Option<MilliSecondsSinceUnixEpoch> = None;
    let mut recount = check.recount;
    let mut responses = Vec::new();
    let mut complete = true;

    for page_index in 0..RELATION_PAGES {
        let Some(page) = relations_page(&room, &check, from.take()).await else {
            if check.guard.armed {
                remember(
                    check.event_id.clone(),
                    Verdict::Failed { attempts: check.attempt + 1, at: Instant::now() },
                );
            }
            check.guard.disarm();
            return;
        };
        recount |= page.foreign_end;
        if let Some(end) = page.creator_end {
            creator_end = Some(end);
            // The SDK stopped at an earlier, foreign end it never showed us.
            recount |= check.sdk_end != Some(end);
        }
        if creator_end.is_some() && !recount {
            break;
        }
        responses.extend(page.responses);
        from = page.next;
        if from.is_none() {
            break;
        }
        if page_index + 1 == RELATION_PAGES {
            complete = false;
        }
    }

    let ended = creator_end.is_some();
    let votes: Votes = if ended && !recount {
        check.sdk_votes.clone()
    } else {
        compile_unstable_poll_results(
            &check.block,
            responses.iter().map(|(sender, timestamp, answers)| PollResponseData {
                sender,
                origin_server_ts: *timestamp,
                selections: answers,
            }),
            creator_end,
        )
        .into_iter()
        .map(|(answer, users)| (answer.to_owned(), users.iter().map(|user| user.to_string()).collect()))
        .collect()
    };

    {
        let mut known = verdicts();
        // Our own end, booked while this ran, outranks the server's lag.
        if matches!(known.get(&check.event_id), Some(Verdict::Ended { .. })) && !ended {
            drop(known);
            check.guard.disarm();
            return;
        }
        remember_in(
            &mut known,
            check.event_id.clone(),
            if ended {
                Verdict::Ended { votes: votes.clone(), complete }
            } else {
                Verdict::Rogue { votes: votes.clone(), complete, checked: Instant::now() }
            },
        );
    }
    check.guard.disarm();

    let view = PollView {
        question: check.block.question.text.clone(),
        undisclosed: check.block.kind == PollKind::Undisclosed,
        max_selections: check.block.max_selections.into(),
        ended,
        complete,
        answers: check
            .block
            .answers
            .iter()
            .map(|answer| (answer.id.clone(), answer.text.clone()))
            .collect(),
        votes,
    };
    sink.emit(verified(room.room_id().as_str(), &check.event_id, payload(&view, Some(room.own_user_id()))));
}

/// The row as it should read now; the model patches its `poll` field.
fn verified(room_id: &str, event_id: &EventId, poll: Value) -> Value {
    event("poll.verified", json!({ "roomId": room_id, "eventId": event_id.as_str(), "poll": poll }))
}

/// The three poll commands. One entry point, so the runtime's dispatch stays a
/// route and the rules stay in this file.
pub async fn handle(command: Command, timeline: Option<Arc<Timeline>>, sink: &Arc<Sink>) {
    let Some(timeline) = timeline else {
        sink.emit(reply_error(command.id(), "no timeline is open".to_owned()));
        return;
    };
    let (id, outcome) = match command {
        Command::PollStart { id, question, answers, undisclosed, max_selections } => {
            (id, start(&timeline, &question, &answers, undisclosed, max_selections).await)
        }
        Command::PollVote { id, event_id, answers } => {
            (id, vote(&timeline, sink, &event_id, answers).await)
        }
        Command::PollEnd { id, event_id, text } => (id, end(&timeline, sink, &event_id, &text).await),
        // Unreachable by construction: the runtime routes these three by name.
        other => (other.id(), Err("not a poll command".to_owned())),
    };
    match outcome {
        Ok(()) => sink.emit(reply_ok(id, json!({ "done": true }))),
        Err(message) => sink.emit(reply_error(id, message)),
    }
}

/// Sends a new poll. Undisclosed keeps the counts closed until it ends - a
/// convention every client honours, and none can be made to.
async fn start(
    timeline: &Timeline,
    question: &str,
    answers: &[String],
    undisclosed: bool,
    max_selections: u64,
) -> Result<(), String> {
    let content = start_content(question, answers, undisclosed, max_selections)?;
    timeline
        .send(content)
        .await
        .map(|_| ())
        .map_err(|error| format!("the poll could not be sent: {error}"))
}

/// The poll an event id names, as the timeline holds it.
async fn loaded_poll(
    timeline: &Timeline,
    id: &EventId,
) -> Result<(EventTimelineItem, PollState), String> {
    let item = timeline
        .item_by_event_id(id)
        .await
        .ok_or_else(|| "this poll is no longer loaded".to_owned())?;
    let TimelineItemContent::MsgLike(content) = item.content() else {
        return Err("this is not a poll".to_owned());
    };
    let MsgLikeKind::Poll(state) = &content.kind else {
        return Err("this is not a poll".to_owned());
    };
    let state = state.clone();
    Ok((item, state))
}

/// Votes, or changes a vote: one response carries the whole selection, because
/// only the latest one per person counts.
async fn vote(
    timeline: &Timeline,
    sink: &Arc<Sink>,
    event_id: &str,
    answers: Vec<String>,
) -> Result<(), String> {
    let id = EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;
    let (_, state) = loaded_poll(timeline, &id).await?;
    let results = state.results();
    // Ended on the creator's word, not the SDK's: what shows as running takes a vote.
    if matches!(verdicts().get(&id), Some(Verdict::Ended { .. })) {
        return Err("the poll has ended".to_owned());
    }
    // Against the poll itself: an answer id it does not know spoils the whole
    // vote, silently and for good.
    if answers.len() as u64 > results.max_selections {
        return Err("too many answers chosen".to_owned());
    }
    if let Some(unknown) = answers
        .iter()
        .find(|answer| !results.answers.iter().any(|known| &&known.id == answer))
    {
        return Err(format!("no such answer: {}", unknown.chars().take(16).collect::<String>()));
    }

    // Subscribed before it is sent: a vote is an aggregation, not a row, so the
    // SDK has no row to mark failed - the tick would just stay.
    let updates = queue_updates(timeline).await?;
    let handle = timeline
        .send(response_content(event_id, answers.clone())?)
        .await
        .map_err(|error| format!("the vote could not be sent: {error}"))?;
    let poll_id = id.clone();
    watch_send(
        timeline.room().room_id().to_string(),
        handle,
        updates,
        move |content| matches!(content,
            AnyMessageLikeEventContent::UnstablePollResponse(response)
                if response.relates_to.event_id == poll_id
                    && response.poll_response.answers == answers),
        Undelivered::Vote { event_id: id },
        sink.clone(),
    );
    Ok(())
}

async fn queue_updates(timeline: &Timeline) -> Result<Receiver<RoomSendQueueUpdate>, String> {
    timeline
        .room()
        .send_queue()
        .subscribe()
        .await
        .map(|(_, updates)| updates)
        .map_err(|error| format!("could not be queued: {error}"))
}

/// What to do when the queue gives the event up for good.
enum Undelivered {
    Vote { event_id: OwnedEventId },
    /// The end is booked when it is queued; undone here, the row restored.
    End { event_id: OwnedEventId, before: Value },
}

/// Watches the room's queue for one event of ours: found by its echo, then
/// followed to sent or failed. Holds no client (the handle's room is weak),
/// so it cannot keep a signed-out client alive; `SEND_WATCH` ends it. A
/// failure the queue retries itself is not reported; one it gave up on is
/// aborted, or its echo would be replayed as a vote on every start.
fn watch_send(
    room_id: String,
    handle: SendHandle,
    mut updates: Receiver<RoomSendQueueUpdate>,
    is_ours: impl Fn(&AnyMessageLikeEventContent) -> bool + Send + Sync + 'static,
    undelivered: Undelivered,
    sink: Arc<Sink>,
) {
    tokio::spawn(async move {
        let _ = tokio::time::timeout(SEND_WATCH, async {
            let mut transaction_id: Option<OwnedTransactionId> = None;
            loop {
                let update = match updates.recv().await {
                    Ok(update) => update,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                match update {
                    RoomSendQueueUpdate::NewLocalEvent(echo) if transaction_id.is_none() => {
                        if let LocalEchoContent::Event { serialized_event, .. } = &echo.content {
                            if serialized_event.deserialize().is_ok_and(|content| is_ours(&content)) {
                                transaction_id = Some(echo.transaction_id);
                            }
                        }
                    }
                    RoomSendQueueUpdate::SentEvent { transaction_id: sent, .. }
                        if transaction_id.as_deref() == Some(&sent) =>
                    {
                        return;
                    }
                    RoomSendQueueUpdate::SendError { transaction_id: failed, is_recoverable, .. }
                        if transaction_id.as_deref() == Some(&failed) && !is_recoverable =>
                    {
                        let _ = handle.abort().await;
                        match &undelivered {
                            Undelivered::Vote { event_id } => sink.emit(event(
                                "poll.voteFailed",
                                json!({ "roomId": room_id, "eventId": event_id.as_str() }),
                            )),
                            Undelivered::End { event_id, before } => {
                                if matches!(verdicts().get(event_id), Some(Verdict::Ended { .. })) {
                                    verdicts().remove(event_id);
                                }
                                sink.emit(verified(&room_id, event_id, before.clone()));
                                sink.emit(event(
                                    "poll.endFailed",
                                    json!({ "roomId": room_id, "eventId": event_id.as_str() }),
                                ));
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }
        })
        .await;
    });
}

/// Ends a poll. Neither server nor SDK checks who may, so the rule is held on
/// the way out: ours or nothing. Our own end is booked as it is queued and
/// taken back if the queue gives it up.
async fn end(timeline: &Timeline, sink: &Arc<Sink>, event_id: &str, text: &str) -> Result<(), String> {
    let id = EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;
    let (item, state) = loaded_poll(timeline, &id).await?;
    let own = timeline.room().own_user_id();
    if item.sender() != own {
        return Err("only the creator can end a poll".to_owned());
    }
    if matches!(verdicts().get(&id), Some(Verdict::Ended { .. })) {
        return Err("the poll has ended".to_owned());
    }
    let before = from_state(&state, Some(own), Some(&id));

    let updates = queue_updates(timeline).await?;
    let handle = timeline
        .send(end_content(event_id, text)?)
        .await
        .map_err(|error| format!("the poll could not be ended: {error}"))?;

    // A stranger's end before ours stopped the SDK's count; the re-count is
    // the one to keep.
    let (votes, complete) = match verdicts().get(&id) {
        Some(Verdict::Rogue { votes, complete, .. }) => (votes.clone(), *complete),
        _ => (state.results().votes.into_iter().collect(), true),
    };
    remember(id.clone(), Verdict::Ended { votes, complete });
    let room_id = timeline.room().room_id().to_string();
    sink.emit(verified(&room_id, &id, from_state(&state, Some(own), Some(&id))));
    let poll_id = id.clone();
    watch_send(
        room_id,
        handle,
        updates,
        move |content| matches!(content,
            AnyMessageLikeEventContent::UnstablePollEnd(end)
                if end.relates_to.event_id == poll_id),
        Undelivered::End { event_id: id, before },
        sink.clone(),
    );
    Ok(())
}

/// A poll before the visibility rule; plain data, so the rule is testable.
pub struct PollView {
    pub question: String,
    /// Results stay closed until the poll ends.
    pub undisclosed: bool,
    pub max_selections: u64,
    pub ended: bool,
    /// Whether every vote was counted, or the count is a floor.
    pub complete: bool,
    /// Answer id and text, in the order the poll declares them.
    pub answers: Vec<(String, String)>,
    /// Answer id to the users who chose it, already aggregated.
    pub votes: Votes,
}

/// What the UI gets. An undisclosed poll still running carries no counts:
/// hiding them in QML would ship what the page must not show.
pub fn payload(view: &PollView, own: Option<&UserId>) -> Value {
    let hidden = view.undisclosed && !view.ended;
    let own_id = own.map(|user| user.as_str());

    let mut voters: Vec<&str> = Vec::new();
    for (_, users) in &view.votes {
        for user in users {
            if !voters.contains(&user.as_str()) {
                voters.push(user);
            }
        }
    }

    let total: usize = view.votes.iter().map(|(_, users)| users.len()).sum();
    let mut mine: Vec<String> = Vec::new();
    let answers = view
        .answers
        .iter()
        .map(|(answer_id, text)| {
            let users = view
                .votes
                .iter()
                .find(|(id, _)| id == answer_id)
                .map(|(_, users)| users.as_slice())
                .unwrap_or(&[]);
            if own_id.is_some_and(|own| users.iter().any(|user| user == own)) {
                mine.push(answer_id.clone());
            }
            let count = users.len();
            json!({
                "id": answer_id,
                "text": bounded(text, ANSWER_CHARS),
                "count": if hidden { Value::Null } else { json!(count) },
                "share": if hidden || total == 0 {
                    Value::Null
                } else {
                    json!(count as f64 / total as f64)
                },
            })
        })
        .collect::<Vec<Value>>();

    json!({
        "question": bounded(&view.question, QUESTION_CHARS),
        "undisclosed": view.undisclosed,
        "ended": view.ended,
        "hidden": hidden,
        "maxSelections": view.max_selections.max(1),
        // Votes and people who cast them: with several selections the two differ.
        "votes": total,
        "voters": voters.len(),
        // False where the count is a floor - a server read that hit its bound.
        "complete": view.complete,
        // Our own selection travels even for a hidden poll - it is ours.
        "mine": mine,
        "answers": answers,
    })
}

/// The SDK's state as the payload needs it. An end counts on the creator's
/// word only; otherwise running, with the votes counted here.
pub fn from_state(poll: &PollState, own: Option<&UserId>, event_id: Option<&EventId>) -> Value {
    let results = poll.results();
    let (ended, votes, complete) = {
        let known = verdicts();
        match event_id.and_then(|id| known.get(id)) {
            Some(Verdict::Ended { votes, complete }) => (true, votes.clone(), *complete),
            Some(Verdict::Rogue { votes, complete, .. }) => (false, votes.clone(), *complete),
            _ => (false, results.votes.into_iter().collect(), true),
        }
    };
    let view = PollView {
        question: results.question,
        undisclosed: results.kind == PollKind::Undisclosed,
        max_selections: results.max_selections,
        ended,
        complete,
        answers: results
            .answers
            .iter()
            .map(|answer| (answer.id.clone(), answer.text.clone()))
            .collect(),
        votes,
    };
    payload(&view, own)
}

/// Answer ids are ours to choose; the index is what stays stable while the
/// poll is written.
fn answer_id(index: usize) -> String {
    format!("a{index}")
}

/// The text a client without poll support shows. Built from the poll itself,
/// so it carries no wording that would need translating in the core.
fn fallback_text(question: &str, answers: &[String]) -> String {
    let mut text = question.to_owned();
    for (index, answer) in answers.iter().enumerate() {
        text.push_str(&format!("\n{}. {answer}", index + 1));
    }
    text
}

fn start_content(
    question: &str,
    answers: &[String],
    undisclosed: bool,
    max_selections: u64,
) -> Result<AnyMessageLikeEventContent, String> {
    let question = question.trim();
    if question.is_empty() {
        return Err("the poll has no question".to_owned());
    }
    let answers: Vec<String> = answers
        .iter()
        .map(|answer| answer.trim().to_owned())
        .filter(|answer| !answer.is_empty())
        .collect();
    if answers.len() < 2 {
        return Err("a poll needs at least two answers".to_owned());
    }
    let listed: Vec<(String, String)> = answers
        .iter()
        .enumerate()
        .map(|(index, text)| (answer_id(index), text.clone()))
        .collect();
    let kind = if undisclosed { PollKind::Undisclosed } else { PollKind::Disclosed };
    let block = build_block(question, kind, max_selections.clamp(1, answers.len() as u64), &listed)
        .ok_or_else(|| "the answers were refused".to_owned())?;

    Ok(AnyMessageLikeEventContent::UnstablePollStart(
        UnstablePollStartEventContent::New(NewUnstablePollStartEventContent::plain_text(
            fallback_text(question, &answers),
            block,
        )),
    ))
}

/// A vote replaces the one before it - the spec counts the latest response per
/// person, so changing a vote is sending the whole selection again.
fn response_content(
    poll_id: &str,
    answers: Vec<String>,
) -> Result<AnyMessageLikeEventContent, String> {
    let id = EventId::parse(poll_id).map_err(|_| "not an event identifier".to_owned())?;
    Ok(AnyMessageLikeEventContent::UnstablePollResponse(
        UnstablePollResponseEventContent::new(answers, id),
    ))
}

fn end_content(poll_id: &str, text: &str) -> Result<AnyMessageLikeEventContent, String> {
    let id = EventId::parse(poll_id).map_err(|_| "not an event identifier".to_owned())?;
    let text = if text.trim().is_empty() { "The poll has ended." } else { text.trim() };
    Ok(AnyMessageLikeEventContent::UnstablePollEnd(UnstablePollEndEventContent::new(
        text.to_owned(),
        id,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(undisclosed: bool, ended: bool) -> PollView {
        PollView {
            question: "When?".to_owned(),
            undisclosed,
            max_selections: 1,
            ended,
            complete: true,
            answers: vec![
                ("a0".to_owned(), "Friday".to_owned()),
                ("a1".to_owned(), "Saturday".to_owned()),
            ],
            votes: vec![
                ("a0".to_owned(), vec!["@one:example.org".to_owned(), "@two:example.org".to_owned()]),
                ("a1".to_owned(), vec!["@three:example.org".to_owned()]),
            ],
        }
    }

    fn id(n: u32) -> OwnedEventId {
        EventId::parse(format!("$poll{n}:example.org")).unwrap()
    }

    /// The verdict map is global; tests that touch it take turns.
    static MAP: Mutex<()> = Mutex::new(());
    fn map_turn() -> MutexGuard<'static, ()> {
        MAP.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn disclosed_poll_carries_counts() {
        let value = payload(&view(false, false), None);
        assert_eq!(value["hidden"], json!(false));
        assert_eq!(value["voters"], json!(3));
        assert_eq!(value["votes"], json!(3));
        assert_eq!(value["complete"], json!(true));
        assert_eq!(value["answers"][0]["count"], json!(2));
        assert!((value["answers"][0]["share"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
    }

    /// The one rule that must not be left to the UI: while an undisclosed poll
    /// runs, the counts do not leave the core.
    #[test]
    fn undisclosed_poll_withholds_counts_until_it_ends() {
        let running = payload(&view(true, false), None);
        assert_eq!(running["hidden"], json!(true));
        assert_eq!(running["answers"][0]["count"], Value::Null);
        assert_eq!(running["answers"][0]["share"], Value::Null);

        let ended = payload(&view(true, true), None);
        assert_eq!(ended["hidden"], json!(false));
        assert_eq!(ended["answers"][0]["count"], json!(2));
    }

    #[test]
    fn own_selection_travels_even_while_hidden() {
        let own = UserId::parse("@two:example.org").unwrap();
        let value = payload(&view(true, false), Some(&own));
        assert_eq!(value["mine"], json!(["a0"]));
        assert_eq!(value["answers"][0]["count"], Value::Null);
    }

    #[test]
    fn a_strangers_text_is_bounded() {
        let mut long = view(false, false);
        long.question = "q".repeat(2000);
        long.answers[0].1 = "a".repeat(1000);
        let value = payload(&long, None);
        assert_eq!(value["question"].as_str().unwrap().chars().count(), QUESTION_CHARS + 1);
        assert_eq!(value["answers"][0]["text"].as_str().unwrap().chars().count(), ANSWER_CHARS + 1);
    }

    #[test]
    fn a_poll_needs_a_question_and_two_answers() {
        assert!(start_content("", &["a".to_owned(), "b".to_owned()], false, 1).is_err());
        assert!(start_content("q", &["a".to_owned(), "  ".to_owned()], false, 1).is_err());
        assert!(start_content("q", &["a".to_owned(), "b".to_owned()], false, 1).is_ok());
    }

    /// The number of selections stays inside what the answers allow, and the
    /// kind survives the trip through the block builder.
    #[test]
    fn start_content_keeps_kind_and_bounds_selections() {
        let content = start_content("q", &["a".to_owned(), "b".to_owned()], true, 9).unwrap();
        let AnyMessageLikeEventContent::UnstablePollStart(UnstablePollStartEventContent::New(
            start,
        )) = content
        else {
            panic!("not a poll start");
        };
        assert_eq!(u64::from(start.poll_start.max_selections), 2);
        assert_eq!(start.poll_start.kind, PollKind::Undisclosed);
    }

    /// A check that never answers must not leave the poll marked pending, or it
    /// is never asked about again. The guard does it on the way out.
    #[test]
    fn a_dropped_check_takes_its_pending_mark_away() {
        let _turn = map_turn();
        let poll = id(1);
        remember(poll.clone(), Verdict::Pending);
        drop(PendingGuard { event_id: poll.clone(), armed: true });
        assert!(verdicts().get(&poll).is_none());

        // A verdict that landed is not touched by a late guard.
        remember(poll.clone(), Verdict::Ended { votes: Vec::new(), complete: true });
        drop(PendingGuard { event_id: poll.clone(), armed: true });
        assert!(matches!(verdicts().get(&poll), Some(Verdict::Ended { .. })));
        verdicts().remove(&poll);
    }

    /// The verdict store never grows past its bound.
    #[test]
    fn verdicts_are_bounded() {
        let _turn = map_turn();
        forget();
        for n in 0..VERDICT_LIMIT as u32 + 5 {
            remember(id(1000 + n), Verdict::Ended { votes: Vec::new(), complete: true });
        }
        assert!(verdicts().len() <= VERDICT_LIMIT);
        forget();
    }

    /// The block rebuilt for the re-count keeps what `From<PollState>` drops.
    #[test]
    fn the_rebuilt_block_keeps_kind_and_selections() {
        let answers = vec![("a0".to_owned(), "x".to_owned()), ("a1".to_owned(), "y".to_owned())];
        let block = build_block("q", PollKind::Disclosed, 3, &answers).unwrap();
        assert_eq!(block.kind, PollKind::Disclosed);
        assert_eq!(u64::from(block.max_selections), 3);
        assert_eq!(block.answers.len(), 2);
    }
}
