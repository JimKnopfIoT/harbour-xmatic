//! Message search and the mention picker's candidates.

use super::*;

/// Folds the room's stored events into its index before anybody searches it.
pub(super) async fn search_index(state: &Arc<State>, id: u64, room_id: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match search::index_room(&client, &room_id).await {
        // The count goes back in the reply; the journal line is the bridge's,
        // like every other one in this app.
        Ok(count) => state.sink.emit(reply_ok(id, json!({ "count": count }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// One page of search results, in the reply rather than as an event: a late
/// answer to an abandoned search then cannot overwrite a newer one.
pub(super) async fn search_room(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    query: String,
    offset: usize,
    limit: usize,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    // A limit of zero is the front end not saying; a limit of a thousand is
    // the front end saying something unhelpful.
    let limit = limit.clamp(1, SEARCH_PAGE_MAX);
    match search::room(&client, &room_id, &query, limit, offset).await {
        Ok((rows, hits)) => {
            // Counted in hits: an unloadable hit is still a hit.
            let more = hits >= limit;
            state.sink.emit(reply_ok(
                id,
                json!({ "rows": rows, "offset": offset, "more": more }),
            ));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// The message a send command turns into. Without a client the mentions are
/// dropped rather than the message: a body still goes out.
pub(super) async fn mention_content(
    state: &Arc<State>,
    room_id: &str,
    body: String,
    mentions: &[String],
) -> matrix_sdk::ruma::events::room::message::RoomMessageEventContent {
    let client = state.client().await;
    mention::text_content(client.as_ref(), room_id, body, mentions).await
}
pub(super) async fn mention_candidates(state: &Arc<State>, id: u64, room_id: String, query: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match mention::candidates(&client, &room_id, &query).await {
        // The query travels back: the picker has moved on by the time a stale
        // answer lands, and it has no request id to tell them apart by.
        Ok(rows) => state.sink.emit(reply_ok(
            id,
            json!({ "roomId": room_id, "query": query, "candidates": rows }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
