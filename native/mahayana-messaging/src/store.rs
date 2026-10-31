use crate::actor::ActorId;
use crate::engine::MessagingState;
use crate::protocol::ServerEnvelope;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MESSAGING_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const MESSAGING_SQLITE_SCHEMA_VERSION: u32 = 2;
pub const MESSAGING_EVENT_JOURNAL_LIMIT: usize = 10_000;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub envelope: ServerEnvelope,
    pub audience: Vec<ActorId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventJournalSlice {
    pub floor_cursor: u64,
    pub current_cursor: u64,
    pub checkpoint_cursor: u64,
    pub has_more: bool,
    pub entries: Vec<JournalEntry>,
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
    #[error("messaging journal event is missing a server cursor")]
    MissingEventCursor,
    #[error("unsupported messaging snapshot schema {actual}; expected {expected}")]
    UnsupportedSchema { expected: u32, actual: u32 },
    #[error("unsupported messaging SQLite schema {actual}; expected at most {expected}")]
    UnsupportedSqliteSchema { expected: u32, actual: u32 },
}

pub trait MessagingStateStore: Send {
    fn load(&self) -> Result<Option<MessagingSnapshot>, StoreError>;
    fn save(&mut self, snapshot: &MessagingSnapshot) -> Result<(), StoreError>;

    fn save_with_events(
        &mut self,
        snapshot: &MessagingSnapshot,
        _events: &[JournalEntry],
    ) -> Result<(), StoreError> {
        self.save(snapshot)
    }

    fn load_event_journal_after(
        &self,
        _cursor: u64,
        _cursor_group_limit: usize,
    ) -> Result<Option<EventJournalSlice>, StoreError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryStateStore {
    snapshot: Option<MessagingSnapshot>,
    journal: Vec<JournalEntry>,
    journal_floor_cursor: u64,
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
        if self.journal.is_empty() {
            self.journal_floor_cursor = self.journal_floor_cursor.max(snapshot.cursor);
        }
        self.snapshot = Some(snapshot.clone());
        Ok(())
    }

    fn save_with_events(
        &mut self,
        snapshot: &MessagingSnapshot,
        events: &[JournalEntry],
    ) -> Result<(), StoreError> {
        validate_snapshot_schema(snapshot.schema_version)?;
        for event in events {
            journal_cursor(event)?;
        }
        self.snapshot = Some(snapshot.clone());
        self.journal.extend_from_slice(events);
        if self.journal.len() > MESSAGING_EVENT_JOURNAL_LIMIT {
            let excess = self.journal.len() - MESSAGING_EVENT_JOURNAL_LIMIT;
            let prune_cursor = journal_cursor(&self.journal[excess - 1])?;
            let remove_count = self
                .journal
                .iter()
                .take_while(|entry| {
                    journal_cursor(entry).is_ok_and(|cursor| cursor <= prune_cursor)
                })
                .count();
            self.journal.drain(0..remove_count);
            self.journal_floor_cursor = prune_cursor;
        }
        Ok(())
    }

    fn load_event_journal_after(
        &self,
        cursor: u64,
        cursor_group_limit: usize,
    ) -> Result<Option<EventJournalSlice>, StoreError> {
        let current_cursor = self.snapshot.as_ref().map_or(0, |snapshot| snapshot.cursor);
        Ok(Some(slice_journal(
            &self.journal,
            self.journal_floor_cursor,
            current_cursor,
            cursor,
            cursor_group_limit,
        )?))
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
        let mut connection = self.open()?;
        migrate_sqlite(&mut connection)
    }

    pub fn import_json_if_empty(
        &mut self,
        legacy: &JsonFileStateStore,
    ) -> Result<bool, StoreError> {
        if self.load()?.is_some() {
            return Ok(false);
        }
        let Some(snapshot) = legacy.load()? else {
            return Ok(false);
        };
        self.save(&snapshot)?;
        Ok(true)
    }

    fn open(&self) -> Result<Connection, StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Connection::open(&self.path)?)
    }

    fn open_initialized(&self) -> Result<Connection, StoreError> {
        let mut connection = self.open()?;
        migrate_sqlite(&mut connection)?;
        Ok(connection)
    }
}

impl MessagingStateStore for SqliteStateStore {
    fn load(&self) -> Result<Option<MessagingSnapshot>, StoreError> {
        let connection = self.open_initialized()?;
        load_sqlite_snapshot(&connection)
    }

    fn save(&mut self, snapshot: &MessagingSnapshot) -> Result<(), StoreError> {
        validate_snapshot_schema(snapshot.schema_version)?;
        let mut connection = self.open_initialized()?;
        let transaction = connection.transaction()?;
        write_snapshot(&transaction, snapshot)?;
        let journal_count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM messaging_event_journal", [], |row| {
                row.get(0)
            })?;
        if journal_count == 0 {
            set_journal_floor(&transaction, snapshot.cursor)?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn save_with_events(
        &mut self,
        snapshot: &MessagingSnapshot,
        events: &[JournalEntry],
    ) -> Result<(), StoreError> {
        validate_snapshot_schema(snapshot.schema_version)?;
        let mut connection = self.open_initialized()?;
        let transaction = connection.transaction()?;
        write_snapshot(&transaction, snapshot)?;
        for event in events {
            let cursor = journal_cursor(event)?;
            transaction.execute(
                "INSERT INTO messaging_event_journal (cursor, audience_json, envelope_json) VALUES (?1, ?2, ?3)",
                params![
                    cursor.to_string(),
                    serde_json::to_string(&event.audience)?,
                    serde_json::to_string(&event.envelope)?
                ],
            )?;
        }
        prune_sqlite_journal(&transaction)?;
        transaction.commit()?;
        Ok(())
    }

    fn load_event_journal_after(
        &self,
        cursor: u64,
        cursor_group_limit: usize,
    ) -> Result<Option<EventJournalSlice>, StoreError> {
        let connection = self.open_initialized()?;
        let floor_cursor = connection
            .query_row(
                "SELECT value FROM messaging_metadata WHERE key = 'journal_floor_cursor'",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()?;
        let current_cursor =
            load_sqlite_snapshot(&connection)?.map_or(0, |snapshot| snapshot.cursor);
        let mut statement = connection.prepare(
            "SELECT audience_json, envelope_json FROM messaging_event_journal ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (audience_json, envelope_json) = row?;
            entries.push(JournalEntry {
                audience: serde_json::from_str(&audience_json)?,
                envelope: serde_json::from_str(&envelope_json)?,
            });
        }
        Ok(Some(slice_journal(
            &entries,
            floor_cursor,
            current_cursor,
            cursor,
            cursor_group_limit,
        )?))
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

fn journal_cursor(entry: &JournalEntry) -> Result<u64, StoreError> {
    entry
        .envelope
        .cursor
        .as_deref()
        .ok_or(StoreError::MissingEventCursor)?
        .parse()
        .map_err(StoreError::from)
}

fn slice_journal(
    entries: &[JournalEntry],
    floor_cursor: u64,
    current_cursor: u64,
    requested_cursor: u64,
    cursor_group_limit: usize,
) -> Result<EventJournalSlice, StoreError> {
    let limit = cursor_group_limit.max(1);
    let mut selected = Vec::new();
    let mut selected_groups = 0usize;
    let mut last_selected_cursor = None;
    let mut has_more = false;

    for entry in entries {
        let cursor = journal_cursor(entry)?;
        if cursor <= requested_cursor {
            continue;
        }
        if last_selected_cursor != Some(cursor) {
            if selected_groups >= limit {
                has_more = true;
                break;
            }
            selected_groups += 1;
            last_selected_cursor = Some(cursor);
        }
        selected.push(entry.clone());
    }

    let checkpoint_cursor = if has_more {
        last_selected_cursor.unwrap_or(requested_cursor)
    } else if selected.is_empty() && requested_cursor < current_cursor {
        requested_cursor
    } else {
        current_cursor
    };

    Ok(EventJournalSlice {
        floor_cursor,
        current_cursor,
        checkpoint_cursor,
        has_more,
        entries: selected,
    })
}

fn load_sqlite_snapshot(connection: &Connection) -> Result<Option<MessagingSnapshot>, StoreError> {
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
    Ok(Some(MessagingSnapshot {
        schema_version,
        cursor: cursor.parse()?,
        saved_at_ms,
        state: serde_json::from_str(&state_json)?,
    }))
}

fn write_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &MessagingSnapshot,
) -> Result<(), StoreError> {
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
    Ok(())
}

fn set_journal_floor(transaction: &Transaction<'_>, cursor: u64) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO messaging_metadata (key, value) VALUES ('journal_floor_cursor', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![cursor.to_string()],
    )?;
    Ok(())
}

fn prune_sqlite_journal(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM messaging_event_journal", [], |row| {
            row.get(0)
        })?;
    let limit = i64::try_from(MESSAGING_EVENT_JOURNAL_LIMIT).unwrap_or(i64::MAX);
    if count <= limit {
        return Ok(());
    }
    let excess = count.saturating_sub(limit);
    let prune_cursor: String = transaction.query_row(
        "SELECT cursor FROM messaging_event_journal ORDER BY sequence ASC LIMIT 1 OFFSET ?1",
        params![excess.saturating_sub(1)],
        |row| row.get(0),
    )?;
    let max_sequence: i64 = transaction.query_row(
        "SELECT MAX(sequence) FROM messaging_event_journal WHERE cursor = ?1",
        params![&prune_cursor],
        |row| row.get(0),
    )?;
    transaction.execute(
        "DELETE FROM messaging_event_journal WHERE sequence <= ?1",
        params![max_sequence],
    )?;
    set_journal_floor(transaction, prune_cursor.parse()?)?;
    Ok(())
}

fn migrate_sqlite(connection: &mut Connection) -> Result<(), StoreError> {
    let actual: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if actual > MESSAGING_SQLITE_SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSqliteSchema {
            expected: MESSAGING_SQLITE_SCHEMA_VERSION,
            actual,
        });
    }
    if actual == 0 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS messaging_snapshot (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 snapshot_schema_version INTEGER NOT NULL,
                 cursor TEXT NOT NULL,
                 saved_at_ms INTEGER NOT NULL,
                 state_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messaging_event_journal (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 cursor TEXT NOT NULL,
                 audience_json TEXT NOT NULL,
                 envelope_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messaging_metadata (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO messaging_metadata (key, value) VALUES ('journal_floor_cursor', '0')",
            [],
        )?;
        transaction.execute_batch("PRAGMA user_version = 2;")?;
        transaction.commit()?;
    } else if actual == 1 {
        let current_cursor = connection
            .query_row(
                "SELECT cursor FROM messaging_snapshot WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| "0".into());
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS messaging_event_journal (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 cursor TEXT NOT NULL,
                 audience_json TEXT NOT NULL,
                 envelope_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messaging_metadata (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        transaction.execute(
            "INSERT OR REPLACE INTO messaging_metadata (key, value) VALUES ('journal_floor_cursor', ?1)",
            params![current_cursor],
        )?;
        transaction.execute_batch("PRAGMA user_version = 2;")?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ServerEvent, FABUSHI_MESSAGING_PROTOCOL_VERSION};
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

    fn journal_entry(cursor: u64, actor: &str) -> JournalEntry {
        JournalEntry {
            envelope: ServerEnvelope {
                protocol_version: FABUSHI_MESSAGING_PROTOCOL_VERSION,
                cursor: Some(cursor.to_string()),
                server_time_ms: i64::try_from(cursor).unwrap_or(i64::MAX),
                event: ServerEvent::FolderDeleted {
                    folder_id: format!("folder-{cursor}"),
                },
            },
            audience: vec![ActorId::new(actor)],
        }
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
    fn sqlite_store_imports_legacy_json_once() {
        let path = temporary_db("legacy-import");
        let legacy_path = path.with_extension("json");
        let mut legacy = JsonFileStateStore::new(&legacy_path);
        let original = MessagingSnapshot::new(MessagingState::default(), 41, 100);
        legacy.save(&original).expect("save legacy snapshot");

        let mut store = SqliteStateStore::new(&path);
        assert!(store
            .import_json_if_empty(&legacy)
            .expect("import legacy snapshot"));
        assert_eq!(
            store.load().expect("load imported snapshot"),
            Some(original)
        );

        let replacement = MessagingSnapshot::new(MessagingState::default(), 99, 200);
        legacy
            .save(&replacement)
            .expect("replace legacy snapshot after import");
        assert!(!store
            .import_json_if_empty(&legacy)
            .expect("skip second legacy import"));
        assert_eq!(
            store.load().expect("keep sqlite snapshot").unwrap().cursor,
            41
        );

        remove_db(&path);
        let _ = fs::remove_file(legacy_path);
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
    fn plain_snapshot_save_preserves_existing_journal() {
        let path = temporary_db("journal-preserve");
        let mut store = SqliteStateStore::new(&path);
        let first = MessagingSnapshot::new(MessagingState::default(), 1, 10);
        store
            .save_with_events(&first, &[journal_entry(1, "human:a")])
            .expect("save journal entry");
        let second = MessagingSnapshot::new(MessagingState::default(), 2, 20);
        store.save(&second).expect("save plain snapshot");
        let slice = store
            .load_event_journal_after(0, 100)
            .expect("load preserved journal")
            .expect("sqlite journal");
        assert_eq!(slice.entries.len(), 1);
        assert_eq!(slice.current_cursor, 2);
        remove_db(&path);
    }

    #[test]
    fn sqlite_v1_migration_sets_journal_floor_to_existing_cursor() {
        let path = temporary_db("v1-migration");
        let connection = Connection::open(&path).expect("open database");
        connection
            .execute_batch(
                "CREATE TABLE messaging_snapshot (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    snapshot_schema_version INTEGER NOT NULL,
                    cursor TEXT NOT NULL,
                    saved_at_ms INTEGER NOT NULL,
                    state_json TEXT NOT NULL
                 );
                 INSERT INTO messaging_snapshot VALUES (1, 1, '41', 100, '{}');
                 PRAGMA user_version = 1;",
            )
            .expect("create v1 database");
        drop(connection);

        let store = SqliteStateStore::new(&path);
        let slice = store
            .load_event_journal_after(40, 100)
            .expect("load migrated journal")
            .expect("sqlite journal");
        assert_eq!(slice.floor_cursor, 41);
        assert_eq!(slice.current_cursor, 41);
        remove_db(&path);
    }

    #[test]
    fn sqlite_store_persists_journal_with_snapshot() {
        let path = temporary_db("journal");
        let mut store = SqliteStateStore::new(&path);
        let snapshot = MessagingSnapshot::new(MessagingState::default(), 3, 30);
        store
            .save_with_events(
                &snapshot,
                &[journal_entry(2, "human:a"), journal_entry(3, "human:b")],
            )
            .expect("save snapshot and journal");
        drop(store);

        let reopened = SqliteStateStore::new(&path);
        let slice = reopened
            .load_event_journal_after(1, 100)
            .expect("load journal")
            .expect("sqlite journal");
        assert_eq!(slice.floor_cursor, 0);
        assert_eq!(slice.current_cursor, 3);
        assert_eq!(slice.checkpoint_cursor, 3);
        assert_eq!(slice.entries.len(), 2);
        assert_eq!(slice.entries[0].audience, vec![ActorId::new("human:a")]);
        remove_db(&path);
    }

    #[test]
    fn journal_pagination_never_splits_a_cursor_group() {
        let mut store = MemoryStateStore::default();
        let snapshot = MessagingSnapshot::new(MessagingState::default(), 3, 30);
        store
            .save_with_events(
                &snapshot,
                &[
                    journal_entry(1, "human:a"),
                    journal_entry(2, "human:a"),
                    journal_entry(2, "human:b"),
                    journal_entry(3, "human:a"),
                ],
            )
            .expect("save memory journal");
        let first = store
            .load_event_journal_after(0, 2)
            .expect("load first page")
            .expect("memory journal");
        assert!(first.has_more);
        assert_eq!(first.checkpoint_cursor, 2);
        assert_eq!(first.entries.len(), 3);
        let second = store
            .load_event_journal_after(first.checkpoint_cursor, 2)
            .expect("load second page")
            .expect("memory journal");
        assert!(!second.has_more);
        assert_eq!(second.checkpoint_cursor, 3);
        assert_eq!(second.entries.len(), 1);
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
