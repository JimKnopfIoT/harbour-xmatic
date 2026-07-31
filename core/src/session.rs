//! Building the Matrix client and persisting the session across restarts.
//!
//! Two things have to survive an app restart: the OAuth client registration
//! plus tokens (a small JSON file) and the state and crypto stores (SQLite,
//! managed by the SDK). Both live below the application data directory, which
//! Sailjail confines to this app.

use std::path::{Path, PathBuf};

use matrix_sdk::{
    authentication::oauth::{ClientId, OAuthSession, UserSession},
    cross_process_lock::CrossProcessLockConfig,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    Client,
};
use serde::{Deserialize, Serialize};

/// Where the core keeps its state. Derived once from the data directory the
/// front end passes in, so no path is hard-coded here.
#[derive(Debug, Clone)]
pub struct Paths {
    pub store: PathBuf,
    pub session_file: PathBuf,
    /// Downloaded attachments. Separate from the store because it is
    /// disposable — every file in it can be fetched again.
    pub media_cache: PathBuf,
}

impl Paths {
    pub fn new(data_dir: &Path, cache_dir: &Path) -> Self {
        Self {
            store: data_dir.join("store"),
            session_file: data_dir.join("session.json"),
            media_cache: cache_dir.join("media"),
        }
    }

    pub fn prepare(&self) -> Result<(), std::io::Error> {
        std::fs::create_dir_all(&self.store)?;
        std::fs::create_dir_all(&self.media_cache)
    }
}

/// The persisted form of a session. `OAuthSession` itself is not serialisable,
/// so its two halves are stored explicitly, together with the homeserver the
/// client has to be rebuilt against.
#[derive(Debug, Serialize, Deserialize)]
pub struct StoredSession {
    pub homeserver: String,
    pub client_id: String,
    pub user: UserSession,
}

impl StoredSession {
    pub fn from_oauth(homeserver: String, session: &OAuthSession) -> Self {
        Self {
            homeserver,
            client_id: session.client_id.as_str().to_owned(),
            user: session.user.clone(),
        }
    }

    pub fn into_oauth(self) -> OAuthSession {
        OAuthSession {
            client_id: ClientId::new(self.client_id),
            user: self.user,
        }
    }
}

/// Builds a client for `server`, which may be a server name such as
/// `matrix.org` or a full homeserver URL.
pub async fn build_client(server: &str, paths: &Paths) -> Result<Client, matrix_sdk::ClientBuildError> {
    Client::builder()
        .server_name_or_homeserver_url(server)
        .sqlite_store(&paths.store, None)
        .handle_refresh_tokens()
        // The holder name has to differ per process. OAuth refresh tokens are
        // single-use: if two instances share one identity, whichever refreshes
        // first invalidates the other's session, and every request afterwards
        // fails with invalid_grant.
        .cross_process_store_config(CrossProcessLockConfig::multi_process(format!(
            "xmatic-{}",
            std::process::id()
        )))
        // Without this the SDK never touches the key backup on its own: the
        // default strategy is Manual. OneShot pulls everything down as soon as
        // the backup key arrives — either from unlocking recovery or from a
        // successful verification — and the per-event fallback covers keys
        // that appear later.
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: false,
            backup_download_strategy: BackupDownloadStrategy::OneShot,
            auto_enable_backups: false,
        })
        .build()
        .await
}

/// Writes the session to disk with owner-only permissions.
pub fn store(session: &StoredSession, path: &Path) -> Result<(), std::io::Error> {
    let json = serde_json::to_vec_pretty(session)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    std::fs::write(path, json)?;
    restrict_permissions(path)
}

/// Reads a previously stored session, or `None` if there is none or it can no
/// longer be parsed — a stale file must never block the user from logging in
/// again.
pub fn load(path: &Path) -> Option<StoredSession> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Removes the stored session. Missing files are not an error.
pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Throws away the state and crypto stores.
///
/// They belong to one device: the crypto store holds that device's identity
/// and keys. Keeping it across a logout makes the next login fail, because the
/// fresh device ID does not match what the store already contains. The keys
/// are worthless without the session anyway.
pub fn reset_store(paths: &Paths) -> Result<(), std::io::Error> {
    if paths.store.exists() {
        std::fs::remove_dir_all(&paths.store)?;
    }
    paths.prepare()
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}
