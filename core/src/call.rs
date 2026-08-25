//! Call signalling.
//!
//! Matrix carries only the negotiation — who rings, who answers, and the
//! session descriptions and ICE candidates the two sides exchange. The media
//! itself never touches the homeserver; it travels directly over WebRTC, which
//! the Qt side drives with GStreamer.
//!
//! Everything here is therefore ordinary room events: nothing in this module
//! knows what an audio stream is.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use matrix_sdk::{
    ruma::{
        api::client::voip::get_turn_server_info,
        events::{
            call::{
                answer::{CallAnswerEventContent, SyncCallAnswerEvent},
                candidates::{CallCandidatesEventContent, Candidate, SyncCallCandidatesEvent},
                hangup::{CallHangupEventContent, SyncCallHangupEvent},
                invite::{CallInviteEventContent, SyncCallInviteEvent},
                SessionDescription,
            },
            AnyMessageLikeEventContent,
        },
        MilliSecondsSinceUnixEpoch, OwnedVoipId, RoomId, UInt, VoipVersionId,
    },
    Client, Room, RoomMemberships,
};
use serde_json::{json, Value};

use crate::protocol::event;
use crate::runtime::Sink;

/// How long an invitation stays valid, in milliseconds. The other side stops
/// showing the call as ringing once this has passed.
const INVITE_LIFETIME: u64 = 60_000;

fn room_of(client: &Client, room_id: &str) -> Result<Room, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())
}

/// Rings the other side, offering the local session description.
pub async fn invite(
    client: &Client,
    room_id: &str,
    call_id: &str,
    party_id: &str,
    sdp: String,
) -> Result<Option<String>, String> {
    let room = room_of(client, room_id)?;

    // The same rule as for an incoming call, and for the same reason: in a
    // room with more than two people the call id is public to every member,
    // and whoever answers first gets the microphone. Placing one there is a
    // deliberate choice, not a default.
    if !policy().groups && room.active_members_count() > 2 {
        return Err("calls in group rooms are switched off".to_owned());
    }

    let mut content = CallInviteEventContent::new(
        OwnedVoipId::from(call_id.to_owned()),
        UInt::new(INVITE_LIFETIME).unwrap_or(UInt::MAX),
        SessionDescription::new("offer".to_owned(), sdp),
        VoipVersionId::V1,
    );
    content.party_id = Some(OwnedVoipId::from(party_id.to_owned()));

    room.send(AnyMessageLikeEventContent::CallInvite(content))
        .await
        .map_err(|error| format!("could not place the call: {}", crate::text::scrub_ids(&error.to_string())))?;

    // Who is allowed to answer. In a two-person room that is the one other
    // member, and binding it here closes the window in which any member of the
    // room could answer first and take the microphone. Anything else answers
    // to nobody, and the engine keeps its own "first answer wins" rule.
    let own = room.own_user_id().to_owned();
    let expected = room
        .members_no_sync(RoomMemberships::JOIN)
        .await
        .ok()
        .and_then(|members| {
            let others: Vec<String> = members
                .iter()
                .map(|member| member.user_id().to_string())
                .filter(|user| user != own.as_str())
                .collect();
            if others.len() == 1 {
                others.into_iter().next()
            } else {
                None
            }
        });
    Ok(expected)
}

/// Accepts a call with the local answer.
pub async fn answer(
    client: &Client,
    room_id: &str,
    call_id: &str,
    party_id: &str,
    sdp: String,
) -> Result<(), String> {
    let room = room_of(client, room_id)?;

    let mut content = CallAnswerEventContent::new(
        SessionDescription::new("answer".to_owned(), sdp),
        OwnedVoipId::from(call_id.to_owned()),
        VoipVersionId::V1,
    );
    content.party_id = Some(OwnedVoipId::from(party_id.to_owned()));

    room.send(AnyMessageLikeEventContent::CallAnswer(content))
        .await
        .map(|_| ())
        .map_err(|error| format!("could not answer: {error}"))
}

/// Forwards locally gathered ICE candidates.
///
/// An empty candidate string signals that gathering has finished, which is how
/// version 1 of the specification says the list ends.
pub async fn candidates(
    client: &Client,
    room_id: &str,
    call_id: &str,
    party_id: &str,
    entries: Vec<Value>,
) -> Result<(), String> {
    let room = room_of(client, room_id)?;

    let list: Vec<Candidate> = entries
        .into_iter()
        .filter_map(|entry| {
            let candidate = entry.get("candidate")?.as_str()?.to_owned();
            let mut parsed = Candidate::new(candidate);
            parsed.sdp_mid = entry
                .get("sdpMid")
                .and_then(|value| value.as_str())
                .map(|value| value.to_owned());
            parsed.sdp_m_line_index = entry
                .get("sdpMLineIndex")
                .and_then(|value| value.as_u64())
                .and_then(|value| UInt::new(value));
            Some(parsed)
        })
        .collect();

    if list.is_empty() {
        return Ok(());
    }

    let mut content =
        CallCandidatesEventContent::new(OwnedVoipId::from(call_id.to_owned()), list, VoipVersionId::V1);
    content.party_id = Some(OwnedVoipId::from(party_id.to_owned()));

    room.send(AnyMessageLikeEventContent::CallCandidates(content))
        .await
        .map(|_| ())
        .map_err(|error| format!("could not send candidates: {error}"))
}

/// Ends a call, whether it was answered or not.
pub async fn hangup(
    client: &Client,
    room_id: &str,
    call_id: &str,
    party_id: &str,
) -> Result<(), String> {
    let room = room_of(client, room_id)?;

    let mut content =
        CallHangupEventContent::new(OwnedVoipId::from(call_id.to_owned()), VoipVersionId::V1);
    content.party_id = Some(OwnedVoipId::from(party_id.to_owned()));

    room.send(AnyMessageLikeEventContent::CallHangup(content))
        .await
        .map(|_| ())
        .map_err(|error| format!("could not hang up: {error}"))
}

/// Credentials for the homeserver's TURN server.
///
/// Without a relay, two devices behind NAT negotiate happily and then hear
/// nothing at all — the call looks connected and stays silent.
pub async fn turn_servers(client: &Client) -> Result<Value, String> {
    let response = client
        .send(get_turn_server_info::v3::Request::new())
        .await
        .map_err(|error| format!("no relay available: {error}"))?;

    Ok(json!({
        "username": response.username,
        "password": response.password,
        "uris": response.uris,
        "ttl": response.ttl.as_secs(),
    }))
}

/// Registers handlers for the four call events.
///
/// Who may make this phone ring.
///
/// Set from the app's privacy page and enforced *here*, before anything is
/// emitted: a call that is not allowed raises no banner, opens no page, wakes
/// nothing - and, deliberately, answers nothing either. A hangup back to the
/// caller would confirm that the account exists and is online, and it is a
/// request this device can be made to send at a stranger's rate.
#[derive(Clone)]
pub struct CallPolicy {
    /// "all", "direct" or "list".
    pub who: String,
    /// Whether a call arriving in a room with more than two members counts.
    pub groups: bool,
    /// Whether an offer with video may open the camera.
    pub video: bool,
    /// Whether a caller has to wait between two calls.
    pub flood: bool,
    /// People who may always call, whatever `who` says.
    pub allowed: Vec<String>,
}

impl Default for CallPolicy {
    fn default() -> Self {
        Self {
            who: "direct".to_owned(),
            groups: false,
            video: false,
            flood: false,
            allowed: Vec::new(),
        }
    }
}

static POLICY: Mutex<Option<CallPolicy>> = Mutex::new(None);
/// When each caller last made the phone ring, and for which call. A caller who
/// is allowed can still not ring in a loop - but a second, genuine attempt
/// after a missed call must get through, which a plain minute-long silence
/// swallowed.
static LAST_RING: Mutex<Option<HashMap<String, (Instant, String)>>> = Mutex::new(None);
const RING_INTERVAL: Duration = Duration::from_secs(10);

/// Drops the policy and the ring history. Called on sign-out: the allow list
/// names people, and it belongs to the account that is leaving.
pub fn forget_state() {
    if let Ok(mut guard) = POLICY.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = LAST_RING.lock() {
        *guard = None;
    }
}

pub fn set_policy(policy: CallPolicy) {
    if let Ok(mut guard) = POLICY.lock() {
        *guard = Some(policy);
    }
}

fn policy() -> CallPolicy {
    POLICY
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default()
}

/// Whether this caller is due a ring, and remembers that they got one.
///
/// A repeat of the *same* call never rings twice - that is the invitation
/// arriving again, not a new attempt. A different call from the same person
/// has to wait out a short floor, which stops a flood without swallowing the
/// second try after a missed call.
fn ring_is_due(sender: &str, call_id: &str, brake: bool) -> bool {
    let Ok(mut guard) = LAST_RING.lock() else {
        return true;
    };
    let seen = guard.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    if let Some((last, last_call)) = seen.get(sender) {
        // The same call arriving again is never a second ring, brake or not:
        // that is the invitation repeating, not a new attempt.
        if last_call == call_id {
            return false;
        }
        if brake && now.duration_since(*last) < RING_INTERVAL {
            return false;
        }
    }
    seen.insert(sender.to_owned(), (now, call_id.to_owned()));
    true
}

/// The policy, applied to one invitation.
///
/// Nothing in here awaits. The SDK runs an event handler inline while it
/// processes a sync response and waits for it, so every await on this path
/// delays the ring itself - measured as "the call was only on screen for a
/// couple of seconds before the other side gave up". `direct_targets` and
/// `active_members_count` read the room's own info in memory; the async
/// `is_direct` differs only for rooms this account has not joined, and a call
/// from a room one has not joined is refused anyway.
fn may_ring(sender: &str, room: &Room) -> bool {
    let policy = policy();

    // A room with more people than the two on the call is where a call id is
    // public to everybody in it - which is what makes a call there worth
    // hijacking. The allow list does not override this: it says who may call,
    // not where from.
    if !policy.groups && room.active_members_count() > 2 {
        return false;
    }

    if policy.allowed.iter().any(|allowed| allowed == sender) {
        return true;
    }

    match policy.who.as_str() {
        "all" => true,
        "list" => false,
        // Somebody this account already has a direct chat with. m.direct is
        // per account, so a two-person room can be flagged for one side and
        // not for the other - and a rule that refuses the call on one side
        // only reads as "she cannot answer". A room with exactly the two of
        // us counts as well.
        _ => !room.direct_targets().is_empty() || room.active_members_count() == 2,
    }
}

/// As everywhere else, the work is spawned: matrix-sdk runs event handlers
/// inline while processing a sync response, and blocking there stops the sync.
pub fn install(client: &Client, sink: Arc<Sink>) {
    client.add_event_handler({
        let sink = sink.clone();
        move |ev: SyncCallInviteEvent, room: Room, client: Client| {
            let sink = sink.clone();
            async move {
                let Some(ev) = ev.as_original() else { return };
                // Our own events echo back through sync; acting on them would
                // make the caller answer, then hang up, its own call.
                if Some(&*ev.sender) == client.user_id() {
                    return;
                }
                // A call event replayed from history — a sync backfill, or the
                // back-pagination that fills a freshly opened room — must not
                // make the phone ring for a call that ended long ago. The
                // invite's own `lifetime` is exactly that window; once the event
                // is older than that, the other side has stopped ringing too, so
                // drop it instead of surfacing a phantom incoming call.
                let now = u64::from(MilliSecondsSinceUnixEpoch::now().get());
                let sent = u64::from(ev.origin_server_ts.get());
                if now.saturating_sub(sent) > u64::from(ev.content.lifetime) {
                    return;
                }

                // Who may make this phone ring, and how often. Decided here
                // rather than in the UI: a refused call must not reach the
                // screen, and must be answered in no way the caller can see.
                // The reason goes out so a silent phone can be told from a
                // phone nobody called.
                if !may_ring(ev.sender.as_str(), &room) {
                    sink.emit(event(
                        "call.blocked",
                        json!({ "reason": "policy", "roomId": room.room_id().as_str() }),
                    ));
                    return;
                }
                if !ring_is_due(
                    ev.sender.as_str(),
                    ev.content.call_id.as_str(),
                    policy().flood,
                ) {
                    sink.emit(event(
                        "call.blocked",
                        json!({ "reason": "repeat", "roomId": room.room_id().as_str() }),
                    ));
                    return;
                }

                // Whether the offer carries video at all. The UI needs it to
                // say what is being offered and to open the camera only when
                // the user chose to - never because the caller asked.
                // `m=video 0 ...` is a *declined* media line: port zero means
                // the sender is offering nothing there.
                let offers_video = ev.content.offer.sdp.lines().any(|line| {
                    line.starts_with("m=video")
                        && line
                            .split_whitespace()
                            .nth(1)
                            .map(|port| port != "0")
                            .unwrap_or(false)
                });

                // How old the invitation is by the time this device rings.
                // Says whether a late ring was made here or on the way.
                sink.emit(event(
                    "call.invite",
                    json!({
                        "roomId": room.room_id().as_str(),
                        "sender": ev.sender.as_str(),
                        "callId": ev.content.call_id.as_str(),
                        "partyId": ev.content.party_id.as_ref().map(|id| id.as_str()),
                        "sdp": ev.content.offer.sdp,
                        "video": offers_video && policy().video,
                        "videoOffered": offers_video,
                        "ageMs": now.saturating_sub(sent),
                    }),
                ));
            }
        }
    });

    client.add_event_handler({
        let sink = sink.clone();
        move |ev: SyncCallAnswerEvent, room: Room, client: Client| {
            let sink = sink.clone();
            async move {
                let Some(ev) = ev.as_original() else { return };
                if Some(&*ev.sender) == client.user_id() {
                    return;
                }
                sink.emit(event(
                    "call.answer",
                    json!({
                        "roomId": room.room_id().as_str(),
                        "sender": ev.sender.as_str(),
                        "callId": ev.content.call_id.as_str(),
                        "sdp": ev.content.answer.sdp,
                    }),
                ));
            }
        }
    });

    client.add_event_handler({
        let sink = sink.clone();
        move |ev: SyncCallCandidatesEvent, room: Room, client: Client| {
            let sink = sink.clone();
            async move {
                let Some(ev) = ev.as_original() else { return };
                if Some(&*ev.sender) == client.user_id() {
                    return;
                }
                let list: Vec<Value> = ev
                    .content
                    .candidates
                    .iter()
                    .map(|candidate| {
                        json!({
                            "candidate": candidate.candidate,
                            "sdpMid": candidate.sdp_mid,
                            "sdpMLineIndex": candidate.sdp_m_line_index.map(u64::from),
                        })
                    })
                    .collect();

                sink.emit(event(
                    "call.candidates",
                    json!({
                        "roomId": room.room_id().as_str(),
                        "sender": ev.sender.as_str(),
                        "callId": ev.content.call_id.as_str(),
                        "candidates": list,
                    }),
                ));
            }
        }
    });

    client.add_event_handler({
        let sink = sink.clone();
        move |ev: SyncCallHangupEvent, room: Room, client: Client| {
            let sink = sink.clone();
            async move {
                let Some(ev) = ev.as_original() else { return };
                if Some(&*ev.sender) == client.user_id() {
                    return;
                }
                sink.emit(event(
                    "call.hangup",
                    json!({
                        "roomId": room.room_id().as_str(),
                        "sender": ev.sender.as_str(),
                        "callId": ev.content.call_id.as_str(),
                    }),
                ));
            }
        }
    });
}
