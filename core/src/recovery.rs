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

use matrix_sdk::{
    encryption::{backups::BackupState, recovery::RecoveryState},
    ruma::RoomId,
    Client,
};
use serde_json::{json, Value};

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

/// What the encryption page shows.
pub async fn status(client: &Client) -> Value {
    let encryption = client.encryption();

    let exists_on_server = encryption
        .backups()
        .exists_on_server()
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
