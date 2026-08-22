use crate::engine::MessagingState;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MESSAGING_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const MESSAGING_SQLITE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagingSnapshot {
    pub schema_version: u32,
    pub cursor: u64,
    pub saved_at_ms: i64,
    pub state: MessagingState,
}

impl MessagingSnapshot {
    pub fn new(state: MessagingState, cursor: u64, saved_at_ms: i64) -> Self {
        Self {
            schema_version: MESSAGING_SNAPSHOT_SCHEMA_VERSION,
            cursor,
            saved_at_ms,
            state,
        }
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("messaging store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("messaging store JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("messaging store SQLite failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("messaging cursor is invalid: {0}")]
    InvalidCursor(#[from] std::num::ParseIntError),
    #[error("unsupported messaging snapshot schema {actual}; expected {expected}")]
    UnsupportedSchema { expected: u32, actual: u32 },
    #[error("unsupported messaging SQLite schema {actual}; expected at most {expected}")]
    UnsupportedSqliteSchema { expected: u32, actual: u32 },
}

pub trait MessagingStateStore: Send {
    fn load(&self) -> Result<Option<MessagingSnapshot>, StoreError>;
    fn save(&mut self, snapshot: &MessagingSnapshot) -> Result<(), StoreError>;
}

#[derive(Debug, Clone, Default)]
pub struct MemoryStateStore {
    snapshot: Option<MessagingSnapshot>,
}

impl MemoryStateStore {
    pub fn snapshot(&self) -> Option<&MessagingSnapshot> {
        self.snapshot.as_ref()
    }
}

impl MessagingStateStore for MemoryStateStore {
    fn load(&self) -> Result<Option<MessagingSnapshot>, StoreError> {
        Ok(self.snapshot.clone())
    }

    fn save(&mut self, snapshot: &MessagingSnapshot) -> Result<(), StoreError> {
        validate_snapshot_schema(snapshot.schema_version)?;
        self.snapshot = Some(snapshot.clone());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct JsonFileStateStore {
    path: PathBuf,
}

impl JsonFileStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn temporary_path(&self) -> PathBuf {
        let mut path = self.path.clone();
        let extension = self
            .path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}.tmp"))
            .unwrap_or_else(|| "tmp".into());
        path.set_extension(extension);
        path
    }
}

impl MessagingStateStore for JsonFileStateStore {
    fn load(&self) -> Result<Option<MessagingSnapshot>, StoreError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let mut file = File::open(&self.path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let snapshot: MessagingSnapshot = serde_json::from_slice(&bytes)?;
        validate_snapshot_schema(snapshot.schema_version)?;
        Ok(Some(snapshot))
    }

    fn save(&mut self, snapshot: &MessagingSnapshot) -> Result<(), StoreError> {
        validate_snapshot_schema(snapshot.schema_version)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary_path = self.temporary_path();
        let payload = serde_json::to_vec(snapshot)?;
        {
            let mut file = File::create(&temporary_path)?;
            file.write_all(&payload)?;
            file.sync_all()?;
        }
        fs::rename(&temporary_path, &self.path)?;
        if let Some(parent) = self.path.parent() {
            if let Ok(directory) = File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SqliteStateStore {
    path: PathBuf,
}

impl SqliteStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn initialize(&self) -> Result<(), StoreError> {
        let connection = self.open()?;
        migrate_sqlite(&connection)
    }

    fn open(&self) -> Result<Connection, StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Connection::open(&self.path)?)
    }

    fn open_initialized(&self) -> Result<Connection, StoreError> {
        let connection = self.open()?;
        migrate_sqlite(&connection)?;
        Ok(connection)
    }
}

impl MessagingStateStore for SqliteStateStore {
    fn load(&self) -> Result<Option<MessagingSnapshot>, StoreError> {
        let connection = self.open_initialized()?;
        let row = connection
            .query_row(
                "SELECT snapshot_schema_version, cursor, saved_at_ms, state_json FROM messaging_snapshot WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;

        let Some((schema_version, cursor, saved_at_ms, state_json)) = row else {
            return Ok(None);
        };
        validate_snapshot_schema(schema_version)?;
        let state = serde_json::from_str(&state_json)?;
        Ok(Some(MessagingSnapshot {
            schema_version,
            cursor: cursor.parse()?,
            saved_at_ms,
            state,
        }))
    }

    fn save(&mut self, snapshot: &MessagingSnapshot) -> Result<(), StoreError> {
        validate_snapshot_schema(snapshot.schema_version)?;
        let mut connection = self.open_initialized()?;
        let transaction = connection.transaction()?;
        let state_json = serde_json::to_string(&snapshot.state)?;
        transaction.execute(
            "INSERT INTO messaging_snapshot (singleton, snapshot_schema_version, cursor, saved_at_ms, state_json) VALUES (1, ?1, ?2, ?3, ?4) ON CONFLICT(singleton) DO UPDATE SET snapshot_schema_version = excluded.snapshot_schema_version, cursor = excluded.cursor, saved_at_ms = excluded.saved_at_ms, state_json = excluded.state_json",
            params![
                snapshot.schema_version,
                snapshot.cursor.to_string(),
                snapshot.saved_at_ms,
                state_json
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn validate_snapshot_schema(actual: u32) -> Result<(), StoreError> {
    if actual != MESSAGING_SNAPSHOT_SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSchema {
            expected: MESSAGING_SNAPSHOT_SCHEMA_VERSION,
            actual,
        });
    }
    Ok(())
}

fn migrate_sqlite(connection: &Connection) -> Result<(), StoreError> {
    let actual: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if actual > MESSAGING_SQLITE_SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSqliteSchema {
            expected: MESSAGING_SQLITE_SCHEMA_VERSION,
            actual,
        });
    }
    if actual == 0 {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS messaging_snapshot (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 snapshot_schema_version INTEGER NOT NULL,
                 cursor TEXT NOT NULL,
                 saved_at_ms INTEGER NOT NULL,
                 state_json TEXT NOT NULL
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_db(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "fabushi-messaging-{name}-{}-{nonce}.sqlite3",
            std::process::id()
        ))
    }

    fn remove_db(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn sqlite_store_initializes_and_round_trips_snapshot() {
        let path = temporary_db("roundtrip");
        let mut store = SqliteStateStore::new(&path);
        assert_eq!(store.load().expect("load empty database"), None);

        let snapshot = MessagingSnapshot::new(MessagingState::default(), u64::MAX, 42);
        store.save(&snapshot).expect("save snapshot");
        drop(store);

        let reopened = SqliteStateStore::new(&path);
        assert_eq!(reopened.load().expect("reload snapshot"), Some(snapshot));
        remove_db(&path);
    }

    #[test]
    fn sqlite_store_overwrites_singleton_snapshot_transactionally() {
        let path = temporary_db("overwrite");
        let mut store = SqliteStateStore::new(&path);
        let first = MessagingSnapshot::new(MessagingState::default(), 1, 10);
        let second = MessagingSnapshot::new(MessagingState::default(), 2, 20);

        store.save(&first).expect("save first snapshot");
        store.save(&second).expect("replace snapshot");

        assert_eq!(store.load().expect("load replacement"), Some(second));
        let connection = Connection::open(&path).expect("open database");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM messaging_snapshot", [], |row| {
                row.get(0)
            })
            .expect("count snapshots");
        assert_eq!(count, 1);
        remove_db(&path);
    }

    #[test]
    fn sqlite_store_rejects_newer_database_schema() {
        let path = temporary_db("future-schema");
        let connection = Connection::open(&path).expect("open database");
        connection
            .execute_batch("PRAGMA user_version = 999;")
            .expect("set future schema");
        drop(connection);

        let store = SqliteStateStore::new(&path);
        assert!(matches!(
            store.load(),
            Err(StoreError::UnsupportedSqliteSchema {
                expected: MESSAGING_SQLITE_SCHEMA_VERSION,
                actual: 999
            })
        ));
        remove_db(&path);
    }

    #[test]
    fn all_state_stores_reject_unknown_snapshot_schema() {
        let invalid = MessagingSnapshot {
            schema_version: MESSAGING_SNAPSHOT_SCHEMA_VERSION + 1,
            cursor: 0,
            saved_at_ms: 0,
            state: MessagingState::default(),
        };

        let mut memory = MemoryStateStore::default();
        assert!(matches!(
            memory.save(&invalid),
            Err(StoreError::UnsupportedSchema { .. })
        ));

        let path = temporary_db("invalid-snapshot");
        let mut sqlite = SqliteStateStore::new(&path);
        assert!(matches!(
            sqlite.save(&invalid),
            Err(StoreError::UnsupportedSchema { .. })
        ));
        remove_db(&path);
    }
}
