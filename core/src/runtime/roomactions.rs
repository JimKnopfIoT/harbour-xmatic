//! What one does to a room: flags, pins, joins, leaving, invites, creation.

use super::*;

pub(super) async fn room_set_notify_mode(state: &Arc<State>, id: u64, room_id: String, mode: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::set_notification_mode(&client, &room_id, &mode).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "mode": mode, "muted": mode == "mute" }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn room_set_favourite(state: &Arc<State>, id: u64, room_id: String, favourite: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::set_favourite(&client, &room_id, favourite).await {
        Ok(()) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id, "favourite": favourite }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn room_set_low_priority(state: &Arc<State>, id: u64, room_id: String, low_priority: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::set_low_priority(&client, &room_id, low_priority).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "lowPriority": low_priority }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn pin_message(state: &Arc<State>, id: u64, event_id: String, pin: bool) {
    let Some(handle) = state.timeline().await else {
        state.sink.emit(reply_error(id, "no open room"));
        return;
    };
    match handle.set_pinned(&event_id, pin).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "pinned": pin })));
            // The banner and the row markers follow this list; the server's
            // answer already contains the change made a moment ago.
            let (ids, preview) = handle.pinned_info().await;
            state.sink.emit(event(
                "timeline.pinned",
                json!({
                    "roomId": handle.room_id(),
                    "eventIds": ids,
                    "preview": preview,
                }),
            ));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn room_check_recipients(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::unverified_recipients(&client, &room_id).await {
        Ok(users) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id, "users": users }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn room_reset_keys(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match timeline::discard_room_key(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "reset": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn direct_chat(state: &Arc<State>, id: u64, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let _serial = state.opening_direct.lock().await;
    match timeline::direct_chat(&client, &user_id).await {
        Ok(room_id) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn join_by_alias(state: &Arc<State>, id: u64, alias: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::join_by_alias(&client, &alias).await {
        Ok(room_id) => state
            .sink
            .emit(reply_ok(id, json!({ "joined": true, "roomId": room_id }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn join_room(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::join(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "joined": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// Answers with everything the room-info page shows.
pub(super) async fn room_info(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::room_info(&client, &room_id).await {
        Ok(info) => state.sink.emit(reply_ok(id, info)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// Joins the room a tombstoned one points at, and answers with its id so the
/// front end can open it.
pub(super) async fn follow_successor(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::follow_successor(&client, &room_id).await {
        Ok(new_room_id) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": new_room_id }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn create_room(state: &Arc<State>, id: u64, room: roomlist::NewRoom) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // Kept for the reply: the request is consumed by the call below, and the
    // front end opens the room under this name before the first diff arrives.
    let name = room.name.trim().to_owned();
    let encrypted = room.encrypted;

    // The room reaches the list through the sync. The reply carries name and
    // encryption so the front end can open it before the first diff.
    match roomlist::create_room(&client, room).await {
        Ok(room_id) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "name": name, "encrypted": encrypted }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn leave_room(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // The row disappears on its own: leaving turns the state to Left, which the
    // filter drops on the next diff. The holding space loses a child.
    match roomlist::leave_room(&client, &room_id).await {
        Ok(()) => {
            state
                .sink
                .emit(reply_ok(id, json!({ "left": true, "roomId": room_id })));
            emit_space_children_soon(state);
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn invite_to_room(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::invite_user(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "invited": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn enable_room_encryption(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match timeline::enable_encryption(&client, &room_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "encrypted": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
