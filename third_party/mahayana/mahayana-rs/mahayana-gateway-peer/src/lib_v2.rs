#[path = "lib.rs"]
mod base;

pub use base::{
    ClarifyLockOutcome, IssuedServerRequest, JSON_RPC_VERSION, PendingServerRequest,
    REQUEST_CANCEL_METHOD, ResolvedServerRequest, SERVER_REQUEST_METHODS, ServerRequestError,
    ServerRequestRegistry, ServerRequestResponse, cancel_notification,
};

use serde_json::{Map, Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const STATE_SCHEMA_VERSION: u64 = 1;
const STATE_FILE_NAME: &str = "gateway-state-v1.json";
const STATE_LOCK_WAIT_ATTEMPTS: usize = 200;
const STATE_LOCK_WAIT: Duration = Duration::from_millis(10);
const STATE_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

/// Durable Rust owner for server->client requests. Every mutating operation is
/// synchronously committed before returning, so an app/process restart can
/// replay unanswered approvals/clarifications instead of reconstructing them
/// from renderer state.
pub struct PersistentServerRequestRegistry {
    inner: ServerRequestRegistry,
    state_path: Option<PathBuf>,
    last_persistence_error: Option<String>,
    restored_requests: usize,
}

impl Default for PersistentServerRequestRegistry {
    fn default() -> Self {
        Self::new(default_state_path())
    }
}

impl PersistentServerRequestRegistry {
    pub fn new(state_path: Option<PathBuf>) -> Self {
        let mut registry = Self {
            inner: ServerRequestRegistry::default(),
            state_path,
            last_persistence_error: None,
            restored_requests: 0,
        };
        if let Some(path) = registry.state_path.clone() {
            match load_requests(&path) {
                Ok(requests) => {
                    registry.restored_requests = requests.len();
                    if let Err(error) = registry.inner.import_state(requests) {
                        registry.last_persistence_error = Some(error.to_string());
                    }
                }
                Err(error) => registry.last_persistence_error = Some(error),
            }
        }
        registry
    }

    pub fn is_response_frame(frame: &Value) -> bool {
        ServerRequestRegistry::is_response_frame(frame)
    }

    pub fn issue(
        &mut self,
        session_id: impl Into<String>,
        method: impl Into<String>,
        params: Value,
    ) -> Result<IssuedServerRequest, ServerRequestError> {
        let issued = self.inner.issue(session_id, method, params)?;
        self.persist();
        Ok(issued)
    }

    pub fn issue_with_questions(
        &mut self,
        session_id: impl Into<String>,
        method: impl Into<String>,
        params: Value,
        question_ids: Vec<String>,
    ) -> Result<IssuedServerRequest, ServerRequestError> {
        let issued = self
            .inner
            .issue_with_questions(session_id, method, params, question_ids)?;
        self.persist();
        Ok(issued)
    }

    pub fn resolve_response(&mut self, frame: &Value) -> Option<ResolvedServerRequest> {
        let resolved = self.inner.resolve_response(frame);
        if resolved.is_some() {
            self.persist();
        }
        resolved
    }

    pub fn lock_answer(
        &mut self,
        request_id: &str,
        question_id: &str,
        answer: impl Into<String>,
    ) -> Result<Option<ClarifyLockOutcome>, ServerRequestError> {
        let outcome = self.inner.lock_answer(request_id, question_id, answer)?;
        if outcome.is_some() {
            self.persist();
        }
        Ok(outcome)
    }

    pub fn cancel_session(&mut self, session_id: Option<&str>, reason: &str) -> Vec<Value> {
        let notifications = self.inner.cancel_session(session_id, reason);
        if !notifications.is_empty() {
            self.persist();
        }
        notifications
    }

    pub fn open_requests(&self, session_id: &str) -> Vec<Value> {
        self.inner.open_requests(session_id)
    }

    pub fn pending_kind(&self, session_id: &str) -> Option<&str> {
        self.inner.pending_kind(session_id)
    }

    pub fn export_state(&self) -> Vec<PendingServerRequest> {
        self.inner.export_state()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn durability_info(&self) -> Value {
        json!({
            "enabled": self.state_path.is_some(),
            "openRequests": self.inner.len(),
            "restoredRequests": self.restored_requests,
            "lastError": self.last_persistence_error,
        })
    }

    fn persist(&mut self) {
        let Some(path) = self.state_path.clone() else {
            return;
        };
        match update_requests(&path, self.inner.export_state()) {
            Ok(()) => self.last_persistence_error = None,
            Err(error) => self.last_persistence_error = Some(error),
        }
    }
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

fn load_requests(path: &Path) -> Result<Vec<PendingServerRequest>, String> {
    let document = read_state_document(path)?;
    let requests = document
        .get("peer")
        .and_then(|peer| peer.get("requests"))
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::from_value(requests).map_err(|error| format!("invalid gateway peer state: {error}"))
}

fn update_requests(path: &Path, requests: Vec<PendingServerRequest>) -> Result<(), String> {
    with_state_lock(path, || {
        let mut document = read_state_document_unlocked(path)?;
        let object = document
            .as_object_mut()
            .ok_or_else(|| "gateway state root must be an object".to_string())?;
        object.insert("schemaVersion".to_string(), Value::from(STATE_SCHEMA_VERSION));
        object.insert("peer".to_string(), json!({"requests": requests}));
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
                return Ok(StateFileLock { path: path.to_path_buf() });
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
            Err(error) => return Err(format!("failed to acquire gateway state lock: {error}")),
        }
    }
    Err("timed out acquiring gateway state lock".to_string())
}

struct StateFileLock { path: PathBuf }

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
    PathBuf::from(format!("{}.tmp-{}-{}", path.display(), std::process::id(), uuid::Uuid::new_v4().simple()))
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
fn set_private_file_permissions(_path: &Path) -> Result<(), String> { Ok(()) }

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to restrict gateway state directory: {error}"))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), String> { Ok(()) }

#[cfg(test)]
mod durability_tests {
    use super::*;

    fn temp_state_path() -> PathBuf {
        std::env::temp_dir().join(format!("mahayana-peer-state-{}.json", uuid::Uuid::new_v4().simple()))
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(backup_path(path));
        let _ = fs::remove_file(PathBuf::from(format!("{}.lock", path.display())));
    }

    #[test]
    fn pending_request_survives_process_style_reopen() {
        let path = temp_state_path();
        {
            let mut registry = PersistentServerRequestRegistry::new(Some(path.clone()));
            registry.issue("session-1", "approval", json!({"requestId":"approval-1"})).unwrap();
            assert_eq!(registry.len(), 1);
        }
        let restored = PersistentServerRequestRegistry::new(Some(path.clone()));
        assert_eq!(restored.len(), 1);
        assert_eq!(restored.open_requests("session-1").len(), 1);
        assert_eq!(restored.restored_requests, 1);
        cleanup(&path);
    }

    #[test]
    fn resolving_request_is_durable_and_preserves_replay_section() {
        let path = temp_state_path();
        ensure_parent(&path).unwrap();
        fs::write(&path, br#"{"schemaVersion":1,"replay":{"events":[]}}"#).unwrap();
        let id;
        {
            let mut registry = PersistentServerRequestRegistry::new(Some(path.clone()));
            id = registry.issue("session-1", "approval", json!({"requestId":"approval-1"})).unwrap().request.id;
            registry.resolve_response(&json!({"jsonrpc":"2.0","id":id,"result":{"decision":"deny"}}));
        }
        let restored = PersistentServerRequestRegistry::new(Some(path.clone()));
        assert!(restored.is_empty());
        let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(document["replay"]["events"].is_array());
        cleanup(&path);
    }
}
