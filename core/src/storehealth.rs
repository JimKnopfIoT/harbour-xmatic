//! Rows the stores can no longer decode. One of them fails every sync the same
//! way, so the sync is stopped until it is gone - the store is worth more than
//! the row, and a row nothing can read is worth nothing.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use matrix_sdk_base::crypto::olm::PickledInboundGroupSession;
use matrix_sdk_base::RoomInfo;
use matrix_sdk_store_encryption::{EncryptedValue, StoreCipher};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::session::{restrict_store, Paths, StoreKey};

/// The SDK reported a failure no retry can fix. Latched, because the next sync
/// meets the same row: only a repair clears it.
static DAMAGE: Mutex<Option<String>> = Mutex::new(None);

/// Both marks are required. A store error is never the network, a decode error
/// is never transient - either alone would also catch a busy database.
const STORE_MARKS: [&str; 4] = [
    "CryptoStoreError",
    "StateStoreError",
    "CryptoStore(",
    "StateStore(",
];
const DECODE_MARKS: [&str; 4] = [
    "Decode(",
    "Syntax(",
    "missing field",
    "control character",
];

/// Classifies one of the SDK's own log lines. `true` the first time a permanent
/// store failure is seen; the caller announces it.
pub fn note_sdk_failure(target: &str, message: &str) -> bool {
    if !target.starts_with("matrix_sdk")
        || !STORE_MARKS.iter().any(|mark| message.contains(mark))
        || !DECODE_MARKS.iter().any(|mark| message.contains(mark))
    {
        return false;
    }
    let mut damage = DAMAGE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if damage.is_some() {
        return false;
    }
    *damage = Some(message.to_owned());
    true
}

pub fn damaged() -> bool {
    DAMAGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
}

/// Only after a repair actually removed something. Clearing it on anything else
/// puts the sync back into the loop the latch was built to end.
pub fn clear() {
    *DAMAGE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// What the scrub looked at. Counted even where nothing was dropped, so the
/// journal can say "looked, found nothing" rather than staying silent.
#[derive(Default, Debug)]
pub struct Report {
    pub checked: usize,
    pub rooms: usize,
    pub room_keys: usize,
}

impl Report {
    pub fn dropped(&self) -> usize {
        self.rooms + self.room_keys
    }
}

/// A table where much more than a few rows fail is not a damaged store, it is a
/// wrong assumption about the table. Two always pass, so a small store is not
/// stuck with one bad row it cannot lose.
fn tolerated(rows: usize) -> usize {
    (rows / 4).max(2)
}

/// Drops the rows the SDK can no longer decode from the two stores that keep
/// one each. Every doubtful case refuses instead of deleting.
pub fn scrub(paths: &Paths, key: Option<&StoreKey>) -> Result<Report, String> {
    let mut report = Report::default();

    let (checked, dropped) = scrub_table(
        &paths.store.join("matrix-sdk-state.sqlite3"),
        key,
        "room_info",
        |bytes| serde_json::from_slice::<RoomInfo>(bytes).map(|_| ()).map_err(drop),
    )?;
    report.checked += checked;
    report.rooms = dropped;

    let (checked, dropped) = scrub_table(
        &paths.store.join("matrix-sdk-crypto.sqlite3"),
        key,
        "inbound_group_session",
        |bytes| {
            rmp_serde::from_slice::<PickledInboundGroupSession>(bytes)
                .map(|_| ())
                .map_err(drop)
        },
    )?;
    report.checked += checked;
    report.room_keys = dropped;

    // The connection may have created a `-wal` or `-shm` under the process
    // umask; the files beside it hold the device identity and the room keys.
    restrict_store(paths);
    Ok(report)
}

/// Rows checked, rows deleted. The decoder is the SDK's own type: "can this be
/// read back" is not a question this file is allowed to answer itself.
fn scrub_table(
    file: &Path,
    key: Option<&StoreKey>,
    table: &str,
    decode: fn(&[u8]) -> Result<(), ()>,
) -> Result<(usize, usize), String> {
    if !file.exists() {
        return Ok((0, 0));
    }
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Read-write without CREATE: a store that is not there is not one to build.
    let connection = Connection::open_with_flags(
        file,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("{name} could not be opened: {error}"))?;
    // The app's own client holds the same file. Waiting costs less than a repair
    // that fails; by the time this runs the sync is stopped, so writers are rare.
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("{name}: {error}"))?;

    if !has_table(&connection, "kv") || !has_table(&connection, table) {
        return Ok((0, 0));
    }
    let cipher = cipher(&connection, key).map_err(|error| format!("{name}: {error}"))?;

    let mut checked = 0usize;
    let mut damaged: Vec<i64> = Vec::new();
    {
        let query = format!("SELECT rowid, data FROM {table}");
        let mut statement = connection
            .prepare(&query)
            .map_err(|error| format!("{name}: {error}"))?;
        let mut rows = statement
            .query([])
            .map_err(|error| format!("{name}: {error}"))?;
        while let Some(row) = rows.next().map_err(|error| format!("{name}: {error}"))? {
            let rowid: i64 = row.get(0).map_err(|error| format!("{name}: {error}"))?;
            let value: Vec<u8> = row.get(1).map_err(|error| format!("{name}: {error}"))?;
            // A value that does not decrypt is not a damaged row - it is the wrong
            // key, and every row would look the same. Stop before anything goes.
            let plain = plaintext(cipher.as_ref(), &value)
                .map_err(|error| format!("{name}: {error}"))?;
            checked += 1;
            if decode(&plain).is_err() {
                damaged.push(rowid);
            }
        }
    }

    if damaged.len() > tolerated(checked) {
        return Err(format!(
            "{name}: {} of {checked} rows in {table} cannot be decoded, which is not a damaged row",
            damaged.len()
        ));
    }

    let query = format!("DELETE FROM {table} WHERE rowid = ?");
    for rowid in &damaged {
        connection
            .execute(&query, [rowid])
            .map_err(|error| format!("{name}: {error}"))?;
    }
    Ok((checked, damaged.len()))
}

fn has_table(connection: &Connection, table: &str) -> bool {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
}

/// The cipher the SDK keeps in the database itself, opened with the same key it
/// was written under. No cipher row means a store from before the key existed.
fn cipher(connection: &Connection, key: Option<&StoreKey>) -> Result<Option<StoreCipher>, String> {
    let stored: Option<Vec<u8>> = connection
        .query_row("SELECT value FROM kv WHERE key = 'cipher'", [], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| format!("the store cipher could not be read: {error}"))?;

    match (stored, key) {
        (None, _) => Ok(None),
        (Some(_), None) => Err("the store is encrypted and its key is not available".to_owned()),
        (Some(stored), Some(key)) => StoreCipher::import_with_key(key, &stored)
            .map(Some)
            .map_err(|error| format!("the store cipher did not open: {error}")),
    }
}

/// The envelope is authenticated: damage on disk fails here, not in the decoder.
/// So a failure means the key is wrong, and the caller must not delete anything.
fn plaintext(cipher: Option<&StoreCipher>, value: &[u8]) -> Result<Vec<u8>, String> {
    let Some(cipher) = cipher else {
        return Ok(value.to_vec());
    };
    let encrypted: EncryptedValue = rmp_serde::from_slice(value)
        .map_err(|error| format!("a value is not an encryption envelope: {error}"))?;
    cipher
        .decrypt_value_data(encrypted)
        .map_err(|error| format!("a value did not decrypt: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNC_FAILURE: &str = "Error while processing room list in sync service: Some( RoomList( \
         SlidingSync( CryptoStoreError( Backend( Decode( Syntax( \"missing field `signing_key`\" ) \
         ) ) ) ) )";

    /// One test for the latch, because it is process-wide and the test runner
    /// would otherwise have two threads in it at once.
    #[test]
    fn only_a_permanent_store_failure_latches_and_only_once() {
        assert!(note_sdk_failure("matrix_sdk_ui::sync_service", SYNC_FAILURE));
        assert!(damaged());
        // Latched: the same failure on every sync must not announce itself again.
        assert!(!note_sdk_failure("matrix_sdk_ui::sync_service", SYNC_FAILURE));

        clear();
        assert!(!damaged());

        // A busy database is a store error that the next attempt clears.
        assert!(!note_sdk_failure(
            "matrix_sdk_ui::sync_service",
            "StateStoreError(Sqlite(SqliteFailure(database is locked)))"
        ));
        // No store mark: one event that will not decrypt is not the store failing.
        assert!(!note_sdk_failure(
            "matrix_sdk_crypto::machine",
            "Failed to decrypt a room event: missing field `signing_key`"
        ));
        assert!(!damaged());
    }

    #[test]
    fn a_table_that_fails_wholesale_is_not_repaired() {
        // Two always pass, a quarter of a large table passes, everything does not.
        assert_eq!(tolerated(0), 2);
        assert_eq!(tolerated(4), 2);
        assert_eq!(tolerated(400), 100);
        assert!(400 > tolerated(400));
    }

    /// A store of the shape the SDK writes: the cipher in `kv`, every value an
    /// encryption envelope. Damaged rows carry authentic ciphertext over broken
    /// plaintext - which is what the device produced, and what a MAC cannot see.
    fn store(path: &Path, key: &[u8; 32], good: usize, bad: usize) {
        let connection = Connection::open(path).expect("open");
        connection
            .execute_batch(
                "CREATE TABLE kv (key TEXT PRIMARY KEY, value BLOB);\
                 CREATE TABLE things (id BLOB PRIMARY KEY, data BLOB);",
            )
            .expect("schema");

        let cipher = StoreCipher::new().expect("cipher");
        connection
            .execute(
                "INSERT INTO kv VALUES ('cipher', ?)",
                [cipher.export_with_key(key).expect("export")],
            )
            .expect("cipher row");

        let write = |id: usize, plain: &[u8]| {
            let envelope = cipher.encrypt_value_data(plain.to_vec()).expect("encrypt");
            connection
                .execute(
                    "INSERT INTO things VALUES (?, ?)",
                    rusqlite::params![
                        id.to_string(),
                        rmp_serde::to_vec_named(&envelope).expect("envelope")
                    ],
                )
                .expect("row");
        };
        for id in 0..good {
            write(id, br#"{"kind":"thing"}"#);
        }
        for id in good..good + bad {
            write(id, b"{\"kind\":\0truncated");
        }
    }

    /// The decoder stands in for the SDK's own types: the question is whether the
    /// plaintext parses, not what it parses into.
    fn parses(bytes: &[u8]) -> Result<(), ()> {
        serde_json::from_slice::<serde_json::Value>(bytes).map(|_| ()).map_err(drop)
    }

    fn rows(path: &Path) -> i64 {
        Connection::open(path)
            .expect("open")
            .query_row("SELECT count(*) FROM things", [], |row| row.get(0))
            .expect("count")
    }

    #[test]
    fn the_damaged_rows_go_and_nothing_else_does() {
        let directory = std::env::temp_dir().join(format!(
            "xmatic-scrub-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&directory).expect("directory");
        let key: StoreKey = zeroize::Zeroizing::new([7u8; 32]);
        let other: StoreKey = zeroize::Zeroizing::new([9u8; 32]);

        let file = directory.join("one.sqlite3");
        store(&file, &key, 9, 1);
        assert_eq!(
            scrub_table(&file, Some(&key), "things", parses).expect("scrub"),
            (10, 1)
        );
        assert_eq!(rows(&file), 9);
        // A second pass finds nothing: the repair is not a thing that keeps eating.
        assert_eq!(
            scrub_table(&file, Some(&key), "things", parses).expect("scrub"),
            (9, 0)
        );

        // The wrong key makes every row look damaged. Refused, and nothing deleted.
        let wrong = directory.join("wrong.sqlite3");
        store(&wrong, &key, 9, 1);
        assert!(scrub_table(&wrong, Some(&other), "things", parses).is_err());
        assert_eq!(rows(&wrong), 10);

        // So does a table this file has the wrong idea about.
        let most = directory.join("most.sqlite3");
        store(&most, &key, 2, 8);
        assert!(scrub_table(&most, Some(&key), "things", parses).is_err());
        assert_eq!(rows(&most), 10);

        let _ = std::fs::remove_dir_all(&directory);
    }
}
