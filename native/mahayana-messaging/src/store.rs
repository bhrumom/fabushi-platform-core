use crate::engine::MessagingState;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MESSAGING_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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
    #[error("unsupported messaging snapshot schema {actual}; expected {expected}")]
    UnsupportedSchema { expected: u32, actual: u32 },
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
        if snapshot.schema_version != MESSAGING_SNAPSHOT_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema {
                expected: MESSAGING_SNAPSHOT_SCHEMA_VERSION,
                actual: snapshot.schema_version,
            });
        }
        Ok(Some(snapshot))
    }

    fn save(&mut self, snapshot: &MessagingSnapshot) -> Result<(), StoreError> {
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
