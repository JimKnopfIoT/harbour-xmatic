//! Lists that name people, kept encrypted - the same envelope as `session.json`.
//! Without a key the file is refused rather than written in the clear.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use matrix_sdk_store_encryption::StoreCipher;
use serde::{Deserialize, Serialize};

use crate::session::StoreKey;

pub type Lists = BTreeMap<String, Vec<String>>;

/// What the file amounts to. `Unreadable` must never be mistaken for empty: a
/// write over an unread file destroys what it could not read.
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
        // Only "there is no file" means writing now is safe. Every other reason is a
        // failed look, and answering that with `Empty` replaces a file nobody read.
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

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    fn scratch(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "xmatic-private-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        path
    }

    fn a_key() -> StoreKey {
        Zeroizing::new([3u8; 32])
    }

    #[test]
    fn a_missing_file_is_empty_and_a_damaged_one_is_unreadable() {
        let path = scratch("shapes");
        assert!(matches!(load(&path, Some(&a_key())), Loaded::Empty));

        std::fs::write(&path, b"not an envelope").expect("write");
        assert!(matches!(load(&path, Some(&a_key())), Loaded::Unreadable));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_written_file_reads_back_and_only_with_its_own_key() {
        let path = scratch("roundtrip");
        let mut lists = Lists::new();
        lists.insert("callers".to_owned(), vec!["@a:example.org".to_owned()]);
        save(&path, Some(&a_key()), &lists).expect("save");

        match load(&path, Some(&a_key())) {
            Loaded::Lists(read) => assert_eq!(read, lists),
            _ => panic!("the file did not read back"),
        }
        // No key and a wrong key are both "could not look", never "empty".
        assert!(matches!(load(&path, None), Loaded::Unreadable));
        let other: StoreKey = Zeroizing::new([9u8; 32]);
        assert!(matches!(load(&path, Some(&other)), Loaded::Unreadable));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_is_written_without_a_key() {
        let path = scratch("nokey");
        assert!(save(&path, None, &Lists::new()).is_err());
        assert!(!path.exists());
    }
}
