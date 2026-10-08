//! A room's members and the moderation on them.

use super::*;

pub(super) async fn members_load(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::load(&client, &room_id).await {
        Ok(rows) => {
            let count = rows.len();
            // One message per batch: every event is serialised, copied over the
            // ABI and parsed again, and a large member list froze the screen.
            const BATCH: usize = 200;
            let mut chunks = rows.chunks(BATCH);
            let first = chunks.next().unwrap_or(&[]);
            state.sink.emit(event(
                "members.diff",
                json!({ "ops": [{ "op": "reset", "values": first }] }),
            ));
            for chunk in chunks {
                state.sink.emit(event(
                    "members.diff",
                    json!({ "ops": [{ "op": "append", "values": chunk }] }),
                ));
            }
            state.sink.emit(reply_ok(id, json!({ "count": count })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn member_remove(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::remove(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "removed": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn member_profile(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::profile(&client, &room_id, &user_id).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn member_ban(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::ban(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "userId": user_id }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn member_unban(state: &Arc<State>, id: u64, room_id: String, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::unban(&client, &room_id, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "userId": user_id }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn member_set_power(state: &Arc<State>, id: u64, room_id: String, user_id: String, power: i64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::set_power(&client, &room_id, &user_id, power).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "userId": user_id, "power": power }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn member_set_ignored(state: &Arc<State>, id: u64, user_id: String, ignored: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::set_ignored(&client, &user_id, ignored).await {
        Ok(()) => state.sink.emit(reply_ok(
            id,
            json!({ "userId": user_id, "ignored": ignored }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn member_withdraw_verification(state: &Arc<State>, id: u64, user_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::withdraw_verification(&client, &user_id).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "userId": user_id }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
