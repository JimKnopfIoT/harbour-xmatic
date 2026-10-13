//! UnifiedPush: the connector, the pusher, the woken process.

use super::*;

use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::push::{PusherError, Unshown, PUSH};
use leghorn::matrix::Pusher;
use leghorn::{Error as PushError, Event as PushEvent, Holds, Leghorn};

/// How long a woken process waits for the front end's store key.
const UNLOCK_LIMIT: Duration = Duration::from_secs(20);
/// After a yield, together under `XMATIC_WAKE_YIELD_MS`: the push in hand may finish,
/// then the tokens are saved. Past `session::BOUNDED_REQUEST`: a request in flight ends first.
const YIELD_FINISH: Duration = Duration::from_millis(6500);
const YIELD_SAVE: Duration = Duration::from_secs(3);
/// One push's handling in the woken process, the wait for the key included.
const EVENT_LIMIT: Duration = Duration::from_secs(35);
/// Once the pushes stopped: what the worker still has, bounded.
const WORK_LIMIT: Duration = Duration::from_secs(30);
/// Giving the registration back at sign-out.
const FORGET_LIMIT: Duration = Duration::from_secs(5);
/// How long a teardown waits for aborted push tasks to be gone.
const TASK_STOP_LIMIT: Duration = Duration::from_secs(3);

/// Push work that may hold a client clone: `teardown` aborts it and waits.
pub(super) fn spawn_push_task<F>(state: &State, work: F) -> tokio::task::JoinHandle<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    // Spawned under the lock: a teardown taking the list cannot miss it.
    let mut tasks = state
        .push_tasks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tasks.retain(|task| !task.is_finished());
    let handle = tokio::spawn(work);
    tasks.push(handle.abort_handle());
    handle
}

/// `spawn_push_task` for one session: refused once `generation` is no longer
/// it. Checked under the list's lock, so a teardown that took the list is past.
fn spawn_session_task<F>(state: &State, generation: u64, work: F) -> bool
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let mut tasks = state
        .push_tasks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if session_generation(state) != Some(generation) {
        return false;
    }
    tasks.retain(|task| !task.is_finished());
    tasks.push(tokio::spawn(work).abort_handle());
    true
}

/// For `teardown`, after the generation moved on: every push task aborted and
/// gone, so none holds the client or writes for the old session.
pub(super) async fn stop_push_tasks(state: &State) {
    let tasks = std::mem::take(
        &mut *state
            .push_tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    for task in &tasks {
        task.abort();
    }
    let deadline = tokio::time::Instant::now() + TASK_STOP_LIMIT;
    while tasks.iter().any(|task| !task.is_finished()) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if tasks.iter().any(|task| !task.is_finished()) {
        log::warn!("a push task outlived the session's teardown");
    }
    state.push_waiting.store(false, Ordering::SeqCst);
    state.push_dirty.store(false, Ordering::SeqCst);
    *state.push_registered.lock().await = None;
}

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
            // One at a time, each a push task: a teardown cuts the handling, not the listener.
            let worker = state.clone();
            let handled = spawn_push_task(&state, async move { push_event(&worker, push).await });
            drop(state);
            let _ = handled.await;
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

pub(super) fn woken(state: &State) -> bool {
    state
        .push_wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
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

/// Whether `generation` is still the session; gates every write of a pusher change.
fn still(state: &State, generation: u64) -> impl Fn() -> bool + Sync + '_ {
    move || session_generation(state) == Some(generation)
}

/// The pushkeys this device has in hand: the live endpoint's and the last one Leghorn kept.
pub(super) async fn own_pushkeys(state: &Arc<State>) -> Vec<String> {
    let mut named = Vec::new();
    if let Some(pusher) = running(state)
        .await
        .and_then(|leghorn| leghorn.endpoint())
        .and_then(|endpoint| endpoint.matrix)
    {
        named.push(pusher.pushkey);
    }
    if let Some(pusher) = leghorn::matrix::last_pusher(&PUSH) {
        if !named.contains(&pusher.pushkey) {
            named.push(pusher.pushkey);
        }
    }
    named
}

/// This session's pushers removed, the record updated only while it lasts.
async fn clear_session_pushers(state: &Arc<State>, generation: u64) -> Option<String> {
    let client = client_of(state, generation).await?;
    let named = own_pushkeys(state).await;
    let live = still(state, generation);
    crate::push::clear_own_pushers(&client, &state.paths.push_owned, &named, &live)
        .await
        .err()
        .map(String::from)
}

/// Whether the old connector or its setting still asks for push. A push.json
/// that asks for nothing is removed, its pusher noted for removal.
fn legacy_on(state: &State, wanted: bool) -> bool {
    let legacy = &state.paths.push_file;
    if !legacy.exists() {
        return false;
    }
    let on = wanted || crate::push::legacy_enabled(legacy);
    if !on {
        crate::push::note_legacy_pushers(legacy, &state.paths.push_owned);
        let _ = std::fs::remove_file(legacy);
    }
    on
}

/// Carries push over from the old connector: the new registration first, then
/// the old one given back. A failure keeps the file for the next start.
async fn migrate_legacy(state: &Arc<State>) {
    match enable(state).await {
        Ok(true) => give_back_legacy(state).await,
        Ok(false) => {}
        Err(error) => push_report(state, Some("error"), Some(error)).await,
    }
}

/// The old connector's tokens back to the distributor, then its file. Its
/// pusher is noted first, for this or the next session to remove.
async fn give_back_legacy(state: &Arc<State>) {
    if !state.paths.push_file.exists() {
        return;
    }
    match crate::push::legacy_unregister(&state.paths.push_file).await {
        Ok(()) => {
            crate::push::note_legacy_pushers(&state.paths.push_file, &state.paths.push_owned);
            let _ = std::fs::remove_file(&state.paths.push_file);
            if let Some(generation) = session_generation(state) {
                sync_reported(state, sync_or_retry(state, generation, None).await).await;
            }
        }
        Err(error) => log(state, "warn", format!("old push registration kept: {error}")),
    }
}

/// After a sign-in or restore: the pusher for this session, or a leftover cleared.
pub(super) async fn push_session_started(state: &Arc<State>, generation: u64) {
    // The woken process registers from its own events.
    if woken(state) {
        return;
    }
    match sync_or_retry(state, generation, None).await {
        Ok(false) => {}
        synced => sync_reported(state, synced).await,
    }
}

/// A complete `push.state` after a pusher change; a refusal is its own state.
async fn sync_reported(state: &Arc<State>, synced: Result<bool, PusherError>) {
    match synced {
        Err(error) if error.rejected => {
            push_report(state, Some("error"), Some(error.message)).await
        }
        other => push_report(state, None, other.err().map(String::from)).await,
    }
}

/// `push_sync`, and a failure tried again once the sync is back. A refusal is not.
async fn sync_or_retry(
    state: &Arc<State>,
    generation: u64,
    pusher: Option<Pusher>,
) -> Result<bool, PusherError> {
    let synced = push_sync(state, generation, pusher).await;
    match &synced {
        Err(error) if error.rejected => state.push_dirty.store(false, Ordering::SeqCst),
        Err(_) => retry_on_running(state, generation),
        Ok(_) => {}
    }
    synced
}

/// One waiter at a time, for this session only; it asks again at every
/// `Running` until the pusher is set or refused. A teardown ends it.
fn retry_on_running(state: &Arc<State>, generation: u64) {
    if session_generation(state) != Some(generation) {
        return;
    }
    state.push_dirty.store(true, Ordering::SeqCst);
    if state.push_waiting.swap(true, Ordering::SeqCst) {
        return;
    }
    let weak = Arc::downgrade(state);
    let runs = crate::push::sync_running_changes();
    if !spawn_session_task(state, generation, wait_for_running(weak, generation, runs)) {
        state.push_waiting.store(false, Ordering::SeqCst);
    }
}

async fn wait_for_running(
    weak: std::sync::Weak<State>,
    generation: u64,
    mut runs: tokio::sync::watch::Receiver<u64>,
) {
    loop {
        while runs.changed().await.is_ok() {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if !state.push_dirty.load(Ordering::SeqCst) {
                break;
            }
            if session_generation(&state) != Some(generation) {
                return;
            }
            match push_sync(&state, generation, None).await {
                // No client after all: still owed.
                Ok(_) if state.push_dirty.load(Ordering::SeqCst) => {}
                Ok(_) => {
                    push_report(&state, None, None).await;
                    break;
                }
                Err(error) if error.rejected => {
                    state.push_dirty.store(false, Ordering::SeqCst);
                    log(&state, "warn", format!("pusher refused, not asked again: {}", error.message));
                    push_report(&state, Some("error"), Some(error.message)).await;
                    break;
                }
                Err(error) => {
                    log(&state, "info", format!("pusher still not set: {}", error.message))
                }
            }
        }
        let Some(state) = weak.upgrade() else {
            return;
        };
        // A debt noted while this waiter was on its way out saw it still waiting.
        state.push_waiting.store(false, Ordering::SeqCst);
        if !state.push_dirty.load(Ordering::SeqCst)
            || session_generation(&state) != Some(generation)
            || state.push_waiting.swap(true, Ordering::SeqCst)
        {
            return;
        }
    }
}

/// Registers the pusher for `generation`, unless it is no longer the session:
/// then nothing is written and a debt stays owed.
async fn push_sync(
    state: &Arc<State>,
    generation: u64,
    pusher: Option<Pusher>,
) -> Result<bool, PusherError> {
    let _serial = state.push_sync.lock().await;
    let Some(client) = client_of(state, generation).await else {
        return Ok(false);
    };
    let live = still(state, generation);
    let owned = &state.paths.push_owned;
    let user = crate::push::account_of(&client)?;
    let pusher = match pusher {
        Some(pusher) => Some(pusher),
        None => match running(state).await {
            Some(leghorn) => leghorn.endpoint().and_then(|endpoint| endpoint.matrix),
            None => None,
        },
    };
    let Some(pusher) = pusher else {
        // Pushers left over from push going off without a session, or from a
        // sign-out of this account that could not reach the homeserver.
        let last = leghorn::matrix::last_pusher(&PUSH);
        if running(state).await.is_none()
            && !leghorn::enabled(&PUSH)
            && (last.is_some() || crate::push::owns_any(owned, &user))
        {
            let named: Vec<String> = last.iter().map(|pusher| pusher.pushkey.clone()).collect();
            crate::push::clear_own_pushers(&client, owned, &named, &live).await?;
            if last.is_some() && live() {
                leghorn::forget_stored(&PUSH)
                    .await
                    .map_err(|error| PusherError::failed(crate::text::scrub_ids(&error.to_string())))?;
            }
        }
        if live() {
            state.push_dirty.store(false, Ordering::SeqCst);
        }
        return Ok(false);
    };
    let Some(gateway) = picked_gateway(state).resolve(pusher.gateway.as_deref()) else {
        // No gateway any more: remove the pusher.
        let was_set = state.push_registered.lock().await.take().is_some();
        if was_set || crate::push::owns(owned, &user, &pusher.pushkey) {
            crate::push::clear_own_pushers(&client, owned, &[pusher.pushkey.clone()], &live).await?;
        }
        if live() {
            state.push_dirty.store(false, Ordering::SeqCst);
        }
        return Ok(false);
    };
    crate::push::register(&client, &pusher, &gateway, owned, &live).await?;
    if !live() {
        return Ok(false);
    }
    *state.push_registered.lock().await = Some((pusher.pushkey, gateway));
    state.push_dirty.store(false, Ordering::SeqCst);
    Ok(true)
}

/// Whether the app's sync runs; it then brings a pushed message itself.
async fn sync_running(state: &Arc<State>) -> bool {
    use matrix_sdk_ui::sync_service::State as SyncState;
    state
        .rooms
        .lock()
        .await
        .as_ref()
        .is_some_and(|handle| matches!(handle.sync.state().get(), SyncState::Running))
}

async fn push_event(state: &Arc<State>, push: PushEvent) {
    match push {
        PushEvent::NewEndpoint(endpoint) => {
            let synced = match session_generation(state) {
                Some(generation) => sync_or_retry(state, generation, endpoint.matrix).await,
                None => Ok(false),
            };
            sync_reported(state, synced).await;
        }
        PushEvent::Message(body) => {
            let mut data = crate::push::message(&body);
            data["fetch"] = json!(!sync_running(state).await);
            state.sink.emit(event("push.message", data));
        }
        PushEvent::Raw(_) => {}
        PushEvent::Unregistered => {
            let failure = {
                let _serial = state.push_sync.lock().await;
                *state.push_registered.lock().await = None;
                match session_generation(state) {
                    Some(generation) => clear_session_pushers(state, generation).await,
                    None => None,
                }
            };
            // Otherwise kept, so the next sign-in can delete the pusher.
            if failure.is_none() {
                if let Some(leghorn) = running(state).await {
                    let _ = leghorn.forget().await;
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
pub(super) async fn push_report(state: &Arc<State>, forced: Option<&str>, error: Option<String>) {
    let leghorn = running(state).await;
    let listed = match &leghorn {
        Some(leghorn) => leghorn.distributors().await,
        None => leghorn::distributors().await,
    }
    .ok();
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
    let derived = crate::push::derive_state(&crate::push::Facts {
        pusher: pusher.is_some(),
        registered,
        gateway: gateway.is_some(),
        endpoint: endpoint.is_some(),
        distributors: listed.as_ref().map(Vec::len),
    });
    // The saved pick, also while the connector is not running.
    let distributor = match &leghorn {
        Some(leghorn) => leghorn.distributor(),
        None => None,
    };
    let mut data = json!({
        "state": forced.unwrap_or(derived),
        "distributors": listed.unwrap_or_default(),
        "distributor": distributor,
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
/// the bus alone while it is off. Answers whether an old push.json is left.
pub(super) async fn push_status(state: &Arc<State>, id: u64, quiet: bool, legacy_wanted: bool) {
    if !leghorn::enabled(&PUSH) && legacy_on(state, legacy_wanted) {
        migrate_legacy(state).await;
    } else if leghorn::enabled(&PUSH) {
        if let Err(error) = push_connector(state).await {
            push_report(state, Some("unavailable"), Some(error)).await;
            state.sink.emit(reply_ok(
                id,
                json!({ "asked": true, "legacy": state.paths.push_file.exists() }),
            ));
            return;
        }
        // The session may have started before the connector did.
        let synced = match session_generation(state) {
            Some(generation) => sync_or_retry(state, generation, None).await,
            None => Ok(false),
        };
        sync_reported(state, synced).await;
        // Push turned on by hand after a migration that waited for a choice.
        give_back_legacy(state).await;
    } else if !quiet {
        push_report(state, None, None).await;
    }
    state.sink.emit(reply_ok(
        id,
        json!({ "asked": true, "legacy": state.paths.push_file.exists() }),
    ));
}

/// Registers with the distributor and sets the pusher. `Ok(false)`: several
/// distributors and none picked yet; the page asks.
async fn enable(state: &Arc<State>) -> Result<bool, String> {
    // No registration before a gateway is picked.
    if picked_gateway(state) == crate::push::Gateway::Unset {
        return Err("choose a push gateway first".to_owned());
    }
    let leghorn = push_connector(state).await?;
    push_report(state, Some("registering"), None).await;
    match leghorn.enable().await {
        Ok(endpoint) => {
            let synced = match session_generation(state) {
                Some(generation) => sync_or_retry(state, generation, endpoint.matrix).await,
                None => Ok(false),
            };
            sync_reported(state, synced).await;
            Ok(true)
        }
        Err(PushError::ChooseDistributor(_)) => {
            if !leghorn::enabled(&PUSH) {
                push_stop(state).await;
            }
            push_report(state, Some("choose-distributor"), None).await;
            Ok(false)
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
        Ok(enabled) => state.sink.emit(reply_ok(id, json!({ "enabled": enabled }))),
        Err(error) => state.sink.emit(reply_error(id, error)),
    }
}

/// The user's pick among several distributors. A switch drops the old registration.
pub(super) async fn push_select(state: &Arc<State>, id: u64, distributor: String, turn_on: bool) {
    // Could not ask is not "not installed".
    let installed = match leghorn::distributors().await {
        Ok(installed) => installed,
        Err(error) => {
            state.sink.emit(reply_error(id, crate::text::scrub_ids(&error.to_string())));
            return;
        }
    };
    if !installed.contains(&distributor) {
        state.sink.emit(reply_error(id, "that push distributor is not installed"));
        return;
    }
    let was_on = leghorn::enabled(&PUSH);
    let current = match running(state).await {
        Some(leghorn) => leghorn.distributor(),
        None => None,
    };
    if was_on && current.as_deref() == Some(distributor.as_str()) {
        push_report(state, None, None).await;
        state.sink.emit(reply_ok(id, json!({ "selected": true })));
        return;
    }
    if was_on {
        if let Some(failure) = drop_registration(state).await {
            push_report(state, Some("error"), Some(failure)).await;
        }
    }
    let selected = match push_connector(state).await {
        Ok(leghorn) => leghorn
            .select(&distributor)
            .await
            .map_err(|error| crate::text::scrub_ids(&error.to_string())),
        Err(error) => Err(error),
    };
    if let Err(error) = selected {
        push_report(state, Some("error"), Some(error.clone())).await;
        state.sink.emit(reply_error(id, error));
        return;
    }
    if turn_on || was_on {
        match enable(state).await {
            Ok(enabled) => state.sink.emit(reply_ok(id, json!({ "selected": true, "enabled": enabled }))),
            Err(error) => {
                push_report(state, Some("error"), Some(error.clone())).await;
                state.sink.emit(reply_error(id, error));
            }
        }
    } else {
        // Saved for later; the name is not held while push is off.
        push_stop(state).await;
        push_report(state, None, None).await;
        state.sink.emit(reply_ok(id, json!({ "selected": true, "enabled": false })));
    }
}

/// Removes the pusher, then everything Leghorn kept, and lets go of the name.
pub(super) async fn push_disable(state: &Arc<State>, id: u64) {
    let failure = drop_registration(state).await;
    push_report(state, None, failure).await;
    state.sink.emit(reply_ok(id, json!({ "enabled": false })));
}

/// What switching off does with the distributor registration.
#[derive(Debug, PartialEq, Eq)]
enum DropStep {
    /// Pushers gone: the registration goes with them.
    Forget,
    ForgetStored,
    /// A pusher is left on the server: the record stays for the next session.
    Disable,
    ConnectAndDisable,
    Nothing,
}

fn drop_step(cleared: bool, running: bool, stored_enabled: bool) -> DropStep {
    match (cleared, running) {
        (true, true) => DropStep::Forget,
        (true, false) => DropStep::ForgetStored,
        (false, true) => DropStep::Disable,
        (false, false) if stored_enabled => DropStep::ConnectAndDisable,
        (false, false) => DropStep::Nothing,
    }
}

/// Pusher, distributor registration and Leghorn's state, then the name.
async fn drop_registration(state: &Arc<State>) -> Option<String> {
    let mut failure = None;
    let mut cleared = false;
    {
        let _serial = state.push_sync.lock().await;
        if let Some(generation) = session_generation(state) {
            match clear_session_pushers(state, generation).await {
                None if still(state, generation)() => cleared = true,
                None => {}
                Some(error) => failure = Some(error),
            }
        }
        *state.push_registered.lock().await = None;
    }
    // A pusher still on the server keeps its record, for the next session to remove.
    let leghorn = running(state).await;
    let enabled = leghorn.is_none() && leghorn::enabled(&PUSH);
    let dropped = match (drop_step(cleared, leghorn.is_some(), enabled), leghorn) {
        (DropStep::Forget, Some(leghorn)) => leghorn.forget().await,
        (DropStep::ForgetStored, _) => leghorn::forget_stored(&PUSH).await,
        (DropStep::Disable, Some(leghorn)) => leghorn.disable().await,
        (DropStep::ConnectAndDisable, _) => match push_connector(state).await {
            Ok(leghorn) => leghorn.disable().await,
            Err(error) => {
                failure = Some(error);
                Ok(())
            }
        },
        _ => Ok(()),
    };
    if let Err(error) = dropped {
        failure = Some(crate::text::scrub_ids(&error.to_string()));
    }
    push_stop(state).await;
    drop_legacy(state).await;
    failure
}

/// Switched off by hand: an old registration is given back, not carried over
/// later; its pusher is noted for removal.
async fn drop_legacy(state: &Arc<State>) {
    if !state.paths.push_file.exists() {
        return;
    }
    if let Err(error) = crate::push::legacy_unregister(&state.paths.push_file).await {
        log(state, "warn", format!("old push registration not given back: {error}"));
    }
    crate::push::note_legacy_pushers(&state.paths.push_file, &state.paths.push_owned);
    let _ = std::fs::remove_file(&state.paths.push_file);
}

/// Unregisters and deletes push state at sign-out. Leghorn's file goes even
/// where the distributor does not answer; only that is reported.
pub(super) async fn push_forget(state: &Arc<State>) -> Option<String> {
    *state.push_registered.lock().await = None;
    let forgotten = match running(state).await {
        // Spawned, not cut off: Leghorn unregisters first, and cut off midway its
        // memory would still say "on".
        Some(leghorn) => {
            let forgetting = tokio::spawn(async move { leghorn.forget().await });
            match tokio::time::timeout(FORGET_LIMIT, forgetting).await {
                Ok(Ok(outcome)) => {
                    outcome.map_err(|error| crate::text::scrub_ids(&error.to_string()))
                }
                Ok(Err(_)) => Err("giving the registration back failed".to_owned()),
                Err(_) => Err("the distributor did not answer".to_owned()),
            }
        }
        None => match tokio::time::timeout(FORGET_LIMIT, leghorn::forget_stored(&PUSH)).await {
            Ok(outcome) => outcome.map_err(|error| crate::text::scrub_ids(&error.to_string())),
            Err(_) => Err("the distributor did not answer".to_owned()),
        },
    };
    push_stop(state).await;
    if let Err(error) = crate::push::remove_registration(&state.paths.push_state[0]) {
        log(state, "warn", format!("the push registration file was not deleted: {error}"));
    }
    // The next account picks its own gateway.
    *state
        .push_gateway
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = crate::push::Gateway::Unset;
    let failure = forgotten.err()?;
    log(state, "warn", format!("push registration not given back to the distributor: {failure}"));
    Some(failure)
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
    if !complete && previous != picked {
        drop_legacy(state).await;
    }
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

/// Turns a push into a banner's data. Where nothing is to be shown the answer
/// says `silent`: the push rules or the server decided that.
pub(super) async fn push_notify(state: &Arc<State>, id: u64, room_id: String, event_id: String) {
    match notification(state, &room_id, &event_id).await {
        Ok(data) => state.sink.emit(reply_ok(id, data)),
        Err(unshown) if unshown.silent() => state.sink.emit(reply_ok(
            id,
            json!({ "silent": true, "reason": unshown.reason() }),
        )),
        Err(unshown) => state.sink.emit(reply_error(id, unshown.reason())),
    }
}

async fn notification(state: &Arc<State>, room_id: &str, event_id: &str) -> Result<Value, Unshown> {
    // Wait for a restore in flight.
    drop(state.session_gate.lock().await);
    let Some(client) = state.client().await else {
        return Err(Unshown::Failed("not signed in".to_owned()));
    };
    // The woken process is the only one and has no sync. The app uses its own
    // sync service, started here where it is not yet - never a second encryption sync.
    let sync = if woken(state) {
        None
    } else {
        ensure_room_list(state).await.map_err(Unshown::Failed)?;
        state
            .rooms
            .lock()
            .await
            .as_ref()
            .map(|handle| handle.sync.clone())
    };
    if sync.is_none() && !woken(state) {
        return Err(Unshown::Failed("the room list is not running".to_owned()));
    }
    crate::push::notification_for(&client, room_id, event_id, sync).await
}

/// The woken process: holds the connector name until the pushes are handled.
/// `push.yield` ends it.
pub(super) async fn push_wake(state: &Arc<State>, id: u64) {
    session::BOUNDED_REQUESTS.store(true, Ordering::SeqCst);
    if !leghorn::enabled(&PUSH) {
        state.sink.emit(reply_ok(id, json!({ "yielded": false })));
        return;
    }
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
    // The name and Leghorn's lock live in this task; a yield aborts it.
    let listener = tokio::spawn({
        let holds = holds.clone();
        async move { leghorn::wake(&PUSH, handler, &holds).await }
    });
    *state
        .push_wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(listener.abort_handle());
    let worker = state.clone();
    let mut work = tokio::spawn(async move {
        while let Some((push, hold)) = received.recv().await {
            // The app takes over; what is queued, its sync brings.
            if worker.push_yielded.load(Ordering::SeqCst) {
                break;
            }
            let message = matches!(push, PushEvent::Message(_));
            if tokio::time::timeout(EVENT_LIMIT, wake_event(&worker, push)).await.is_err() {
                log(&worker, "warn", "a push took too long to read".to_owned());
                // Could not read it is not nothing came: said, not what.
                if message {
                    worker.sink.emit(event("push.banner", json!({ "generic": true })));
                }
            }
            drop(hold);
        }
    });
    if state.push_yielded.load(Ordering::SeqCst) {
        listener.abort();
    }
    let outcome = listener.await;
    // The push in hand finishes, so a token it refreshed is in the client. A
    // yield arriving meanwhile shortens the wait.
    let started = tokio::time::Instant::now();
    let mut yielded_at = None;
    loop {
        if tokio::time::timeout(Duration::from_millis(250), &mut work).await.is_ok() {
            break;
        }
        if yielded_at.is_none() && state.push_yielded.load(Ordering::SeqCst) {
            yielded_at = Some(tokio::time::Instant::now());
        }
        let over = match yielded_at {
            Some(at) => at.elapsed() >= YIELD_FINISH,
            None => started.elapsed() >= WORK_LIMIT,
        };
        if over {
            work.abort();
            break;
        }
    }
    let yielded = state.push_yielded.load(Ordering::SeqCst);
    // Saves refreshed tokens before the store is let go.
    // A restore holding the gate persists by itself before it lets go.
    if yielded {
        if tokio::time::timeout(YIELD_SAVE, lifecycle::close_session(state)).await.is_err() {
            log(state, "warn", "yielded before the session could be saved".to_owned());
        }
    } else {
        lifecycle::close_session(state).await;
    }
    *state
        .push_wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    match outcome {
        Ok(Ok(())) => state.sink.emit(reply_ok(id, json!({ "yielded": yielded }))),
        Ok(Err(error)) => state
            .sink
            .emit(reply_error(id, crate::text::scrub_ids(&error.to_string()))),
        Err(_) => state.sink.emit(reply_ok(id, json!({ "yielded": true }))),
    }
}

pub(super) fn push_yield(state: &Arc<State>, id: u64) {
    state.push_yielded.store(true, Ordering::SeqCst);
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

/// The session for the woken process, restored only now that a push with a
/// valid token arrived. The front end reads the key on `push.unlock`.
async fn wake_session(state: &Arc<State>) -> bool {
    if session_generation(state).is_some() {
        return true;
    }
    let mut answered = state.push_unlocked.subscribe();
    if !state.push_unlock_asked.swap(true, Ordering::SeqCst) {
        state.sink.emit(event("push.unlock", json!({})));
    }
    let _ = tokio::time::timeout(UNLOCK_LIMIT, answered.wait_for(|done| *done)).await;
    session_generation(state).is_some()
}

/// The store key the woken process asked for, or none where secretsd would not
/// give it quietly. Refused unless asked.
pub(super) async fn push_unlock(state: &Arc<State>, id: u64, store_key: Option<String>) {
    if !woken(state) || !state.push_unlock_asked.load(Ordering::SeqCst) {
        if let Some(mut key) = store_key {
            use zeroize::Zeroize;
            key.zeroize();
        }
        state.sink.emit(reply_error(id, "no push is waiting for the store"));
        return;
    }
    restore_session(state, id, store_key).await;
    state.push_unlocked.send_replace(true);
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
            if !wake_session(state).await {
                // Locked after a reboot: that something came is said, not what.
                state
                    .sink
                    .emit(event("push.banner", json!({ "generic": true })));
                return;
            }
            match notification(state, room_id, event_id).await {
                Ok(data) => state.sink.emit(event("push.banner", data)),
                Err(unshown) if unshown.silent() => {}
                Err(unshown) => {
                    log(
                        state,
                        "info",
                        format!("push shown without its message: {}", unshown.reason()),
                    );
                    state
                        .sink
                        .emit(event("push.banner", json!({ "generic": true })));
                }
            }
        }
        // The distributor's own messages are not pushes: no store key for them.
        // Without a session from a push, the app's next start registers.
        PushEvent::NewEndpoint(endpoint) => match session_generation(state) {
            Some(generation) => {
                if let Err(error) = push_sync(state, generation, endpoint.matrix).await {
                    log(
                        state,
                        "warn",
                        format!("the new push address was not registered: {}", error.message),
                    );
                }
            }
            None => log(state, "info", "the new push address waits for the app".to_owned()),
        },
        PushEvent::Unregistered => {
            let failure = match session_generation(state) {
                Some(generation) => {
                    let _serial = state.push_sync.lock().await;
                    clear_session_pushers(state, generation).await
                }
                None => Some("the pusher waits for the app to be removed".to_owned()),
            };
            push_report(state, Some("unregistered"), failure).await;
        }
        PushEvent::Failed(error) => {
            log(state, "warn", crate::text::scrub_ids(&error.to_string()));
        }
        PushEvent::Raw(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{drop_step, DropStep};

    #[test]
    fn switching_off_forgets_only_where_no_pusher_is_left() {
        assert_eq!(drop_step(true, true, true), DropStep::Forget);
        assert_eq!(drop_step(true, false, true), DropStep::ForgetStored);
        assert_eq!(drop_step(false, true, true), DropStep::Disable);
        assert_eq!(drop_step(false, false, true), DropStep::ConnectAndDisable);
        assert_eq!(drop_step(false, false, false), DropStep::Nothing);
    }
}
