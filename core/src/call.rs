//! Call signalling: Matrix carries the negotiation only. The media travels
//! over WebRTC, which the Qt side drives with GStreamer.

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

/// Whether an SDP offers a video line that is actually live. Walks the section:
/// a port of zero counts as refused only where no `a=bundle-only` follows before
/// the next media line.
fn offers_active_video(sdp: &str) -> bool {
    let mut in_video = false;
    let mut port_zero = false;
    for line in sdp.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with("m=") {
            // The previous video section ended without `a=bundle-only`.
            if in_video && !port_zero {
                return true;
            }
            in_video = line.starts_with("m=video");
            port_zero = in_video
                && line
                    .split_whitespace()
                    .nth(1)
                    .map(|port| port == "0")
                    .unwrap_or(false);
            continue;
        }
        if in_video && port_zero && line == "a=bundle-only" {
            return true;
        }
    }
    in_video && !port_zero
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

    // Same rule as for an incoming call: in a room with more than two people the
    // call id is public and whoever answers first gets the microphone.
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

    // Who is allowed to answer. Binding it here closes the window in which any
    // member could answer first; the engine keeps its "first answer wins" rule.
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

/// Forwards gathered ICE candidates. An empty candidate string ends the list,
/// as version 1 of the specification has it.
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

/// Credentials for the homeserver's TURN server: without a relay two devices
/// behind NAT negotiate happily and then hear nothing.
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

/// Who may make this phone ring, enforced here before anything is emitted -
/// and answered in no way: a hangup would confirm the account is online.
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
/// When each caller last rang, and for which call. A second genuine attempt
/// after a missed call must get through, which a flat minute swallowed.
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

/// Whether this caller is due a ring. The same call never rings twice - that is
/// the invitation arriving again - a different one waits out a short floor.
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

/// Nothing here awaits: the SDK runs handlers inline while processing a sync,
/// so every await delays the ring itself.
fn may_ring(sender: &str, room: &Room) -> bool {
    let policy = policy();

    // A room with more people is where the call id is public, which is what makes
    // a call there worth hijacking. The allow list says who may call, not where from.
    if !policy.groups && room.active_members_count() > 2 {
        return false;
    }

    if policy.allowed.iter().any(|allowed| allowed == sender) {
        return true;
    }

    match policy.who.as_str() {
        "all" => true,
        "list" => false,
        // A direct chat, not merely a room with two people in it - which is any
        // room a stranger shares. `m.direct` is per account, by the server.
        _ => !room.direct_targets().is_empty(),
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
                // A replayed invite must not ring for a call that ended: its own `lifetime` is
                // the window. The skew is far larger - a fast clock would eat every call.
                const CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;
                let now = u64::from(MilliSecondsSinceUnixEpoch::now().get());
                let sent = u64::from(ev.origin_server_ts.get());
                if now.saturating_sub(sent) > u64::from(ev.content.lifetime) + CLOCK_SKEW_MS {
                    return;
                }

                // Decided here, not in the UI: a refused call must not reach the screen. The
                // reason goes out so a silent phone can be told from one nobody called.
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

                // Whether the offer carries video, so the UI can say what is offered and
                // open the camera only on the user's choice.
                //
                // Port 0 alone does *not* mean declined. Under BUNDLE every media
                // line after the first is written with port 0 plus `a=bundle-only`
                // and runs over the first line's transport (RFC 9143) - which is
                // exactly what `bundle-policy=max-bundle` makes webrtcbin do, so
                // xmatic's own video offers looked declined to xmatic. Measured on
                // the device: `m=video 0 … | a=bundle-only | a=sendrecv |
                // a=rtpmap:96 VP8/90000`. Declined is port 0 *without* it.
                let offers_video = offers_active_video(&ev.content.offer.sdp);

                // The media lines of their offer, and nothing else: port and codec
                // numbers, no addresses. Without this "no video offered" is a
                // claim about a string nobody has seen.
                let media_lines: Vec<&str> = ev
                    .content
                    .offer
                    .sdp
                    .lines()
                    .filter(|line| line.starts_with("m="))
                    .collect();
                sink.emit(event(
                    "core.log",
                    json!({
                        "level": "warn",
                        "target": "xmatic",
                        "message": format!(
                            "incoming offer media lines: {}",
                            media_lines.join(" | ")
                        ),
                    }),
                ));

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
                // An answer from this account is another of its devices picking
                // the call up - the party id says which one. Dropped outright,
                // the phones that did not answer keep ringing, because nothing
                // else tells them the call is taken.
                if Some(&*ev.sender) == client.user_id() {
                    sink.emit(event(
                        "call.answeredElsewhere",
                        json!({
                            "roomId": room.room_id().as_str(),
                            "callId": ev.content.call_id.as_str(),
                            "partyId": ev.content.party_id.as_ref().map(|id| id.as_str()),
                        }),
                    ));
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

#[cfg(test)]
mod video_offer_tests {
    use super::offers_active_video;

    #[test]
    fn bundle_only_is_not_a_refusal() {
        let sdp = "v=0\r\na=group:BUNDLE audio0 video1\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:audio0\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=bundle-only\r\na=sendrecv\r\na=rtpmap:96 VP8/90000\r\n";
        assert!(offers_active_video(sdp));
    }

    #[test]
    fn port_zero_alone_is_a_refusal() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=inactive\r\n";
        assert!(!offers_active_video(sdp));
    }

    #[test]
    fn a_plain_port_offers_video() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 97\r\na=sendrecv\r\n";
        assert!(offers_active_video(sdp));
    }

    #[test]
    fn a_voice_call_offers_none() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=sendrecv\r\n";
        assert!(!offers_active_video(sdp));
    }

    #[test]
    fn a_later_section_does_not_rescue_it() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\nm=application 0 UDP/DTLS/SCTP webrtc-datachannel\r\na=bundle-only\r\n";
        assert!(!offers_active_video(sdp));
    }
}
