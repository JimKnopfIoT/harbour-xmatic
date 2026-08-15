//! Building the Matrix client and persisting the session across restarts.
//!
//! Two things have to survive an app restart: the OAuth client registration
//! plus tokens (a small JSON file) and the state and crypto stores (SQLite,
//! managed by the SDK). Both live below the application data directory, which
//! Sailjail confines to this app.

use std::path::{Path, PathBuf};

use base64::Engine;
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    authentication::oauth::{ClientId, OAuthSession, UserSession},
    cross_process_lock::CrossProcessLockConfig,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    AuthSession, Client, SqliteStoreConfig,
};
use matrix_sdk_store_encryption::StoreCipher;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// The 32-byte store key the front end fetched from Sailfish Secrets.
/// Wiped on drop. When it is absent — no secrets daemon, denied permission —
/// everything below degrades to the unencrypted behaviour instead of
/// blocking, and says so once.
pub type StoreKey = Zeroizing<[u8; 32]>;

/// Decodes the base64 key from the front end. `None` for anything that is
/// not exactly 32 bytes — a truncated key must not silently become a
/// different key.
pub fn decode_key(encoded: &str) -> Option<StoreKey> {
    let mut bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let result = <[u8; 32]>::try_from(bytes.as_slice()).ok().map(Zeroizing::new);
    bytes.zeroize();
    result
}

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

/// The persisted form of a session, in one of two shapes: OAuth (browser and
/// device-code logins) or classic (`m.login.password`).
///
/// `untagged`, and the OAuth variant first, because every `session.json`
/// written before the password login existed is OAuth-shaped and has to keep
/// parsing — a tag would invalidate all of them. The variants cannot be
/// confused: OAuth carries `client_id` + `user`, the classic shape carries
/// `matrix`. `OAuthSession` itself is not serialisable, so its two halves are
/// stored explicitly; the SDK's `MatrixSession` is.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StoredSession {
    OAuth {
        homeserver: String,
        client_id: String,
        user: UserSession,
    },
    Matrix {
        homeserver: String,
        matrix: MatrixSession,
    },
}

impl StoredSession {
    pub fn from_oauth(homeserver: String, session: &OAuthSession) -> Self {
        Self::OAuth {
            homeserver,
            client_id: session.client_id.as_str().to_owned(),
            user: session.user.clone(),
        }
    }

    pub fn from_matrix(homeserver: String, session: MatrixSession) -> Self {
        Self::Matrix {
            homeserver,
            matrix: session,
        }
    }

    pub fn homeserver(&self) -> &str {
        match self {
            Self::OAuth { homeserver, .. } | Self::Matrix { homeserver, .. } => homeserver,
        }
    }

    /// The SDK session to restore, whichever auth API owns it.
    pub fn into_auth_session(self) -> AuthSession {
        match self {
            Self::OAuth {
                client_id, user, ..
            } => AuthSession::OAuth(
                OAuthSession {
                    client_id: ClientId::new(client_id),
                    user,
                }
                .into(),
            ),
            Self::Matrix { matrix, .. } => AuthSession::Matrix(matrix),
        }
    }
}

/// Whether the SQLite stores were created under the store key.
///
/// The marker matters because a store created *without* encryption has no
/// cipher row at all: opening it with a key would silently mint a fresh
/// cipher and turn every existing value into garbage. So the key is only
/// applied to stores born encrypted — a fresh (empty) store directory with a
/// key available gets the marker and starts encrypted, an existing store
/// without the marker keeps opening unencrypted until a sign-out clears it.
pub fn store_marked_encrypted(paths: &Paths) -> bool {
    paths.store.join(".encrypted").exists()
}

fn store_key_applies(paths: &Paths, key: Option<&StoreKey>) -> bool {
    if key.is_none() {
        return false;
    }
    if store_marked_encrypted(paths) {
        return true;
    }
    let fresh = match std::fs::read_dir(&paths.store) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true,
    };
    if fresh {
        // Losing the marker while keeping the store would flip the store
        // back to "open without key" and shred it — so it is written before
        // the store exists, and a failed write means staying unencrypted.
        std::fs::write(paths.store.join(".encrypted"), b"xmatic store key v1\n").is_ok()
    } else {
        false
    }
}

/// Builds a client for `server`, which may be a server name such as
/// `matrix.org` or a full homeserver URL.
///
/// `key` encrypts the SQLite stores — applied under the rules of
/// `store_key_applies`, so existing unencrypted stores keep working.
pub async fn build_client(
    server: &str,
    paths: &Paths,
    key: Option<&StoreKey>,
) -> Result<Client, matrix_sdk::ClientBuildError> {
    let store_key = if store_key_applies(paths, key) { key } else { None };
    let store_config =
        SqliteStoreConfig::new(&paths.store).key(store_key.map(|key| &**key));

    Client::builder()
        .server_name_or_homeserver_url(server)
        .sqlite_store_with_config_and_cache_path(store_config, None::<&Path>)
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
        // default strategy is Manual.
        //
        // `AfterDecryptionFailure` rather than `OneShot`, which this used to
        // be. `OneShot` downloads the whole backup once, the moment the key
        // arrives, and nothing else ever: its failure is only logged and the
        // backup still reports itself as enabled (`encryption/backups/mod.rs`),
        // so one bad moment on mobile data leaves the history permanently
        // unreadable while the UI claims all is well. The SDK says of that call
        // itself that it "is not paginated" and "doesn't work for any sizeable
        // account". Only `AfterDecryptionFailure` registers the per-event
        // handler that fetches a missing key when a message actually cannot be
        // read — the two are an exact equality check in the SDK
        // (`encryption/mod.rs`), not a fallback chain, which is what the
        // comment here used to claim.
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: false,
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            auto_enable_backups: false,
        })
        .build()
        .await
}

/// The encrypted shape of `session.json`: a fresh `StoreCipher` per write,
/// exported under the store key, next to the data it encrypted. The same
/// cipher the SDK's SQLite stores use — no second cryptography to audit.
#[derive(Serialize, Deserialize)]
struct EncryptedSession {
    cipher: String,
    data: String,
}

#[derive(Serialize, Deserialize)]
struct EncryptedEnvelope {
    encrypted: EncryptedSession,
}

/// Writes the session to disk with owner-only permissions — encrypted under
/// the store key when there is one, plaintext otherwise (degrade, not block).
pub fn store(
    session: &StoredSession,
    path: &Path,
    key: Option<&StoreKey>,
) -> Result<(), std::io::Error> {
    let invalid = |error: String| std::io::Error::new(std::io::ErrorKind::InvalidData, error);

    let json = if let Some(key) = key {
        let cipher = StoreCipher::new().map_err(|error| invalid(error.to_string()))?;
        let engine = base64::engine::general_purpose::STANDARD;
        let envelope = EncryptedEnvelope {
            encrypted: EncryptedSession {
                cipher: engine.encode(
                    cipher
                        .export_with_key(&**key)
                        .map_err(|error| invalid(error.to_string()))?,
                ),
                data: engine.encode(
                    cipher
                        .encrypt_value(session)
                        .map_err(|error| invalid(error.to_string()))?,
                ),
            },
        };
        serde_json::to_vec_pretty(&envelope).map_err(|error| invalid(error.to_string()))?
    } else {
        serde_json::to_vec_pretty(session).map_err(|error| invalid(error.to_string()))?
    };

    std::fs::write(path, json)?;
    restrict_permissions(path)
}

/// Reads a previously stored session, or `None` if there is none or it can no
/// longer be read — a stale file must never block the user from logging in
/// again.
///
/// Both shapes are accepted regardless of the key: a plaintext file from
/// before the store key existed still loads (and is rewritten encrypted after
/// the next successful restore), and an encrypted file without a key is
/// simply gone — the user signs in again, which is the honest outcome when
/// the secrets store lost the key.
pub fn load(path: &Path, key: Option<&StoreKey>) -> Option<StoredSession> {
    let bytes = std::fs::read(path).ok()?;
    if let Ok(session) = serde_json::from_slice::<StoredSession>(&bytes) {
        return Some(session);
    }

    let envelope = serde_json::from_slice::<EncryptedEnvelope>(&bytes).ok()?;
    let key = key?;
    let engine = base64::engine::general_purpose::STANDARD;
    let cipher = StoreCipher::import_with_key(
        &**key,
        &engine.decode(envelope.encrypted.cipher).ok()?,
    )
    .ok()?;
    cipher
        .decrypt_value(&engine.decode(envelope.encrypted.data).ok()?)
        .ok()
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
