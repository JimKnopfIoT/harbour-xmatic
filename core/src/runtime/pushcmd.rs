//! UnifiedPush: the connector, the pusher, a woken fetch.

use super::*;

pub(super) async fn push_connector(state: &Arc<State>) -> Result<Arc<leghorn::Leghorn>, String> {
    if !state.push_connector {
        return Err("the push connector does not run in this process".to_owned());
    }
    state
        .push
        .get_or_init(|| async {
            let (started, events) = crate::push::start().await;
            *state.push_events.lock().await = Some(events);
            started.map(Arc::new)
        })
        .await
        .clone()
}

pub(super) async fn push_listen(state: Weak<State>) {
    let events = {
        let Some(state) = state.upgrade() else {
            return;
        };
        if push_connector(&state).await.is_err() {
            return;
        }
        // Migration from the pre-Leghorn push.json.
        let legacy = state.paths.push_file.clone();
        if legacy.exists() {
            let was_on = crate::push::legacy_enabled(&legacy);
            let _ = std::fs::remove_file(&legacy);
            if was_on {
                push_enable(&state, 0).await;
            }
        }
        let events = state.push_events.lock().await.take();
        events
    };
    let Some(mut events) = events else {
        return;
    };
    while let Some(push) = events.recv().await {
        let Some(state) = state.upgrade() else {
            return;
        };
        push_event(&state, push).await;
    }
}

pub(super) async fn push_event(state: &Arc<State>, push: leghorn::Event) {
    match push {
        leghorn::Event::NewEndpoint(endpoint) => {
            // Leghorn may not report itself enabled yet.
            let synced = push_sync(state, None, endpoint.matrix).await;
            push_report(state, None, synced.err()).await;
        }
        leghorn::Event::Message(body) => {
            state
                .sink
                .emit(event("push.message", crate::push::message(&body)));
        }
        leghorn::Event::Raw(_) => {}
        leghorn::Event::Unregistered => {
            *state.push_registered.lock().await = None;
            let mut failure = None;
            if let Some(client) = state.client().await {
                failure = crate::push::clear_own_pushers(&client).await.err();
            }
            push_report(state, Some("unregistered"), failure).await;
        }
        leghorn::Event::Failed(error) => {
            let error = crate::text::scrub_ids(&error.to_string());
            push_report(state, Some("error"), Some(error)).await;
        }
    }
}

/// Registers Leghorn's pusher when there is a client. Runs on new endpoints and sign-in.
pub(super) async fn push_sync(
    state: &Arc<State>,
    client: Option<Client>,
    pusher: Option<leghorn::matrix::Pusher>,
) -> Result<bool, String> {
    let pusher = match pusher {
        Some(pusher) => Some(pusher),
        None => match push_connector(state).await {
            Ok(leghorn) => leghorn.endpoint().and_then(|endpoint| endpoint.matrix),
            Err(_) => None,
        },
    };
    let Some(pusher) = pusher else {
        // Pusher left behind by an unregister in a woken process.
        if let (Ok(_), Some(_)) = (
            push_connector(state).await,
            leghorn::matrix::last_pusher(&crate::push::PUSH),
        ) {
            let client = match client {
                Some(client) => Some(client),
                None => state.client().await,
            };
            if let Some(client) = client {
                crate::push::clear_own_pushers(&client).await?;
            }
        }
        return Ok(false);
    };
    let client = match client {
        Some(client) => client,
        None => match state.client().await {
            Some(client) => client,
            None => return Ok(false),
        },
    };
    let _serial = state.push_sync.lock().await;
    crate::push::register(&client, &pusher).await?;
    *state.push_registered.lock().await = Some(pusher.pushkey);
    Ok(true)
}

/// Always a complete `push.state`; partial ones blank the page.
pub(super) async fn push_report(state: &Arc<State>, forced: Option<&str>, error: Option<String>) {
    let leghorn = match push_connector(state).await {
        Ok(leghorn) => leghorn,
        Err(start) => {
            state.sink.emit(event(
                "push.state",
                json!({
                    "state": "unavailable",
                    "error": error.unwrap_or(start),
                    "distributors": [],
                    "distributor": null,
                    "enabled": false,
                    "registered": false,
                    "gateway": null,
                }),
            ));
            return;
        }
    };
    let distributors = leghorn.distributors().await.unwrap_or_default();
    let endpoint = leghorn.endpoint();
    let pusher = endpoint.as_ref().and_then(|endpoint| endpoint.matrix.clone());
    let registered = match (&pusher, state.push_registered.lock().await.as_ref()) {
        (Some(pusher), Some(pushkey)) => pusher.pushkey == *pushkey,
        _ => false,
    };
    let derived = if endpoint.is_some() {
        if registered {
            "on"
        } else {
            "registering"
        }
    } else if distributors.is_empty() {
        "no-distributor"
    } else {
        "off"
    };
    let mut data = json!({
        "state": forced.unwrap_or(derived),
        "distributors": distributors,
        "distributor": leghorn.distributor(),
        "enabled": endpoint.is_some(),
        "registered": registered,
        "gateway": pusher.map(|pusher| pusher.gateway),
    });
    if let Some(error) = error {
        data["error"] = json!(error);
    }
    state.sink.emit(event("push.state", data));
}

pub(super) async fn push_status(state: &Arc<State>, id: u64) {
    push_report(state, None, None).await;
    state.sink.emit(reply_ok(id, json!({ "asked": true })));
}

pub(super) async fn push_enable(state: &Arc<State>, id: u64) {
    let leghorn = match push_connector(state).await {
        Ok(leghorn) => leghorn,
        Err(error) => {
            push_report(state, None, None).await;
            state.sink.emit(reply_error(id, error));
            return;
        }
    };
    push_report(state, Some("registering"), None).await;
    let mut enabled = leghorn.enable().await;
    if let Err(leghorn::Error::ChooseDistributor(names)) = &enabled {
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
            let synced = push_sync(state, None, endpoint.matrix).await;
            push_report(state, None, synced.err()).await;
            state.sink.emit(reply_ok(id, json!({ "enabled": true })));
        }
        Err(leghorn::Error::NoDistributor) => {
            push_report(state, Some("no-distributor"), None).await;
            state
                .sink
                .emit(reply_error(id, "no push distributor is installed"));
        }
        Err(error) => {
            let error = crate::text::scrub_ids(&error.to_string());
            push_report(state, Some("error"), Some(error.clone())).await;
            state.sink.emit(reply_error(id, error));
        }
    }
}

pub(super) async fn push_disable(state: &Arc<State>, id: u64) {
    let mut failure = None;
    if let Some(client) = state.client().await {
        failure = crate::push::clear_own_pushers(&client).await.err();
    }
    *state.push_registered.lock().await = None;
    if let Ok(leghorn) = push_connector(state).await {
        if let Err(error) = leghorn.disable().await {
            failure = Some(crate::text::scrub_ids(&error.to_string()));
        }
    }
    push_report(state, None, failure).await;
    state.sink.emit(reply_ok(id, json!({ "enabled": false })));
}

/// Turns a push into a banner. Answers with an error where the push rules say
/// not to show it: silence is the right outcome then.
pub(super) async fn push_notify(state: &Arc<State>, id: u64, room_id: String, event_id: String) {
    // Wait for a restore in flight.
    drop(state.session_gate.lock().await);
    let Some(client) = state.client().await else {
        state.sink.emit(reply_error(id, "not signed in"));
        return;
    };
    // The running sync service where there is one - the woken process has none,
    // and there it really is the only process.
    let sync = state
        .rooms
        .lock()
        .await
        .as_ref()
        .map(|handle| handle.sync.clone());
    match crate::push::notification_for(&client, &room_id, &event_id, sync).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(error) => state.sink.emit(reply_error(id, error)),
    }
}

