//! UnifiedPush: the connector, the pusher, the woken process.

use super::*;

use crate::push::PUSH;
use leghorn::matrix::Pusher;
use leghorn::{Error as PushError, Event as PushEvent, Holds, Leghorn};

/// The connector, started here and nowhere else.
async fn push_connector(state: &Arc<State>) -> Result<Arc<Leghorn>, String> {
    let mut slot = state.push.lock().await;
    if let Some(leghorn) = slot.as_ref() {
        return Ok(leghorn.clone());
    }
    let (started, mut events) = crate::push::start().await;
    let leghorn = Arc::new(started?);
    *slot = Some(leghorn.clone());
    drop(slot);
    let weak = Arc::downgrade(state);
    let listener = tokio::spawn(async move {
        while let Some(push) = events.recv().await {
            let Some(state) = weak.upgrade() else {
                return;
            };
            push_event(&state, push).await;
        }
    });
    if let Some(old) = state.push_listener.lock().await.replace(listener) {
        old.abort();
    }
    Ok(leghorn)
}

/// Releases the connector name. The listener ends by itself.
async fn push_stop(state: &Arc<State>) {
    drop(state.push.lock().await.take());
}

async fn running(state: &Arc<State>) -> Option<Arc<Leghorn>> {
    state.push.lock().await.clone()
}

fn picked_gateway(state: &State) -> crate::push::Gateway {
    state
        .push_gateway
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The signed-in client while `generation` is still the session.
async fn client_of(state: &Arc<State>, generation: u64) -> Option<Client> {
    let slot = state.slot();
    if slot.generation != generation || slot.phase != Phase::Session {
        return None;
    }
    state.client.lock().await.clone()
}

fn session_generation(state: &State) -> Option<u64> {
    let slot = state.slot();
    (slot.phase == Phase::Session).then_some(slot.generation)
}

/// Whether the old connector's push.json held a registration. One without is removed.
fn legacy_on(state: &State) -> bool {
    let legacy = &state.paths.push_file;
    if !legacy.exists() {
        return false;
    }
    let was_on = crate::push::legacy_enabled(legacy);
    if !was_on {
        let _ = std::fs::remove_file(legacy);
    }
    was_on
}

/// After a sign-in or restore: the pusher for this session, or a leftover cleared.
pub(super) async fn push_session_started(state: &Arc<State>, generation: u64) {
    // The woken process holds Leghorn's lock; it registers from its own events.
    if state
        .push_wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
    {
        return;
    }
    match push_sync(state, generation, None).await {
        Ok(false) => {}
        Ok(true) => push_report(state, None, None).await,
        Err(error) => push_report(state, Some("error"), Some(error)).await,
    }
}

/// Registers the pusher for `generation`, unless it is no longer the session.
async fn push_sync(
    state: &Arc<State>,
    generation: u64,
    pusher: Option<Pusher>,
) -> Result<bool, String> {
    let _serial = state.push_sync.lock().await;
    let Some(client) = client_of(state, generation).await else {
        return Ok(false);
    };
    let pusher = match pusher {
        Some(pusher) => Some(pusher),
        None => match running(state).await {
            Some(leghorn) => leghorn.endpoint().and_then(|endpoint| endpoint.matrix),
            None => None,
        },
    };
    let Some(pusher) = pusher else {
        // Pusher left over from push going off without a session.
        if running(state).await.is_none()
            && !leghorn::enabled(&PUSH)
            && leghorn::matrix::last_pusher(&PUSH).is_some()
        {
            crate::push::clear_own_pushers(&client).await?;
            if client_of(state, generation).await.is_some() {
                leghorn::forget_stored(&PUSH)
                    .await
                    .map_err(|error| crate::text::scrub_ids(&error.to_string()))?;
            }
        }
        return Ok(false);
    };
    let Some(gateway) = picked_gateway(state).resolve(pusher.gateway.as_deref()) else {
        // No gateway any more: remove the pusher.
        if state.push_registered.lock().await.take().is_some() {
            crate::push::clear_own_pushers(&client).await?;
        }
        return Ok(false);
    };
    crate::push::register(&client, &pusher, &gateway).await?;
    *state.push_registered.lock().await = Some((pusher.pushkey, gateway));
    Ok(true)
}

async fn push_event(state: &Arc<State>, push: PushEvent) {
    match push {
        PushEvent::NewEndpoint(endpoint) => {
            let synced = match session_generation(state) {
                Some(generation) => push_sync(state, generation, endpoint.matrix).await,
                None => Ok(false),
            };
            push_report(state, None, synced.err()).await;
        }
        PushEvent::Message(body) => {
            state
                .sink
                .emit(event("push.message", crate::push::message(&body)));
        }
        PushEvent::Raw(_) => {}
        PushEvent::Unregistered => {
            *state.push_registered.lock().await = None;
            let mut failure = None;
            if let Some(client) = state.client().await {
                failure = crate::push::clear_own_pushers(&client).await.err();
                // Otherwise kept, so the next sign-in can delete the pusher.
                if failure.is_none() {
                    if let Some(leghorn) = running(state).await {
                        let _ = leghorn.forget().await;
                    }
                }
            }
            push_stop(state).await;
            push_report(state, Some("unregistered"), failure).await;
        }
        PushEvent::Failed(error) => {
            let error = crate::text::scrub_ids(&error.to_string());
            push_report(state, Some("error"), Some(error)).await;
        }
    }
}

/// Always a complete `push.state`; partial ones blank the page.
async fn push_report(state: &Arc<State>, forced: Option<&str>, error: Option<String>) {
    let leghorn = running(state).await;
    let distributors = match &leghorn {
        Some(leghorn) => leghorn.distributors().await,
        None => leghorn::distributors().await,
    }
    .unwrap_or_default();
    let endpoint = leghorn.as_ref().and_then(|leghorn| leghorn.endpoint());
    let pusher = endpoint
        .as_ref()
        .and_then(|endpoint| endpoint.matrix.clone());
    let server_gateway = pusher.as_ref().and_then(|pusher| pusher.gateway.clone());
    let picked = picked_gateway(state);
    let gateway = picked.resolve(server_gateway.as_deref());
    let registered = match (
        &pusher,
        &gateway,
        state.push_registered.lock().await.as_ref(),
    ) {
        (Some(pusher), Some(gateway), Some((pushkey, registered))) => {
            pusher.pushkey == *pushkey && gateway == registered
        }
        _ => false,
    };
    let derived = if pusher.is_some() {
        if registered {
            "on"
        } else if gateway.is_none() {
            "needs-gateway"
        } else {
            "registering"
        }
    } else if endpoint.is_some() {
        "registering"
    } else if distributors.is_empty() {
        "no-distributor"
    } else {
        "off"
    };
    let mut data = json!({
        "state": forced.unwrap_or(derived),
        "distributors": distributors,
        "distributor": leghorn.as_ref().and_then(|leghorn| leghorn.distributor()),
        "enabled": endpoint.is_some(),
        "registered": registered,
        "gateway": gateway,
        "gatewayMode": picked.mode(),
        // "yes", "no" or "unknown" once registered; null before.
        "serverGateway": endpoint.as_ref().and_then(|endpoint| endpoint.matrix_support()),
        "serverGatewayUrl": server_gateway,
        // Host only: the rest of the address is a secret.
        "pushService": endpoint.as_ref().map(|endpoint| crate::push::host(&endpoint.url)),
    });
    if let Some(error) = error {
        data["error"] = json!(error);
    }
    state.sink.emit(event("push.state", data));
}

/// Reports push state. Starts the connector only if push is on; `quiet` leaves
/// the bus alone while it is off.
pub(super) async fn push_status(state: &Arc<State>, id: u64, quiet: bool) {
    if legacy_on(state) {
        // Kept until push is on again, so a failed start is tried at the next one.
        match enable(state).await {
            Ok(()) => {
                let _ = std::fs::remove_file(&state.paths.push_file);
            }
            Err(error) => push_report(state, Some("error"), Some(error)).await,
        }
    } else if leghorn::enabled(&PUSH) {
        if let Err(error) = push_connector(state).await {
            push_report(state, Some("unavailable"), Some(error)).await;
            state.sink.emit(reply_ok(id, json!({ "asked": true })));
            return;
        }
        // The session may have started before the connector did.
        let synced = match session_generation(state) {
            Some(generation) => push_sync(state, generation, None).await,
            None => Ok(false),
        };
        push_report(state, None, synced.err()).await;
    } else if !quiet {
        push_report(state, None, None).await;
    }
    state.sink.emit(reply_ok(id, json!({ "asked": true })));
}

async fn enable(state: &Arc<State>) -> Result<(), String> {
    // No registration before a gateway is picked.
    if picked_gateway(state) == crate::push::Gateway::Unset {
        return Err("choose a push gateway first".to_owned());
    }
    let leghorn = push_connector(state).await?;
    push_report(state, Some("registering"), None).await;
    let mut enabled = leghorn.enable().await;
    if let Err(PushError::ChooseDistributor(names)) = &enabled {
        // No distributor choice yet: take the first.
        if let Some(first) = names.first().cloned() {
            enabled = match leghorn.select(&first).await {
                Ok(()) => leghorn.enable().await,
                Err(error) => Err(error),
            };
        }
    }
    match enabled {
        Ok(endpoint) => {
            let synced = match session_generation(state) {
                Some(generation) => push_sync(state, generation, endpoint.matrix).await,
                None => Ok(false),
            };
            push_report(state, None, synced.err()).await;
            Ok(())
        }
        Err(PushError::NoDistributor) => {
            push_stop(state).await;
            push_report(state, Some("no-distributor"), None).await;
            Err("no push distributor is installed".to_owned())
        }
        Err(error) => {
            if !leghorn::enabled(&PUSH) {
                push_stop(state).await;
            }
            let error = crate::text::scrub_ids(&error.to_string());
            push_report(state, Some("error"), Some(error.clone())).await;
            Err(error)
        }
    }
}

pub(super) async fn push_enable(state: &Arc<State>, id: u64) {
    match enable(state).await {
        Ok(()) => state.sink.emit(reply_ok(id, json!({ "enabled": true }))),
        Err(error) => state.sink.emit(reply_error(id, error)),
    }
}

/// Removes the pusher, then everything Leghorn kept, and lets go of the name.
pub(super) async fn push_disable(state: &Arc<State>, id: u64) {
    let failure = drop_registration(state).await;
    push_report(state, None, failure).await;
    state.sink.emit(reply_ok(id, json!({ "enabled": false })));
}

/// Pusher, distributor registration and Leghorn's state, then the name.
async fn drop_registration(state: &Arc<State>) -> Option<String> {
    let mut failure = None;
    let mut cleared = false;
    {
        let _serial = state.push_sync.lock().await;
        if let Some(client) = state.client().await {
            match crate::push::clear_own_pushers(&client).await {
                Ok(()) => cleared = true,
                Err(error) => failure = Some(error),
            }
        }
        *state.push_registered.lock().await = None;
    }
    // A pusher still on the server keeps its record, for the next session to remove.
    let dropped = match (cleared, running(state).await) {
        (true, Some(leghorn)) => leghorn.forget().await,
        (true, None) => leghorn::forget_stored(&PUSH).await,
        (false, Some(leghorn)) => leghorn.disable().await,
        (false, None) if leghorn::enabled(&PUSH) => match push_connector(state).await {
            Ok(leghorn) => leghorn.disable().await,
            Err(error) => {
                failure = Some(error);
                Ok(())
            }
        },
        (false, None) => Ok(()),
    };
    if let Err(error) = dropped {
        failure = Some(crate::text::scrub_ids(&error.to_string()));
    }
    push_stop(state).await;
    failure
}

/// Unregisters and deletes push state at sign-out.
pub(super) async fn push_forget(state: &Arc<State>) {
    *state.push_registered.lock().await = None;
    let forgotten = match running(state).await {
        Some(leghorn) => tokio::time::timeout(std::time::Duration::from_secs(5), leghorn.forget())
            .await
            .unwrap_or(Ok(())),
        None => leghorn::forget_stored(&PUSH).await,
    };
    if let Err(error) = forgotten {
        log(state, "warn", format!("push state not removed: {error}"));
    }
    push_stop(state).await;
}

/// A different pick drops the registration; a complete one registers again.
pub(super) async fn push_set_gateway(
    state: &Arc<State>,
    id: u64,
    mode: String,
    gateway: String,
    turn_on: bool,
) {
    let picked = match crate::push::Gateway::parse(&mode, &gateway) {
        Ok(picked) => picked,
        Err(error) => {
            state.sink.emit(reply_error(id, error));
            return;
        }
    };
    let previous = std::mem::replace(
        &mut *state
            .push_gateway
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        picked.clone(),
    );
    let complete = picked != crate::push::Gateway::Unset;
    let mut start = turn_on && complete && running(state).await.is_none();
    if previous != picked && running(state).await.is_some() {
        let failure = drop_registration(state).await;
        start = complete;
        if !complete {
            push_report(state, None, failure).await;
        }
    }
    if start {
        if let Err(error) = enable(state).await {
            push_report(state, Some("error"), Some(error)).await;
        }
    }
    state.sink.emit(reply_ok(id, json!({ "set": true })));
}

/// Turns a push into a banner. Answers with an error where the push rules say
/// not to show it: silence is the right outcome then.
pub(super) async fn push_notify(state: &Arc<State>, id: u64, room_id: String, event_id: String) {
    match notification(state, &room_id, &event_id).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(error) => state.sink.emit(reply_error(id, error)),
    }
}

async fn notification(state: &Arc<State>, room_id: &str, event_id: &str) -> Result<Value, String> {
    // Wait for a restore in flight.
    drop(state.session_gate.lock().await);
    let Some(client) = state.client().await else {
        return Err("not signed in".to_owned());
    };
    // The running sync service where there is one - the woken process has none,
    // and there it really is the only process.
    let sync = state
        .rooms
        .lock()
        .await
        .as_ref()
        .map(|handle| handle.sync.clone());
    crate::push::notification_for(&client, room_id, event_id, sync).await
}

/// The woken process: holds the connector name until the pushes are handled.
/// `push.yield` ends it.
pub(super) async fn push_wake(state: &Arc<State>, id: u64) {
    if !leghorn::enabled(&PUSH) {
        state.sink.emit(reply_ok(id, json!({ "yielded": false })));
        return;
    }
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let holds = Holds::default();
    let (jobs, mut received) = tokio::sync::mpsc::unbounded_channel();
    let handler = {
        let holds = holds.clone();
        move |push: PushEvent| {
            let job = (push, holds.hold());
            let jobs = jobs.clone();
            async move {
                let _ = jobs.send(job);
                Ok::<(), std::convert::Infallible>(())
            }
        }
    };
    let worker = state.clone();
    let task = tokio::spawn(async move {
        let work = async {
            while let Some((push, hold)) = received.recv().await {
                wake_event(&worker, push).await;
                drop(hold);
            }
        };
        let (served, ()) = tokio::join!(leghorn::wake(&PUSH, handler, &holds), work);
        served
    });
    *state
        .push_wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(task.abort_handle());
    if state.push_yielded.load(std::sync::atomic::Ordering::SeqCst) {
        task.abort();
    }
    let outcome = task.await;
    if state.push_yielded.load(std::sync::atomic::Ordering::SeqCst) {
        // A refreshed token waiting for the gate is saved; a restore holding it refreshes none.
        let gate = state.session_gate.lock();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), gate).await;
    } else {
        lifecycle::close_session(state).await;
    }
    *state
        .push_wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    match outcome {
        Ok(Ok(())) => state.sink.emit(reply_ok(id, json!({ "yielded": false }))),
        Ok(Err(error)) => state
            .sink
            .emit(reply_error(id, crate::text::scrub_ids(&error.to_string()))),
        Err(_) => state.sink.emit(reply_ok(id, json!({ "yielded": true }))),
    }
}

pub(super) fn push_yield(state: &Arc<State>, id: u64) {
    state
        .push_yielded
        .store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(task) = state
        .push_wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
    {
        task.abort();
    }
    state.sink.emit(reply_ok(id, json!({})));
}

async fn wake_event(state: &Arc<State>, push: PushEvent) {
    match push {
        PushEvent::Message(body) => {
            let target = crate::push::message(&body);
            let (Some(room_id), Some(event_id)) =
                (target["roomId"].as_str(), target["eventId"].as_str())
            else {
                return;
            };
            match notification(state, room_id, event_id).await {
                Ok(data) => state.sink.emit(event("push.banner", data)),
                Err(error) if error == "filtered out" || error == "redacted" => {}
                Err(error) => {
                    log(
                        state,
                        "info",
                        format!("push shown without its message: {error}"),
                    );
                    state
                        .sink
                        .emit(event("push.banner", json!({ "generic": true })));
                }
            }
        }
        PushEvent::NewEndpoint(endpoint) => {
            drop(state.session_gate.lock().await);
            if let Some(generation) = session_generation(state) {
                if let Err(error) = push_sync(state, generation, endpoint.matrix).await {
                    log(
                        state,
                        "warn",
                        format!("the new push address was not registered: {error}"),
                    );
                }
            }
        }
        PushEvent::Unregistered => {
            drop(state.session_gate.lock().await);
            if let Some(client) = state.client().await {
                let _ = crate::push::clear_own_pushers(&client).await;
            }
            state
                .sink
                .emit(event("push.state", json!({ "state": "unregistered" })));
        }
        PushEvent::Failed(error) => {
            log(state, "warn", crate::text::scrub_ids(&error.to_string()));
        }
        PushEvent::Raw(_) => {}
    }
}
