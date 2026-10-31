//! Mahayana-owned long-running session and resilience contracts.
//!
//! This module absorbs the useful ideas behind active/foreign session
//! registries, lifecycle journals, session search, retry budgets, and circuit
//! breakers into a provider-neutral kernel surface.

use crate::SessionId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_MAX_EVENTS_PER_SESSION: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionOrigin {
    Local,
    Imported { source: String },
    Remote { backend_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleState {
    Opening,
    Active,
    Paused,
    Closing,
    Closed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    Registered,
    Activated,
    Paused,
    Resumed,
    Interrupted,
    Compacted,
    Heartbeat,
    Failed,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub sequence: u64,
    pub session_id: SessionId,
    pub kind: SessionEventKind,
    pub occurred_at_ms: i64,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub id: SessionId,
    pub origin: SessionOrigin,
    pub state: SessionLifecycleState,
    pub title: Option<String>,
    pub workspace_root: Option<String>,
    pub model: Option<String>,
    pub tags: BTreeSet<String>,
    pub resumable: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_sequence: u64,
    pub failure: Option<String>,
}

impl SessionDescriptor {
    pub fn new(id: SessionId, origin: SessionOrigin, created_at_ms: i64) -> Self {
        Self {
            id,
            origin,
            state: SessionLifecycleState::Opening,
            title: None,
            workspace_root: None,
            model: None,
            tags: BTreeSet::new(),
            resumable: true,
            created_at_ms,
            updated_at_ms: created_at_ms,
            last_sequence: 0,
            failure: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        let title = title.into();
        if !title.trim().is_empty() {
            self.title = Some(title);
        }
        self
    }

    pub fn with_workspace(mut self, workspace_root: impl Into<String>) -> Self {
        let workspace_root = workspace_root.into();
        if !workspace_root.trim().is_empty() {
            self.workspace_root = Some(workspace_root);
        }
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        let model = model.into();
        if !model.trim().is_empty() {
            self.model = Some(model);
        }
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if !tag.trim().is_empty() {
            self.tags.insert(tag);
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRegistry {
    sessions: BTreeMap<String, SessionDescriptor>,
    events: BTreeMap<String, Vec<SessionEvent>>,
    next_sequence: u64,
    max_events_per_session: usize,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self {
            sessions: BTreeMap::new(),
            events: BTreeMap::new(),
            next_sequence: 1,
            max_events_per_session: DEFAULT_MAX_EVENTS_PER_SESSION,
        }
    }
}

impl SessionRegistry {
    pub fn with_event_retention(max_events_per_session: usize) -> Result<Self, ResilienceError> {
        if max_events_per_session == 0 {
            return Err(ResilienceError::InvalidEventRetention);
        }
        Ok(Self {
            max_events_per_session,
            ..Self::default()
        })
    }

    pub fn register(&mut self, mut descriptor: SessionDescriptor) -> Result<(), ResilienceError> {
        let key = descriptor.id.as_str().to_string();
        if key.trim().is_empty() {
            return Err(ResilienceError::InvalidSessionId);
        }
        if self.sessions.contains_key(&key) {
            return Err(ResilienceError::SessionExists(key));
        }
        let sequence = self.push_event(
            descriptor.id.clone(),
            SessionEventKind::Registered,
            descriptor.created_at_ms,
            None,
        )?;
        descriptor.last_sequence = sequence;
        self.sessions.insert(key, descriptor);
        Ok(())
    }

    pub fn session(&self, id: &SessionId) -> Option<&SessionDescriptor> {
        self.sessions.get(id.as_str())
    }

    pub fn sessions(&self) -> impl Iterator<Item = &SessionDescriptor> {
        self.sessions.values()
    }

    pub fn active_sessions(&self) -> Vec<&SessionDescriptor> {
        self.sessions
            .values()
            .filter(|session| {
                matches!(
                    session.state,
                    SessionLifecycleState::Opening
                        | SessionLifecycleState::Active
                        | SessionLifecycleState::Paused
                        | SessionLifecycleState::Closing
                )
            })
            .collect()
    }

    pub fn resumable_sessions(&self) -> Vec<&SessionDescriptor> {
        self.sessions
            .values()
            .filter(|session| {
                session.resumable
                    && matches!(
                        session.state,
                        SessionLifecycleState::Paused
                            | SessionLifecycleState::Closed
                            | SessionLifecycleState::Failed
                    )
            })
            .collect()
    }

    pub fn events(&self, id: &SessionId) -> &[SessionEvent] {
        self.events
            .get(id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn activate(&mut self, id: &SessionId, at_ms: i64) -> Result<(), ResilienceError> {
        self.transition(
            id,
            SessionLifecycleState::Active,
            SessionEventKind::Activated,
            at_ms,
            None,
        )
    }

    pub fn pause(&mut self, id: &SessionId, at_ms: i64) -> Result<(), ResilienceError> {
        self.transition(
            id,
            SessionLifecycleState::Paused,
            SessionEventKind::Paused,
            at_ms,
            None,
        )
    }

    pub fn resume(&mut self, id: &SessionId, at_ms: i64) -> Result<(), ResilienceError> {
        self.transition(
            id,
            SessionLifecycleState::Active,
            SessionEventKind::Resumed,
            at_ms,
            None,
        )
    }

    pub fn begin_close(&mut self, id: &SessionId, at_ms: i64) -> Result<(), ResilienceError> {
        self.transition(
            id,
            SessionLifecycleState::Closing,
            SessionEventKind::Interrupted,
            at_ms,
            Some("close_requested".into()),
        )
    }

    pub fn close(&mut self, id: &SessionId, at_ms: i64) -> Result<(), ResilienceError> {
        self.transition(
            id,
            SessionLifecycleState::Closed,
            SessionEventKind::Closed,
            at_ms,
            None,
        )
    }

    pub fn fail(
        &mut self,
        id: &SessionId,
        at_ms: i64,
        reason: impl Into<String>,
    ) -> Result<(), ResilienceError> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(ResilienceError::InvalidFailureReason);
        }
        self.transition(
            id,
            SessionLifecycleState::Failed,
            SessionEventKind::Failed,
            at_ms,
            Some(reason.clone()),
        )?;
        self.sessions
            .get_mut(id.as_str())
            .expect("transition verified session exists")
            .failure = Some(reason);
        Ok(())
    }

    pub fn heartbeat(&mut self, id: &SessionId, at_ms: i64) -> Result<(), ResilienceError> {
        let state = self
            .session(id)
            .ok_or_else(|| ResilienceError::SessionNotFound(id.as_str().to_string()))?
            .state;
        if !matches!(
            state,
            SessionLifecycleState::Opening
                | SessionLifecycleState::Active
                | SessionLifecycleState::Paused
                | SessionLifecycleState::Closing
        ) {
            return Err(ResilienceError::InvalidTransition {
                session: id.as_str().to_string(),
                from: state,
                to: state,
            });
        }
        let sequence = self.push_event(id.clone(), SessionEventKind::Heartbeat, at_ms, None)?;
        let session = self
            .sessions
            .get_mut(id.as_str())
            .expect("session was verified above");
        session.updated_at_ms = at_ms;
        session.last_sequence = sequence;
        Ok(())
    }

    pub fn mark_compacted(
        &mut self,
        id: &SessionId,
        at_ms: i64,
        detail: Option<String>,
    ) -> Result<(), ResilienceError> {
        let state = self
            .session(id)
            .ok_or_else(|| ResilienceError::SessionNotFound(id.as_str().to_string()))?
            .state;
        if !matches!(
            state,
            SessionLifecycleState::Active | SessionLifecycleState::Paused
        ) {
            return Err(ResilienceError::InvalidTransition {
                session: id.as_str().to_string(),
                from: state,
                to: state,
            });
        }
        let sequence = self.push_event(id.clone(), SessionEventKind::Compacted, at_ms, detail)?;
        let session = self
            .sessions
            .get_mut(id.as_str())
            .expect("session was verified above");
        session.updated_at_ms = at_ms;
        session.last_sequence = sequence;
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<&SessionDescriptor> {
        if limit == 0 {
            return Vec::new();
        }
        let needle = query.trim().to_ascii_lowercase();
        let mut matches = self
            .sessions
            .values()
            .filter(|session| {
                if needle.is_empty() {
                    return true;
                }
                contains_case_insensitive(session.id.as_str(), &needle)
                    || session
                        .title
                        .as_deref()
                        .is_some_and(|value| contains_case_insensitive(value, &needle))
                    || session
                        .workspace_root
                        .as_deref()
                        .is_some_and(|value| contains_case_insensitive(value, &needle))
                    || session
                        .model
                        .as_deref()
                        .is_some_and(|value| contains_case_insensitive(value, &needle))
                    || session
                        .tags
                        .iter()
                        .any(|tag| contains_case_insensitive(tag, &needle))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.id.as_str().cmp(right.id.as_str()))
        });
        matches.truncate(limit);
        matches
    }

    fn transition(
        &mut self,
        id: &SessionId,
        next: SessionLifecycleState,
        event: SessionEventKind,
        at_ms: i64,
        detail: Option<String>,
    ) -> Result<(), ResilienceError> {
        let current = self
            .sessions
            .get(id.as_str())
            .ok_or_else(|| ResilienceError::SessionNotFound(id.as_str().to_string()))?
            .state;
        if !transition_allowed(current, next) {
            return Err(ResilienceError::InvalidTransition {
                session: id.as_str().to_string(),
                from: current,
                to: next,
            });
        }
        let sequence = self.push_event(id.clone(), event, at_ms, detail)?;
        let session = self
            .sessions
            .get_mut(id.as_str())
            .expect("session was verified above");
        session.state = next;
        session.updated_at_ms = at_ms;
        session.last_sequence = sequence;
        if next != SessionLifecycleState::Failed {
            session.failure = None;
        }
        Ok(())
    }

    fn push_event(
        &mut self,
        session_id: SessionId,
        kind: SessionEventKind,
        occurred_at_ms: i64,
        detail: Option<String>,
    ) -> Result<u64, ResilienceError> {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(ResilienceError::SequenceExhausted)?;
        let events = self
            .events
            .entry(session_id.as_str().to_string())
            .or_default();
        events.push(SessionEvent {
            sequence,
            session_id,
            kind,
            occurred_at_ms,
            detail,
        });
        if events.len() > self.max_events_per_session {
            let excess = events.len() - self.max_events_per_session;
            events.drain(0..excess);
        }
        Ok(sequence)
    }
}

fn transition_allowed(from: SessionLifecycleState, to: SessionLifecycleState) -> bool {
    use SessionLifecycleState::{Active, Closed, Closing, Failed, Opening, Paused};
    matches!(
        (from, to),
        (Opening, Active)
            | (Opening, Failed)
            | (Opening, Closed)
            | (Active, Paused)
            | (Active, Closing)
            | (Active, Failed)
            | (Paused, Active)
            | (Paused, Closing)
            | (Paused, Failed)
            | (Closing, Closed)
            | (Closing, Failed)
            | (Failed, Active)
            | (Closed, Active)
    )
}

fn contains_case_insensitive(value: &str, lower_needle: &str) -> bool {
    value.to_ascii_lowercase().contains(lower_needle)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CircuitPolicy {
    pub failure_threshold: u32,
    pub open_duration_ms: i64,
    pub half_open_max_requests: u32,
}

impl CircuitPolicy {
    pub fn new(
        failure_threshold: u32,
        open_duration_ms: i64,
        half_open_max_requests: u32,
    ) -> Result<Self, ResilienceError> {
        if failure_threshold == 0 || open_duration_ms <= 0 || half_open_max_requests == 0 {
            return Err(ResilienceError::InvalidCircuitPolicy);
        }
        Ok(Self {
            failure_threshold,
            open_duration_ms,
            half_open_max_requests,
        })
    }
}

impl Default for CircuitPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            open_duration_ms: 30_000,
            half_open_max_requests: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum CircuitDecision {
    Allow,
    Reject { retry_at_ms: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreaker {
    policy: CircuitPolicy,
    state: CircuitState,
    consecutive_failures: u32,
    opened_at_ms: Option<i64>,
    half_open_in_flight: u32,
}

impl CircuitBreaker {
    pub fn new(policy: CircuitPolicy) -> Self {
        Self {
            policy,
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at_ms: None,
            half_open_in_flight: 0,
        }
    }

    pub fn state(&self) -> CircuitState {
        self.state
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn acquire(&mut self, now_ms: i64) -> CircuitDecision {
        match self.state {
            CircuitState::Closed => CircuitDecision::Allow,
            CircuitState::Open => {
                let opened_at = self.opened_at_ms.unwrap_or(now_ms);
                let retry_at = opened_at.saturating_add(self.policy.open_duration_ms);
                if now_ms < retry_at {
                    CircuitDecision::Reject {
                        retry_at_ms: retry_at,
                    }
                } else {
                    self.state = CircuitState::HalfOpen;
                    self.half_open_in_flight = 1;
                    CircuitDecision::Allow
                }
            }
            CircuitState::HalfOpen => {
                if self.half_open_in_flight >= self.policy.half_open_max_requests {
                    CircuitDecision::Reject {
                        retry_at_ms: now_ms.saturating_add(1),
                    }
                } else {
                    self.half_open_in_flight = self.half_open_in_flight.saturating_add(1);
                    CircuitDecision::Allow
                }
            }
        }
    }

    pub fn record_success(&mut self) {
        self.state = CircuitState::Closed;
        self.consecutive_failures = 0;
        self.opened_at_ms = None;
        self.half_open_in_flight = 0;
    }

    pub fn record_failure(&mut self, now_ms: i64) {
        if self.state == CircuitState::HalfOpen {
            self.open(now_ms);
            return;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= self.policy.failure_threshold {
            self.open(now_ms);
        }
    }

    fn open(&mut self, now_ms: i64) {
        self.state = CircuitState::Open;
        self.opened_at_ms = Some(now_ms);
        self.half_open_in_flight = 0;
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(CircuitPolicy::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl RetryPolicy {
    pub fn new(
        max_attempts: u32,
        base_delay_ms: u64,
        max_delay_ms: u64,
    ) -> Result<Self, ResilienceError> {
        if max_attempts == 0 || base_delay_ms == 0 || max_delay_ms < base_delay_ms {
            return Err(ResilienceError::InvalidRetryPolicy);
        }
        Ok(Self {
            max_attempts,
            base_delay_ms,
            max_delay_ms,
        })
    }

    pub fn can_retry(&self, completed_attempts: u32) -> bool {
        completed_attempts < self.max_attempts
    }

    pub fn delay_for_attempt(&self, completed_attempts: u32) -> u64 {
        if completed_attempts == 0 {
            return 0;
        }
        let shift = completed_attempts.saturating_sub(1).min(63);
        self.base_delay_ms
            .saturating_mul(1_u64 << shift)
            .min(self.max_delay_ms)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay_ms: 250,
            max_delay_ms: 8_000,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResilienceError {
    #[error("session id is empty")]
    InvalidSessionId,
    #[error("session already exists: {0}")]
    SessionExists(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("invalid session transition for {session}: {from:?} -> {to:?}")]
    InvalidTransition {
        session: String,
        from: SessionLifecycleState,
        to: SessionLifecycleState,
    },
    #[error("failure reason must not be empty")]
    InvalidFailureReason,
    #[error("event retention must be greater than zero")]
    InvalidEventRetention,
    #[error("session event sequence exhausted")]
    SequenceExhausted,
    #[error("invalid circuit breaker policy")]
    InvalidCircuitPolicy,
    #[error("invalid retry policy")]
    InvalidRetryPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_registry_preserves_local_imported_and_remote_sessions() {
        let mut registry = SessionRegistry::default();
        let local = SessionId::from_string("local-1");
        let imported = SessionId::from_string("imported-1");
        let remote = SessionId::from_string("remote-1");
        registry
            .register(
                SessionDescriptor::new(local.clone(), SessionOrigin::Local, 1)
                    .with_title("Mahayana kernel")
                    .with_workspace("/repo/fabushi")
                    .with_tag("kernel"),
            )
            .unwrap();
        registry
            .register(SessionDescriptor::new(
                imported.clone(),
                SessionOrigin::Imported {
                    source: "archive".into(),
                },
                2,
            ))
            .unwrap();
        registry
            .register(SessionDescriptor::new(
                remote.clone(),
                SessionOrigin::Remote {
                    backend_id: "mahayana-cloud".into(),
                },
                3,
            ))
            .unwrap();
        registry.activate(&local, 4).unwrap();
        registry.activate(&imported, 5).unwrap();
        registry.activate(&remote, 6).unwrap();
        assert_eq!(registry.active_sessions().len(), 3);
        assert_eq!(registry.search("fabushi", 10).len(), 1);
        assert_eq!(registry.search("kernel", 10).len(), 1);
    }

    #[test]
    fn pause_resume_close_and_reopen_are_explicit() {
        let mut registry = SessionRegistry::default();
        let id = SessionId::from_string("session-a");
        registry
            .register(SessionDescriptor::new(id.clone(), SessionOrigin::Local, 1))
            .unwrap();
        registry.activate(&id, 2).unwrap();
        registry.pause(&id, 3).unwrap();
        assert_eq!(
            registry.session(&id).unwrap().state,
            SessionLifecycleState::Paused
        );
        registry.resume(&id, 4).unwrap();
        registry.begin_close(&id, 5).unwrap();
        registry.close(&id, 6).unwrap();
        assert_eq!(registry.resumable_sessions().len(), 1);
        registry.activate(&id, 7).unwrap();
        assert_eq!(
            registry.session(&id).unwrap().state,
            SessionLifecycleState::Active
        );
    }

    #[test]
    fn invalid_lifecycle_transition_fails_closed() {
        let mut registry = SessionRegistry::default();
        let id = SessionId::from_string("session-b");
        registry
            .register(SessionDescriptor::new(id.clone(), SessionOrigin::Local, 1))
            .unwrap();
        assert!(matches!(
            registry.pause(&id, 2),
            Err(ResilienceError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn event_retention_is_bounded() {
        let mut registry = SessionRegistry::with_event_retention(3).unwrap();
        let id = SessionId::from_string("session-c");
        registry
            .register(SessionDescriptor::new(id.clone(), SessionOrigin::Local, 1))
            .unwrap();
        registry.activate(&id, 2).unwrap();
        registry.heartbeat(&id, 3).unwrap();
        registry.heartbeat(&id, 4).unwrap();
        assert_eq!(registry.events(&id).len(), 3);
        assert_eq!(registry.events(&id)[0].kind, SessionEventKind::Activated);
    }

    #[test]
    fn circuit_breaker_opens_rejects_probes_and_recovers() {
        let policy = CircuitPolicy::new(2, 100, 1).unwrap();
        let mut breaker = CircuitBreaker::new(policy);
        assert_eq!(breaker.acquire(0), CircuitDecision::Allow);
        breaker.record_failure(1);
        assert_eq!(breaker.acquire(2), CircuitDecision::Allow);
        breaker.record_failure(3);
        assert_eq!(breaker.state(), CircuitState::Open);
        assert_eq!(
            breaker.acquire(50),
            CircuitDecision::Reject { retry_at_ms: 103 }
        );
        assert_eq!(breaker.acquire(103), CircuitDecision::Allow);
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        assert_eq!(
            breaker.acquire(103),
            CircuitDecision::Reject { retry_at_ms: 104 }
        );
        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.consecutive_failures(), 0);
    }

    #[test]
    fn half_open_failure_reopens_circuit() {
        let policy = CircuitPolicy::new(1, 10, 1).unwrap();
        let mut breaker = CircuitBreaker::new(policy);
        breaker.record_failure(1);
        assert_eq!(breaker.acquire(11), CircuitDecision::Allow);
        breaker.record_failure(12);
        assert_eq!(breaker.state(), CircuitState::Open);
        assert_eq!(
            breaker.acquire(15),
            CircuitDecision::Reject { retry_at_ms: 22 }
        );
    }

    #[test]
    fn retry_policy_is_bounded_and_exponential() {
        let policy = RetryPolicy::new(5, 100, 350).unwrap();
        assert_eq!(policy.delay_for_attempt(0), 0);
        assert_eq!(policy.delay_for_attempt(1), 100);
        assert_eq!(policy.delay_for_attempt(2), 200);
        assert_eq!(policy.delay_for_attempt(3), 350);
        assert!(policy.can_retry(4));
        assert!(!policy.can_retry(5));
    }
}
