//! Key backup and recovery. Megolm keys reach only the devices that existed
//! when a message was sent; the backup is what lets a later one read history.

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

/// Forwards every change of backup and recovery state. The key normally arrives
/// after a verification and the SDK acts on it alone - nothing informs the caller.
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

            // The whole status rather than the one value that changed: the page reads them
            // together, and a field costs nothing next to a to-device round trip.
            sink.emit(event("encryption.changed", status(&client).await));
        }
    })
}

/// What the encryption page shows.
pub async fn status(client: &Client) -> Value {
    let encryption = client.encryption();

    // `fetch_exists_on_server`, because the cached answer is stale for a backup
    // made elsewhere - and `null` where the request failed: a failure is not a no.
    let exists_on_server = match encryption.backups().fetch_exists_on_server().await {
        Ok(exists) => Some(exists),
        Err(_) => encryption.backups().exists_on_server().await.ok(),
    };

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
    // Asked before the key is offered to anything: an account with no secret
    // storage has nothing a recovery key could open, and the SDK's words are worse.
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
