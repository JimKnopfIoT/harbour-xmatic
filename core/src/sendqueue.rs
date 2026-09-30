//! Parked send requests and the two ways out.
//! A parked request blocks every later one in its room.

use std::sync::Arc;

use matrix_sdk::{
    ruma::{
        events::{room::message::Relation, AnyMessageLikeEventContent},
        RoomId,
    },
    send_queue::{LocalEcho, LocalEchoContent},
    Client, QueueWedgeError,
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
        let (kind, error) = match &echo.content {
            LocalEchoContent::Event { serialized_event, send_error: Some(error), .. } => {
                (event_kind(serialized_event.deserialize().ok()), error.clone())
            }
            LocalEchoContent::Redaction { send_error: Some(error), .. } => ("deletion", error.clone()),
            _ => continue,
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
            _ => "message",
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
        _ => "other",
    }
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
    fn a_duplicate_reaction_is_already_there() {
        let duplicate = QueueWedgeError::GenericApiError {
            msg: "the server returned an error: [400 / M_DUPLICATE_ANNOTATION] Can't send same reaction twice"
                .to_owned(),
        };
        assert!(already_there(&duplicate));
        assert!(!already_there(&QueueWedgeError::CrossVerificationRequired));
    }
}
