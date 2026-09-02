//! Building the client and keeping the session across restarts.
//! Session file and SQLite stores, both under the app data directory.

use std::path::{Path, PathBuf};

use base64::Engine;
use matrix_sdk::search_index::SearchIndexStoreKind;
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    authentication::oauth::{ClientId, OAuthSession, UserSession},
    cross_process_lock::CrossProcessLockConfig,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    AuthSession, Client, SqliteStoreConfig, ThreadingSupport,
};
use matrix_sdk_store_encryption::StoreCipher;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// The 32-byte store key the front end read from Sailfish Secrets.
/// Wiped on drop.
pub type StoreKey = Zeroizing<[u8; 32]>;

/// Base64 in, 32 bytes out. Any other length is `None` - a truncated
/// key must not silently become a different key.
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
    /// Recorded voice messages. Written and cleared by the Qt side, which owns
    /// the wipe setting - the core never touches them.
    pub voice_cache: PathBuf,
    /// Encrypted lists that name people (see private.rs).
    pub private_file: PathBuf,
    /// UnifiedPush token, subscription keys, distributor. Beside the
    /// session - a lost one leaves the server's endpoint pointing nowhere.
    pub push_file: PathBuf,
    /// Tantivy index per room. Beside the store rather than in the cache:
    /// expensive to rebuild, and it holds message text.
    pub search_index: PathBuf,
}

impl Paths {
    pub fn new(data_dir: &Path, cache_dir: &Path) -> Self {
        Self {
            store: data_dir.join("store"),
            session_file: data_dir.join("session.json"),
            media_cache: cache_dir.join("media"),
            voice_cache: cache_dir.join("voice"),
            private_file: data_dir.join("private.json"),
            push_file: data_dir.join("push.json"),
            search_index: data_dir.join("search"),
        }
    }

    pub fn prepare(&self) -> Result<(), std::io::Error> {
        std::fs::create_dir_all(&self.store)?;
        std::fs::create_dir_all(&self.search_index)?;
        std::fs::create_dir_all(&self.media_cache)
    }
}

/// OAuth or classic (`m.login.password`), `untagged` so every session
/// file written before the password login keeps parsing.
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

/// Whether the stores were created under the key. Opening a plaintext
/// store with one mints a fresh cipher and turns it into garbage.
pub fn store_marked_encrypted(paths: &Paths) -> bool {
    paths.store.join(".encrypted").exists()
}

/// Stores and session file are separate facts: the marker never changes
/// side, the session file follows whatever key exists at each write.
pub struct StorageState {
    /// The SQLite stores (state, crypto, event cache) are encrypted.
    pub store_encrypted: bool,
    /// A session file exists.
    pub session_present: bool,
    /// That session file is an encryption envelope rather than plaintext.
    pub session_encrypted: bool,
    /// A store key is available in this process. False means the secrets
    /// service could not deliver one, so nothing can be encrypted right now.
    pub key_available: bool,
}

impl StorageState {
    /// Everything on disk encrypted. No session file yet counts as
    /// encrypted - there is nothing else to protect.
    pub fn fully_encrypted(&self) -> bool {
        self.store_encrypted && (!self.session_present || self.session_encrypted)
    }
}

/// Reads the storage state without opening anything.
pub fn storage_state(paths: &Paths, key: Option<&StoreKey>) -> StorageState {
    let (session_present, session_encrypted) = match std::fs::read(&paths.session_file) {
        Ok(mut bytes) => {
            // Only the shape is looked at, and the buffer goes away right
            // after: it holds an access token either way.
            let encrypted = serde_json::from_slice::<EncryptedEnvelope>(&bytes).is_ok();
            bytes.zeroize();
            (true, encrypted)
        }
        // "Could not read" is not "not there": an unreadable file counts as
        // present and not known to be encrypted.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, false),
        Err(_) => (true, false),
    };

    StorageState {
        store_encrypted: store_marked_encrypted(paths),
        session_present,
        session_encrypted,
        key_available: key.is_some(),
    }
}

/// Open under the key, open an existing plaintext store, or refuse.
/// Every doubtful case takes the answer that changes nothing on disk.
fn store_plan<'a>(paths: &Paths, key: Option<&'a StoreKey>) -> Result<Option<&'a StoreKey>, String> {
    if store_marked_encrypted(paths) {
        // The SDK does *not* refuse a missing key: with `None` it opens
        // cipherless and writes plaintext beside the ciphertext. So this does.
        let key = key.ok_or("the local store is encrypted and its key is not available")?;
        return Ok(Some(key));
    }

    let fresh = match std::fs::read_dir(&paths.store) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    if !fresh {
        return Ok(None);
    }

    let key = key.ok_or("no store key: the local store is not created unencrypted")?;
    // Marker before the store exists: losing it flips the store back to "open
    // without key". The directory may not be there yet.
    std::fs::create_dir_all(&paths.store)
        .map_err(|error| format!("the store directory could not be created: {error}"))?;
    std::fs::write(paths.store.join(".encrypted"), b"xmatic store key v1\n")
        .map_err(|error| format!("the store could not be marked as encrypted: {error}"))?;
    Ok(Some(key))
}

/// Builds a client for `server` - server name or full URL. `store_plan`
/// decides the store; a new one is never created without a key.
pub async fn build_client(
    server: &str,
    paths: &Paths,
    key: Option<&StoreKey>,
) -> Result<Client, String> {
    let store_key = store_plan(paths, key)?;
    let store_config =
        SqliteStoreConfig::new(&paths.store).key(store_key.map(|key| &**key));

    // Search index follows the store's decision, password is the key in
    // base64: same directory, same key, so deriving one buys nothing.
    let search_store = match store_key {
        Some(key) => SearchIndexStoreKind::EncryptedDirectory(
            paths.search_index.clone(),
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &**key),
        ),
        None => SearchIndexStoreKind::UnencryptedDirectory(paths.search_index.clone()),
    };

    let client = Client::builder()
        .search_index_store(search_store)
        .server_name_or_homeserver_url(server)
        // The homeserver stays what discovery decided. The SDK would otherwise
        // take `well_known` from the login answer, unchecked, http included.
        .respect_login_well_known(false)
        .sqlite_store_with_config_and_cache_path(store_config, None::<&Path>)
        .handle_refresh_tokens()
        // SingleProcess, and `src/instancelock.cpp` is what makes it true.
        // The multi-process lease costs ~50 fsyncs/s while the app idles.
        .cross_process_store_config(CrossProcessLockConfig::SingleProcess)
        // Threading on, or a thread is an empty page: the event cache keeps a
        // thread's events only behind this flag.
        .with_threading_support(ThreadingSupport::Enabled { with_subscriptions: false })
        // `AfterDecryptionFailure`, never `OneShot`: that one downloads once,
        // logs its failure and still reports the backup as enabled.
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: false,
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            auto_enable_backups: false,
        })
        .build()
        .await
        // Phrased here, not at five call sites - and the refusal above needs a
        // message no caller can prefix into a wrong claim.
        .map_err(|error| {
            format!("homeserver unreachable: {}", crate::text::scrub_ids(&error.to_string()))
        });

    // The SDK creates its databases with the process umask. They hold the
    // device identity and the room keys.
    if client.is_ok() {
        restrict_store(paths);
    }
    client
}

/// Encrypted `session.json`: a fresh `StoreCipher` per write, exported
/// under the store key. The same cipher the SDK's stores use.
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

    // Temp file, restricted, then renamed. `write` truncates in place, and
    // a short session file reads as `Locked`.
    let temporary = path.with_extension("json.new");
    {
        use std::io::Write;
        let mut file = create_private(&temporary)?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    restrict_permissions(path)
}

/// Writes bytes to a private file the same way the session is written: temp
/// file at 0600, synced, renamed over the old one.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let temporary = path.with_extension("new");
    {
        use std::io::Write;
        let mut file = create_private(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    restrict_permissions(path)
}

/// Creates a file only this user can read, without a moment at 0644 in
/// between.
#[cfg(unix)]
fn create_private(path: &Path) -> Result<std::fs::File, std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> Result<std::fs::File, std::io::Error> {
    std::fs::File::create(path)
}

/// 0600 on everything in the store directory: matrix-sdk-sqlite creates
/// its databases with the process umask.
pub fn restrict_store(paths: &Paths) {
    let Ok(entries) = std::fs::read_dir(&paths.store) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let _ = restrict_permissions(&path);
        }
    }
}


/// What the session file amounts to. `Locked` may never be mistaken for
/// `None` - `None` is the one outcome a fresh login resets the store on.
pub enum LoadOutcome {
    /// No session file.
    None,
    /// A readable session.
    Session(StoredSession),
    /// An encrypted session that the available key (if any) does not open.
    Locked,
}

/// Reads the stored session. Both shapes load whatever key is at hand; a
/// plaintext file is rewritten encrypted after the next successful restore.
pub fn load(path: &Path, key: Option<&StoreKey>) -> LoadOutcome {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        // Only "no file" is "no session". Every other read error is a failed
        // look, and answering that with `None` costs the crypto store.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadOutcome::None;
        }
        Err(_) => return LoadOutcome::Locked,
    };
    if let Ok(session) = serde_json::from_slice::<StoredSession>(&bytes) {
        return LoadOutcome::Session(session);
    }

    let Ok(envelope) = serde_json::from_slice::<EncryptedEnvelope>(&bytes) else {
        // Neither shape: locked rather than absent. Nothing here may lead to a
        // reset the user did not ask for.
        return LoadOutcome::Locked;
    };
    let Some(key) = key else {
        return LoadOutcome::Locked;
    };
    let engine = base64::engine::general_purpose::STANDARD;
    let decoded_cipher = match engine.decode(envelope.encrypted.cipher) {
        Ok(bytes) => bytes,
        Err(_) => return LoadOutcome::Locked,
    };
    let Ok(cipher) = StoreCipher::import_with_key(&**key, &decoded_cipher) else {
        return LoadOutcome::Locked;
    };
    let decoded_data = match engine.decode(envelope.encrypted.data) {
        Ok(bytes) => bytes,
        Err(_) => return LoadOutcome::Locked,
    };
    match cipher.decrypt_value(&decoded_data) {
        Ok(session) => LoadOutcome::Session(session),
        Err(_) => LoadOutcome::Locked,
    }
}

/// Removes the stored session. Missing files are not an error.
pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Throws away state and crypto stores. They belong to one device: kept
/// across a logout, the next login fails on the device ID.
pub fn reset_store(paths: &Paths) -> Result<(), std::io::Error> {
    if paths.store.exists() {
        std::fs::remove_dir_all(&paths.store)?;
    }
    // The search index goes with them: built from what the store held, and
    // the one place message text lives outside it.
    if paths.search_index.exists() {
        std::fs::remove_dir_all(&paths.search_index)?;
    }
    // And the push registration - it names a device about to stop existing
    // and its endpoint is a secret anyone holding it can push with.
    if paths.push_file.exists() {
        std::fs::remove_file(&paths.push_file)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of its own per test, removed at the end.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "xmatic-test-{name}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("sandbox");
            Sandbox(path)
        }

        fn paths(&self) -> Paths {
            Paths::new(&self.0, &self.0)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn a_key() -> StoreKey {
        Zeroizing::new([7u8; 32])
    }

    #[test]
    fn a_new_store_is_never_created_without_a_key() {
        let sandbox = Sandbox::new("fresh");
        let paths = sandbox.paths();
        assert!(store_plan(&paths, None).is_err());
        assert!(!paths.store.join(".encrypted").exists());
    }

    #[test]
    fn a_new_store_is_marked_before_it_exists() {
        let sandbox = Sandbox::new("marked");
        let paths = sandbox.paths();
        let key = a_key();
        assert!(store_plan(&paths, Some(&key)).expect("plan").is_some());
        assert!(paths.store.join(".encrypted").exists());
    }

    #[test]
    fn an_existing_plaintext_store_keeps_opening_in_the_clear() {
        let sandbox = Sandbox::new("legacy");
        let paths = sandbox.paths();
        std::fs::create_dir_all(&paths.store).expect("store");
        std::fs::write(paths.store.join("matrix-sdk-state.sqlite3"), b"x").expect("db");
        let key = a_key();
        // Neither with a key nor without it may the marker appear over data
        // that was written without one.
        assert!(store_plan(&paths, Some(&key)).expect("plan").is_none());
        assert!(store_plan(&paths, None).expect("plan").is_none());
        assert!(!paths.store.join(".encrypted").exists());
    }

    #[test]
    fn a_marked_store_without_its_key_is_refused() {
        let sandbox = Sandbox::new("locked");
        let paths = sandbox.paths();
        std::fs::create_dir_all(&paths.store).expect("store");
        std::fs::write(paths.store.join(".encrypted"), b"x").expect("marker");
        assert!(store_plan(&paths, None).is_err());
        let key = a_key();
        assert!(store_plan(&paths, Some(&key)).expect("plan").is_some());
    }

    #[test]
    fn a_missing_session_file_is_none_and_an_unreadable_one_is_locked() {
        let sandbox = Sandbox::new("load");
        let paths = sandbox.paths();
        assert!(matches!(load(&paths.session_file, None), LoadOutcome::None));
        // A directory where a file is expected: readable metadata, unreadable
        // content - the shape a failed look has.
        std::fs::create_dir_all(&paths.session_file).expect("dir");
        assert!(matches!(load(&paths.session_file, None), LoadOutcome::Locked));
    }
}
