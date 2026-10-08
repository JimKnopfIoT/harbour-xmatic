//! The public room directory and a space's hierarchy.

use super::*;

pub(super) async fn directory_search(state: &Arc<State>, id: u64, pattern: String, server: Option<String>) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let via_server = match server.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        None => None,
        Some(name) => match matrix_sdk::ruma::OwnedServerName::try_from(name) {
            Ok(name) => Some(name),
            Err(_) => {
                state.sink.emit(reply_error(id, "not a valid server name"));
                return;
            }
        },
    };

    let mut slot = state.directory.lock().await;
    if slot.is_none() {
        let (task, orders) = directory::start(client, state.sink.clone());
        *slot = Some(DirectoryHandle { task, orders });
    }
    let sent = slot
        .as_ref()
        .expect("just ensured")
        .orders
        .send(directory::Order::Search(pattern, via_server))
        .is_ok();
    drop(slot);

    if sent {
        state.sink.emit(reply_ok(id, json!({ "searching": true })));
    } else {
        state.sink.emit(reply_error(id, "the search is not running"));
    }
}
pub(super) async fn directory_more(state: &Arc<State>, id: u64) {
    let slot = state.directory.lock().await;
    let sent = slot
        .as_ref()
        .map(|handle| handle.orders.send(directory::Order::More).is_ok())
        .unwrap_or(false);
    drop(slot);

    if sent {
        state.sink.emit(reply_ok(id, json!({ "searching": true })));
    } else {
        state.sink.emit(reply_error(id, "the search is not running"));
    }
}
pub(super) async fn directory_stop(state: &Arc<State>, id: u64) {
    if let Some(handle) = state.directory.lock().await.take() {
        handle.task.abort();
    }
    state.sink.emit(reply_ok(id, json!({ "searching": false })));
}
pub(super) async fn space_hierarchy(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::space_hierarchy(&client, &room_id).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
