//! The open room and thread: timeline, sending, media, receipts.

use super::*;
use std::sync::atomic::Ordering;

/// How long building a view may wait for the server. `/context`, the pinned
/// events and the encryption state go out with the SDK's unbounded retry.
const OPEN_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A view still being built outside the lock, so a later open or a close can
/// stop it. `key` is the room, or for a thread its root.
pub(super) struct PendingOpen {
    key: String,
    token: String,
    cancel: Arc<tokio::sync::Notify>,
}

/// What building a view came to.
enum Built {
    View(timeline::TimelineHandle),
    Failed(String),
    TimedOut,
    Cancelled,
}

/// Runs the request half under the time limit and the cancel signal, then the
/// local half to its end: a task cut off there would leak streams holding the client.
fn spawn_view<B, S>(
    cancel: Arc<tokio::sync::Notify>,
    build: B,
    start: impl FnOnce(timeline::BuiltView) -> S + Send + 'static,
) -> tokio::task::JoinHandle<Built>
where
    B: std::future::Future<Output = Result<timeline::BuiltView, String>> + Send + 'static,
    S: std::future::Future<Output = timeline::TimelineHandle> + Send + 'static,
{
    tokio::spawn(async move {
        let built = tokio::select! {
            built = tokio::time::timeout(OPEN_LIMIT, build) => built,
            _ = cancel.notified() => return Built::Cancelled,
        };
        match built {
            Err(_) => Built::TimedOut,
            Ok(Err(message)) => Built::Failed(message),
            Ok(Ok(view)) => Built::View(start(view).await),
        }
    })
}

/// The handle, or the reply that says why there is none.
fn built_handle(built: Result<Built, tokio::task::JoinError>) -> Result<timeline::TimelineHandle, String> {
    match built {
        Ok(Built::View(handle)) => Ok(handle),
        Ok(Built::Failed(message)) => Err(message),
        Ok(Built::TimedOut) => Err("the server did not answer in time".to_owned()),
        Ok(Built::Cancelled) => Err(SUPERSEDED.to_owned()),
        Err(_) => Err("the command failed unexpectedly".to_owned()),
    }
}

/// Whether a close meant this view. The token decides; a close without one
/// (an older front end) goes by the key, and an empty key means any.
fn same_view(close_key: &str, close_token: &str, key: &str, token: &str) -> bool {
    if !close_token.is_empty() {
        close_token == token
    } else {
        close_key.is_empty() || close_key == key
    }
}

fn pending(slot: &std::sync::Mutex<Option<PendingOpen>>) -> std::sync::MutexGuard<'_, Option<PendingOpen>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Stops whatever is still building, room and thread; nothing builds into a
/// session that is ending.
pub(super) fn abandon_opens(state: &Arc<State>) {
    state.open_generation.fetch_add(1, Ordering::SeqCst);
    state.thread_generation.fetch_add(1, Ordering::SeqCst);
    for slot in [&state.pending_open, &state.pending_thread] {
        if let Some(previous) = pending(slot).take() {
            previous.cancel.notify_one();
        }
    }
}

/// Closes the thread and stops one still building, under the lock its install takes.
async fn drop_thread(state: &Arc<State>) {
    let _opening = state.opening_thread.lock().await;
    state.thread_generation.fetch_add(1, Ordering::SeqCst);
    if let Some(building) = pending(&state.pending_thread).take() {
        building.cancel.notify_one();
    }
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }
}

/// Answer for an open a later one replaced: the front end has moved on.
pub(super) const SUPERSEDED: &str = "superseded by a later open";

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

    // A later open wins: what an earlier one is still fetching stops here.
    let generation = state.open_generation.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(previous) = pending(&state.pending_open).take() {
        previous.cancel.notify_one();
    }

    // One open at a time: leaving a room and stepping back in puts a close and an
    // open in flight together. Its own lock, so timeline commands stay free.
    let opening = state.opening.lock().await;
    if state.open_generation.load(Ordering::SeqCst) != generation {
        state.sink.emit(reply_error(id, SUPERSEDED));
        return;
    }

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
    let opened = json!({
        "open": true,
        "readMarker": marker,
        "readReceipt": fallback_marker,
        "rebuilt": true,
        "can": permissions,
        "directWith": direct_peer,
    });

    {
        let mut open = state.timeline.lock().await;
        if let Some(previous) = open.take() {
            // The receipt setting is chosen when the timeline is built, so a
            // changed one has to rebuild even for the room already open. Another
            // token is another view: its stream would be dropped as foreign.
            if previous.room_id() == room_id
                && focus.is_empty()
                && !rebuild
                && previous.is_live()
                && previous.tracks_receipts() == receipts
                && previous.token() == token
            {
                open.replace(previous);
                let mut kept = opened;
                kept["rebuilt"] = json!(false);
                state.sink.emit(reply_ok(id, kept));
                return;
            }
            previous.close().await;
        }
    }

    // Past the early return, so re-entering a room keeps its thread - a stream
    // left running would emit `thread.diff` for the room just left.
    drop_thread(state).await;

    // Sliding sync sends one event per response for rooms in the list. A room
    // being read has to be subscribed, or newer messages never arrive.
    if let Ok(parsed) = matrix_sdk::ruma::RoomId::parse(&room_id) {
        state
            .subscribe(move |rooms| rooms.open = Some(parsed))
            .await;
    }

    // A slice or the pinned view is a request, and so is the live view of a room
    // whose encryption state is unknown: built outside the lock, bounded, stoppable.
    let cancel = Arc::new(tokio::sync::Notify::new());
    let build = {
        let (client, room_id, focus) = (client.clone(), room_id.clone(), focus.clone());
        async move { timeline::build(&client, &room_id, &focus, receipts).await }
    };
    let start = {
        let (client, room_id, focus, token) =
            (client.clone(), room_id.clone(), focus.clone(), token.clone());
        let sink = state.sink.clone();
        move |view| async move {
            timeline::start(&client, &room_id, &focus, receipts, &token, sink, view).await
        }
    };
    let build = spawn_view(cancel.clone(), build, start);
    {
        let mut slot = pending(&state.pending_open);
        // Replaced between the bump and here: this one is already stale.
        if state.open_generation.load(Ordering::SeqCst) != generation {
            cancel.notify_one();
        } else {
            *slot = Some(PendingOpen { key: room_id.clone(), token: token.clone(), cancel });
        }
    }
    // Registered before the lock goes, so a close that follows finds it.
    drop(opening);
    let outcome = build.await;

    let _opening = state.opening.lock().await;
    let current = state.open_generation.load(Ordering::SeqCst) == generation;
    if current {
        pending(&state.pending_open).take();
    }
    // Replaced meanwhile: whatever came of it, nobody is waiting for it.
    let handle = match (built_handle(outcome), current) {
        (Ok(handle), true) => handle,
        (Ok(handle), false) => {
            handle.close().await;
            state.sink.emit(reply_error(id, SUPERSEDED));
            return;
        }
        (Err(message), current) => {
            state.sink.emit(reply_error(id, if current { message } else { SUPERSEDED.to_owned() }));
            return;
        }
    };
    *state.timeline.lock().await = Some(Arc::new(handle));
    state.sink.emit(reply_ok(id, opened));
}
/// A tapped Matrix link: which room is meant, and are we in it.
pub(super) async fn resolve_room(state: &Arc<State>, id: u64, address: String, local: bool) {
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    match roomlist::resolve(&client, &address, local).await {
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
pub(super) async fn close_timeline(state: &Arc<State>, id: u64, room_id: String, token: String) {
    // The lock the open takes: leaving one room and entering another puts both
    // in flight, and the close used to take the fresh handle away.
    let _opening = state.opening.lock().await;
    let open = state
        .timeline
        .lock()
        .await
        .as_ref()
        .map(|handle| (handle.room_id.clone(), handle.token().to_owned()));
    match open {
        Some((open_room, open_token)) => {
            if !same_view(&room_id, &token, &open_room, &open_token) {
                state.sink.emit(reply_ok(id, json!({ "open": true })));
                return;
            }
        }
        // Nothing installed; a view still building is stopped only if it is this one.
        None => {
            let mut slot = pending(&state.pending_open);
            let meant = slot
                .as_ref()
                .map(|building| same_view(&room_id, &token, &building.key, &building.token))
                .unwrap_or(false);
            if meant {
                state.open_generation.fetch_add(1, Ordering::SeqCst);
                if let Some(building) = slot.take() {
                    building.cancel.notify_one();
                }
            }
        }
    }
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close().await;
    }
    // A thread never outlives its room's view.
    drop_thread(state).await;
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

    // Same shape as a focused room open: the thread's history is a request.
    let generation = state.thread_generation.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(previous) = pending(&state.pending_thread).take() {
        previous.cancel.notify_one();
    }
    let opening = state.opening_thread.lock().await;
    if let Some(previous) = state.thread.lock().await.take() {
        previous.close().await;
    }

    let cancel = Arc::new(tokio::sync::Notify::new());
    let build = {
        let (client, room_id, root) = (client.clone(), room_id.clone(), root_event_id.clone());
        async move { timeline::build_thread(&client, &room_id, &root).await }
    };
    let start = {
        let (client, room_id, root, token) =
            (client.clone(), room_id.clone(), root_event_id.clone(), token.clone());
        let sink = state.sink.clone();
        move |view| async move {
            timeline::start_thread(&client, &room_id, &root, &token, sink, view).await
        }
    };
    let build = spawn_view(cancel.clone(), build, start);
    {
        let mut slot = pending(&state.pending_thread);
        if state.thread_generation.load(Ordering::SeqCst) != generation {
            cancel.notify_one();
        } else {
            *slot = Some(PendingOpen { key: root_event_id.clone(), token: token.clone(), cancel });
        }
    }
    drop(opening);
    let outcome = build.await;

    let _opening = state.opening_thread.lock().await;
    let current = state.thread_generation.load(Ordering::SeqCst) == generation;
    if current {
        pending(&state.pending_thread).take();
    }
    // Another thread, the room's close or a new room came first.
    let handle = match (built_handle(outcome), current) {
        (Ok(handle), true) => handle,
        (Ok(handle), false) => {
            handle.close().await;
            state.sink.emit(reply_error(id, SUPERSEDED));
            return;
        }
        (Err(message), current) => {
            state.sink.emit(reply_error(id, if current { message } else { SUPERSEDED.to_owned() }));
            return;
        }
    };
    if let Some(previous) = state.thread.lock().await.replace(Arc::new(handle)) {
        previous.close().await;
    }
    state.sink.emit(reply_ok(id, json!({ "open": true })));
}
/// Closes the open thread. A close naming another thread is ignored: commands
/// are independent tasks and can arrive in either order.
pub(super) async fn close_thread(state: &Arc<State>, id: u64, root_event_id: String) {
    // The lock the open installs under, so a close cannot slip between its check and it.
    let _opening = state.opening_thread.lock().await;
    {
        let mut slot = pending(&state.pending_thread);
        let meant = slot
            .as_ref()
            .map(|building| same_view(&root_event_id, "", &building.key, &building.token))
            .unwrap_or(false);
        if meant {
            state.thread_generation.fetch_add(1, Ordering::SeqCst);
            if let Some(building) = slot.take() {
                building.cancel.notify_one();
            }
        }
    }
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
pub(super) async fn paginate_timeline(state: &Arc<State>, id: u64, room_id: String, forwards: bool) {
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
        Some((_, handle)) if forwards => handle.paginate_forwards().await,
        Some((_, handle)) => handle.paginate().await,
        None => Err("no timeline is open".to_owned()),
    };

    match outcome {
        Ok(paginated) => state.sink.emit(reply_ok(
            id,
            json!({
                "forwards": forwards,
                "reachedStart": matches!(paginated, timeline::Paginated::Start),
                "reachedEnd": matches!(paginated, timeline::Paginated::End),
            }),
        )),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}
/// The front end's rows drifted from the core's: the view named by its token
/// sends a full reset. Another view's token is ignored, not an error.
pub(super) async fn resync_timeline(
    state: &Arc<State>,
    id: u64,
    room_id: String,
    token: String,
    thread: bool,
) {
    let handle = if thread { state.thread().await } else { state.timeline().await };
    let meant = handle
        .as_ref()
        .map(|handle| handle.room_id() == room_id && handle.token() == token)
        .unwrap_or(false);
    if let (true, Some(handle)) = (meant, handle) {
        handle.resync();
    }
    state.sink.emit(reply_ok(id, json!({ "resync": meant })));
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
pub(super) async fn edit_message(
    state: &Arc<State>,
    id: u64,
    event_id: String,
    body: String,
    mentions: Vec<String>,
) {
    if body.trim().is_empty() {
        state.sink.emit(reply_error(id, "an edit cannot be empty"));
        return;
    }

    let outcome = match state.timeline().await {
        Some(handle) => {
            let client = state.client().await;
            handle.edit(client.as_ref(), &event_id, body, &mentions).await
        }
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
    mentions: Vec<String>,
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
    let caption = caption.trim();
    let caption = if caption.is_empty() {
        None
    } else {
        let client = state.client().await;
        Some(mention::caption_content(client.as_ref(), &open_room, caption, &mentions).await)
    };
    match media::send(
        &timeline,
        &path,
        &mime_type,
        caption,
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

#[cfg(test)]
mod tests {
    use super::same_view;

    #[test]
    fn a_close_with_a_token_means_only_that_view() {
        assert!(same_view("!a:x", "6", "!a:x", "6"));
        // Leaving and re-entering one room: the late close of the old view
        // must not take the new one, although the room is the same.
        assert!(!same_view("!a:x", "5", "!a:x", "6"));
    }

    #[test]
    fn a_close_without_a_token_goes_by_the_room() {
        assert!(same_view("!a:x", "", "!a:x", "6"));
        assert!(!same_view("!b:x", "", "!a:x", "6"));
        assert!(same_view("", "", "!a:x", "6"));
    }
}
