//! Interactive device verification (SAS). Verified devices of one account
//! share missing Megolm keys; one verification is tracked at a time.

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::{
    encryption::verification::{
        SasState, SasVerification, Verification, VerificationRequest, VerificationRequestState,
    },
    ruma::{OwnedRoomId, UserId},
    ruma::events::{
        key::verification::{
            accept::ToDeviceKeyVerificationAcceptEvent, cancel::ToDeviceKeyVerificationCancelEvent,
            key::ToDeviceKeyVerificationKeyEvent, request::ToDeviceKeyVerificationRequestEvent,
            start::ToDeviceKeyVerificationStartEvent,
        },
        room::message::{MessageType, OriginalSyncRoomMessageEvent},
    },
    Client,
};
use serde_json::json;
use tokio::sync::Mutex;

use crate::protocol::event;
use crate::runtime::Sink;

/// The verification currently on screen.
pub struct ActiveVerification {
    flow_id: String,
    request: VerificationRequest,
    sas: Mutex<Option<SasVerification>>,
}

impl ActiveVerification {
    pub fn flow_id(&self) -> &str {
        &self.flow_id
    }

    /// Agrees to the request; the emoji follow once both sides exchanged keys.
    pub async fn accept(&self) -> Result<(), String> {
        self.request
            .accept()
            .await
            .map_err(|error| crate::text::scrub_ids(&format!("could not accept: {error}")))
    }

    /// Confirms that the emoji match on both screens.
    pub async fn confirm(&self) -> Result<(), String> {
        let sas = self.sas.lock().await.clone();
        match sas {
            Some(sas) => sas
                .confirm()
                .await
                .map_err(|error| format!("could not confirm: {error}")),
            None => Err("the emoji have not been exchanged yet".to_owned()),
        }
    }

    /// The emoji did not match: `m.mismatched_sas`, not an ordinary cancel. It is
    /// the one signal that says somebody may be in the middle.
    pub async fn mismatch(&self) -> Result<(), String> {
        let sas = self.sas.lock().await.clone();
        match sas {
            Some(sas) => sas
                .mismatch()
                .await
                .map_err(|error| format!("could not report the mismatch: {error}")),
            None => Err("the emoji have not been exchanged yet".to_owned()),
        }
    }

    /// Rejects the request, or aborts a running comparison.
    pub async fn cancel(&self) -> Result<(), String> {
        let sas = self.sas.lock().await.clone();
        if let Some(sas) = sas {
            return sas
                .cancel()
                .await
                .map_err(|error| format!("could not cancel: {error}"));
        }
        self.request
            .cancel()
            .await
            .map_err(|error| format!("could not cancel: {error}"))
    }
}

/// Cancels whatever is tracked, before a new request goes out: a second request
/// for the same user makes the SDK cancel both.
pub async fn cancel_active(slot: &Slot) {
    let active = slot.lock().await.take();
    if let Some(active) = active {
        let _ = active.cancel().await;
    }
}

/// Clears the slot, but only if it still holds this flow. Two verifications can
/// overlap, and a late finisher must not evict the one on screen.
async fn release(slot: &Slot, flow_id: &str) {
    let mut guard = slot.lock().await;
    let matches = guard
        .as_ref()
        .map(|active| active.flow_id == flow_id)
        .unwrap_or(false);
    if matches {
        guard.take();
    }
}

/// Announces a request to the UI and drives it to completion in the background.
async fn track(sink: Arc<Sink>, request: VerificationRequest, slot: Slot) {
    let flow_id = request.flow_id().to_owned();

    sink.emit(event(
        "verification.request",
        json!({
            "flowId": flow_id,
            "userId": request.other_user_id().as_str(),
            "isSelf": request.is_self_verification(),
            // The transport decides where the follow-up events arrive: to-device through
            // the encryption sync, in-room only with the room's timeline.
            "inRoom": request.room_id().is_some(),
            "weStarted": request.we_started(),
        }),
    ));

    let active = Arc::new(ActiveVerification {
        flow_id: flow_id.clone(),
        request: request.clone(),
        sas: Mutex::new(None),
    });
    *slot.lock().await = Some(active.clone());

    // Wait for the flow to become a SAS. Subscribing before the first state read
    // is what makes it race-free - the stream yields only newer versions.
    let mut changes = request.changes();
    let sas = loop {
        let state = request.state();
        report_state(&sink, &flow_id, &state);

        match state {
            VerificationRequestState::Transitioned { verification } => {
                match verification {
                    Verification::SasV1(sas) => break sas,
                    _ => {
                        // Only the emoji flow is implemented; anything else is
                        // declined rather than left hanging.
                        let _ = request.cancel().await;
                        sink.emit(event("verification.cancelled", json!({ "flowId": flow_id })));
                        release(&slot, &flow_id).await;
                        return;
                    }
                }
            }
            VerificationRequestState::Ready { .. } => {
                // Only the side that asked starts it: an incoming start moves the request to
                // Transitioned on its own, so the responder waits.
                if request.we_started() {
                    match request.start_sas().await {
                        Ok(Some(sas)) => break sas,
                        Ok(None) => {}
                        Err(error) => {
                            sink.emit(event(
                                "verification.failed",
                                json!({ "flowId": flow_id, "message": crate::text::scrub_ids(&error.to_string()) }),
                            ));
                        }
                    }
                }
            }
            VerificationRequestState::Cancelled(_) => {
                sink.emit(event("verification.cancelled", json!({ "flowId": flow_id })));
                release(&slot, &flow_id).await;
                return;
            }
            VerificationRequestState::Done => {
                sink.emit(event("verification.done", json!({ "flowId": flow_id })));
                release(&slot, &flow_id).await;
                return;
            }
            _ => {}
        }

        if changes.next().await.is_none() {
            release(&slot, &flow_id).await;
            return;
        }
    };

    *active.sas.lock().await = Some(sas.clone());
    drive_sas(sink, sas, flow_id, slot).await;
}

/// Publishes the flow's stage. Carries no identifiers — it exists so a stalled
/// verification can be told apart from a rejected one in the log.
fn report_state(sink: &Arc<Sink>, flow_id: &str, state: &VerificationRequestState) {
    let name = match state {
        VerificationRequestState::Created { .. } => "created",
        VerificationRequestState::Requested { .. } => "requested",
        VerificationRequestState::Ready { .. } => "ready",
        VerificationRequestState::Transitioned { .. } => "transitioned",
        VerificationRequestState::Done => "done",
        VerificationRequestState::Cancelled(_) => "cancelled",
    };
    sink.emit(event(
        "verification.stage",
        json!({ "flowId": flow_id, "stage": name }),
    ));
}

/// Reports the emoji, then the outcome. The state stream is the authority:
/// polling predicates loses the moment the keys are exchanged.
async fn drive_sas(sink: Arc<Sink>, sas: SasVerification, flow_id: String, slot: Slot) {
    // Accepting is what sends `m.key.verification.accept`. If it fails the
    // other side waits forever, so the error must not be swallowed.
    if !sas.we_started() {
        if let Err(error) = sas.accept().await {
            sink.emit(event(
                "verification.failed",
                json!({ "flowId": flow_id, "message": crate::text::scrub_ids(&format!("could not accept: {error}")) }),
            ));
            sink.emit(event("verification.cancelled", json!({ "flowId": flow_id })));
            release(&slot, &flow_id).await;
            return;
        }
    }

    let mut changes = sas.changes();
    let mut state = sas.state();

    loop {
        report_sas_state(&sink, &flow_id, &state);

        match state {
            SasState::KeysExchanged { emojis, .. } => {
                match emojis {
                    Some(emojis) => {
                        let list: Vec<_> = emojis
                            .emojis
                            .iter()
                            .map(|item| {
                                json!({ "symbol": item.symbol, "description": item.description })
                            })
                            .collect();
                        sink.emit(event(
                            "verification.emoji",
                            json!({ "flowId": flow_id, "emoji": list }),
                        ));
                    }
                    None => {
                        // The other side did not agree to the emoji method.
                        let _ = sas.cancel().await;
                        sink.emit(event(
                            "verification.failed",
                            json!({
                                "flowId": flow_id,
                                "message": "the other device did not offer emoji comparison",
                            }),
                        ));
                        break;
                    }
                }
            }
            SasState::Done { .. } => {
                sink.emit(event("verification.done", json!({ "flowId": flow_id })));
                break;
            }
            SasState::Cancelled(_) => {
                sink.emit(event("verification.cancelled", json!({ "flowId": flow_id })));
                break;
            }
            _ => {}
        }

        match changes.next().await {
            Some(next) => state = next,
            None => break,
        }
    }

    release(&slot, &flow_id).await;
}

/// Publishes the SAS stage, without any identifier.
fn report_sas_state(sink: &Arc<Sink>, flow_id: &str, state: &SasState) {
    let name = match state {
        SasState::Created { .. } => "sas-created",
        SasState::Started { .. } => "sas-started",
        SasState::Accepted { .. } => "sas-accepted",
        SasState::KeysExchanged { .. } => "sas-keys",
        SasState::Confirmed => "sas-confirmed",
        SasState::Done { .. } => "sas-done",
        SasState::Cancelled(_) => "sas-cancelled",
    };
    sink.emit(event(
        "verification.stage",
        json!({ "flowId": flow_id, "stage": name }),
    ));
}

/// Where the runtime keeps the verification that is currently on screen.
pub type Slot = Arc<Mutex<Option<Arc<ActiveVerification>>>>;

/// Starts a verification with another user or one's own device. Whoever asks
/// drives the SAS instead of waiting.
pub async fn request(
    client: &Client,
    sink: Arc<Sink>,
    slot: Slot,
    user_id: &str,
) -> Result<Option<OwnedRoomId>, String> {
    let user_id = UserId::parse(user_id).map_err(|_| "not a user identifier".to_owned())?;

    // Local store first, then the homeserver: without the second step a user whose
    // keys were never fetched reads as "has no identity".
    let encryption = client.encryption();
    let identity = match encryption
        .get_user_identity(&user_id)
        .await
        .map_err(|error| format!("could not look up the user: {error}"))?
    {
        Some(identity) => identity,
        None => encryption
            .request_user_identity(&user_id)
            .await
            .map_err(|error| format!("could not fetch the user's identity: {error}"))?
            .ok_or_else(|| {
                "this user has no encryption identity yet — they need to set up \
                 encryption (key backup) in their client first"
                    .to_owned()
            })?,
    };

    let request = identity
        .request_verification()
        .await
        .map_err(|error| format!("could not ask for verification: {error}"))?;

    // The room the flow runs in, from the request rather than looked up: a room
    // counts as a direct chat only once `m.direct` has come back through a sync.
    let room_id = request.room_id().map(|room| room.to_owned());

    tokio::spawn(track(sink, request, slot));
    Ok(room_id)
}

/// Logs that a verification message reached this device: a stall is either "it
/// never arrives" or "it arrives and nothing reacts".
fn note_arrival(sink: &Arc<Sink>, kind: &str) {
    sink.emit(event(
        "verification.stage",
        json!({ "stage": format!("received-{kind}") }),
    ));
}

/// Registers the handlers for incoming requests: to-device events, or for
/// in-room verification a message with its own msgtype.
pub fn install(client: &Client, sink: Arc<Sink>, slot: Slot) {
    client.add_event_handler({
        let sink = sink.clone();
        let slot = slot.clone();
        move |ev: ToDeviceKeyVerificationRequestEvent, client: Client| {
            let sink = sink.clone();
            let slot = slot.clone();
            async move {
                if let Some(request) = client
                    .encryption()
                    .get_verification_request(&ev.sender, &ev.content.transaction_id)
                    .await
                {
                    // matrix-sdk awaits handlers inline while processing a sync, so anything long
                    // has to be spawned - awaiting wedges the encryption sync loop.
                    tokio::spawn(track(sink, request, slot));
                }
            }
        }
    });

    // Probes for the rest of the to-device flow. They only observe.
    client.add_event_handler({
        let sink = sink.clone();
        move |_: ToDeviceKeyVerificationStartEvent| {
            let sink = sink.clone();
            async move { note_arrival(&sink, "start") }
        }
    });

    client.add_event_handler({
        let sink = sink.clone();
        move |_: ToDeviceKeyVerificationAcceptEvent| {
            let sink = sink.clone();
            async move { note_arrival(&sink, "accept") }
        }
    });

    client.add_event_handler({
        let sink = sink.clone();
        move |_: ToDeviceKeyVerificationKeyEvent| {
            let sink = sink.clone();
            async move { note_arrival(&sink, "key") }
        }
    });

    client.add_event_handler({
        let sink = sink.clone();
        move |_: ToDeviceKeyVerificationCancelEvent| {
            let sink = sink.clone();
            async move { note_arrival(&sink, "cancel") }
        }
    });

    client.add_event_handler({
        let sink = sink.clone();
        let slot = slot.clone();
        move |ev: OriginalSyncRoomMessageEvent, client: Client| {
            let sink = sink.clone();
            let slot = slot.clone();
            async move {
                if !matches!(&ev.content.msgtype, MessageType::VerificationRequest(_)) {
                    return;
                }
                match client
                    .encryption()
                    .get_verification_request(&ev.sender, &ev.event_id)
                    .await
                {
                    Some(request) => {
                        note_arrival(&sink, "room-request");
                        tokio::spawn(track(sink, request, slot));
                    }
                    // The crypto layer ignores an in-room request when the sender's device list
                    // was never fetched. Fetching the identity now makes the next attempt land.
                    None => {
                        note_arrival(&sink, "room-request-unmatched");
                        let encryption = client.encryption();
                        let sender = ev.sender.clone();
                        tokio::spawn(async move {
                            let _ = encryption.request_user_identity(&sender).await;
                        });
                    }
                }
            }
        }
    });
}
