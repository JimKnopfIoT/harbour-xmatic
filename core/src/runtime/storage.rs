//! The local stores: status, scrub and repair.

use super::*;

pub(super) fn storage_status(state: &Arc<State>, id: u64) {
    let key = state.store_key();
    let storage = session::storage_state(&state.paths, key.as_ref());
    state.sink.emit(reply_ok(
        id,
        json!({
            "encrypted": storage.fully_encrypted(),
            "storeEncrypted": storage.store_encrypted,
            "sessionPresent": storage.session_present,
            "sessionEncrypted": storage.session_encrypted,
            "keyAvailable": storage.key_available,
            // True where the stores could be encrypted but are not and a key exists - the
            // only case in which offering it is honest. A sign-out away, not a switch.
            "canEncrypt": storage.key_available && !storage.store_encrypted,
        }),
    ));
}
/// Walks every row of two SQLite files, so off the runtime's workers: there are
/// two of them, and a large crypto store takes long enough to be noticed.
pub(super) async fn scrub_stores(
    state: &Arc<State>,
    scope: storehealth::Scope,
) -> Result<storehealth::Report, String> {
    let paths = state.paths.clone();
    let key = state.store_key();
    tokio::task::spawn_blocking(move || storehealth::scrub(&paths, key.as_ref(), scope))
        .await
        .map_err(|error| format!("the repair did not run: {error}"))?
}
/// Drops what the stores can no longer decode and lets the sync go on. Runs
/// with the client open: the rows it removes are rows nothing could read.
pub(super) async fn repair_storage(state: &Arc<State>, id: u64) {
    // The store the latched line named, not both: the wide sweep let a defect in
    // room data reach the room keys. Under the gate: never beside a reset.
    let gate = state.session_gate.lock().await;
    let scrubbed = scrub_stores(state, storehealth::damage_scope()).await;
    drop(gate);
    let report = match scrubbed {
        Ok(report) => report,
        Err(error) => {
            state
                .sink
                .emit(reply_error(id, crate::text::scrub_ids(&error)));
            return;
        }
    };

    // Only where something actually went: clearing the latch on a repair that
    // found nothing puts the sync straight back into the loop it ended.
    if report.dropped() > 0 {
        storehealth::clear();
        if let Some(handle) = state.rooms.lock().await.as_ref() {
            handle.sync.start().await;
        }
    }
    log(
        state,
        "warn",
        format!(
            "repair: {} of {} stored rows could not be decoded ({} room(s), {} room key(s), {} of them with no backup) \
             and were put aside into the quarantine",
            report.dropped(),
            report.checked,
            report.rooms,
            report.room_keys,
            report.room_keys_unsaved
        ),
    );

    state.sink.emit(reply_ok(
        id,
        json!({
            "checked": report.checked,
            "dropped": report.dropped(),
            "rooms": report.rooms,
            "roomKeys": report.room_keys,
            // What the key backup had no copy of - the only part of this a user
            // can lose, and what the journal alone used to carry.
            "roomKeysUnsaved": report.room_keys_unsaved,
            "kept": report.kept,
        }),
    ));
}
