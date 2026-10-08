//! The room list and the space list, and space membership.

use super::*;

/// The room list's sync service, started if needed: which page runs first is
/// the user's start-page choice. The lock stops two racing starts.
pub(super) async fn ensure_room_list(
    state: &Arc<State>,
) -> Result<std::sync::Arc<matrix_sdk_ui::room_list_service::RoomListService>, String> {
    let mut rooms = state.rooms.lock().await;
    if rooms.is_none() {
        let client = state.client().await.ok_or_else(|| "not signed in".to_owned())?;
        // A store from before the connection was ours to choose may hold a position
        // that outlived a rebuild; it moves on once, here.
        if let Err(error) = session::migrate_sync_connection(&state.paths) {
            log(state, "warn", format!("could not record the sync connection: {error}"));
        }
        let connection_id = session::sync_connection_id(&state.paths);
        let handle = roomlist::start(&client, state.sink.clone(), connection_id).await?;
        *rooms = Some(handle);
    }
    Ok(rooms.as_ref().expect("just ensured").service())
}
pub(super) async fn start_room_list(state: &Arc<State>, id: u64) {
    match ensure_room_list(state).await {
        Ok(_) => {
            // The chat list draws a room's space over its picture, so the map
            // must not wait for the space page to be opened.
            emit_space_children_soon(state);
            state.sink.emit(reply_ok(id, json!({ "running": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn filter_room_list(state: &Arc<State>, id: u64, pattern: String) {
    let applied = match &*state.rooms.lock().await {
        Some(handle) => handle.set_filter(pattern),
        None => false,
    };

    if applied {
        state.sink.emit(reply_ok(id, json!({ "filtered": true })));
    } else {
        state.sink.emit(reply_error(id, "the room list is not running"));
    }
}
/// One more page of rooms. The dynamic adapter holds one page and grows only
/// when told; asking again once everything is loaded does nothing.
pub(super) async fn load_more_rooms(state: &Arc<State>, id: u64) {
    let asked = match &*state.rooms.lock().await {
        Some(handle) => handle.load_more(),
        None => false,
    };

    if asked {
        state.sink.emit(reply_ok(id, json!({ "asked": true })));
    } else {
        state.sink.emit(reply_error(id, "the room list is not running"));
    }
}
pub(super) async fn stop_room_list(state: &Arc<State>, id: u64) {
    // The space streams borrow the same room list service, so they go first.
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }
    if let Some(task) = state.spaces.lock().await.take() {
        task.abort();
    }
    if let Some(handle) = state.rooms.lock().await.take() {
        handle.stop().await;
    }
    state.sink.emit(reply_ok(id, json!({ "running": false })));
}
pub(super) async fn start_spaces(state: &Arc<State>, id: u64) {
    if state.spaces.lock().await.is_some() {
        state.sink.emit(reply_ok(id, json!({ "running": true })));
        return;
    }

    let service = match ensure_room_list(state).await {
        Ok(service) => service,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };

    match roomlist::spawn_spaces(service, state.sink.clone()).await {
        Ok(task) => {
            *state.spaces.lock().await = Some(task);
            state.sink.emit(reply_ok(id, json!({ "running": true })));
            emit_space_children(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// Emits the space child structure now and once more shortly after: a state
/// event written with `send_state_event` lands locally on the next sync.
pub(super) fn emit_space_children_soon(state: &Arc<State>) {
    let state = state.clone();
    tokio::spawn(async move {
        emit_space_children(&state).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        emit_space_children(&state).await;
    });
}
pub(super) async fn stop_spaces(state: &Arc<State>, id: u64) {
    if let Some(task) = state.spaces.lock().await.take() {
        task.abort();
    }
    state.sink.emit(reply_ok(id, json!({ "running": false })));
}
/// Recomputes the per-space child structure for the badges. Called whenever it
/// can have changed - list started, space created, child added or removed.
pub(super) async fn emit_space_children(state: &Arc<State>) {
    if let Some(client) = state.client().await {
        let data = roomlist::space_children_map(&client).await;
        state.sink.emit(event("spaces.children", data));
    }
}
pub(super) async fn open_space(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let service = match ensure_room_list(state).await {
        Ok(service) => service,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };

    // Opening a different space replaces the previous one: the UI shows one
    // space's rooms at a time, mirroring how the timeline is handled.
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }

    match roomlist::spawn_space_children(&client, &room_id, service, state.sink.clone()).await {
        Ok(task) => {
            *state.open_space.lock().await = Some(task);
            state.sink.emit(reply_ok(id, json!({ "open": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn close_space(state: &Arc<State>, id: u64) {
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }
    state.sink.emit(reply_ok(id, json!({ "open": false })));
}
pub(super) async fn create_space(state: &Arc<State>, id: u64, name: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // The new space arrives in the overview through the running sync, so the
    // reply only has to report the id.
    match roomlist::create_space(&client, &name).await {
        Ok(room_id) => {
            state.sink.emit(reply_ok(id, json!({ "roomId": room_id })));
            emit_space_children(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn leave_space(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // The space list updates on its own: leaving turns the room's state to
    // Left, which the "non left" filter drops on the next diff.
    match roomlist::leave_space(&client, &room_id).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "left": true })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn add_space_child(state: &Arc<State>, id: u64, space_id: String, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match roomlist::add_child(&client, &space_id, &room_id).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "added": true })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn remove_space_child(state: &Arc<State>, id: u64, space_id: String, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match roomlist::remove_child(&client, &space_id, &room_id).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "removed": true })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
