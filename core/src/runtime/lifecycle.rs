//! The session's life cycle: restore, the three sign-ins, persisting,
//! expiry and sign-out. Everything here runs under `session_gate`; see `gate`.

use super::*;

pub(super) async fn restore_session(state: &Arc<State>, id: u64, store_key: Option<String>) {
    let gate = state.session_gate.lock().await;
    // A restore or a sign-in holds the client: it stays, and so does its key.
    if !gate::key_replaceable(state.slot()) {
        if let Some(mut key) = store_key {
            use zeroize::Zeroize;
            key.zeroize();
        }
        state.sink.emit(reply_ok(id, state.session_data().await));
        return;
    }
    restore_stored_session(state, &gate, id, store_key, true).await
}

/// `repair` is spent on the first try: a store that is still unreadable after
/// its damaged rows went is not a store one more round of deleting helps.
async fn restore_stored_session(
    state: &Arc<State>,
    gate: &Gate<'_>,
    id: u64,
    store_key: Option<String>,
    repair: bool,
) {
    // A key handed in with the command replaces the one from start - but only a
    // well-formed one: a garbled key must not become "no key".
    if let Some(mut encoded) = store_key {
        use zeroize::Zeroize;
        let decoded = session::decode_key(&encoded);
        encoded.zeroize();
        if decoded.is_some() {
            *state
                .store_key
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = decoded;
        }
    }
    let key = state.store_key();

    let stored = match session::load(&state.paths.session_file, key.as_ref()) {
        session::LoadOutcome::Session(stored) => stored,
        session::LoadOutcome::None => {
            state.sink.emit(reply_ok(id, json!({ "state": "none" })));
            return;
        }
        // Session there, key not: a state of its own, never "no session". The UI
        // retries, and no login may reset the store while the file is on disk.
        session::LoadOutcome::Locked => {
            let data = json!({ "state": "locked" });
            state.sink.emit(reply_ok(id, data.clone()));
            state.sink.emit(event("session.changed", data));
            return;
        }
        // Unknown format, not a wrong key: no reset offered.
        session::LoadOutcome::Newer => {
            let data = json!({ "state": "newer" });
            state.sink.emit(reply_ok(id, data.clone()));
            state.sink.emit(event("session.changed", data));
            return;
        }
    };

    let homeserver = stored.homeserver().to_owned();
    let homeserver_url = stored.homeserver_url().map(str::to_owned);

    // An encrypted store without its key is the same locked state — opening
    // it anyway would surface as decryption garbage three layers further down.
    if session::store_marked_encrypted(&state.paths) && key.is_none() {
        let data = json!({ "state": "locked" });
        state.sink.emit(reply_ok(id, data.clone()));
        state.sink.emit(event("session.changed", data));
        return;
    }

    // Kept address: no network. Older files: discovery once.
    let target = match &homeserver_url {
        Some(url) => session::Target::Resolved(url),
        None => session::Target::Discover(&homeserver),
    };
    let built = tokio::time::timeout(
        BUILD_LIMIT,
        session::build_client_at(target, &state.paths, key.as_ref()),
    )
    .await
    .unwrap_or_else(|_| Err(session::BuildFailure::Unreachable("no answer from the homeserver".to_owned())));
    let client = match built {
        Ok(client) => client,
        // Never an error: the front end would show the login page.
        Err(session::BuildFailure::Unreachable(reason)) => {
            let data = json!({ "state": "offline", "reason": reason });
            state.sink.emit(reply_ok(id, data.clone()));
            state.sink.emit(event("session.changed", data));
            return;
        }
        Err(failure @ session::BuildFailure::Store(_)) => {
            let data = json!({ "state": "unreadable", "reason": failure.to_string() });
            state.sink.emit(reply_ok(id, data.clone()));
            state.sink.emit(event("session.changed", data));
            return;
        }
        Err(session::BuildFailure::Refused(reason)) => {
            state.sink.emit(reply_error(id, reason));
            return;
        }
    };

    // Before the token reaches the client: https only.
    if let Err(message) = require_https(&client) {
        state.sink.emit(reply_error(id, message));
        return;
    }

    if let Err(error) = client
        .restore_session_with(stored.into_auth_session(), RoomLoadSettings::default())
        .await
    {
        // A store that cannot be read back is not an ended session: account and
        // crypto store are fine, and a login over them would cost the device.
        if matches!(error, matrix_sdk::Error::StateStore(_)) {
            // One row is not the store. The client goes first, because the retry
            // builds a second one over the same directory, then the rows nothing
            // can decode go, then the restore gets its one second try.
            drop(client);
            if repair {
                // `Error::StateStore` is what got here; the crypto store is not
                // implicated and is not walked.
                match scrub_stores(state, storehealth::Scope::State).await {
                    Ok(report) if report.dropped() > 0 => {
                        log(state, "warn", format!(
                            "dropped {} unreadable row(s) of {} from the local data",
                            report.dropped(),
                            report.checked
                        ));
                        Box::pin(restore_stored_session(state, gate, id, None, false)).await;
                        return;
                    }
                    Ok(_) => {}
                    Err(error) => log(state, "warn", format!("the local data was not repaired: {error}")),
                }
            }
            let data = json!({
                "state": "unreadable",
                "reason": crate::text::scrub_ids(&error.to_string()),
            });
            state.sink.emit(reply_ok(id, data.clone()));
            state.sink.emit(event("session.changed", data));
            return;
        }
        // Local failure: the data, not the session.
        let data = json!({
            "state": "unreadable",
            "reason": crate::text::scrub_ids(&error.to_string()),
        });
        state.sink.emit(reply_ok(id, data.clone()));
        state.sink.emit(event("session.changed", data));
        return;
    }

    // Rewritten once per restore, so a plaintext session file becomes encrypted
    // now - a classic session has no refresh to do it eventually.
    let generation = state.assign(gate, client.clone(), Phase::LoggingIn).await;
    begin_session(state, gate, &client, homeserver, generation).await;
    let data = state.session_data().await;
    state.sink.emit(reply_ok(id, data.clone()));
    state.sink.emit(event("session.changed", data));
}

/// The way out of `unreadable`: drops what the next sync rebuilds, restores
/// again. Refused while a client holds the store.
pub(super) async fn rebuild_store(state: &Arc<State>, id: u64) {
    {
        // Released before the restore below.
        let _gate = state.session_gate.lock().await;
        if state.client.lock().await.is_some() {
            state.sink.emit(reply_error(id, "the local data is in use"));
            return;
        }
        if let Err(error) = session::rebuild_store(&state.paths) {
            state.sink.emit(reply_error(
                id,
                crate::text::scrub_ids(&format!("the local data could not be rebuilt: {error}")),
            ));
            return;
        }
    }
    // The damage was in what this just deleted. Left standing, the latch stops
    // the sync over a store that no longer exists - and this is the way out the
    // page offers, so it has to be one.
    storehealth::clear();
    restore_session(state, id, None).await;
}

/// Clears the ground for a login that starts a new device: stops a sign-in
/// in progress, drops its client, then resets. Resetting under an open store shreds it.
async fn prepare_fresh_login(
    state: &Arc<State>,
    gate: &Gate<'_>,
    key: Option<&session::StoreKey>,
) -> Result<(), String> {
    cancel_login(state, gate).await;
    drop(state.unassign(gate).await);
    match session::load(&state.paths.session_file, key) {
        session::LoadOutcome::None => {
            session::reset_store(&state.paths)
                .map_err(|error| format!("could not clear old data: {error}"))?;
            storehealth::clear();
        }
        session::LoadOutcome::Session(_) => {}
        // A locked session is a session. Logging in over it resets the store its key
        // still protects; the way out of a lost key is an explicit sign-out.
        session::LoadOutcome::Locked => {
            return Err(
                "a session is stored but its key is not available; unlock or sign out first"
                    .to_owned(),
            );
        }
        session::LoadOutcome::Newer => {
            return Err("a session is stored by a newer version of the app".to_owned());
        }
    }
    state
        .paths
        .prepare()
        .map_err(|error| format!("could not prepare storage: {error}"))
}

/// Phase one of every sign-in, under the gate: one key read, a fresh store, a
/// client assigned before any network wait so a restore finds it taken.
async fn fresh_login_client(
    state: &Arc<State>,
    gate: &Gate<'_>,
    homeserver: &str,
) -> Result<(Client, u64), String> {
    let key = state.store_key();
    prepare_fresh_login(state, gate, key.as_ref()).await?;
    let client = tokio::time::timeout(
        BUILD_LIMIT,
        session::build_client(homeserver, &state.paths, key.as_ref()),
    )
    .await
    .map_err(|_| "no answer from the homeserver".to_owned())??;
    let generation = state.assign(gate, client.clone(), Phase::LoggingIn).await;
    Ok((client, generation))
}

/// Stops a sign-in's network part and waits until it is gone, client clone
/// included. Under the gate, so the task can at most be waiting for it.
async fn cancel_login(state: &Arc<State>, _gate: &Gate<'_>) {
    if let Some(pending) = state.pending.lock().await.take() {
        pending.shutdown.shutdown();
        if let Some(client) = &*state.client.lock().await {
            client.oauth().abort_login(&pending.state).await;
        }
    }
    let task = state.login_task.lock().await.take();
    if let Some(task) = task {
        task.handle.abort();
        let _ = task.handle.await;
        answer_once(&state.sink, &task.answered, reply_error(task.reply_to, "the sign-in was cancelled"));
    }
}

/// Phase three, under the gate: observers first, so no refresh slips past
/// the write, then the session file.
async fn begin_session(
    state: &Arc<State>,
    gate: &Gate<'_>,
    client: &Client,
    homeserver: String,
    generation: u64,
) {
    verification::install(client, state.sink.clone(), state.verification.clone());
    call::install(client, state.sink.clone());
    // The backup unlocks itself once a verification hands the key over, and only
    // this stream says so. Kept, because it holds a client clone.
    let recovery_task = recovery::watch(client, state.sink.clone());
    let session_task = watch_session(state, client, homeserver.clone(), generation);
    persist(state, gate, client, homeserver).await;
    state
        .observers
        .lock()
        .await
        .extend([recovery_task, session_task]);
    state.with_slot(gate, |slot| slot.advance(generation, Phase::Session));
    // After the gate is released; it checks the generation itself.
    let push = state.clone();
    tokio::spawn(async move { super::pushcmd::push_session_started(&push, generation).await });
}

/// Saves tokens and drops the client without signing out. For the woken process.
pub(super) async fn close_session(state: &Arc<State>) {
    let gate = state.session_gate.lock().await;
    let client = state.client.lock().await.clone();
    if let (Some(client), Phase::Session) = (client, state.slot().phase) {
        // Save refreshed tokens.
        if let session::LoadOutcome::Session(stored) =
            session::load(&state.paths.session_file, state.store_key().as_ref())
        {
            persist(state, &gate, &client, stored.homeserver().to_owned()).await;
        }
    }
    stop_observers(state).await;
    drop(state.unassign(&gate).await);
}

/// Refuses a homeserver not reached over https. Asked of the client, since
/// `.well-known` may name an http URL and the SDK then goes insecure.
fn require_https(client: &Client) -> Result<(), String> {
    if client.homeserver().scheme() == "https" {
        return Ok(());
    }
    Err("this homeserver is not reached over https".to_owned())
}

pub(super) async fn start_login(state: &Arc<State>, id: u64, homeserver: String) {
    let gate = state.session_gate.lock().await;
    if let Err(message) = gate::login_entry(state.slot(), LoginKind::Start) {
        state.sink.emit(reply_error(id, message));
        return;
    }
    let (client, generation) = match fresh_login_client(state, &gate, &homeserver).await {
        Ok(built) => built,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };
    let worker = state.clone();
    spawn_login(state, &gate, id, move |answered| {
        browser_login(worker, id, answered, client, homeserver, generation)
    })
    .await;
}

/// Discovery, the authorization URL and the wait for the browser, all in the
/// task a sign-out can stop. Answers the command itself.
async fn browser_login(
    state: Arc<State>,
    id: u64,
    answered: Arc<std::sync::atomic::AtomicBool>,
    client: Client,
    homeserver: String,
    generation: u64,
) {
    let reply = |value: Value| answer_once(&state.sink, &answered, value);
    // Every way out below that is not a running login gives the client back.
    let release = |message: String| {
        let state = state.clone();
        let answered = answered.clone();
        async move {
            let gate = state.session_gate.lock().await;
            state.unassign_if(&gate, generation).await;
            drop(gate);
            answer_once(&state.sink, &answered, reply_error(id, message));
        }
    };

    if let Err(message) = require_https(&client) {
        release(message).await;
        return;
    }

    // Which sign-in does this server speak? Only the affirmative `NotSupported`
    // offers the password form - a transport error is never a downgrade.
    match client.oauth().server_metadata().await {
        Ok(_) => {}
        Err(OAuthDiscoveryError::NotSupported) => {
            let flows = login::login_flows(&client).await;
            match flows.as_ref().map(|list| list.iter().any(|flow| flow == "password")) {
                Ok(true) => {
                    // No scheme check: anything but https was refused above. The client is kept
                    // so `login.password` reuses this discovery instead of trusting the UI.
                    let gate = state.session_gate.lock().await;
                    let kept = state.with_slot(&gate, |slot| slot.advance(generation, Phase::Prelogin));
                    drop(gate);
                    if kept {
                        reply(reply_ok(id, json!({ "passwordLogin": true })));
                    } else {
                        reply(reply_error(id, "the sign-in was cancelled"));
                    }
                }
                // Naming the method the server wants is the point: told only "sign-in
                // failed", a user retypes a password that was never wrong.
                Ok(false) => {
                    let sso = flows
                        .as_ref()
                        .map(|list| list.iter().any(|flow| flow == "sso"))
                        .unwrap_or(false);
                    release(
                        if sso {
                            "this server signs in through its own web page (SSO), which this app cannot do yet"
                        } else {
                            "this server offers no sign-in method this app supports"
                        }
                        .to_owned(),
                    )
                    .await;
                }
                Err(_) => {
                    release(flows.err().unwrap_or_else(|| "sign-in methods unknown".to_owned()))
                        .await
                }
            }
            return;
        }
        Err(error) => {
            release(format!(
                "sign-in discovery failed: {}",
                crate::text::scrub_ids(&error.to_string())
            ))
            .await;
            return;
        }
    }

    let pending = match login::start(&client, homeserver).await {
        Ok(pending) => pending,
        Err(message) => {
            release(message).await;
            return;
        }
    };

    {
        let gate = state.session_gate.lock().await;
        if !state.slot().holds(generation) {
            pending.redirect.shutdown_handle().shutdown();
            drop(gate);
            reply(reply_error(id, "the sign-in was cancelled"));
            return;
        }
        *state.pending.lock().await = Some(PendingLogin {
            shutdown: pending.redirect.shutdown_handle(),
            state: pending.state,
        });
    }
    // The URL goes back right away; the flow itself finishes whenever the user
    // is done in the browser.
    reply(reply_ok(id, json!({ "url": pending.url.to_string() })));

    let outcome = login::finish(&client, pending.redirect).await;
    let gate = state.session_gate.lock().await;
    if !state.slot().holds(generation) {
        return;
    }
    state.pending.lock().await.take();
    match outcome {
        Ok(true) => {
            begin_session(&state, &gate, &client, pending.homeserver, generation).await;
            drop(gate);
            let data = state.session_data().await;
            state.sink.emit(event("session.changed", data));
        }
        Ok(false) => {
            state.unassign_if(&gate, generation).await;
            drop(gate);
            state
                .sink
                .emit(event("login.aborted", json!({ "state": "none" })));
        }
        Err(message) => {
            state.unassign_if(&gate, generation).await;
            drop(gate);
            state
                .sink
                .emit(event("login.failed", json!({ "message": crate::text::scrub_ids(&message) })));
        }
    }
}

/// Signs in with `m.login.password`; the reply is the whole outcome. The
/// checks that put the form on screen are repeated here, not trusted.
pub(super) async fn password_login(
    state: &Arc<State>,
    id: u64,
    homeserver: String,
    user: String,
    password: Secret,
) {
    // Reuse the client `login.start` built and vetted. The reset belongs to
    // whoever builds the client - running it again deleted the open files.
    let gate = state.session_gate.lock().await;
    let (client, generation) = match gate::login_entry(state.slot(), LoginKind::Password) {
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
        Ok(Entry::Reuse(generation)) => {
            let cached = state.client.lock().await.clone();
            let Some(client) = cached else {
                state.sink.emit(reply_error(id, "the sign-in was cancelled"));
                return;
            };
            state.with_slot(&gate, |slot| slot.advance(generation, Phase::LoggingIn));
            (client, generation)
        }
        Ok(Entry::Fresh) => match fresh_login_client(state, &gate, &homeserver).await {
            Ok(built) => built,
            Err(message) => {
                state.sink.emit(reply_error(id, message));
                return;
            }
        },
    };

    // The network part as a task of its own, so a sign-out can stop it and
    // wait for its client clone. It answers the command itself.
    let worker = state.clone();
    spawn_login(state, &gate, id, move |answered| async move {
        let reply = |value: Value| answer_once(&worker.sink, &answered, value);
        let outcome = password_attempt(&client, &user, &password).await;
        let gate = worker.session_gate.lock().await;
        if !worker.slot().holds(generation) {
            drop(gate);
            reply(reply_error(id, "the sign-in was cancelled"));
            return;
        }
        match outcome {
            // A wrong password keeps the vetted client: the form stays usable.
            Err(message) => {
                worker.with_slot(&gate, |slot| slot.advance(generation, Phase::Prelogin));
                drop(gate);
                reply(reply_error(id, message));
            }
            Ok(()) => {
                begin_session(&worker, &gate, &client, homeserver, generation).await;
                drop(gate);
                let data = worker.session_data().await;
                reply(reply_ok(id, data.clone()));
                worker.sink.emit(event("session.changed", data));
            }
        }
    })
    .await;
}

/// The checks that put the form on screen, repeated rather than trusted, then
/// the request itself.
async fn password_attempt(client: &Client, user: &str, password: &Secret) -> Result<(), String> {
    require_https(client)?;
    match client.oauth().server_metadata().await {
        Err(OAuthDiscoveryError::NotSupported) => {}
        Ok(_) => {
            return Err(
                "this server signs in through its own page, not with a password here".to_owned(),
            )
        }
        Err(error) => {
            return Err(format!(
                "sign-in discovery failed: {}",
                crate::text::scrub_ids(&error.to_string())
            ))
        }
    }
    login::password(client, user, password.as_str()).await
}

/// Begins the device-code login: the reply carries a short URL and a code, and
/// the approval happens on some other device.
pub(super) async fn start_device_login(state: &Arc<State>, id: u64, homeserver: String) {
    let gate = state.session_gate.lock().await;
    if let Err(message) = gate::login_entry(state.slot(), LoginKind::Start) {
        state.sink.emit(reply_error(id, message));
        return;
    }
    let (client, generation) = match fresh_login_client(state, &gate, &homeserver).await {
        Ok(built) => built,
        Err(message) => {
            state.sink.emit(reply_error(id, message));
            return;
        }
    };
    let worker = state.clone();
    spawn_login(state, &gate, id, move |answered| async move {
        let reply = |value: Value| answer_once(&worker.sink, &answered, value);
        let started = match require_https(&client) {
            Ok(()) => login::start_device(&client).await,
            Err(message) => Err(message),
        };
        let pending = match started {
            Ok(pending) => pending,
            Err(message) => {
                let gate = worker.session_gate.lock().await;
                worker.unassign_if(&gate, generation).await;
                drop(gate);
                reply(reply_error(id, message));
                return;
            }
        };
        // URL and code go back right away; the polling finishes whenever the
        // user approves on the other device.
        reply(reply_ok(
            id,
            json!({
                "verificationUri": pending.verification_uri,
                "verificationUriComplete": pending.verification_uri_complete,
                "userCode": pending.user_code,
            }),
        ));

        let outcome = login::finish_device(&client, pending).await;
        let gate = worker.session_gate.lock().await;
        if !worker.slot().holds(generation) {
            return;
        }
        match outcome {
            Ok(()) => {
                begin_session(&worker, &gate, &client, homeserver, generation).await;
                drop(gate);
                let data = worker.session_data().await;
                worker.sink.emit(event("session.changed", data));
            }
            Err(message) => {
                worker.unassign_if(&gate, generation).await;
                drop(gate);
                worker
                    .sink
                    .emit(event("login.failed", json!({ "message": crate::text::scrub_ids(&message) })));
            }
        }
    })
    .await;
}

/// Writes the session to disk. A failure costs the restart, not the running
/// session, so it is an event rather than an aborted login.
async fn persist(state: &Arc<State>, _gate: &Gate<'_>, client: &Client, homeserver: String) {
    // Whichever auth API owns the session: OAuth for browser and device code, the
    // Matrix API for the password login. Only tokens are stored.
    let url = client.homeserver().to_string();
    let stored = if let Some(oauth_session) = client.oauth().full_session() {
        StoredSession::from_oauth(homeserver, url, &oauth_session)
    } else if let Some(matrix_session) = client.matrix_auth().session() {
        StoredSession::from_matrix(homeserver, url, matrix_session)
    } else {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": "session could not be persisted" }),
        ));
        return;
    };
    if let Err(error) = session::store(&stored, &state.paths.session_file, state.store_key().as_ref()) {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": crate::text::scrub_ids(&format!("session could not be saved: {error}")) }),
        ));
    }
}

/// Keeps `session.json` in step with rotating tokens: persisting only at login
/// leaves a spent refresh token and every start looks like a forced re-login.
fn watch_session(
    state: &Arc<State>,
    client: &Client,
    homeserver: String,
    generation: u64,
) -> tokio::task::JoinHandle<()> {
    let state = state.clone();
    let client = client.clone();
    let mut changes = client.subscribe_to_session_changes();
    tokio::spawn(async move {
        loop {
            match changes.recv().await {
                Ok(SessionChange::TokensRefreshed) => {
                    // Said out loud: a refresh is invisible, and a request in flight across one
                    // comes back as "token is not active", which reads as an ended session.
                    state.sink.emit(event("session.refreshed", json!({})));
                    let gate = state.session_gate.lock().await;
                    // A client already signed out writes nothing.
                    if state.slot().holds(generation) {
                        persist(&state, &gate, &client, homeserver.clone()).await;
                    }
                }
                Ok(SessionChange::UnknownToken(_)) => {
                    if !session_ended(&client).await {
                        log(&state, "warn", "a token refresh failed; the session is kept".to_owned());
                        continue;
                    }
                    // Gone for good. Own task: the teardown aborts every observer, this one too.
                    let teardown = state.clone();
                    tokio::spawn(async move {
                        if !session_expired(&teardown, generation).await {
                            return;
                        }
                        teardown.sink.emit(event(
                            "session.expired",
                            json!({ "message": "the session has expired, please sign in again" }),
                        ));
                    });
                    // Nothing more can arrive on this subscription, and this task's clone is the
                    // last thing keeping the old client alive.
                    break;
                }
                // Missing a refresh notification only costs a redundant write
                // next time; keep listening.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    })
}

/// Whether `UnknownToken` ended the session. The SDK reports every failed
/// password-login refresh as one, so it asks once more.
async fn session_ended(client: &Client) -> bool {
    if client.oauth().full_session().is_some() {
        return true;
    }
    match client.refresh_access_token().await {
        Ok(()) => false,
        // No refresh token: the access token was the session.
        Err(matrix_sdk::RefreshTokenError::RefreshTokenRequired) => true,
        Err(matrix_sdk::RefreshTokenError::MatrixAuth(error)) => matches!(
            error.client_api_error_kind(),
            Some(matrix_sdk::ruma::api::error::ErrorKind::UnknownToken(_))
        ),
        Err(_) => false,
    }
}

pub(super) async fn registration_url(state: &Arc<State>, id: u64, homeserver: String) {
    // No store: a second pool beside a signed-in client, or one a reset is
    // deleting, is what the session gate exists to prevent.
    let built = tokio::time::timeout(BUILD_LIMIT, session::build_storeless_client(&homeserver))
        .await
        .unwrap_or_else(|_| Err("no answer from the homeserver".to_owned()));
    let client = match built {
        Ok(client) => client,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, error));
            return;
        }
    };

    if let Err(message) = require_https(&client) {
        state.sink.emit(reply_error(id, message));
        return;
    }

    match login::registration_url(&client).await {
        Ok(url) => state.sink.emit(reply_ok(id, json!({ "url": url }))),
        Err(message) => state.sink.emit(reply_error(id, message)),
    }
}

pub(super) async fn abort_login(state: &Arc<State>, id: u64) {
    let gate = state.session_gate.lock().await;
    cancel_login(state, &gate).await;
    // A signed-in client is not a sign-in to abort.
    if state.slot().phase != Phase::Session {
        drop(state.unassign(&gate).await);
    }
    drop(gate);
    state.sink.emit(reply_ok(id, json!({ "state": "none" })));
}

/// Stops the observers that hold a client clone. Called before the client is
/// dropped, by both the deliberate sign-out and an expired session.
async fn stop_observers(state: &Arc<State>) {
    for task in state.observers.lock().await.drain(..) {
        task.abort();
    }
}

/// The teardown of a sign-out, minus telling the server and minus the stores:
/// the account is unchanged. The session file goes, or the next start fails.
/// Only for the client that expired: a late one leaves a newer sign-in alone.
/// The rooms an account asked to follow go with it: the next account's server
/// must not be sent the old one's room IDs.
async fn forget_subscriptions(state: &Arc<State>) {
    *state.subscriptions.lock().await = Subscriptions::default();
    *state.call_subscribed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

async fn session_expired(state: &Arc<State>, generation: u64) -> bool {
    let gate = state.session_gate.lock().await;
    if !state.slot().holds(generation) {
        return false;
    }
    stop_observers(state).await;
    forget_subscriptions(state).await;
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close().await;
    }
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }
    // Same as in `logout`: the verification request holds a client.
    verification::cancel_active(&state.verification).await;
    drop(state.verification.lock().await.take());
    if let Some(handle) = state.directory.lock().await.take() {
        handle.task.abort();
    }
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }
    if let Some(task) = state.spaces.lock().await.take() {
        task.abort();
    }
    if let Some(handle) = state.rooms.lock().await.take() {
        handle.stop().await;
    }
    drop(state.unassign(&gate).await);
    session::forget(&state.paths.session_file);
    true
}

pub(super) async fn logout(state: &Arc<State>, id: u64) {
    // A restore would build a client over the deleted store.
    let gate = state.session_gate.lock().await;
    // A sign-in in progress holds a client clone and would write the session
    // file back over the reset below.
    cancel_login(state, &gate).await;
    // First, because everything below assumes nothing else is holding the
    // client - the store is deleted at the end of this function.
    stop_observers(state).await;
    forget_subscriptions(state).await;
    if let Some(handle) = state.timeline.lock().await.take() {
        handle.close().await;
    }
    // A thread handle holds the timeline, and through it the client and the open
    // pool - `reset_store` below would delete the directory under it.
    if let Some(handle) = state.thread.lock().await.take() {
        handle.close().await;
    }
    // A verification in progress holds a `Client` of its own: signing out during
    // one left the store open under `reset_store`.
    verification::cancel_active(&state.verification).await;
    drop(state.verification.lock().await.take());
    if let Some(handle) = state.directory.lock().await.take() {
        handle.task.abort();
    }
    if let Some(task) = state.open_space.lock().await.take() {
        task.abort();
    }
    if let Some(task) = state.spaces.lock().await.take() {
        task.abort();
    }
    if let Some(handle) = state.rooms.lock().await.take() {
        handle.stop().await;
    }
    // The handles above are ours to close, the command tasks are not - they get a
    // moment before the store goes. Bounded, see `drain_commands`.
    state.drain_commands(std::time::Duration::from_secs(2)).await;
    let client = state.unassign(&gate).await;
    // A pusher update already under way lands before the pushers are cleared.
    let push_serial =
        tokio::time::timeout(std::time::Duration::from_secs(5), state.push_sync.lock()).await;
    if let Some(client) = client {
        // Before the session, or the homeserver keeps posting to an endpoint
        // nothing can name. Bounded and best effort, like the sign-out.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::push::clear_own_pushers(&client),
        )
        .await;
        // Best effort and bounded: the local session must go even where the server
        // cannot be reached, and an unanswered logout used to hold up the whole wipe.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), client.logout()).await;
        // Explicit: `reset_store` below must not run while a client still has
        // the store directory open.
        drop(client);
    }
    drop(push_serial);
    // Unregister and delete Leghorn's state.
    super::pushcmd::push_forget(state).await;
    session::forget(&state.paths.session_file);
    // The lists that name people belong to the account that is leaving.
    session::forget(&state.paths.private_file);
    // Same for what is only in memory: remembered display names, the call
    // policy with its allow list, and who rang when.
    timeline::forget_senders();
    linkpreview::forget();
    poll::forget();
    roomlist::forget_name_requests();
    members::forget_asked();
    call::forget_state();
    // The latch outlived the store it was about: it stops the sync, and the
    // store the next sign-in builds is not the one that was damaged.
    storehealth::clear();
    if let Err(error) = session::reset_store(&state.paths) {
        state.sink.emit(event(
            "session.warning",
            json!({ "message": crate::text::scrub_ids(&format!("local data could not be cleared: {error}")) }),
        ));
    }

    state.sink.emit(reply_ok(id, json!({ "state": "none" })));
    state
        .sink
        .emit(event("session.changed", json!({ "state": "none" })));
}
