//! The open room and thread: timeline, sending, media, receipts.

use super::*;

pub(super) async fn open_timeline(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    focus: String,
    receipts: bool,
    token: String,
    rebuild: bool,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    // One open at a time: leaving a room and stepping back in puts a close and an
    // open in flight together. Its own lock, so timeline commands stay free.
    let _opening = state.opening.lock().await;

    // Opening another room replaces the timeline. The guard is bound to a name:
    // as an `if let` temporary it lives to the end of the body and deadlocks.
    let (marker, fallback_marker) = timeline::own_read_marker(&client, &room_id).await;
    // Permissions and the two-party peer, read before the timeline lock: store
    // reads under that lock froze every room switch once.
    let (permissions, direct_peer) = match matrix_sdk::ruma::RoomId::parse(&room_id)
        .ok()
        .and_then(|parsed| client.get_room(&parsed))
    {
        Some(room) => (
            members::room_permissions(&client, &room).await,
            members::direct_peer(&client, &room).await,
        ),
        None => (json!({}), None),
    };

    {
        let mut open = state.timeline.lock().await;
        if let Some(previous) = open.take() {
            // The receipt setting is chosen when the timeline is built, so a
            // changed one has to rebuild even for the room already open.
            if previous.room_id() == room_id
                && focus.is_empty()
                && !rebuild
                && previous.is_live()
                && previous.tracks_receipts() == receipts
            {
                open.replace(previous);
                state.sink.emit(reply_ok(
                    id,
                    json!({
                        "open": true,
                        "readMarker": marker,
                        "readReceipt": fallback_marker,
                        "rebuilt": false,
                        "can": permissions,
                        "directWith": direct_peer,
                    }),
                ));
                return;
            }
            previous.close().await;
        }
    }

    // Past the early return, so re-entering a room keeps its thread - a stream
    // left running would emit `thread.diff` for the room just left.
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }

    // Sliding sync sends one event per response for rooms in the list. A room
    // being read has to be subscribed, or newer messages never arrive.
    if let Ok(parsed) = matrix_sdk::ruma::RoomId::parse(&room_id) {
        state
            .subscribe(move |rooms| rooms.open = Some(parsed))
            .await;
    }

    match timeline::open(&client, &room_id, &focus, receipts, &token, state.sink.clone()).await {
        Ok(handle) => {
            *state.timeline.lock().await = Some(Arc::new(handle));
            // Rebuilt, so the view starts from nothing and may open where
            // reading stopped; the branch above kept the rows it had.
            state.sink.emit(reply_ok(
                id,
                json!({
                    "open": true,
                    "readMarker": marker,
                    // The branch that matters: a room opened for the first time rebuilds, and
                    // only a re-entered one took the branch that already carried this.
                    "readReceipt": fallback_marker,
                    "rebuilt": true,
                    "can": permissions,
                    "directWith": direct_peer,
                }),
            ));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// A tapped Matrix link: which room is meant, and are we in it.
pub(super) async fn resolve_room(state: &Arc<State>, id: u64, address: String) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::resolve(&client, &address).await {
        Ok(mut data) => {
            if let Some(object) = data.as_object_mut() {
                object.insert("address".to_owned(), json!(address));
            }
            state.sink.emit(reply_ok(id, data));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// The chat list's "mark as read": no timeline is opened for it, so it cannot
/// go through the open handle the way the room's own does.
pub(super) async fn mark_room_read(state: &Arc<State>, id: u64, room_id: String, receipt: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::mark_read(&client, &room_id, receipt).await {
        Ok(marked) => state
            .sink
            .emit(reply_ok(id, json!({ "roomId": room_id, "marked": marked }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn close_timeline(state: &Arc<State>, id: u64, room_id: String) {
    // The lock the open takes: leaving one room and entering another puts both
    // in flight, and the close used to take the fresh handle away.
    let _opening = state.opening.lock().await;
    if !room_id.is_empty() {
        let open_room = state
            .timeline
            .lock()
            .await
            .as_ref()
            .map(|handle| handle.room_id.clone());
        if let Some(open_room) = open_room {
            if open_room != room_id {
                state.sink.emit(reply_ok(id, json!({ "open": true })));
                return;
            }
        }
    }
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close().await;
    }
    // A thread never outlives its room's view.
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }
    state.sink.emit(reply_ok(id, json!({ "open": false })));
}
pub(super) async fn open_thread(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    root_event_id: String,
    token: String,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let _opening = state.opening_thread.lock().await;

    if let Some(previous) = state.thread.lock().await.take() {
        previous.close().await;
    }

    match timeline::open_thread(&client, &room_id, &root_event_id, &token, state.sink.clone()).await
    {
        Ok(handle) => {
            *state.thread.lock().await = Some(Arc::new(handle));
            state.sink.emit(reply_ok(id, json!({ "open": true })));
        }
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// Closes the open thread. A close naming another thread is ignored: commands
/// are independent tasks and can arrive in either order.
pub(super) async fn close_thread(state: &Arc<State>, id: u64, root_event_id: String) {
    let mut open = state.thread.lock().await;
    let matches = open
        .as_ref()
        .map(|handle| root_event_id.is_empty() || handle.thread_root() == root_event_id)
        .unwrap_or(false);
    if matches {
        if let Some(handle) = open.take() {
            handle.close().await;
        }
    }
    drop(open);
    state.sink.emit(reply_ok(id, json!({ "open": false })));
}
pub(super) async fn send_thread_message(state: &Arc<State>, id: u64, body: String, mentions: Vec<String>) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "nothing to send"));
        return;
    }

    let outcome = match state.thread().await {
        Some(handle) => {
            let content = mention_content(state, handle.room_id(), body, &mentions).await;
            handle.send_content(content).await
        }
        None => Err("no thread is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn paginate_thread(state: &Arc<State>, id: u64) {
    let outcome = match state.thread().await {
        Some(handle) => handle.paginate().await,
        None => Err("no thread is open".to_owned()),
    };

    match outcome {
        Ok(paginated) => state.sink.emit(reply_ok(
            id,
            json!({ "reachedStart": matches!(paginated, timeline::Paginated::Start) }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn paginate_timeline(state: &Arc<State>, id: u64, room_id: String) {
    // Room and handle from the same guard: a switch while this was in flight
    // otherwise paginated the room the user had just moved to, on behalf of the
    // one they left - one uninvited history request per switch, against a server
    // that may be rate-limiting.
    let open = state
        .timeline()
        .await
        .map(|handle| (handle.room_id.clone(), handle));
    let outcome = match open {
        Some((open_room, _)) if !room_id.is_empty() && open_room != room_id => {
            Err("the room this was asked for is no longer open".to_owned())
        }
        Some((_, handle)) => handle.paginate().await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(paginated) => state.sink.emit(reply_ok(
            id,
            json!({
                "reachedStart": matches!(paginated, timeline::Paginated::Start),
            }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn send_message(state: &Arc<State>, id: u64, body: String, mentions: Vec<String>) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "nothing to send"));
        return;
    }

    let outcome = match state.sendable_timeline().await {
        Ok(handle) => {
            let content = mention_content(state, handle.room_id(), body, &mentions).await;
            handle.send_content(content).await
        }
        Err(message) => Err(message),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn reply_message(
    state: &Arc<State>,
    id: u64,
    event_id: String,
    body: String,
    mentions: Vec<String>,
) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "a reply cannot be empty"));
        return;
    }

    let outcome = match state.sendable_timeline().await {
        Ok(handle) => {
            let content = mention_content(state, handle.room_id(), body, &mentions).await;
            handle.reply_content(&event_id, content).await
        }
        Err(message) => Err(message),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn edit_message(state: &Arc<State>, id: u64, event_id: String, body: String) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "an edit cannot be empty"));
        return;
    }

    let outcome = match state.timeline().await {
        Some(handle) => handle.edit(&event_id, body).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "edited": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn react(state: &Arc<State>, id: u64, event_id: String, key: String) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.toggle_reaction(&event_id, &key).await,
        None => Err("no timeline is open".to_owned()),
    };
    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "reacted": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn redact_message(state: &Arc<State>, id: u64, event_id: String, txn_id: String) {
    let outcome = match state.timeline().await {
        Some(handle) => handle.redact(&event_id, &txn_id).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "deleted": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// Zero means "not measured". A half-known pair is no measurement either: a
/// client told a width and no height lays out worse than one told nothing.
pub(super) fn dimensions(width: u64, height: u64) -> Option<(u64, u64)> {
    if width > 0 && height > 0 {
        Some((width, height))
    } else {
        None
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn send_media(
    state: &Arc<State>,
    id: u64,
    path: String,
    mime_type: String,
    caption: String,
    reply_to: String,
    voice: bool,
    duration: u64,
    width: u64,
    height: u64,
    still: MediaStill,
    room_id: String,
) {
    // Cloned out of the guard: the attachment upload takes a while and must
    // not hold the lock. The room is read from the same handle, under the same
    // guard, so the check below and the send cannot be about two rooms.
    let (open_room, timeline) = match state.sendable_timeline().await {
        Ok(handle) => (handle.room_id.clone(), handle.timeline()),
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };

    // A video's still is decoded before the command is sent, which takes long
    // enough for the user to be in another room by then. Sending into whatever
    // is open would put a private video in the wrong conversation, and nothing
    // takes that back - so this refuses rather than guesses.
    if !room_id.is_empty() && room_id != open_room {
        state.sink.emit(reply_error(
            id,
            "the room this attachment was meant for is no longer open",
        ));
        return;
    }

    // One field, two lengths: a recording of one's own is a voice message, a
    // video is a video. Which one it is the type says, not the number.
    let length = if duration > 0 { Some(duration) } else { None };
    let voice = if voice { Some(duration) } else { None };
    let dimensions = dimensions(width, height);
    match media::send(
        &timeline,
        &path,
        &mime_type,
        &caption,
        &reply_to,
        voice,
        dimensions,
        length,
        &still,
    )
    .await
    {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "sent": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// Forwards either a picture or a piece of text to another room, without
/// disturbing the timeline that is currently open.
pub(super) async fn forward(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    body: String,
    path: String,
    mime_type: String,
    width: u64,
    height: u64,
    still: MediaStill,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    let outcome = if !path.is_empty() {
        let mime = if mime_type.is_empty() {
            "application/octet-stream".to_owned()
        } else {
            mime_type
        };
        media::forward_file(
            &client,
            &room_id,
            &path,
            &mime,
            dimensions(width, height),
            None,
            &still,
        )
        .await
    } else if !body.trim().is_empty() {
        media::forward_text(&client, &room_id, body).await
    } else {
        Err("nothing to forward".to_owned())
    };

    match outcome {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "forwarded": true }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn fetch_media(
    state: &Arc<State>,
    id: u64,
    source: Value,
    thumbnail: bool,
    size: u64,
    limit: u64,
) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };

    match media::fetch(&client, &state.paths.media_cache, source, thumbnail, size, limit).await {
        Ok(path) => state.sink.emit(reply_ok(id, json!({ "path": path }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
pub(super) async fn mark_read(state: &Arc<State>, id: u64, receipt: bool) {
    // The room goes back with the answer: a receipt does not reliably return as a
    // diff, and the badge may only be cleared for the room this marked.
    let handle = state.timeline().await;
    let room_id = handle
        .as_ref()
        .map(|handle| handle.room_id().to_owned())
        .unwrap_or_default();
    let outcome = match handle {
        Some(handle) => handle.mark_read(receipt).await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(read) => state
            .sink
            .emit(reply_ok(id, json!({ "read": read, "roomId": room_id }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
