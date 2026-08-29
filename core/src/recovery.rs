//! Key backup and recovery.
//!
//! Megolm keys are handed out to the devices that exist when a message is
//! sent. A device that joins later has no way to ask the server for them —
//! the server only ever saw ciphertext. Key backup closes that gap: the keys
//! are uploaded encrypted under a recovery key that only the user holds, and
//! any later device can pull them down again.
//!
//! This is therefore both the answer to "my history is unreadable on the new
//! phone" and the insurance against losing it entirely when a device dies.

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::{
    encryption::{backups::BackupState, recovery::RecoveryState},
    ruma::RoomId,
    Client,
};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

use crate::protocol::event;
use crate::runtime::Sink;

fn recovery_state_name(state: RecoveryState) -> &'static str {
    match state {
        RecoveryState::Unknown => "unknown",
        RecoveryState::Enabled => "enabled",
        RecoveryState::Disabled => "disabled",
        RecoveryState::Incomplete => "incomplete",
    }
}

fn backup_state_name(state: BackupState) -> &'static str {
    match state {
        BackupState::Unknown => "unknown",
        BackupState::Creating => "creating",
        BackupState::Enabling => "enabling",
        BackupState::Resuming => "resuming",
        BackupState::Enabled => "enabled",
        BackupState::Downloading => "downloading",
        BackupState::Disabling => "disabling",
    }
}

/// Forwards every change of the backup and recovery state to the front end.
///
/// Neither is a value that can be read once and kept. The backup key normally
/// arrives *after* a successful verification, as an `m.secret.send` to-device
/// message, and the SDK acts on it entirely by itself: it resumes the backup
/// and flips the state (`encryption/backups/mod.rs`,
/// `encryption/recovery/mod.rs`). Nothing informs the caller, and there is no
/// reply to hang a refresh off.
///
/// Without this the front end kept the snapshot it took while signing in, when
/// the backup was still off — and then skipped fetching the room keys for every
/// room opened afterwards, because that is gated on the cached flag. Verifying
/// a device, whose entire purpose is making the old messages readable, changed
/// nothing at all until the app was restarted.
pub fn watch(client: &Client, sink: Arc<Sink>) -> JoinHandle<()> {
    let client = client.clone();
    tokio::spawn(async move {
        let mut backups = client.encryption().backups().state_stream();
        let mut recovery = client.encryption().recovery().state_stream();

        loop {
            // Either stream ending means the client is going away.
            let alive = tokio::select! {
                next = backups.next() => next.is_some(),
                next = recovery.next() => next.is_some(),
            };
            if !alive {
                break;
            }

            // The whole status rather than the one value that changed: the two
            // states are read together by the page that shows them, and one
            // extra field costs nothing next to a to-device round trip.
            sink.emit(event("encryption.changed", status(&client).await));
        }
    })
}

/// What the encryption page shows.
pub async fn status(client: &Client) -> Value {
    let encryption = client.encryption();

    // `fetch_exists_on_server`, not `exists_on_server`: the latter caches its
    // answer for the lifetime of the process and only forgets it when this
    // device creates or deletes a backup itself. The SDK's own documentation
    // says "Do not use this method if you need an accurate answer". Setting
    // recovery up on another client while xmatic runs would otherwise leave
    // this page offering "Set up backup" for the rest of the session — and the
    // button then fails, because enabling checks the uncached answer and
    // refuses with "backup exists on server".
    let exists_on_server = encryption
        .backups()
        .fetch_exists_on_server()
        .await
        .unwrap_or(false);

    json!({
        "recovery": recovery_state_name(encryption.recovery().state()),
        "backup": backup_state_name(encryption.backups().state()),
        "backupEnabled": encryption.backups().are_enabled().await,
        "backupOnServer": exists_on_server,
        "crossSigned": encryption
            .cross_signing_status()
            .await
            .map(|status| status.is_complete())
            .unwrap_or(false),
    })
}

/// Unlocks the backup with the user's recovery key or passphrase and imports
/// everything stored under it.
pub async fn recover(client: &Client, key: &str) -> Result<(), String> {
    // Asked before the key is offered to anything. An account with no secret
    // storage has nothing a recovery key could open, and the SDK answers that
    // with "the info about the secret key could not have been found in the
    // account data of the user" - true, and not a sentence to put in front of
    // somebody who has just typed out a key. The UI hides the field in this
    // state; this is the second half of the same guard, for every other caller.
    if client.encryption().recovery().state() == RecoveryState::Disabled {
        return Err("this account has no key backup to unlock; set one up first".to_owned());
    }
    client
        .encryption()
        .recovery()
        .recover(key.trim())
        .await
        .map_err(|error| format!("recovery failed: {error}"))
}

/// Sets up backup and returns the recovery key, which is shown to the user
/// exactly once.
pub async fn enable(client: &Client) -> Result<String, String> {
    client
        .encryption()
        .recovery()
        .enable()
        .await
        .map_err(|error| format!("could not enable backup: {error}"))
}

/// Pulls the room keys for one room out of the backup, so an already open
/// timeline can decrypt what it was missing.
pub async fn fetch_room_keys(client: &Client, room_id: &str) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    client
        .encryption()
        .backups()
        .download_room_keys_for_room(&parsed)
        .await
        .map_err(|error| format!("could not fetch room keys: {error}"))
}
