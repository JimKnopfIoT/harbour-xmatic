//! Rows the stores can no longer decode. One of them fails every sync the same
//! way, so the sync is stopped until it is gone - the store is worth more than
//! the row. Nothing is deleted outright: every row is copied into
//! `store/quarantine.sqlite3` first - every column of it, so it is a row that
//! could be written back, not two fields - because "unreadable here" and
//! "worthless" are the same thing only until the next SDK version.
//!
//! What that copy is and is not: the bytes are exactly the store's own, so they
//! are encrypted where the store is and plain where a legacy store is plain.
//! Nothing in this app reads the file back - putting a row back needs a tool
//! with the store key and a decoder that handles it. The file goes with the
//! store on sign-out and survives `rebuild_store`.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use matrix_sdk_base::crypto::olm::PickledInboundGroupSession;
use matrix_sdk_base::RoomInfo;
use matrix_sdk_store_encryption::{EncryptedValue, StoreCipher};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};

use crate::session::{restrict_store, Paths, StoreKey};

/// The SDK reported a failure no retry can fix. Latched, because the next sync
/// meets the same row: only a repair clears it. What is kept is the store it
/// came from, not the line - the line is classified where it is still raw and
/// must not be held on to afterwards.
static DAMAGE: Mutex<Option<Scope>> = Mutex::new(None);

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
    *damage = Some(scope_of(message));
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

/// Which store the latched failure came from. A room that will not load is no
/// reason to walk the room keys: the wide sweep was the reason one defect in
/// throwaway data could reach the only data nothing can fetch again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    State,
    Crypto,
    Both,
}

impl Scope {
    fn state(self) -> bool {
        matches!(self, Scope::State | Scope::Both)
    }

    fn crypto(self) -> bool {
        matches!(self, Scope::Crypto | Scope::Both)
    }
}

/// Where the latched line said the damage is. Both marks, or none, means both
/// stores - the reading decides what is looked at, never what is deleted.
pub fn damage_scope() -> Scope {
    DAMAGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .unwrap_or(Scope::Both)
}

fn scope_of(message: &str) -> Scope {
    match (message.contains("CryptoStore"), message.contains("StateStore")) {
        (true, false) => Scope::Crypto,
        (false, true) => Scope::State,
        _ => Scope::Both,
    }
}

/// What the scrub looked at. Counted even where nothing was dropped, so the
/// journal can say "looked, found nothing" rather than staying silent.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Report {
    pub checked: usize,
    pub rooms: usize,
    pub room_keys: usize,
    /// Room keys the server had no copy of. The number that decides whether the
    /// user lost anything, and the one the journal used to leave out.
    pub room_keys_unsaved: usize,
    /// Rows put aside before they went. Always equal to `dropped()`, because a
    /// failed copy stops the deletion - kept as its own number so the journal
    /// says both, and a future change that breaks the equality is visible.
    pub kept: usize,
}

impl Report {
    pub fn dropped(&self) -> usize {
        self.rooms + self.room_keys
    }
}

/// Above this the table is wrong, not damaged - checked inside the read loop so
/// a store whose every row fails cannot be bought into memory first.
const MOST_EVER_DROPPED: usize = 5000;

/// A table where much more than a few rows fail is not a damaged store, it is a
/// wrong assumption about the table. Two always pass, so a small store is not
/// stuck with one bad row it cannot lose.
fn tolerated(rows: usize) -> usize {
    (rows / 4).max(2)
}

/// A table the SDK keeps one decodable value per row in, with the column that
/// names the row. Deleting by `rowid` raced the SDK's own inserts: SQLite hands
/// out a freed one again, so the row deleted need not be the row read.
struct TableSpec {
    table: &'static str,
    key: &'static str,
}

/// One row that will not decode, as it stands on disk. `whole` is every column
/// of it, so what is put aside can be written back.
struct Damaged {
    key: Vec<u8>,
    value: Vec<u8>,
    backed_up: bool,
    whole: String,
}

/// Every column of one row as JSON: text and numbers as themselves, a blob as
/// `{"blob":"<hex>"}`. Not meant to be pretty - meant to be complete.
fn whole_row(row: &rusqlite::Row<'_>, columns: &[String]) -> Result<String, String> {
    use rusqlite::types::ValueRef;
    let mut fields = serde_json::Map::new();
    for (at, column) in columns.iter().enumerate() {
        let value = row
            .get_ref(at)
            .map_err(|error| format!("a column could not be read: {error}"))?;
        let encoded = match value {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(number) => Value::from(number),
            ValueRef::Real(number) => Value::from(number),
            ValueRef::Text(bytes) => Value::from(String::from_utf8_lossy(bytes).into_owned()),
            ValueRef::Blob(bytes) => {
                let mut hex = String::with_capacity(bytes.len() * 2);
                for byte in bytes {
                    use std::fmt::Write;
                    let _ = write!(hex, "{byte:02x}");
                }
                json!({ "blob": hex })
            }
        };
        fields.insert(column.clone(), encoded);
    }
    Ok(Value::Object(fields).to_string())
}

#[derive(Default)]
struct Outcome {
    checked: usize,
    dropped: usize,
    unsaved: usize,
    kept: usize,
}

/// Where the rows go instead of away.
fn quarantine_file(paths: &Paths) -> std::path::PathBuf {
    paths.store.join("quarantine.sqlite3")
}

/// Drops the rows the SDK can no longer decode from the store the failure came
/// from. Every doubtful case refuses instead of deleting.
pub fn scrub(paths: &Paths, key: Option<&StoreKey>, scope: Scope) -> Result<Report, String> {
    let mut report = Report::default();
    let quarantine = quarantine_file(paths);

    if scope.state() {
        let outcome = scrub_table(
            &paths.store.join("matrix-sdk-state.sqlite3"),
            key,
            &TableSpec { table: "room_info", key: "room_id" },
            |bytes| serde_json::from_slice::<RoomInfo>(bytes).map(|_| ()).map_err(drop),
            &quarantine,
        )?;
        report.checked += outcome.checked;
        report.rooms = outcome.dropped;
        report.kept += outcome.kept;
    }

    if scope.crypto() {
        let outcome = scrub_table(
            &paths.store.join("matrix-sdk-crypto.sqlite3"),
            key,
            &TableSpec { table: "inbound_group_session", key: "session_id" },
            |bytes| {
                rmp_serde::from_slice::<PickledInboundGroupSession>(bytes)
                    .map(|_| ())
                    .map_err(drop)
            },
            &quarantine,
        )?;
        report.checked += outcome.checked;
        report.room_keys = outcome.dropped;
        report.room_keys_unsaved = outcome.unsaved;
        report.kept += outcome.kept;
    }

    // The connection may have created a `-wal` or `-shm` under the process
    // umask; the files beside it hold the device identity and the room keys.
    restrict_store(paths);
    Ok(report)
}

/// What was looked at and what went. The decoder is the SDK's own type: "can
/// this be read back" is not a question this file is allowed to answer itself.
fn scrub_table(
    file: &Path,
    key: Option<&StoreKey>,
    spec: &TableSpec,
    decode: fn(&[u8]) -> Result<(), ()>,
    quarantine: &Path,
) -> Result<Outcome, String> {
    if !file.exists() {
        return Ok(Outcome::default());
    }
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Read-write without CREATE: a store that is not there is not one to build.
    let mut connection = Connection::open_with_flags(
        file,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("{name} could not be opened: {error}"))?;
    // The app's own client holds the same file. Waiting costs less than a repair
    // that fails; by the time this runs the sync is stopped, so writers are rare.
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("{name}: {error}"))?;

    if !has_table(&connection, "kv") || !has_table(&connection, spec.table) {
        return Ok(Outcome::default());
    }
    let cipher = cipher(&connection, key).map_err(|error| format!("{name}: {error}"))?;

    let mut checked = 0usize;
    let mut damaged: Vec<Damaged> = Vec::new();
    {
        // Every column, not the two this file cares about: a row put aside has to
        // be a row that can come back, and `room_id`, `backed_up`, `sender_key`
        // and `state` are NOT NULL in their tables and cannot be reconstructed
        // from a pickle that does not decode.
        let query = format!("SELECT * FROM {}", spec.table);
        let mut statement = connection
            .prepare(&query)
            .map_err(|error| format!("{name}: {error}"))?;
        let columns: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let key_at = columns
            .iter()
            .position(|column| column == spec.key)
            .ok_or_else(|| format!("{name}: {} has no column {}", spec.table, spec.key))?;
        let data_at = columns
            .iter()
            .position(|column| column == "data")
            .ok_or_else(|| format!("{name}: {} has no column data", spec.table))?;
        let backed_up_at = columns.iter().position(|column| column == "backed_up");
        let mut rows = statement
            .query([])
            .map_err(|error| format!("{name}: {error}"))?;
        while let Some(row) = rows.next().map_err(|error| format!("{name}: {error}"))? {
            let id: Vec<u8> = row.get(key_at).map_err(|error| format!("{name}: {error}"))?;
            let value: Vec<u8> = row.get(data_at).map_err(|error| format!("{name}: {error}"))?;
            // A value that does not decrypt is not a damaged row - it is the wrong
            // key, and every row would look the same. Stop before anything goes.
            let plain = plaintext(cipher.as_ref(), &value)
                .map_err(|error| format!("{name}: {error}"))?;
            checked += 1;
            if decode(&plain).is_err() {
                // Unreadable counts as unsaved: the number is a warning, and a
                // warning that guesses low is the one that misleads.
                let backed_up = backed_up_at
                    .map(|at| row.get::<_, i64>(at).map(|flag| flag != 0).unwrap_or(false))
                    .unwrap_or(false);
                let whole = whole_row(row, &columns)?;
                damaged.push(Damaged { key: id, value, backed_up, whole });
                // Held in memory until the loop ends, and each entry is now the
                // whole row rather than eight bytes of rowid. A table that fails
                // wholesale is refused anyway - refuse it before buying it,
                // because on the 32-bit port that allocation is the crash.
                if damaged.len() > MOST_EVER_DROPPED {
                    return Err(format!(
                        "{name}: more than {MOST_EVER_DROPPED} rows in {} cannot be decoded, \
                         which is not a damaged row",
                        spec.table
                    ));
                }
            }
        }
    }

    if damaged.is_empty() {
        return Ok(Outcome { checked, ..Outcome::default() });
    }
    if damaged.len() > tolerated(checked) {
        return Err(format!(
            "{name}: {} of {checked} rows in {} cannot be decoded, which is not a damaged row",
            damaged.len(),
            spec.table
        ));
    }

    // Kept before it goes, and the error direction is the one that deletes
    // nothing: whoever cannot put a row aside does not get to remove it.
    let kept = keep_aside(quarantine, &name, spec.table, &damaged)?;
    let unsaved = damaged.iter().filter(|row| !row.backed_up).count();

    // One transaction: a repair that stops halfway must not leave a store that
    // reports a failure over rows it has already dropped.
    // `AND data = ?`: the key says which row, the value says it is still the row
    // that failed. Between the read above and this delete the SDK can write the
    // same key again - a key backup import or a verification hands over a *good*
    // copy under the same session id, and deleting by key alone would take it.
    // The count below turns that into a refusal instead of a loss.
    let query = format!(
        "DELETE FROM {} WHERE {} = ? AND data = ?",
        spec.table, spec.key
    );
    let transaction = connection
        .transaction()
        .map_err(|error| format!("{name}: {error}"))?;
    let mut removed = 0usize;
    for row in &damaged {
        removed += transaction
            .execute(&query, rusqlite::params![&row.key[..], &row.value[..]])
            .map_err(|error| format!("{name}: {error}"))?;
    }
    // What was counted is what the statement actually hit. A key bound as the
    // wrong type matches nothing and raises no error - and "dropped" is what
    // clears the latch, so a number nobody measured would restart the sync over
    // damage that is still there.
    if removed != damaged.len() {
        return Err(format!(
            "{name}: {} of {} unreadable rows in {} were rewritten or gone while the repair ran",
            damaged.len() - removed,
            damaged.len(),
            spec.table
        ));
    }
    transaction
        .commit()
        .map_err(|error| format!("{name}: {error}"))?;

    Ok(Outcome { checked, dropped: removed, unsaved, kept })
}

/// Copies the rows into a store-side file before they are deleted: every column,
/// with the values exactly as they stood. Encrypted where the store is - this is
/// the store's own content under the store's own cipher. No code here reads it
/// back; it exists so the decision is reversible by hand, not so the app can
/// undo it.
/// Opens the quarantine, replacing it once if it cannot be opened or prepared.
/// A file damaged by a killed process would otherwise block **every** future
/// repair for the life of the install - the sync stays stopped and the only way
/// out on offer is a sign-out, which is what this module exists to avoid. Only
/// a broken *open* justifies that; a failed write (a full disk) leaves the file
/// alone, because it may hold rows from an earlier run.
fn open_quarantine(path: &Path) -> Result<Connection, String> {
    const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS quarantine (\
         store TEXT NOT NULL,\
         source TEXT NOT NULL,\
         key BLOB NOT NULL,\
         data BLOB NOT NULL,\
         row_json TEXT NOT NULL DEFAULT '',\
         taken_at INTEGER NOT NULL,\
         PRIMARY KEY (store, source, key)\
     ) WITHOUT ROWID;";

    let prepare = |path: &Path| -> Result<Connection, String> {
        let connection = Connection::open(path)
            .map_err(|error| format!("the quarantine could not be opened: {error}"))?;
        connection
            .execute_batch(SCHEMA)
            .map_err(|error| format!("the quarantine could not be prepared: {error}"))?;
        Ok(connection)
    };

    match prepare(path) {
        Ok(connection) => Ok(connection),
        Err(first) => {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let beside = path.with_file_name(format!(
                    "{}{suffix}",
                    path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default()
                ));
                let _ = std::fs::remove_file(&beside);
            }
            prepare(path).map_err(|second| format!("{first}; and again after replacing it: {second}"))
        }
    }
}

fn keep_aside(path: &Path, store: &str, table: &str, rows: &[Damaged]) -> Result<usize, String> {
    let mut connection = open_quarantine(path)?;

    let taken_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0);

    let transaction = connection
        .transaction()
        .map_err(|error| format!("the quarantine could not be written: {error}"))?;
    let mut kept = 0usize;
    {
        let mut statement = transaction
            // REPLACE, not IGNORE: if the same row failed once before, the copy
            // worth keeping is the one being taken out of the store now.
            .prepare(
                "INSERT OR REPLACE INTO quarantine \
                 (store, source, key, data, row_json, taken_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .map_err(|error| format!("the quarantine could not be written: {error}"))?;
        for row in rows {
            kept += statement
                .execute(rusqlite::params![
                    store,
                    table,
                    &row.key[..],
                    &row.value[..],
                    &row.whole,
                    taken_at
                ])
                .map_err(|error| format!("the quarantine could not be written: {error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("the quarantine could not be written: {error}"))?;
    Ok(kept)
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

        // Classified raw, published scrubbed. `tracing` writes a field as one
        // word without spaces, and the scrubber replaces a whole word as soon as
        // any part of it looks like an identifier - so this very line reaches the
        // sink as `<id>`. Classified after scrubbing, the latch never closes and
        // the flapping loop this module exists to end comes back.
        let raw = "source=CryptoStoreError(Decode(serde.Error))";
        assert_eq!(crate::text::scrub_ids(raw), "<id>");
        assert!(note_sdk_failure("matrix_sdk_ui::sync_service", raw));
        assert_eq!(damage_scope(), Scope::Crypto);
        clear();
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
        store_with_backup(path, key, good, bad, 0)
    }

    /// `saved` of the damaged rows carry a key backup, the rest do not. The key
    /// column is a BLOB, the way the SDK writes its hashed identifiers.
    fn store_with_backup(path: &Path, key: &[u8; 32], good: usize, bad: usize, saved: usize) {
        let connection = Connection::open(path).expect("open");
        connection
            .execute_batch(
                "CREATE TABLE kv (key TEXT PRIMARY KEY, value BLOB);\
                 CREATE TABLE things (id BLOB PRIMARY KEY, backed_up INTEGER NOT NULL, data BLOB);",
            )
            .expect("schema");

        let cipher = StoreCipher::new().expect("cipher");
        connection
            .execute(
                "INSERT INTO kv VALUES ('cipher', ?)",
                [cipher.export_with_key(key).expect("export")],
            )
            .expect("cipher row");

        let write = |id: usize, backed_up: bool, plain: &[u8]| {
            let envelope = cipher.encrypt_value_data(plain.to_vec()).expect("encrypt");
            connection
                .execute(
                    "INSERT INTO things VALUES (?, ?, ?)",
                    rusqlite::params![
                        id.to_string().into_bytes(),
                        i64::from(backed_up),
                        rmp_serde::to_vec_named(&envelope).expect("envelope")
                    ],
                )
                .expect("row");
        };
        for id in 0..good {
            write(id, true, br#"{"kind":"thing"}"#);
        }
        for id in good..good + bad {
            write(id, id - good < saved, b"{\"kind\":\0truncated");
        }
    }

    /// The same store, but the table has no `backed_up` column at all - the shape
    /// of `room_info`.
    fn store_without_backup(path: &Path, key: &[u8; 32], good: usize, bad: usize) {
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
                        id.to_string().into_bytes(),
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

    const PLAIN: TableSpec = TableSpec { table: "things", key: "id" };

    fn scratch(what: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "xmatic-{what}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&directory).expect("directory");
        directory
    }

    fn quarantined(path: &Path) -> i64 {
        Connection::open(path)
            .expect("open")
            .query_row("SELECT count(*) FROM quarantine", [], |row| row.get(0))
            .expect("count")
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
        let directory = scratch("scrub");
        let key: StoreKey = zeroize::Zeroizing::new([7u8; 32]);
        let other: StoreKey = zeroize::Zeroizing::new([9u8; 32]);
        let quarantine = directory.join("quarantine.sqlite3");

        let file = directory.join("one.sqlite3");
        store(&file, &key, 9, 1);
        let outcome = scrub_table(&file, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");
        assert_eq!((outcome.checked, outcome.dropped), (10, 1));
        assert_eq!(rows(&file), 9);
        // The row is not gone, it is aside: unreadable today is not worthless.
        assert_eq!(outcome.kept, 1);
        assert_eq!(quarantined(&quarantine), 1);

        // A second pass finds nothing: the repair is not a thing that keeps eating.
        let outcome = scrub_table(&file, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");
        assert_eq!((outcome.checked, outcome.dropped, outcome.kept), (9, 0, 0));

        // The wrong key makes every row look damaged. Refused, and nothing deleted.
        let wrong = directory.join("wrong.sqlite3");
        store(&wrong, &key, 9, 1);
        assert!(scrub_table(&wrong, Some(&other), &PLAIN, parses, &quarantine).is_err());
        assert_eq!(rows(&wrong), 10);

        // So does a table this file has the wrong idea about.
        let most = directory.join("most.sqlite3");
        store(&most, &key, 2, 8);
        assert!(scrub_table(&most, Some(&key), &PLAIN, parses, &quarantine).is_err());
        assert_eq!(rows(&most), 10);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The deletion names the row, it does not count it: a `rowid` SQLite handed
    /// out again would have pointed at a healthy row by the time it was used.
    #[test]
    fn the_row_that_goes_is_the_row_that_failed() {
        let directory = scratch("named");
        let key: StoreKey = zeroize::Zeroizing::new([3u8; 32]);
        let quarantine = directory.join("quarantine.sqlite3");
        let file = directory.join("named.sqlite3");
        // The damaged row is written last, so it holds the highest rowid - the one
        // SQLite reuses first.
        store(&file, &key, 4, 1);

        let kept: Vec<Vec<u8>> = {
            let connection = Connection::open(&file).expect("open");
            let mut statement = connection.prepare("SELECT id FROM things").expect("prepare");
            let ids = statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("ids");
            ids
        };

        scrub_table(&file, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");

        let left: Vec<Vec<u8>> = {
            let connection = Connection::open(&file).expect("open");
            let mut statement = connection.prepare("SELECT id FROM things").expect("prepare");
            let ids = statement
                .query_map([], |row| row.get::<_, Vec<u8>>(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("ids");
            ids
        };
        assert_eq!(left, kept[..4].to_vec());
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The number the user needs is not how many keys went but how many of them
    /// the server never had.
    #[test]
    fn room_keys_without_a_backup_are_counted_apart() {
        let directory = scratch("backup");
        let key: StoreKey = zeroize::Zeroizing::new([5u8; 32]);
        let quarantine = directory.join("quarantine.sqlite3");

        let file = directory.join("keys.sqlite3");
        store_with_backup(&file, &key, 20, 4, 3);
        let outcome = scrub_table(&file, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");
        assert_eq!(outcome.dropped, 4);
        assert_eq!(outcome.unsaved, 1);

        // A table that has no such column says nothing about backups, and a claim
        // nobody checked must not read as "all safe". The column decides, not a
        // flag this file carries about the table.
        let plain = directory.join("plain.sqlite3");
        store_without_backup(&plain, &key, 20, 4);
        let outcome = scrub_table(&plain, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");
        assert_eq!((outcome.dropped, outcome.unsaved), (4, 4));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Whoever cannot put a row aside does not get to delete it.
    #[test]
    fn a_quarantine_that_cannot_be_written_stops_the_repair() {
        let directory = scratch("noquarantine");
        let key: StoreKey = zeroize::Zeroizing::new([11u8; 32]);
        let file = directory.join("one.sqlite3");
        store(&file, &key, 9, 1);

        // A directory where the file should be: opening it fails, and the rows stay.
        let blocked = directory.join("quarantine.sqlite3");
        std::fs::create_dir_all(&blocked).expect("blocked");
        assert!(scrub_table(&file, Some(&key), &PLAIN, parses, &blocked).is_err());
        assert_eq!(rows(&file), 10);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The quarantine keeps the row that is being taken out now, not the one it
    /// happened to see first.
    #[test]
    fn the_same_row_failing_twice_keeps_the_newer_copy() {
        let directory = scratch("twice");
        let key: StoreKey = zeroize::Zeroizing::new([13u8; 32]);
        let quarantine = directory.join("quarantine.sqlite3");

        let first = directory.join("a.sqlite3");
        store(&first, &key, 4, 1);
        scrub_table(&first, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");
        assert_eq!(quarantined(&quarantine), 1);

        // The same store name and the same key, a second time: one row, not two,
        // and it is the second one.
        std::fs::remove_file(&first).expect("remove");
        store(&first, &key, 4, 1);
        scrub_table(&first, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");
        assert_eq!(quarantined(&quarantine), 1);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A quarantine a killed process left half-written must not block every
    /// future repair. Replaced once; a failing *write* is not (that is the test
    /// above, which hands it a directory).
    #[test]
    fn a_broken_quarantine_is_replaced_rather_than_fatal() {
        let directory = scratch("brokenq");
        let key: StoreKey = zeroize::Zeroizing::new([17u8; 32]);
        let quarantine = directory.join("quarantine.sqlite3");
        std::fs::write(&quarantine, b"this is not a database").expect("write");

        let file = directory.join("one.sqlite3");
        store(&file, &key, 4, 1);
        let outcome = scrub_table(&file, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");
        assert_eq!((outcome.dropped, outcome.kept), (1, 1));
        assert_eq!(quarantined(&quarantine), 1);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// "Put aside" has to mean the row can come back. Every column is kept, not
    /// just key and data - `backed_up` and the rest are NOT NULL in the real
    /// tables and cannot be rebuilt from a pickle that does not decode.
    #[test]
    fn the_quarantine_keeps_every_column() {
        let directory = scratch("whole");
        let key: StoreKey = zeroize::Zeroizing::new([19u8; 32]);
        let quarantine = directory.join("quarantine.sqlite3");
        let file = directory.join("one.sqlite3");
        store_with_backup(&file, &key, 2, 1, 1);

        scrub_table(&file, Some(&key), &PLAIN, parses, &quarantine).expect("scrub");

        let kept: String = Connection::open(&quarantine)
            .expect("open")
            .query_row("SELECT row_json FROM quarantine", [], |row| row.get(0))
            .expect("row");
        let fields: serde_json::Value = serde_json::from_str(&kept).expect("json");
        // The whole row: the key, the value, and the column this file would
        // otherwise have dropped on the floor.
        assert!(fields.get("id").and_then(|value| value.get("blob")).is_some());
        assert!(fields.get("data").and_then(|value| value.get("blob")).is_some());
        assert_eq!(fields.get("backed_up"), Some(&serde_json::json!(1)));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A room that will not load is no reason to walk the room keys.
    #[test]
    fn the_scope_follows_the_line_that_latched() {
        assert_eq!(scope_of("CryptoStoreError( Backend( Decode( ... ) ) )"), Scope::Crypto);
        assert_eq!(scope_of("StateStoreError(Backend(Syntax(...)))"), Scope::State);
        // Both named, or neither: look at both, which is what the old code did
        // for every failure whatever it said.
        assert_eq!(scope_of("StateStore( CryptoStore( ... ) )"), Scope::Both);
        assert_eq!(scope_of("something else entirely"), Scope::Both);
        assert!(Scope::State.state() && !Scope::State.crypto());
        assert!(Scope::Crypto.crypto() && !Scope::Crypto.state());
    }
}
