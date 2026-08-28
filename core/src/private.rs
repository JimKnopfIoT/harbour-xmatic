//! Lists that name people, kept encrypted.
//!
//! Who may call, and who the send warning is switched off for: both are a
//! piece of the user's social graph and have no business lying in a plain
//! settings file. Same envelope as `session.json` - a fresh `StoreCipher` per
//! write, exported under the device's store key - so there is no second
//! cryptography to audit. Without a store key the file is refused rather than
//! written in the clear: a list of names is not worth degrading for.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use matrix_sdk_store_encryption::StoreCipher;
use serde::{Deserialize, Serialize};

use crate::session::StoreKey;

pub type Lists = BTreeMap<String, Vec<String>>;

/// What the file on disk amounts to. `Unreadable` must never be mistaken for
/// `Lists::new()`: the store key is device-lock-bound and missing after every
/// reboot until the system dialog runs, and a write over an unread file
/// destroys what it could not read.
pub enum Loaded {
    /// There is no file yet.
    Empty,
    /// The file was read and decrypted.
    Lists(Lists),
    /// The file is there and could not be read - no key, wrong key, damaged.
    Unreadable,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    cipher: String,
    data: String,
}

fn invalid(error: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

pub fn load(path: &Path, key: Option<&StoreKey>) -> Loaded {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        // Only "there is no file" means nothing was ever written and writing
        // now is safe. Every other reason - out of file descriptors, an I/O
        // error, a sandbox that refused for a moment - means "I could not
        // look", and answering that with `Empty` invites the next write to
        // replace a file it never read. That is the caller's allow list gone,
        // silently, in a state where nobody gets through any more. The
        // doc comment on `Loaded` above says exactly this; the code did not.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded::Empty;
        }
        Err(_) => return Loaded::Unreadable,
    };
    let Some(key) = key else {
        return Loaded::Unreadable;
    };
    let Ok(envelope) = serde_json::from_slice::<Envelope>(&raw) else {
        return Loaded::Unreadable;
    };
    let engine = base64::engine::general_purpose::STANDARD;
    let (Ok(cipher_blob), Ok(data)) = (
        engine.decode(&envelope.cipher),
        engine.decode(&envelope.data),
    ) else {
        return Loaded::Unreadable;
    };
    let Ok(cipher) = StoreCipher::import_with_key(&**key, &cipher_blob) else {
        return Loaded::Unreadable;
    };
    match cipher.decrypt_value(&data) {
        Ok(lists) => Loaded::Lists(lists),
        Err(_) => Loaded::Unreadable,
    }
}

pub fn save(path: &Path, key: Option<&StoreKey>, lists: &Lists) -> Result<(), std::io::Error> {
    let key = key.ok_or_else(|| invalid("no store key".to_owned()))?;
    let cipher = StoreCipher::new().map_err(|error| invalid(error.to_string()))?;
    let engine = base64::engine::general_purpose::STANDARD;
    let envelope = Envelope {
        cipher: engine.encode(
            cipher
                .export_with_key(&**key)
                .map_err(|error| invalid(error.to_string()))?,
        ),
        data: engine.encode(
            cipher
                .encrypt_value(lists)
                .map_err(|error| invalid(error.to_string()))?,
        ),
    };
    let json = serde_json::to_vec(&envelope).map_err(|error| invalid(error.to_string()))?;

    crate::session::write_private(path, &json)
}
