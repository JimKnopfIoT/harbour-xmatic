//! Key backup, recovery, room keys and verification requests.

use super::*;

pub(super) async fn encryption_status(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    let status = recovery::status(&client).await;
    state.sink.emit(reply_ok(id, status));
}
pub(super) async fn encryption_recover(state: &Arc<State>, id: u64, key: Secret) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match recovery::recover(&client, key.as_str()).await {
        Ok(()) => {
            let status = recovery::status(&client).await;
            state.sink.emit(reply_ok(id, status.clone()));
            state.sink.emit(event("encryption.changed", status));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn encryption_enable_backup(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match recovery::enable(&client).await {
        Ok(key) => {
            // The recovery key is shown once and never stored: writing it down
            // is the user's job, and keeping a copy here would defeat it.
            state.sink.emit(reply_ok(id, json!({ "recoveryKey": key })));
            let status = recovery::status(&client).await;
            state.sink.emit(event("encryption.changed", status));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn fetch_room_keys(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match recovery::fetch_room_keys(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "fetched": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn request_verification(state: &Arc<State>, id: u64, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // An empty user means "my own other devices".
    let target = if user_id.trim().is_empty() {
        match client.user_id() {
            Some(own) => own.as_str().to_owned(),
            None => {
                state.sink.emit(reply_error(id, "own user is unknown"));
                return;
            }
        }
    } else {
        user_id.trim().to_owned()
    };

    // A stale flow has to be taken down before a new one is asked for, or the
    // SDK cancels both and the retry is dead on arrival.
    verification::cancel_active(&state.verification).await;

    match verification::request(
        &client,
        state.sink.clone(),
        state.verification.clone(),
        &target,
    )
    .await
    {
        Ok(room_id) => {
            // Verifying another user runs in the direct chat, and an unsubscribed room
            // delivers one event per sync - the acceptance would look like a stall.
            if let Some(room_id) = room_id {
                state
                    .subscribe(move |rooms| rooms.verification = Some(room_id))
                    .await;
            }
            state.sink.emit(reply_ok(id, json!({ "requested": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

/// The three things the user can do with a verification on screen.
pub(super) enum Step {
    Accept,
    Confirm,
    Cancel,
    Mismatch,
}
pub(super) async fn verification_step(state: &Arc<State>, id: u64, step: Step) {
    let active = state.verification.lock().await.clone();
    let Some(active) = active else {
        state
            .sink
            .emit(reply_error(id, "no verification is in progress"));
        return;
    };

    let outcome = match step {
        Step::Accept => active.accept().await,
        Step::Confirm => active.confirm().await,
        Step::Cancel => active.cancel().await,
        Step::Mismatch => active.mismatch().await,
    };

    match outcome {
        Ok(()) => state
            .sink
            .emit(reply_ok(id, json!({ "flowId": active.flow_id() }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
