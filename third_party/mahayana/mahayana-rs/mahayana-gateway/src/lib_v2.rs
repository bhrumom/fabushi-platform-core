#[path = "lib.rs"]
mod base;

pub use base::{
    DEFAULT_REPLAY_BYTES_PER_SESSION, DEFAULT_REPLAY_EVENTS_PER_SESSION, DEFAULT_REPLAY_SESSIONS,
    DispatchResult, GatewayRuntime, ReplayLimits, ReplaySlice, RpcFailure,
};

use mahayana_core::RuntimeEvent;
use mahayana_gateway_protocol::{GATEWAY_PROTOCOL_VERSION, GatewayEvent, GatewayEventEnvelope};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const STATE_SCHEMA_VERSION: u64 = 1;
const STATE_FILE_NAME: &str = "gateway-state-v1.json";
const STATE_LOCK_WAIT_ATTEMPTS: usize = 200;
const STATE_LOCK_WAIT: Duration = Duration::from_millis(10);
const STATE_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

/// Rust-owned gateway state with a durable replay journal.
///
/// The transport-neutral state machine remains in `lib.rs`; this wrapper adds
/// process-restart durability without moving sequencing or transcript authority
/// into Electron/React. A restart deliberately creates a new replay epoch and
/// rehydrates retained semantic events under that epoch. Clients that still
/// hold the previous epoch therefore take the normal reset/reload path instead
/// of accepting stale sequence numbers.
pub struct GatewayState {
    inner: base::GatewayState,
    limits: ReplayLimits,
    journal: VecDeque<GatewayEventEnvelope>,
    state_path: Option<PathBuf>,
    last_persistence_error: Option<String>,
    restored_events: usize,
}

impl GatewayState {
    pub fn new(limits: ReplayLimits) -> Self {
        Self::with_path(limits, default_state_path())
    }

    fn with_path(limits: ReplayLimits, state_path: Option<PathBuf>) -> Self {
        let mut state = Self {
            inner: base::GatewayState::new(limits),
            limits,
            journal: VecDeque::new(),
            state_path,
            last_persistence_error: None,
            restored_events: 0,
        };
        if let Some(path) = state.state_path.clone() {
            match load_replay_events(&path) {
                Ok(events) => {
                    if let Err(error) = state.restore_events(events) {
                        state.last_persistence_error = Some(error);
                    }
                }
                Err(error) => state.last_persistence_error = Some(error),
            }
        }
        state
    }

    pub fn replay_epoch(&self) -> &str {
        self.inner.replay_epoch()
    }

    pub fn register_turn(&mut self, turn_id: impl Into<String>, session_id: impl Into<String>) {
        self.inner.register_turn(turn_id, session_id);
    }

    pub fn emit(
        &mut self,
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        timestamp_ms: i64,
        event: GatewayEvent,
    ) -> GatewayEventEnvelope {
        let envelope = self
            .inner
            .emit(session_id, turn_id, timestamp_ms, event);
        self.record_events(std::slice::from_ref(&envelope));
        envelope
    }

    pub fn ingest_runtime_event(
        &mut self,
        event: &RuntimeEvent,
        timestamp_ms: i64,
    ) -> Vec<GatewayEventEnvelope> {
        let events = self.inner.ingest_runtime_event(event, timestamp_ms);
        self.record_events(&events);
        events
    }

    pub fn replay_since(
        &self,
        session_id: &str,
        last_seen: u64,
        expected_epoch: Option<&str>,
    ) -> ReplaySlice {
        self.inner
            .replay_since(session_id, last_seen, expected_epoch)
    }

    pub fn interrupt_turn(
        &mut self,
        turn_id: &str,
        timestamp_ms: i64,
    ) -> Vec<GatewayEventEnvelope> {
        let events = self.inner.interrupt_turn(turn_id, timestamp_ms);
        self.record_events(&events);
        events
    }

    pub fn durability_info(&self) -> Value {
        json!({
            "enabled": self.state_path.is_some(),
            "retainedEvents": self.journal.len(),
            "restoredEvents": self.restored_events,
            "lastError": self.last_persistence_error,
        })
    }

    fn restore_events(&mut self, events: Vec<GatewayEventEnvelope>) -> Result<(), String> {
        let mut active_turns = HashMap::<String, String>::new();
        let mut restored = 0usize;
        for envelope in events {
            if envelope.protocol_version != GATEWAY_PROTOCOL_VERSION {
                return Err(format!(
                    "unsupported persisted gateway protocol version {}",
                    envelope.protocol_version
                ));
            }
            let timestamp_ms = chrono::DateTime::parse_from_rfc3339(&envelope.timestamp)
                .map_err(|error| format!("invalid persisted gateway timestamp: {error}"))?
                .timestamp_millis();
            let is_complete = matches!(envelope.event, GatewayEvent::MessageComplete(_));
            let session_id = envelope.session_id;
            let turn_id = envelope.turn_id;
            let normalized = self.inner.emit(
                session_id.clone(),
                turn_id.clone(),
                timestamp_ms,
                envelope.event,
            );
            if is_complete {
                active_turns.remove(&turn_id);
            } else {
                active_turns.insert(turn_id.clone(), session_id.clone());
            }
            self.journal.push_back(normalized);
            restored = restored.saturating_add(1);
        }
        for (turn_id, session_id) in active_turns {
            self.inner.register_turn(turn_id, session_id);
        }
        self.restored_events = restored;
        self.prune_journal();
        Ok(())
    }

    fn record_events(&mut self, events: &[GatewayEventEnvelope]) {
        if events.is_empty() {
            return;
        }
        self.journal.extend(events.iter().cloned());
        self.prune_journal();
        if let Some(path) = self.state_path.clone() {
            if let Err(error) = update_replay_events(&path, &self.journal) {
                self.last_persistence_error = Some(error);
            } else {
                self.last_persistence_error = None;
            }
        }
    }

    fn prune_journal(&mut self) {
        let max_sessions = self.limits.max_sessions.max(1);
        let max_events = self.limits.max_events_per_session.max(1);
        let max_bytes = self.limits.max_bytes_per_session.max(1);

        let mut newest_sessions = Vec::<String>::new();
        let mut seen = HashSet::<String>::new();
        for event in self.journal.iter().rev() {
            if seen.insert(event.session_id.clone()) {
                newest_sessions.push(event.session_id.clone());
                if newest_sessions.len() >= max_sessions {
                    break;
                }
            }
        }
        let keep = newest_sessions.into_iter().collect::<HashSet<_>>();
        self.journal.retain(|event| keep.contains(&event.session_id));

        for session_id in keep {
            loop {
                let mut count = 0usize;
                let mut bytes = 0usize;
                for event in self
                    .journal
                    .iter()
                    .filter(|event| event.session_id == session_id)
                {
                    count = count.saturating_add(1);
                    bytes = bytes.saturating_add(
                        serde_json::to_vec(event)
                            .map(|encoded| encoded.len())
                            .unwrap_or(usize::MAX),
                    );
                }
                if count <= max_events && bytes <= max_bytes {
                    break;
                }
                let Some(index) = self
                    .journal
                    .iter()
                    .position(|event| event.session_id == session_id)
                else {
                    break;
                };
                self.journal.remove(index);
            }
        }
    }
}

pub fn dispatch_request<R: GatewayRuntime>(
    method: &str,
    params: &Value,
    runtime: &mut R,
    state: &mut GatewayState,
    timestamp_ms: i64,
) -> Result<DispatchResult, RpcFailure> {
    let mut dispatched = base::dispatch_request(
        method,
        params,
        runtime,
        &mut state.inner,
        timestamp_ms,
    )?;
    state.record_events(&dispatched.events);
    if method == "gateway.info"
        && let Some(object) = dispatched.result.as_object_mut()
    {
        object.insert("durability".to_string(), state.durability_info());
    }
    Ok(dispatched)
}

fn default_state_path() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("MAHAYANA_GATEWAY_STATE") {
        let configured = configured.to_string_lossy();
        if configured.eq_ignore_ascii_case("memory") || configured.eq_ignore_ascii_case("off") {
            return None;
        }
        if !configured.trim().is_empty() {
            return Some(PathBuf::from(configured.as_ref()));
        }
    }
    if let Some(home) = std::env::var_os("MAHAYANA_HOME") {
        return Some(PathBuf::from(home).join(STATE_FILE_NAME));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".mahayana").join(STATE_FILE_NAME))
}

fn load_replay_events(path: &Path) -> Result<Vec<GatewayEventEnvelope>, String> {
    let document = read_state_document(path)?;
    let Some(replay) = document.get("replay") else {
        return Ok(Vec::new());
    };
    let events = replay
        .get("events")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::from_value(events).map_err(|error| format!("invalid gateway replay state: {error}"))
}

fn update_replay_events(
    path: &Path,
    events: &VecDeque<GatewayEventEnvelope>,
) -> Result<(), String> {
    with_state_lock(path, || {
        let mut document = read_state_document_unlocked(path)?;
        let object = document
            .as_object_mut()
            .ok_or_else(|| "gateway state root must be an object".to_string())?;
        object.insert("schemaVersion".to_string(), Value::from(STATE_SCHEMA_VERSION));
        object.insert(
            "replay".to_string(),
            json!({"events": events.iter().cloned().collect::<Vec<_>>() }),
        );
        write_state_document_unlocked(path, &document)
    })
}

fn read_state_document(path: &Path) -> Result<Value, String> {
    with_state_lock(path, || read_state_document_unlocked(path))
}

fn read_state_document_unlocked(path: &Path) -> Result<Value, String> {
    let backup = backup_path(path);
    let source = if path.exists() {
        Some(path)
    } else if backup.exists() {
        Some(backup.as_path())
    } else {
        None
    };
    let Some(source) = source else {
        return Ok(Value::Object(Map::new()));
    };
    let bytes = fs::read(source)
        .map_err(|error| format!("failed to read gateway state {}: {error}", source.display()))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse gateway state {}: {error}", source.display()))?;
    let object = document
        .as_object()
        .ok_or_else(|| "gateway state root must be an object".to_string())?;
    if let Some(version) = object.get("schemaVersion").and_then(Value::as_u64)
        && version != STATE_SCHEMA_VERSION
    {
        return Err(format!(
            "unsupported gateway state schema version {version}; expected {STATE_SCHEMA_VERSION}"
        ));
    }
    Ok(document)
}

fn write_state_document_unlocked(path: &Path, document: &Value) -> Result<(), String> {
    ensure_parent(path)?;
    let temporary = temporary_path(path);
    let backup = backup_path(path);
    let bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("failed to encode gateway state: {error}"))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("failed to create gateway state temp file: {error}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("failed to flush gateway state: {error}"))?;
    set_private_file_permissions(&temporary)?;

    if backup.exists() {
        fs::remove_file(&backup)
            .map_err(|error| format!("failed to remove stale gateway state backup: {error}"))?;
    }
    if path.exists() {
        fs::rename(path, &backup)
            .map_err(|error| format!("failed to stage gateway state backup: {error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() && !path.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(format!("failed to install gateway state: {error}"));
    }
    if backup.exists() {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

fn with_state_lock<T>(path: &Path, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    ensure_parent(path)?;
    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    let _lock = acquire_state_lock(&lock_path)?;
    action()
}

fn acquire_state_lock(path: &Path) -> Result<StateFileLock, String> {
    for _ in 0..STATE_LOCK_WAIT_ATTEMPTS {
        match OpenOptions::new().create_new(true).write(true).open(path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                let _ = file.sync_all();
                set_private_file_permissions(path)?;
                return Ok(StateFileLock {
                    path: path.to_path_buf(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|elapsed| elapsed > STATE_LOCK_STALE_AFTER);
                if stale {
                    let _ = fs::remove_file(path);
                    continue;
                }
                thread::sleep(STATE_LOCK_WAIT);
            }
            Err(error) => {
                return Err(format!("failed to acquire gateway state lock: {error}"));
            }
        }
    }
    Err("timed out acquiring gateway state lock".to_string())
}

struct StateFileLock {
    path: PathBuf,
}

impl Drop for StateFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create gateway state directory: {error}"))?;
        set_private_directory_permissions(parent)?;
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.tmp-{}-{}",
        path.display(),
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ))
}

fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", path.display()))
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to restrict gateway state permissions: {error}"))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to restrict gateway state directory: {error}"))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod durability_tests {
    use super::*;
    use mahayana_gateway_protocol::{MessageDeltaPayload, MessageStartPayload};

    fn temp_state_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "mahayana-gateway-state-{}.json",
            uuid::Uuid::new_v4().simple()
        ))
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(backup_path(path));
        let _ = fs::remove_file(PathBuf::from(format!("{}.lock", path.display())));
    }

    #[test]
    fn restart_rehydrates_replay_under_a_fresh_epoch() {
        let path = temp_state_path();
        let first_epoch;
        {
            let mut state = GatewayState::with_path(ReplayLimits::default(), Some(path.clone()));
            first_epoch = state.replay_epoch().to_string();
            state.emit(
                "session-1",
                "turn-1",
                1_700_000_000_000,
                GatewayEvent::MessageStart(MessageStartPayload {
                    role: Some("assistant".to_string()),
                }),
            );
            state.emit(
                "session-1",
                "turn-1",
                1_700_000_000_001,
                GatewayEvent::MessageDelta(MessageDeltaPayload {
                    text: "hello".to_string(),
                    rendered: None,
                }),
            );
            assert_eq!(state.replay_since("session-1", 0, None).events.len(), 2);
        }

        let restored = GatewayState::with_path(ReplayLimits::default(), Some(path.clone()));
        assert_ne!(restored.replay_epoch(), first_epoch);
        let replay = restored.replay_since("session-1", 0, None);
        assert_eq!(replay.latest_seq, 2);
        assert_eq!(replay.events.len(), 2);
        assert_eq!(replay.events[0].kind(), "message.start");
        assert_eq!(replay.events[1].kind(), "message.delta");
        assert_eq!(restored.restored_events, 2);
        cleanup(&path);
    }

    #[test]
    fn shared_state_writer_preserves_non_replay_sections() {
        let path = temp_state_path();
        ensure_parent(&path).unwrap();
        fs::write(
            &path,
            br#"{"schemaVersion":1,"peer":{"requests":[{"id":"srq-test"}]}}"#,
        )
        .unwrap();
        let mut state = GatewayState::with_path(ReplayLimits::default(), Some(path.clone()));
        state.emit(
            "session-1",
            "turn-1",
            1_700_000_000_000,
            GatewayEvent::MessageDelta(MessageDeltaPayload {
                text: "persist".to_string(),
                rendered: None,
            }),
        );
        let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["peer"]["requests"][0]["id"], "srq-test");
        assert_eq!(document["replay"]["events"].as_array().unwrap().len(), 1);
        cleanup(&path);
    }
}
