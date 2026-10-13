//! Parked send requests and the two ways out.
//! A parked request blocks every later one in its room.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use matrix_sdk::{
    ruma::{
        events::{
            room::message::{MessageType, Relation},
            AnyMessageLikeEventContent,
        },
        RoomId,
    },
    send_queue::{LocalEcho, LocalEchoContent},
    Client, HttpError, QueueWedgeError,
};
use serde_json::{json, Value};

use crate::protocol::{reply_error, reply_ok, Command};
use crate::runtime::Sink;
use crate::text::scrub_ids;

pub async fn handle(command: Command, client: Option<Client>, sink: &Arc<Sink>) {
    let Some(client) = client else {
        sink.emit(reply_error(command.id(), "not signed in".to_owned()));
        return;
    };
    let (id, outcome) = match command {
        Command::QueueStuck { id, room_id } => (id, stuck(&client, &room_id).await),
        Command::QueueRetry { id, room_id, txn_id } => {
            (id, retry(&client, &room_id, &txn_id).await.map(|()| json!({ "done": true })))
        }
        Command::QueueDiscard { id, room_id, txn_id } => (
            id,
            discard(&client, &room_id, &txn_id)
                .await
                .map(|reload| json!({ "done": true, "roomId": room_id, "reload": reload })),
        ),
        // Unreachable by construction: the runtime routes these three by name.
        other => (other.id(), Err("not a send queue command".to_owned())),
    };
    match outcome {
        Ok(data) => sink.emit(reply_ok(id, data)),
        Err(message) => sink.emit(reply_error(id, message)),
    }
}

/// Failed attempts a queued attachment gets before it is parked. Each one is
/// the whole upload again, and the SDK's timeout grows with the size.
const ATTACHMENT_ATTEMPTS: u32 = 3;
/// A request's count starts over this long after its first failure.
const COUNT_WINDOW: Duration = Duration::from_secs(6 * 3600);
/// A text the server keeps refusing is parked only after both: many tries
/// and a long time. Only an answer from the server counts (`counts_as_attempt`).
const TEXT_ATTEMPTS: u32 = 10;
const TEXT_PATIENCE: Duration = Duration::from_secs(30 * 60);

/// First wait before a room's queue is woken after a failure.
const FIRST_DELAY: Duration = Duration::from_secs(5);
/// Ceiling for the doubling: a long outage settles at one attempt a minute.
const MAX_DELAY: Duration = Duration::from_secs(60);
/// A failure this long after the room's previous one starts the wait over.
const SETTLED: Duration = Duration::from_secs(180);

/// Counted failures of one request and when the first of them came.
#[derive(Clone, Copy)]
struct Tries {
    count: u32,
    first: Instant,
}

/// The SDK keeps retrying a request whose failure it calls recoverable, for
/// ever. Counted per request; a reconnect resets the waits, never the counts.
#[derive(Default)]
struct Parking {
    attempts: HashMap<(String, String), Tries>,
    parked: HashSet<(String, String)>,
    /// Rooms whose queue an error switched off and that may be woken again.
    waiting: HashSet<String>,
    /// Per room: the last wait and when the failure behind it came.
    backoff: HashMap<String, (Duration, Instant)>,
}

impl Parking {
    /// One more counted failure of `txn`; true once the request is parked.
    fn failed(&mut self, room: &str, txn: &str, attachment: bool, now: Instant) -> bool {
        let key = (room.to_owned(), txn.to_owned());
        if self.parked.contains(&key) {
            return true;
        }
        self.waiting.insert(room.to_owned());
        let tries = self.attempts.entry(key.clone()).or_insert(Tries { count: 0, first: now });
        if now.saturating_duration_since(tries.first) > COUNT_WINDOW {
            *tries = Tries { count: 0, first: now };
        }
        tries.count += 1;
        let park = if attachment {
            tries.count >= ATTACHMENT_ATTEMPTS
        } else {
            tries.count >= TEXT_ATTEMPTS && now.saturating_duration_since(tries.first) >= TEXT_PATIENCE
        };
        if park {
            self.attempts.remove(&key);
            self.parked.insert(key);
            self.waiting.remove(room);
        }
        park
    }

    /// How long the room waits before its queue is woken, doubling per room.
    fn next_delay(&mut self, room: &str, now: Instant) -> Duration {
        let delay = match self.backoff.get(room) {
            Some((last, at)) if now.saturating_duration_since(*at) <= SETTLED => (*last * 2).min(MAX_DELAY),
            _ => FIRST_DELAY,
        };
        self.backoff.insert(room.to_owned(), (delay, now));
        delay
    }

    /// What is left of the room's current wait; zero where it has none.
    fn remaining(&self, room: &str, now: Instant) -> Duration {
        self.backoff
            .get(room)
            .map(|(delay, at)| (*at + *delay).saturating_duration_since(now))
            .unwrap_or(Duration::ZERO)
    }

    /// The line is back: the waits start over. Counts stay, or a flapping
    /// line would never let an attachment reach its limit.
    fn line_back(&mut self) {
        self.backoff.clear();
    }

    /// The user decided: send again from zero, or it is gone.
    fn release(&mut self, room: &str, txn: &str) {
        let key = (room.to_owned(), txn.to_owned());
        self.attempts.remove(&key);
        self.parked.remove(&key);
    }

    /// Drops the counts of requests the queue no longer holds.
    fn keep_only(&mut self, room: &str, queued: &HashSet<String>) {
        self.attempts.retain(|(r, t), _| r != room || queued.contains(t));
        self.parked.retain(|(r, t)| r != room || queued.contains(t));
    }

    fn is_parked(&self, room: &str, txn: &str) -> bool {
        self.parked.contains(&(room.to_owned(), txn.to_owned()))
    }

    fn room_parked(&self, room: &str) -> bool {
        self.parked.iter().any(|(r, _)| r == room)
    }
}

static PARKING: LazyLock<Mutex<Parking>> = LazyLock::new(|| Mutex::new(Parking::default()));

fn parking() -> std::sync::MutexGuard<'static, Parking> {
    PARKING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Teardown: the next session starts with nothing parked.
pub fn forget() {
    *parking() = Parking::default();
}

/// How a recoverable failure ended: with the server's status, without an
/// answer (`connected`: something was sent), or before the network.
enum Failure {
    Answered(u16),
    Unanswered { connected: bool },
    Local,
}

fn failure(error: &matrix_sdk::Error) -> Failure {
    match error {
        matrix_sdk::Error::Http(http) => match http.as_ref() {
            HttpError::Reqwest(error) => match error.status() {
                Some(status) => Failure::Answered(status.as_u16()),
                None => Failure::Unanswered { connected: !error.is_connect() },
            },
            other => match other.as_client_api_error() {
                Some(api) => Failure::Answered(api.status_code.as_u16()),
                None => Failure::Unanswered { connected: true },
            },
        },
        _ => Failure::Local,
    }
}

/// Whether a failure cost a try. A rate limit is the server's pacing; a text
/// costs one only where the server answered, an upload wherever bytes went out.
fn counts_as_attempt(failure: &Failure, attachment: bool) -> bool {
    match failure {
        Failure::Answered(status) => *status != 429,
        Failure::Unanswered { connected } => attachment && *connected,
        Failure::Local => true,
    }
}

/// A recoverable failure of request `txn_id` in `room_id`, named by the SDK's
/// `SendError` (for media: the event's transaction). True when now parked.
pub async fn recoverable_failure(
    client: &Client,
    room_id: &str,
    txn_id: &str,
    error: &matrix_sdk::Error,
) -> bool {
    let queued = match echoes(client, room_id).await {
        Ok(queued) => queued,
        // Unreadable: retried as before, never parked on a guess.
        Err(_) => {
            parking().waiting.insert(room_id.to_owned());
            return false;
        }
    };
    let attachment = queued
        .iter()
        .find(|echo| echo.transaction_id.as_str() == txn_id)
        .is_some_and(|echo| match &echo.content {
            LocalEchoContent::Event { serialized_event, .. } => {
                event_kind(serialized_event.deserialize().ok()) == "attachment"
            }
            _ => false,
        });
    let mut parking = parking();
    let present: HashSet<String> =
        queued.iter().map(|echo| echo.transaction_id.to_string()).collect();
    parking.keep_only(room_id, &present);
    if !counts_as_attempt(&failure(error), attachment) {
        parking.waiting.insert(room_id.to_owned());
        return false;
    }
    parking.failed(room_id, txn_id, attachment, Instant::now())
}

/// The wait before `room_id`'s queue is woken again after this failure.
pub fn retry_delay(room_id: &str) -> Duration {
    parking().next_delay(room_id, Instant::now())
}

/// Switches one room's queue back on, unless something in it is parked.
pub fn wake_room(client: &Client, room_id: &str) {
    {
        let mut parking = parking();
        if parking.room_parked(room_id) {
            return;
        }
        parking.waiting.remove(room_id);
    }
    wake(client, room_id);
}

/// The sync is running again: every room with something queued goes on,
/// parked rooms excepted, and the waits start over.
pub async fn wake_running(client: &Client) {
    parking().line_back();
    for room_id in rooms_off(client).await {
        wake_room(client, &room_id);
    }
}

/// After missed failures: each room that may be off, with what is left of its
/// wait. A parked one is left out, so it is never switched on, not even for a moment.
pub async fn rooms_to_wake(client: &Client) -> Vec<(String, Duration)> {
    let rooms = rooms_off(client).await;
    let parking = parking();
    let now = Instant::now();
    rooms
        .into_iter()
        .filter(|room_id| !parking.room_parked(room_id))
        .map(|room_id| {
            let delay = parking.remaining(&room_id, now);
            (room_id, delay)
        })
        .collect()
}

/// Rooms an error may have switched off: those noted, those with anything queued.
async fn rooms_off(client: &Client) -> HashSet<String> {
    let mut rooms: HashSet<String> = parking().waiting.iter().cloned().collect();
    if let Ok(unsent) = client.state_store().load_rooms_with_unsent_requests().await {
        rooms.extend(unsent.into_iter().map(|room_id| room_id.to_string()));
    }
    rooms
}

/// Rooms holding a request the SDK refused for good, from an earlier run too.
pub async fn rooms_with_wedged(client: &Client) -> Vec<String> {
    let Ok(unsent) = client.state_store().load_rooms_with_unsent_requests().await else {
        return Vec::new();
    };
    let mut rooms = Vec::new();
    for room_id in unsent {
        let room_id = room_id.to_string();
        let Ok(queued) = echoes(client, &room_id).await else { continue };
        let wedged = queued.iter().any(|echo| match &echo.content {
            LocalEchoContent::Event { send_error, .. } | LocalEchoContent::Redaction { send_error, .. } => {
                send_error.is_some()
            }
            LocalEchoContent::React { .. } => false,
        });
        if wedged {
            rooms.push(room_id);
        }
    }
    rooms
}

/// Queued requests in order, from the store.
async fn echoes(client: &Client, room_id: &str) -> Result<Vec<LocalEcho>, String> {
    let (echoes, _updates) = room(client, room_id)?
        .send_queue()
        .subscribe()
        .await
        .map_err(|error| format!("the send queue could not be read: {}", scrub_ids(&error.to_string())))?;
    Ok(echoes)
}

/// Parked requests, blocker first. Echoes the room id. A reaction the server
/// already has is taken out here; `reload` asks for the timeline anew.
async fn stuck(client: &Client, room_id: &str) -> Result<Value, String> {
    let mut rows = Vec::new();
    let mut reload = false;
    for echo in echoes(client, room_id).await? {
        let parked = parking().is_parked(room_id, echo.transaction_id.as_str());
        let (kind, error) = match &echo.content {
            LocalEchoContent::Event { serialized_event, send_error: Some(error), .. } => {
                (event_kind(serialized_event.deserialize().ok()), Some(error.clone()))
            }
            LocalEchoContent::Redaction { send_error: Some(error), .. } => ("deletion", Some(error.clone())),
            LocalEchoContent::Event { serialized_event, .. } if parked => {
                (event_kind(serialized_event.deserialize().ok()), None)
            }
            LocalEchoContent::Redaction { .. } if parked => ("deletion", None),
            _ => continue,
        };
        // Parked here, not by the SDK: no wedge error, the line or the server gave out.
        let Some(error) = error else {
            rows.push(json!({
                "txnId": echo.transaction_id.as_str(),
                "kind": kind,
                "reason": if kind == "attachment" { "unsteady" } else { "lingering" },
                "detail": "",
            }));
            continue;
        };
        // Left out only once it is really gone; otherwise it blocks, and says so.
        if kind == "reaction" && already_there(&error) {
            if let LocalEchoContent::Event { send_handle, .. } = &echo.content {
                if let Ok(true) = send_handle.abort().await {
                    reload = true;
                    continue;
                }
            }
        }
        rows.push(json!({
            "txnId": echo.transaction_id.as_str(),
            "kind": kind,
            "reason": reason(&error),
            "detail": scrub_ids(&error.to_string()),
        }));
    }
    if reload {
        wake(client, room_id);
    }
    Ok(json!({ "roomId": room_id, "rows": rows, "reload": reload }))
}

/// The server has this reaction already, from another device of the account.
fn already_there(error: &QueueWedgeError) -> bool {
    matches!(error, QueueWedgeError::GenericApiError { msg } if msg.contains("M_DUPLICATE_ANNOTATION"))
}

/// Resends a parked request; a deletion is aborted and queued anew.
async fn retry(client: &Client, room_id: &str, txn_id: &str) -> Result<(), String> {
    let echo = find(client, room_id, txn_id).await?;
    parking().release(room_id, txn_id);
    let outcome = resend(client, room_id, echo).await;
    wake(client, room_id);
    outcome
}

async fn resend(client: &Client, room_id: &str, echo: LocalEcho) -> Result<(), String> {
    match echo.content {
        LocalEchoContent::Event { send_handle, .. } => send_handle
            .unwedge()
            .await
            .map_err(|error| format!("could not send it again: {}", scrub_ids(&error.to_string()))),
        LocalEchoContent::Redaction { redacts, reason, send_handle, .. } => {
            let aborted = send_handle
                .abort()
                .await
                .map_err(|error| format!("could not send it again: {}", scrub_ids(&error.to_string())))?;
            // Not aborted: already sent.
            if !aborted {
                return Ok(());
            }
            room(client, room_id)?
                .send_queue()
                .redact(redacts, reason.as_deref())
                .await
                .map(|_| ())
                .map_err(|error| format!("could not delete: {}", scrub_ids(&error.to_string())))
        }
        LocalEchoContent::React { .. } => Err("a reaction is withdrawn, not resent".to_owned()),
    }
}

/// Takes a parked request out of the queue. `reload`: a removed duplicate
/// reaction echo takes the server's same-key reaction with it.
async fn discard(client: &Client, room_id: &str, txn_id: &str) -> Result<bool, String> {
    let echo = find(client, room_id, txn_id).await?;
    parking().release(room_id, txn_id);
    let aborted = match echo.content {
        LocalEchoContent::Event { serialized_event, send_handle, send_error } => {
            let duplicate = event_kind(serialized_event.deserialize().ok()) == "reaction"
                && send_error.as_ref().is_some_and(already_there);
            send_handle.abort().await.map(|done| done.then_some(duplicate))
        }
        LocalEchoContent::Redaction { send_handle, .. } => {
            send_handle.abort().await.map(|done| done.then_some(false))
        }
        LocalEchoContent::React { send_handle, .. } => {
            send_handle.abort().await.map(|done| done.then_some(false))
        }
    };
    wake(client, room_id);
    match aborted {
        Ok(Some(reload)) => Ok(reload),
        // Gone meanwhile: sent or taken out.
        Ok(None) => Err("it is no longer queued".to_owned()),
        Err(error) => Err(format!("could not discard it: {}", scrub_ids(&error.to_string()))),
    }
}

/// The SDK disables a room's queue after any failure; `unwedge`/`abort` only nudge.
fn wake(client: &Client, room_id: &str) {
    if let Ok(room) = room(client, room_id) {
        room.send_queue().set_enabled(true);
    }
}

fn room(client: &Client, room_id: &str) -> Result<matrix_sdk::Room, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    client.get_room(&parsed).ok_or_else(|| "this room is not known".to_owned())
}

async fn find(client: &Client, room_id: &str, txn_id: &str) -> Result<LocalEcho, String> {
    echoes(client, room_id)
        .await?
        .into_iter()
        .find(|echo| echo.transaction_id.as_str() == txn_id)
        .ok_or_else(|| "it is no longer queued".to_owned())
}

/// What the parked request would have done, as the front end names it.
fn event_kind(content: Option<AnyMessageLikeEventContent>) -> &'static str {
    match content {
        Some(AnyMessageLikeEventContent::RoomMessage(message)) => match message.relates_to {
            Some(Relation::Replacement(_)) => "edit",
            _ => match message.msgtype {
                MessageType::Image(_) | MessageType::Video(_) | MessageType::Audio(_) | MessageType::File(_) => {
                    "attachment"
                }
                _ => "message",
            },
        },
        Some(AnyMessageLikeEventContent::Reaction(_)) => "reaction",
        Some(AnyMessageLikeEventContent::UnstablePollResponse(_)) => "vote",
        Some(
            AnyMessageLikeEventContent::UnstablePollStart(_)
            | AnyMessageLikeEventContent::UnstablePollEnd(_),
        ) => "poll",
        _ => "other",
    }
}

/// Reason code for the front end; `detail` carries the SDK's words.
fn reason(error: &QueueWedgeError) -> &'static str {
    match error {
        QueueWedgeError::InsecureDevices { .. } => "insecureDevices",
        QueueWedgeError::IdentityViolations { .. } => "identityViolation",
        QueueWedgeError::CrossVerificationRequired => "ownVerification",
        QueueWedgeError::GenericApiError { msg } if too_large(msg) => "tooLarge",
        _ => "other",
    }
}

/// The server refused the upload for its size. Sending it again uploads it
/// again and is refused again; only discarding helps.
fn too_large(message: &str) -> bool {
    message.contains("M_TOO_LARGE") || message.contains("[413")
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

    #[test]
    fn a_replacement_is_an_edit_and_a_plain_message_is_not() {
        let plain = RoomMessageEventContent::text_plain("hello");
        assert_eq!(event_kind(Some(AnyMessageLikeEventContent::RoomMessage(plain.clone()))), "message");

        let target = matrix_sdk::ruma::owned_event_id!("$target:example.org");
        let edit = RoomMessageEventContent::text_plain("* hello")
            .make_replacement(matrix_sdk::ruma::events::room::message::ReplacementMetadata::new(target, None));
        assert_eq!(event_kind(Some(AnyMessageLikeEventContent::RoomMessage(edit))), "edit");
        assert_eq!(event_kind(None), "other");
    }

    #[test]
    fn the_verification_reasons_have_their_own_codes() {
        assert_eq!(reason(&QueueWedgeError::CrossVerificationRequired), "ownVerification");
        assert_eq!(
            reason(&QueueWedgeError::IdentityViolations { users: Vec::new() }),
            "identityViolation"
        );
        assert_eq!(reason(&QueueWedgeError::GenericApiError { msg: "x".to_owned() }), "other");
    }

    #[test]
    fn a_refused_size_has_its_own_code() {
        let matrix = QueueWedgeError::GenericApiError {
            msg: "the server returned an error: [413 / M_TOO_LARGE] Upload request body is too large".to_owned(),
        };
        assert_eq!(reason(&matrix), "tooLarge");
        // A proxy in front of the server answers without Matrix JSON.
        let proxy = QueueWedgeError::GenericApiError {
            msg: "the server returned an error: [413 Payload Too Large] <non-json bytes>".to_owned(),
        };
        assert_eq!(reason(&proxy), "tooLarge");
        let other = QueueWedgeError::GenericApiError { msg: "[403 / M_FORBIDDEN] no".to_owned() };
        assert_eq!(reason(&other), "other");
    }

    #[test]
    fn an_attachment_is_parked_after_its_share_of_attempts() {
        let mut parking = Parking::default();
        for _ in 1..ATTACHMENT_ATTEMPTS {
            assert!(!fail(&mut parking, "!r", "t1", true));
        }
        assert!(fail(&mut parking, "!r", "t1", true));
        assert!(parking.is_parked("!r", "t1"));
        assert!(parking.room_parked("!r"));
        // A parked room is not woken with the others.
        assert!(!parking.waiting.contains("!r"));
    }

    fn fail(parking: &mut Parking, room: &str, txn: &str, attachment: bool) -> bool {
        parking.failed(room, txn, attachment, Instant::now())
    }

    #[test]
    fn a_text_counts_only_what_the_server_answered() {
        let lost = Failure::Unanswered { connected: true };
        assert!(!counts_as_attempt(&lost, false));
        assert!(counts_as_attempt(&lost, true));
        assert!(!counts_as_attempt(&Failure::Unanswered { connected: false }, true));
        assert!(!counts_as_attempt(&Failure::Answered(429), false));
        assert!(counts_as_attempt(&Failure::Answered(502), false));
    }

    #[test]
    fn a_texts_count_starts_over_after_the_window() {
        let mut parking = Parking::default();
        let start = Instant::now();
        assert!(!parking.failed("!r", "m", false, start));
        let later = start + COUNT_WINDOW + Duration::from_secs(1);
        for _ in 1..TEXT_ATTEMPTS {
            assert!(!parking.failed("!r", "m", false, later));
        }
        assert!(!parking.is_parked("!r", "m"));
    }

    #[test]
    fn a_message_rides_out_a_burst_of_failures() {
        let mut parking = Parking::default();
        let now = Instant::now();
        for _ in 0..100 {
            assert!(!parking.failed("!r", "m1", false, now));
        }
        assert!(!parking.room_parked("!r"));
        assert!(parking.waiting.contains("!r"));
    }

    #[test]
    fn a_message_the_server_refuses_for_half_an_hour_is_parked() {
        let mut parking = Parking::default();
        let start = Instant::now();
        for _ in 1..TEXT_ATTEMPTS {
            assert!(!parking.failed("!r", "m1", false, start));
        }
        // Enough tries, not enough time: still waiting.
        assert!(!parking.failed("!r", "m1", false, start + TEXT_PATIENCE / 2));
        assert!(parking.failed("!r", "m1", false, start + TEXT_PATIENCE));
        assert!(parking.is_parked("!r", "m1"));
        // Long, but only two counted tries: a dead network in between costs none.
        assert!(!parking.failed("!r", "m2", false, start));
        assert!(!parking.failed("!r", "m2", false, start + TEXT_PATIENCE * 4));
    }

    #[test]
    fn the_failing_request_is_counted_not_the_first_in_line() {
        let mut parking = Parking::default();
        // A text ahead of the photo in the echo list does not take its count.
        for _ in 0..ATTACHMENT_ATTEMPTS {
            fail(&mut parking, "!r", "photo", true);
        }
        assert!(parking.is_parked("!r", "photo"));
        assert!(!parking.is_parked("!r", "text"));
    }

    #[test]
    fn the_line_coming_back_keeps_counts_and_parking() {
        let mut parking = Parking::default();
        for _ in 0..ATTACHMENT_ATTEMPTS {
            fail(&mut parking, "!p", "b", true);
        }
        // A flapping line: back after every failure, the third still parks.
        for _ in 1..ATTACHMENT_ATTEMPTS {
            assert!(!fail(&mut parking, "!r", "a", true));
            parking.line_back();
        }
        assert!(fail(&mut parking, "!r", "a", true));
        assert!(parking.room_parked("!p"));
    }

    #[test]
    fn an_attachments_count_starts_over_after_its_window() {
        let mut parking = Parking::default();
        let start = Instant::now();
        for _ in 1..ATTACHMENT_ATTEMPTS {
            assert!(!parking.failed("!r", "a", true, start));
        }
        let later = start + COUNT_WINDOW + Duration::from_secs(1);
        assert!(!parking.failed("!r", "a", true, later));
        assert!(!parking.is_parked("!r", "a"));
    }

    #[test]
    fn what_is_left_of_a_wait() {
        let mut parking = Parking::default();
        let now = Instant::now();
        assert_eq!(parking.remaining("!a", now), Duration::ZERO);
        let delay = parking.next_delay("!a", now);
        assert_eq!(parking.remaining("!a", now + Duration::from_secs(2)), delay - Duration::from_secs(2));
        assert_eq!(parking.remaining("!a", now + delay * 2), Duration::ZERO);
    }

    #[test]
    fn the_wait_doubles_per_room_and_starts_over_when_settled() {
        let mut parking = Parking::default();
        let now = Instant::now();
        assert_eq!(parking.next_delay("!a", now), FIRST_DELAY);
        assert_eq!(parking.next_delay("!a", now), FIRST_DELAY * 2);
        // Another room is not held up by the first one's wait.
        assert_eq!(parking.next_delay("!b", now), FIRST_DELAY);
        for _ in 0..10 {
            parking.next_delay("!a", now);
        }
        assert_eq!(parking.next_delay("!a", now), MAX_DELAY);
        assert_eq!(parking.next_delay("!a", now + SETTLED + Duration::from_secs(1)), FIRST_DELAY);
        parking.line_back();
        assert_eq!(parking.next_delay("!a", now), FIRST_DELAY);
    }

    #[test]
    fn a_released_request_starts_from_zero() {
        let mut parking = Parking::default();
        for _ in 0..ATTACHMENT_ATTEMPTS {
            fail(&mut parking, "!r", "t1", true);
        }
        parking.release("!r", "t1");
        assert!(!parking.room_parked("!r"));
        assert!(!fail(&mut parking, "!r", "t1", true));
    }

    #[test]
    fn counts_of_requests_gone_from_the_queue_are_dropped() {
        let mut parking = Parking::default();
        fail(&mut parking, "!r", "gone", true);
        for _ in 0..ATTACHMENT_ATTEMPTS {
            fail(&mut parking, "!r", "parked", true);
        }
        fail(&mut parking, "!other", "kept", true);
        parking.keep_only("!r", &HashSet::new());
        assert!(!parking.room_parked("!r"));
        assert!(parking.attempts.contains_key(&("!other".to_owned(), "kept".to_owned())));
    }

    #[test]
    fn an_attachment_is_named_as_one() {
        let image = RoomMessageEventContent::new(MessageType::Image(
            matrix_sdk::ruma::events::room::message::ImageMessageEventContent::plain(
                "a.jpg".to_owned(),
                matrix_sdk::ruma::OwnedMxcUri::from("mxc://example.org/a"),
            ),
        ));
        assert_eq!(event_kind(Some(AnyMessageLikeEventContent::RoomMessage(image))), "attachment");
    }

    #[test]
    fn a_duplicate_reaction_is_already_there() {
        let duplicate = QueueWedgeError::GenericApiError {
            msg: "the server returned an error: [400 / M_DUPLICATE_ANNOTATION] Can't send same reaction twice"
                .to_owned(),
        };
        assert!(already_there(&duplicate));
        assert!(!already_there(&QueueWedgeError::CrossVerificationRequired));
    }
}
