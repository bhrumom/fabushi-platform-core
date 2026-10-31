//! Bidirectional JSON-RPC state shared by Mahayana gateway transports.
//!
//! `mahayana-gateway` owns client -> server methods and ordered runtime events.
//! This crate owns the opposite direction: a Rust runtime can ask an attached
//! surface a question, keep the request open by id, accept a JSON-RPC response,
//! cancel it, or replay the still-open request after a transport reconnect.
//! No renderer is the authority for whether a request is still pending.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

pub const JSON_RPC_VERSION: &str = "2.0";
pub const REQUEST_CANCEL_METHOD: &str = "request.cancel";

/// Mahayana-owned server-request vocabulary. The names intentionally follow
/// the current Hermes peer-RPC semantics, while payloads remain product-owned.
pub const SERVER_REQUEST_METHODS: &[&str] = &[
    "clarify",
    "approval",
    "sudo",
    "secret",
    "vault.unlock_prompt",
    "vault.save_login",
    "vault.code",
    "terminal.read",
    "preview.read",
    "window.read",
    "preview.act",
    "tour",
];

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ServerRequestError {
    #[error("unsupported Mahayana server-request method: {0}")]
    UnsupportedMethod(String),
    #[error("server-request id must use the srq- prefix")]
    InvalidRequestId,
    #[error("server-request params must be a JSON object")]
    ParamsMustBeObject,
    #[error("unknown clarify question id: {0}")]
    UnknownQuestion(String),
    #[error("invalid persisted server-request state: {0}")]
    InvalidPersistedState(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingServerRequest {
    pub id: String,
    pub session_id: String,
    pub method: String,
    pub params: Value,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub question_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub locked_answers: BTreeMap<String, String>,
}

impl PendingServerRequest {
    pub fn frame(&self) -> Value {
        let mut params = object_or_empty(&self.params);
        params.insert(
            "sessionId".to_string(),
            Value::String(self.session_id.clone()),
        );
        json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": self.id,
            "method": self.method,
            "params": params,
        })
    }

    /// Reconnect snapshot. Locked clarify answers are carried forward so a
    /// renderer can restore which questions are already immutable.
    pub fn reconnect_frame(&self) -> Value {
        let mut frame = self.frame();
        if !self.locked_answers.is_empty()
            && let Some(params) = frame.get_mut("params").and_then(Value::as_object_mut)
        {
            params.insert(
                "answers".to_string(),
                serde_json::to_value(&self.locked_answers).unwrap_or(Value::Null),
            );
        }
        frame
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerRequestResponse {
    Result(Value),
    Error(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedServerRequest {
    pub request: PendingServerRequest,
    pub response: ServerRequestResponse,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IssuedServerRequest {
    pub request: PendingServerRequest,
    pub frame: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClarifyLockOutcome {
    pub remaining_question_ids: Vec<String>,
    pub resolution: Option<ResolvedServerRequest>,
}

#[derive(Debug, Clone, Default)]
pub struct ServerRequestRegistry {
    open: HashMap<String, PendingServerRequest>,
    order: VecDeque<String>,
}

impl ServerRequestRegistry {
    pub fn issue(
        &mut self,
        session_id: impl Into<String>,
        method: impl Into<String>,
        params: Value,
    ) -> Result<IssuedServerRequest, ServerRequestError> {
        self.issue_with_questions(session_id, method, params, Vec::new())
    }

    pub fn issue_with_questions(
        &mut self,
        session_id: impl Into<String>,
        method: impl Into<String>,
        params: Value,
        question_ids: Vec<String>,
    ) -> Result<IssuedServerRequest, ServerRequestError> {
        let id = format!("srq-{}", &Uuid::new_v4().simple().to_string()[..12]);
        self.issue_with_id(
            id,
            session_id.into(),
            method.into(),
            params,
            question_ids,
            now_ms(),
        )
    }

    /// Deterministic constructor used by durable replay/import and tests.
    pub fn issue_with_id(
        &mut self,
        id: String,
        session_id: String,
        method: String,
        params: Value,
        question_ids: Vec<String>,
        created_at_ms: u64,
    ) -> Result<IssuedServerRequest, ServerRequestError> {
        validate_method(&method)?;
        validate_id(&id)?;
        if !params.is_object() && !params.is_null() {
            return Err(ServerRequestError::ParamsMustBeObject);
        }

        let request = PendingServerRequest {
            id: id.clone(),
            session_id,
            method,
            params,
            created_at_ms,
            question_ids,
            locked_answers: BTreeMap::new(),
        };
        let frame = request.frame();
        self.open.insert(id.clone(), request.clone());
        self.order.retain(|existing| existing != &id);
        self.order.push_back(id);
        Ok(IssuedServerRequest { request, frame })
    }

    pub fn is_response_frame(frame: &Value) -> bool {
        frame.get("method").is_none()
            && frame.get("id").is_some()
            && (frame.get("result").is_some() || frame.get("error").is_some())
    }

    /// Resolves one client response. Unknown/expired ids are ignored so a late
    /// renderer response can never resurrect a request the Rust owner closed.
    pub fn resolve_response(&mut self, frame: &Value) -> Option<ResolvedServerRequest> {
        if !Self::is_response_frame(frame) {
            return None;
        }
        let id = frame.get("id")?.as_str()?;
        let request = self.open.remove(id)?;
        self.order.retain(|existing| existing != id);

        let response = if let Some(error) = frame.get("error") {
            ServerRequestResponse::Error(error.clone())
        } else {
            let mut result = frame
                .get("result")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| Value::Object(Map::new()));
            merge_locked_answers(&request, &mut result);
            ServerRequestResponse::Result(result)
        };

        Some(ResolvedServerRequest { request, response })
    }

    /// Locks one batch-clarify answer. The last lock resolves the request with
    /// the full answer set, matching the same Rust-owned lifecycle as a normal
    /// JSON-RPC response.
    pub fn lock_answer(
        &mut self,
        request_id: &str,
        question_id: &str,
        answer: impl Into<String>,
    ) -> Result<Option<ClarifyLockOutcome>, ServerRequestError> {
        let Some(request) = self.open.get_mut(request_id) else {
            return Ok(None);
        };
        if !request
            .question_ids
            .iter()
            .any(|candidate| candidate == question_id)
        {
            return Err(ServerRequestError::UnknownQuestion(question_id.to_string()));
        }
        request
            .locked_answers
            .insert(question_id.to_string(), answer.into());
        let remaining = request
            .question_ids
            .iter()
            .filter(|id| !request.locked_answers.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();

        if !remaining.is_empty() {
            return Ok(Some(ClarifyLockOutcome {
                remaining_question_ids: remaining,
                resolution: None,
            }));
        }

        let request = self.open.remove(request_id).expect("request exists");
        self.order.retain(|existing| existing != request_id);
        let result = json!({"answers": request.locked_answers});
        Ok(Some(ClarifyLockOutcome {
            remaining_question_ids: Vec::new(),
            resolution: Some(ResolvedServerRequest {
                request,
                response: ServerRequestResponse::Result(result),
            }),
        }))
    }

    /// Withdraws matching open requests and returns the peer notifications a
    /// transport must send to tear down renderer cards/locks.
    pub fn cancel_session(&mut self, session_id: Option<&str>, reason: &str) -> Vec<Value> {
        let ids = self
            .order
            .iter()
            .filter_map(|id| {
                self.open.get(id).and_then(|request| {
                    if session_id.is_none_or(|session_id| request.session_id == session_id) {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();

        let mut notifications = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(request) = self.open.remove(&id) {
                notifications.push(cancel_notification(&request, reason));
            }
        }
        self.order.retain(|id| self.open.contains_key(id));
        notifications
    }

    pub fn open_requests(&self, session_id: &str) -> Vec<Value> {
        self.order
            .iter()
            .filter_map(|id| self.open.get(id))
            .filter(|request| request.session_id == session_id)
            .map(PendingServerRequest::reconnect_frame)
            .collect()
    }

    pub fn pending_kind(&self, session_id: &str) -> Option<&str> {
        self.order
            .iter()
            .filter_map(|id| self.open.get(id))
            .find(|request| request.session_id == session_id)
            .map(|request| request.method.as_str())
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// Export only Rust-owned state. Transport clients never become the source
    /// of truth for pending requests; a canonical store can persist this value
    /// and restore it after process restart.
    pub fn export_state(&self) -> Vec<PendingServerRequest> {
        self.order
            .iter()
            .filter_map(|id| self.open.get(id))
            .cloned()
            .collect()
    }

    pub fn import_state(
        &mut self,
        requests: impl IntoIterator<Item = PendingServerRequest>,
    ) -> Result<(), ServerRequestError> {
        self.open.clear();
        self.order.clear();
        let mut requests = requests.into_iter().collect::<Vec<_>>();
        requests.sort_by_key(|request| request.created_at_ms);
        for request in requests {
            validate_method(&request.method)?;
            validate_id(&request.id)?;
            if !request.params.is_object() && !request.params.is_null() {
                return Err(ServerRequestError::InvalidPersistedState(format!(
                    "{} params are not an object",
                    request.id
                )));
            }
            let id = request.id.clone();
            self.open.insert(id.clone(), request);
            self.order.push_back(id);
        }
        Ok(())
    }
}

pub fn cancel_notification(request: &PendingServerRequest, reason: &str) -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "method": REQUEST_CANCEL_METHOD,
        "params": {
            "id": request.id,
            "method": request.method,
            "reason": reason,
            "sessionId": request.session_id,
        }
    })
}

fn merge_locked_answers(request: &PendingServerRequest, result: &mut Value) {
    if request.locked_answers.is_empty() {
        return;
    }
    let Some(object) = result.as_object_mut() else {
        return;
    };
    let mut answers = BTreeMap::new();
    if let Some(client_answers) = object.get("answers").and_then(Value::as_object) {
        for (question_id, answer) in client_answers {
            if let Some(answer) = answer.as_str() {
                answers.insert(question_id.clone(), answer.to_string());
            }
        }
    }
    // The Rust request registry is authoritative across reconnects. A stale
    // renderer may fill an unlocked answer, but it cannot replace a sealed one.
    answers.extend(request.locked_answers.clone());
    object.insert(
        "answers".to_string(),
        serde_json::to_value(answers).unwrap_or(Value::Null),
    );
}

fn object_or_empty(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn validate_method(method: &str) -> Result<(), ServerRequestError> {
    if SERVER_REQUEST_METHODS.contains(&method) {
        Ok(())
    } else {
        Err(ServerRequestError::UnsupportedMethod(method.to_string()))
    }
}

fn validate_id(id: &str) -> Result<(), ServerRequestError> {
    if id.starts_with("srq-") && id.len() > 4 {
        Ok(())
    } else {
        Err(ServerRequestError::InvalidRequestId)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_approval(registry: &mut ServerRequestRegistry) -> IssuedServerRequest {
        registry
            .issue_with_id(
                "srq-123456789abc".to_string(),
                "session-1".to_string(),
                "approval".to_string(),
                json!({"requestId": "approval-1", "title": "Run shell"}),
                Vec::new(),
                10,
            )
            .unwrap()
    }

    #[test]
    fn server_request_uses_peer_json_rpc_and_stable_request_id() {
        let mut registry = ServerRequestRegistry::default();
        let issued = issue_approval(&mut registry);
        assert_eq!(issued.frame["jsonrpc"], "2.0");
        assert_eq!(issued.frame["id"], "srq-123456789abc");
        assert_eq!(issued.frame["method"], "approval");
        assert_eq!(issued.frame["params"]["sessionId"], "session-1");
        assert_eq!(registry.pending_kind("session-1"), Some("approval"));
    }

    #[test]
    fn response_resolves_only_the_matching_open_request() {
        let mut registry = ServerRequestRegistry::default();
        issue_approval(&mut registry);
        assert!(
            registry
                .resolve_response(&json!({"jsonrpc": "2.0", "id": "srq-missing", "result": {}}))
                .is_none()
        );
        let resolved = registry
            .resolve_response(&json!({
                "jsonrpc": "2.0",
                "id": "srq-123456789abc",
                "result": {"decision": "allow-once"}
            }))
            .unwrap();
        assert_eq!(resolved.request.method, "approval");
        assert_eq!(
            resolved.response,
            ServerRequestResponse::Result(json!({"decision": "allow-once"}))
        );
        assert!(registry.is_empty());
    }

    #[test]
    fn cancel_emits_one_shared_request_cancel_notification() {
        let mut registry = ServerRequestRegistry::default();
        issue_approval(&mut registry);
        let cancelled = registry.cancel_session(Some("session-1"), "interrupted");
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0]["method"], REQUEST_CANCEL_METHOD);
        assert_eq!(cancelled[0]["params"]["id"], "srq-123456789abc");
        assert_eq!(cancelled[0]["params"]["reason"], "interrupted");
        assert!(registry.is_empty());
    }

    #[test]
    fn reconnect_replays_unanswered_request_and_locked_clarify_answers() {
        let mut registry = ServerRequestRegistry::default();
        registry
            .issue_with_id(
                "srq-clarify0001".to_string(),
                "session-2".to_string(),
                "clarify".to_string(),
                json!({"questions": [{"id": "q1"}, {"id": "q2"}]}),
                vec!["q1".to_string(), "q2".to_string()],
                20,
            )
            .unwrap();
        let lock = registry
            .lock_answer("srq-clarify0001", "q1", "yes")
            .unwrap()
            .unwrap();
        assert_eq!(lock.remaining_question_ids, vec!["q2"]);
        assert!(lock.resolution.is_none());

        let replay = registry.open_requests("session-2");
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0]["id"], "srq-clarify0001");
        assert_eq!(replay[0]["params"]["answers"]["q1"], "yes");
    }

    #[test]
    fn last_clarify_lock_resolves_with_full_answer_set() {
        let mut registry = ServerRequestRegistry::default();
        registry
            .issue_with_id(
                "srq-clarify0002".to_string(),
                "session-2".to_string(),
                "clarify".to_string(),
                json!({}),
                vec!["q1".to_string(), "q2".to_string()],
                20,
            )
            .unwrap();
        registry
            .lock_answer("srq-clarify0002", "q1", "alpha")
            .unwrap();
        let last = registry
            .lock_answer("srq-clarify0002", "q2", "beta")
            .unwrap()
            .unwrap();
        let resolution = last.resolution.unwrap();
        assert_eq!(
            resolution.response,
            ServerRequestResponse::Result(json!({"answers": {"q1": "alpha", "q2": "beta"}}))
        );
        assert!(registry.is_empty());
    }

    #[test]
    fn locked_clarify_answer_wins_over_stale_client_response() {
        let mut registry = ServerRequestRegistry::default();
        registry
            .issue_with_id(
                "srq-clarify0003".to_string(),
                "session-3".to_string(),
                "clarify".to_string(),
                json!({}),
                vec!["q1".to_string(), "q2".to_string()],
                30,
            )
            .unwrap();
        registry
            .lock_answer("srq-clarify0003", "q1", "server-locked")
            .unwrap();
        let resolved = registry
            .resolve_response(&json!({
                "jsonrpc": "2.0",
                "id": "srq-clarify0003",
                "result": {"answers": {"q1": "stale-client", "q2": "client-open"}}
            }))
            .unwrap();
        assert_eq!(
            resolved.response,
            ServerRequestResponse::Result(json!({
                "answers": {"q1": "server-locked", "q2": "client-open"}
            }))
        );
    }

    #[test]
    fn exported_state_round_trips_for_future_canonical_restart_store() {
        let mut registry = ServerRequestRegistry::default();
        issue_approval(&mut registry);
        let state = registry.export_state();

        let mut restored = ServerRequestRegistry::default();
        restored.import_state(state).unwrap();
        assert_eq!(restored.open_requests("session-1").len(), 1);
        assert_eq!(restored.pending_kind("session-1"), Some("approval"));
    }
}
