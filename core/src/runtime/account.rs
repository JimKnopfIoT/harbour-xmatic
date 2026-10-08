//! The account itself: profile, display name, avatar, ignored users.

use super::*;

pub(super) async fn account_get(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match profile::get(&client).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn account_set_display_name(state: &Arc<State>, id: u64, name: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match profile::set_display_name(&client, &name).await {
        Ok(()) => {
            state.sink.emit(reply_ok(id, json!({ "saved": true })));
            emit_profile(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn account_set_avatar(state: &Arc<State>, id: u64, path: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match profile::set_avatar(&client, &path).await {
        Ok(data) => {
            state.sink.emit(reply_ok(id, data));
            emit_profile(state).await;
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// Re-reads the profile from the server and pushes it, so every page showing
/// it updates after a change without asking again.
pub(super) async fn emit_profile(state: &Arc<State>) {
    if let Some(client) = state.client().await {
        if let Ok(data) = profile::get(&client).await {
            state.sink.emit(event("profile.changed", data));
        }
    }
}
pub(super) async fn account_ignored_users(state: &Arc<State>, id: u64) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match members::ignored(&client).await {
        Ok(users) => state.sink.emit(reply_ok(id, json!({ "users": users }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
