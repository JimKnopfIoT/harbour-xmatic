//! Call plumbing between the call module and the open room.

use super::*;

/// Keeps the room of a call in the subscription set. Only the outgoing side
/// and answering pass here; an idle invitation is read unsubscribed.
pub(super) async fn set_call_room(state: &Arc<State>, room_id: Option<&str>) {
    let parsed = room_id.and_then(|room| RoomId::parse(room).ok());
    if room_id.is_some() && parsed.is_none() {
        return;
    }
    state.subscribe(move |rooms| rooms.call = parsed).await;
}
pub(super) async fn hold_call_room(state: &Arc<State>, room_id: &str, call_id: &str) {
    *state
        .call_subscribed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(call_id.to_owned());
    set_call_room(state, Some(room_id)).await;
}

pub(super) async fn release_call_room(state: &Arc<State>, call_id: &str) {
    let mine = {
        let mut current = state
            .call_subscribed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mine = current.as_deref() == Some(call_id);
        if mine {
            *current = None;
        }
        mine
    };
    if mine {
        set_call_room(state, None).await;
    }
}

pub(super) async fn call_step<F, Fut>(state: &Arc<State>, id: u64, action: F)
where
    F: FnOnce(Client) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match action(client).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn turn_servers(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match call::turn_servers(&client).await {
        Ok(servers) => state.sink.emit(reply_ok(id, servers)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
