//! UnifiedPush: the connector, the pusher, a woken fetch.

use super::*;

/// The connector, started on demand. One per process; the handle stays.
pub(super) async fn push_handle(state: &Arc<State>) -> ()
{
    let mut slot = state.push.lock().await;
    if slot.is_none() {
        *slot = Some(crate::push::start(
            state.paths.push_file.clone(),
            state.sink.clone(),
        ));
    }
}
/// What UnifiedPush looks like here. Answered as a `push.state` event: the
/// connector lives on its own thread, two channels away.
pub(super) async fn push_status(state: &Arc<State>, id: u64) {
    push_handle(state).await;
    if let Some(handle) = state.push.lock().await.as_ref() {
        handle.status();
    }
    state.sink.emit(reply_ok(id, json!({ "asked": true })));
}
/// Registers with a distributor; the endpoint follows as `push.endpoint`. The
/// gateway is a setting, not the core's to remember.
pub(super) async fn push_enable(state: &Arc<State>, id: u64, gateway: String) {
    if !crate::push::gateway_is_sound(&gateway) {
        state.sink.emit(reply_error(
            id,
            "the push gateway has to be an https address",
        ));
        return;
    }
    if gateway.trim().is_empty() {
        state
            .sink
            .emit(reply_error(id, "no push gateway configured"));
        return;
    }
    push_handle(state).await;
    if let Some(handle) = state.push.lock().await.as_ref() {
        handle.enable();
    }
    state.sink.emit(reply_ok(id, json!({ "enabled": true })));
}
/// Gives the registration back and deletes the pusher. Both: a pusher left
/// behind keeps pointing at an endpoint that no longer exists.
pub(super) async fn push_disable(state: &Arc<State>, id: u64, endpoint: String) {
    // By app id, not by the endpoint: that is never persisted, and a pusher
    // nobody can name keeps the server posting for ever.
    {
        if let Some(client) = state.client().await {
            if !endpoint.is_empty() {
                let _ = crate::push::clear_pusher(&client, &endpoint).await;
            }
            if let Err(error) = crate::push::clear_own_pushers(&client).await {
                // Said, not fatal: the registration goes either way, and a
                // pusher that outlives it only wastes the server's attempts.
                state.sink.emit(event(
                    "push.state",
                    json!({ "state": "off", "error": error }),
                ));
            }
        }
    }
    push_handle(state).await;
    if let Some(handle) = state.push.lock().await.as_ref() {
        handle.disable();
    }
    state.sink.emit(reply_ok(id, json!({ "enabled": false })));
}
/// Turns a push into a banner. Answers with an error where the push rules say
/// not to show it: silence is the right outcome then.
pub(super) async fn push_notify(state: &Arc<State>, id: u64, room_id: String, event_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    // The running sync service where there is one - the woken process has none,
    // and there it really is the only process.
    let sync = state
        .rooms
        .lock()
        .await
        .as_ref()
        .map(|handle| handle.sync.clone());
    match crate::push::notification_for(&client, &room_id, &event_id, sync).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(error) => state.sink.emit(reply_error(id, error)),
    }
}
/// The second half of turning it on: the endpoint goes to the homeserver.
pub(super) async fn push_pusher(
    state: &Arc<State>,
    id: u64,
    endpoint: String,
    p256dh: String,
    auth: String,
    gateway: String,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match crate::push::set_pusher(&client, &endpoint, &p256dh, &auth, &gateway).await {
        Ok(()) => {
            state.sink.emit(event("push.state", json!({ "state": "on" })));
            state.sink.emit(reply_ok(id, json!({ "registered": true })));
        }
        Err(error) => {
            state.sink.emit(event(
                "push.state",
                json!({ "state": "error", "error": error.clone() }),
            ));
            state.sink.emit(reply_error(id, error));
        }
    }
}
