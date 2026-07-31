//! Call signalling.
//!
//! Matrix carries only the negotiation — who rings, who answers, and the
//! session descriptions and ICE candidates the two sides exchange. The media
//! itself never touches the homeserver; it travels directly over WebRTC, which
//! the Qt side drives with GStreamer.
//!
//! Everything here is therefore ordinary room events: nothing in this module
//! knows what an audio stream is.

use std::sync::Arc;

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
    Client, Room,
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
) -> Result<(), String> {
    let room = room_of(client, room_id)?;

    let mut content = CallInviteEventContent::new(
        OwnedVoipId::from(call_id.to_owned()),
        UInt::new(INVITE_LIFETIME).unwrap_or(UInt::MAX),
        SessionDescription::new("offer".to_owned(), sdp),
        VoipVersionId::V1,
    );
    content.party_id = Some(OwnedVoipId::from(party_id.to_owned()));

    room.send(AnyMessageLikeEventContent::CallInvite(content))
        .await
        .map(|_| ())
        .map_err(|error| format!("could not place the call: {error}"))
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
                sink.emit(event(
                    "call.invite",
                    json!({
                        "roomId": room.room_id().as_str(),
                        "sender": ev.sender.as_str(),
                        "callId": ev.content.call_id.as_str(),
                        "partyId": ev.content.party_id.as_ref().map(|id| id.as_str()),
                        "sdp": ev.content.offer.sdp,
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
                        "callId": ev.content.call_id.as_str(),
                    }),
                ));
            }
        }
    });
}
