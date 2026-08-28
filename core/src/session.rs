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
    AuthSession, Client, SqliteStoreConfig, ThreadingSupport,
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
    /// Voice messages recorded on this device. Written by the Qt side, which
    /// derives the same path; cleared here because a sign-out has to reach
    /// everything readable.
    pub voice_cache: PathBuf,
    /// Encrypted lists that name people (see private.rs).
    pub private_file: PathBuf,
}

impl Paths {
    pub fn new(data_dir: &Path, cache_dir: &Path) -> Self {
        Self {
            store: data_dir.join("store"),
            session_file: data_dir.join("session.json"),
            media_cache: cache_dir.join("media"),
            voice_cache: cache_dir.join("voice"),
            private_file: data_dir.join("private.json"),
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

/// What the app's own files on disk amount to.
///
/// The two halves are reported separately because they are separate facts and
/// can genuinely disagree. The SQLite stores carry the `.encrypted` marker from
/// the moment they were created and can never change side afterwards
/// (`store_key_applies` explains why), while the session file is rewritten on
/// every successful restore and follows whatever key exists at that moment. A
/// device whose stores predate the store key therefore has an encrypted session
/// next to plaintext stores — collapsing that into one word would claim a
/// protection that only covers half of it, which is precisely what the UI is
/// meant to stop doing.
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
    /// True when everything that exists on disk is encrypted. An install with
    /// no session file yet counts as encrypted if the stores are: there is
    /// nothing else to protect.
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
        // Not there is one thing; could not be read is another. Answering the
        // second with "no session file" makes `fully_encrypted()` report the
        // good case, and the privacy page states it as fact. A statement about
        // encryption may not rest on a failed look: an unreadable file counts
        // as present and not known to be encrypted.
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

fn store_key_applies(paths: &Paths, key: Option<&StoreKey>) -> bool {
    if key.is_none() {
        return false;
    }
    if store_marked_encrypted(paths) {
        return true;
    }
    // "Could not look" is not "empty". Answering it with `true` writes the
    // marker and hands the key to a store that may already hold unencrypted
    // data - which the SDK then re-ciphers, and that is the total loss this
    // whole function exists to avoid. The failure direction of a test whose
    // wrong answer destroys data is the one that changes nothing.
    //
    // A directory that is not there yet is the exception, and it is spelled
    // out rather than left to a caller: today every `build_client` runs after
    // a `prepare()` that creates it, so `ENOENT` cannot happen - but that is
    // another function's guarantee, and one that a later caller can forget.
    // Not there and nothing in it are the same thing to this question.
    let fresh = match std::fs::read_dir(&paths.store) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
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

    let client = Client::builder()
        .server_name_or_homeserver_url(server)
        // The homeserver stays what discovery decided, whatever the login
        // response would like it to be. The SDK otherwise takes the `well_known`
        // out of a successful password login and calls `set_homeserver` with it,
        // unchecked - so a server could answer the sign-in with an http address
        // and every request after it, access token included, would go in the
        // clear past the https rule the login paths enforce.
        .respect_login_well_known(false)
        .sqlite_store_with_config_and_cache_path(store_config, None::<&Path>)
        .handle_refresh_tokens()
        // SingleProcess, deliberately. The multi-process lock guards against a
        // second process on the same store (single-use OAuth refresh tokens),
        // and `src/instancelock.cpp` is what makes sure there is none: an flock
        // taken before the store opens. That guard is the justification for this
        // line. The launcher covers only the icon tap, not the D-Bus activation
        // the share service and notifications use.
        // The lock is not free either - its lease is re-written into
        // the crypto store every 50 ms (EXTEND_LEASE_EVERY_MS), which kept
        // ~50 fsyncs/s and a 60 Hz tokio tick running while the app was idle:
        // measured 3.4% CPU against 0.9% for a comparable client. The builder
        // default is MultiProcess("main"), so this must stay explicit.
        .cross_process_store_config(CrossProcessLockConfig::SingleProcess)
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
        // Threading has to be switched on here or a thread is an empty page.
        // Measured on the device: a thread opened from a message showed its
        // root and nothing else, an own reply appeared as a local echo and was
        // gone again on the next visit, and a reply written in another client
        // never arrived - while the same reply stood in the room's own
        // timeline, so it was neither a delivery nor a decryption problem.
        //
        // The reason is one condition in the event cache: every piece of
        // thread bookkeeping - sorting a synced event into its thread's linked
        // chunk, and updating the thread at all - sits behind
        // `if self.state.enabled_thread_support`
        // (`event_cache/caches/room/state.rs`), and that flag is this one.
        // With it off, a `TimelineFocus::Thread` timeline has nothing to be
        // fed from and nothing to load.
        //
        // It is not free. With threading on, a reply inside a thread no longer
        // raises the room's unread and notification counts
        // (`event_cache/caches/read_receipts.rs` returns early for any event
        // carrying a thread root), because a client with threads is expected
        // to count them per thread, which this app does not do yet. That is
        // the trade: thread replies still appear in the room's timeline, they
        // just no longer make the badge rise on their own. A thread that works
        // is worth more than a count that includes it.
        //
        // `with_subscriptions: false`: the other half of the flag is thread
        // subscriptions (MSC4306/4308), which needs a server that advertises
        // the feature and changes what the room list requests. Not needed to
        // read a thread.
        .with_threading_support(ThreadingSupport::Enabled { with_subscriptions: false })
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: false,
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            auto_enable_backups: false,
        })
        .build()
        .await;

    // The SDK creates its databases with the process umask. They hold the
    // device identity and the room keys.
    if client.is_ok() {
        restrict_store(paths);
    }
    client
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

    // Temp file, restricted before a byte goes in, then renamed over the old
    // one. `write` truncates in place: an interruption leaves a short file,
    // and a short session file reads as `Locked` - the state whose only exit
    // deletes the crypto store.
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

/// 0600 on every file the store directory holds. matrix-sdk-sqlite creates its
/// databases with the process umask, so the keys would otherwise be readable
/// by anything running as this user outside the sandbox.
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


/// Reads a previously stored session, or `None` if there is none or it can no
/// longer be read — a stale file must never block the user from logging in
/// again.
///
/// Both shapes are accepted regardless of the key: a plaintext file from
/// before the store key existed still loads (and is rewritten encrypted after
/// the next successful restore), and an encrypted file without a key is
/// simply gone — the user signs in again, which is the honest outcome when
/// the secrets store lost the key.
/// What the session file on disk amounts to.
///
/// `Locked` is the case that must never be mistaken for `None`: the file is
/// there and is an encryption envelope, but the store key is missing or does
/// not open it. Treating that as "no session" put the login page on screen
/// and let the next login reset the store — a transient failure of the
/// secrets service (a locked collection after a reboot) became a lost device
/// and a recovery-key re-login for every affected user.
pub enum LoadOutcome {
    /// No session file.
    None,
    /// A readable session.
    Session(StoredSession),
    /// An encrypted session that the available key (if any) does not open.
    Locked,
}

pub fn load(path: &Path, key: Option<&StoreKey>) -> LoadOutcome {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        // Only "there is no file" means there is no session. Every other
        // reason - a busy device out of file descriptors, a sandbox that
        // refused for a moment, an I/O error - means "I could not look", and
        // that must never be answered with "nothing is here": `None` is the
        // one outcome a fresh login resets the store on, so a moment's bad
        // luck would take the crypto store and the device identity with it.
        // Reported as locked instead: the same wall an unavailable key gets,
        // with the same way out.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadOutcome::None;
        }
        Err(_) => return LoadOutcome::Locked,
    };
    if let Ok(session) = serde_json::from_slice::<StoredSession>(&bytes) {
        return LoadOutcome::Session(session);
    }

    let Ok(envelope) = serde_json::from_slice::<EncryptedEnvelope>(&bytes) else {
        // Neither shape: not a session this build can read. Reported as
        // locked rather than absent for the same reason — nothing here may
        // lead to a reset the user did not ask for.
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
