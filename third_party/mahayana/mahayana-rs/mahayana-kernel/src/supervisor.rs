//! Mahayana-owned supervision primitives for long-running agent work.
//!
//! These contracts are provider-neutral. They deliberately model the useful
//! reliability ideas behind modern coding agents without exposing Codex or
//! Grok Build implementation types in Mahayana's public kernel.

use crate::{ApprovalMode, Capability, ExecutionPolicy, RiskLevel};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Running,
    Paused,
    Verifying,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    BestEffort,
    ObjectiveRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleStatus {
    Pending,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationOracle {
    pub name: String,
    pub required: bool,
    pub status: OracleStatus,
    pub evidence: Option<String>,
}

impl VerificationOracle {
    pub fn required(name: impl Into<String>) -> Result<Self, SupervisorError> {
        Self::new(name, true)
    }

    pub fn advisory(name: impl Into<String>) -> Result<Self, SupervisorError> {
        Self::new(name, false)
    }

    fn new(name: impl Into<String>, required: bool) -> Result<Self, SupervisorError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(SupervisorError::InvalidOracle(
                "oracle name is empty".into(),
            ));
        }
        Ok(Self {
            name,
            required,
            status: OracleStatus::Pending,
            evidence: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopDisposition {
    Allow,
    Warn,
    Interrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPolicy {
    pub warn_after: u32,
    pub interrupt_after: u32,
}

impl LoopPolicy {
    pub fn new(warn_after: u32, interrupt_after: u32) -> Result<Self, SupervisorError> {
        if warn_after < 2 || interrupt_after <= warn_after {
            return Err(SupervisorError::InvalidLoopPolicy);
        }
        Ok(Self {
            warn_after,
            interrupt_after,
        })
    }
}

impl Default for LoopPolicy {
    fn default() -> Self {
        Self {
            warn_after: 2,
            interrupt_after: 4,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopState {
    last_fingerprint: Option<String>,
    consecutive_occurrences: u32,
}

impl LoopState {
    pub fn observe(&mut self, fingerprint: &str, policy: LoopPolicy) -> LoopDisposition {
        if self.last_fingerprint.as_deref() == Some(fingerprint) {
            self.consecutive_occurrences = self.consecutive_occurrences.saturating_add(1);
        } else {
            self.last_fingerprint = Some(fingerprint.to_owned());
            self.consecutive_occurrences = 1;
        }
        if self.consecutive_occurrences >= policy.interrupt_after {
            LoopDisposition::Interrupt
        } else if self.consecutive_occurrences >= policy.warn_after {
            LoopDisposition::Warn
        } else {
            LoopDisposition::Allow
        }
    }

    pub fn reset(&mut self) {
        self.last_fingerprint = None;
        self.consecutive_occurrences = 0;
    }

    pub fn consecutive_occurrences(&self) -> u32 {
        self.consecutive_occurrences
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub id: String,
    pub fingerprint: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub succeeded: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub goal: String,
    pub dependencies: BTreeSet<String>,
    pub state: TaskState,
    pub verification_mode: VerificationMode,
    pub oracles: BTreeMap<String, VerificationOracle>,
    pub attempts: Vec<AttemptRecord>,
    pub loop_policy: LoopPolicy,
    pub loop_state: LoopState,
    pub failure: Option<String>,
}

impl TaskRecord {
    pub fn new(
        id: impl Into<String>,
        goal: impl Into<String>,
        dependencies: impl IntoIterator<Item = String>,
        verification_mode: VerificationMode,
    ) -> Result<Self, SupervisorError> {
        let id = id.into();
        let goal = goal.into();
        if id.trim().is_empty() {
            return Err(SupervisorError::InvalidTask("task id is empty".into()));
        }
        if goal.trim().is_empty() {
            return Err(SupervisorError::InvalidTask("task goal is empty".into()));
        }
        let dependencies = dependencies.into_iter().collect::<BTreeSet<_>>();
        if dependencies.contains(&id) {
            return Err(SupervisorError::InvalidTask(
                "task cannot depend on itself".into(),
            ));
        }
        Ok(Self {
            id,
            goal,
            dependencies,
            state: TaskState::Pending,
            verification_mode,
            oracles: BTreeMap::new(),
            attempts: Vec::new(),
            loop_policy: LoopPolicy::default(),
            loop_state: LoopState::default(),
            failure: None,
        })
    }

    pub fn add_oracle(&mut self, oracle: VerificationOracle) -> Result<(), SupervisorError> {
        if self.oracles.contains_key(&oracle.name) {
            return Err(SupervisorError::OracleExists(oracle.name));
        }
        self.oracles.insert(oracle.name.clone(), oracle);
        Ok(())
    }

    pub fn record_oracle(
        &mut self,
        name: &str,
        status: OracleStatus,
        evidence: Option<String>,
    ) -> Result<(), SupervisorError> {
        let oracle = self
            .oracles
            .get_mut(name)
            .ok_or_else(|| SupervisorError::OracleNotFound(name.to_owned()))?;
        oracle.status = status;
        oracle.evidence = evidence;
        Ok(())
    }

    fn verify_completion(&self) -> Result<(), SupervisorError> {
        if self.verification_mode == VerificationMode::BestEffort {
            return Ok(());
        }
        if self.oracles.values().all(|oracle| !oracle.required) {
            return Err(SupervisorError::VerificationBlocked(
                "objective verification requires at least one required oracle".into(),
            ));
        }
        for oracle in self.oracles.values().filter(|oracle| oracle.required) {
            match oracle.status {
                OracleStatus::Passed => {}
                OracleStatus::Pending => {
                    return Err(SupervisorError::VerificationBlocked(format!(
                        "required oracle is pending: {}",
                        oracle.name
                    )));
                }
                OracleStatus::Failed => {
                    return Err(SupervisorError::VerificationFailed(format!(
                        "required oracle failed: {}",
                        oracle.name
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskSupervisor {
    tasks: BTreeMap<String, TaskRecord>,
}

impl TaskSupervisor {
    pub fn add_task(&mut self, task: TaskRecord) -> Result<(), SupervisorError> {
        if self.tasks.contains_key(&task.id) {
            return Err(SupervisorError::TaskExists(task.id));
        }
        for dependency in &task.dependencies {
            if !self.tasks.contains_key(dependency) {
                return Err(SupervisorError::DependencyNotFound(dependency.clone()));
            }
        }
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    pub fn task(&self, id: &str) -> Option<&TaskRecord> {
        self.tasks.get(id)
    }

    pub fn task_mut(&mut self, id: &str) -> Result<&mut TaskRecord, SupervisorError> {
        self.tasks
            .get_mut(id)
            .ok_or_else(|| SupervisorError::TaskNotFound(id.to_owned()))
    }

    pub fn ready_task_ids(&self) -> Vec<String> {
        self.tasks
            .values()
            .filter(|task| task.state == TaskState::Pending)
            .filter(|task| {
                task.dependencies.iter().all(|dependency| {
                    self.tasks
                        .get(dependency)
                        .is_some_and(|task| task.state == TaskState::Succeeded)
                })
            })
            .map(|task| task.id.clone())
            .collect()
    }

    pub fn start_task(&mut self, id: &str) -> Result<(), SupervisorError> {
        if !self.ready_task_ids().iter().any(|ready| ready == id) {
            return Err(SupervisorError::TaskNotReady(id.to_owned()));
        }
        self.task_mut(id)?.state = TaskState::Running;
        Ok(())
    }

    pub fn pause_task(&mut self, id: &str) -> Result<(), SupervisorError> {
        self.transition(id, TaskState::Running, TaskState::Paused)
    }

    pub fn resume_task(&mut self, id: &str) -> Result<(), SupervisorError> {
        self.transition(id, TaskState::Paused, TaskState::Running)
    }

    fn transition(
        &mut self,
        id: &str,
        expected: TaskState,
        next: TaskState,
    ) -> Result<(), SupervisorError> {
        let task = self.task_mut(id)?;
        if task.state != expected {
            return Err(SupervisorError::InvalidTransition {
                task: id.to_owned(),
                from: task.state,
                to: next,
            });
        }
        task.state = next;
        Ok(())
    }

    pub fn begin_attempt(
        &mut self,
        id: &str,
        fingerprint: impl Into<String>,
        started_at_ms: i64,
    ) -> Result<(String, LoopDisposition), SupervisorError> {
        let task = self.task_mut(id)?;
        if task.state != TaskState::Running {
            return Err(SupervisorError::InvalidTransition {
                task: id.to_owned(),
                from: task.state,
                to: TaskState::Running,
            });
        }
        let fingerprint = fingerprint.into();
        if fingerprint.trim().is_empty() {
            return Err(SupervisorError::InvalidAttempt(
                "attempt fingerprint is empty".into(),
            ));
        }
        let disposition = task.loop_state.observe(&fingerprint, task.loop_policy);
        let attempt_id = format!("attempt:{}:{}", task.id, task.attempts.len() + 1);
        task.attempts.push(AttemptRecord {
            id: attempt_id.clone(),
            fingerprint,
            started_at_ms,
            finished_at_ms: None,
            succeeded: None,
        });
        Ok((attempt_id, disposition))
    }

    pub fn finish_attempt(
        &mut self,
        task_id: &str,
        attempt_id: &str,
        finished_at_ms: i64,
        succeeded: bool,
    ) -> Result<(), SupervisorError> {
        let task = self.task_mut(task_id)?;
        let attempt = task
            .attempts
            .iter_mut()
            .find(|attempt| attempt.id == attempt_id)
            .ok_or_else(|| SupervisorError::AttemptNotFound(attempt_id.to_owned()))?;
        if attempt.finished_at_ms.is_some() {
            return Err(SupervisorError::InvalidAttempt(format!(
                "attempt already finished: {attempt_id}"
            )));
        }
        attempt.finished_at_ms = Some(finished_at_ms);
        attempt.succeeded = Some(succeeded);
        if succeeded {
            task.loop_state.reset();
        }
        Ok(())
    }

    pub fn begin_verification(&mut self, id: &str) -> Result<(), SupervisorError> {
        self.transition(id, TaskState::Running, TaskState::Verifying)
    }

    pub fn complete_task(&mut self, id: &str) -> Result<(), SupervisorError> {
        let task = self.task_mut(id)?;
        if !matches!(task.state, TaskState::Running | TaskState::Verifying) {
            return Err(SupervisorError::InvalidTransition {
                task: id.to_owned(),
                from: task.state,
                to: TaskState::Succeeded,
            });
        }
        task.verify_completion()?;
        task.state = TaskState::Succeeded;
        task.failure = None;
        Ok(())
    }

    pub fn fail_task(
        &mut self,
        id: &str,
        reason: impl Into<String>,
    ) -> Result<(), SupervisorError> {
        let task = self.task_mut(id)?;
        if matches!(task.state, TaskState::Succeeded | TaskState::Failed) {
            return Err(SupervisorError::InvalidTransition {
                task: id.to_owned(),
                from: task.state,
                to: TaskState::Failed,
            });
        }
        task.state = TaskState::Failed;
        task.failure = Some(reason.into());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PermissionKey {
    pub capability: Capability,
    pub target: String,
}

impl PermissionKey {
    pub fn new(capability: Capability, target: impl Into<String>) -> Result<Self, SupervisorError> {
        let target = target.into();
        if target.trim().is_empty() {
            return Err(SupervisorError::InvalidPermissionTarget);
        }
        Ok(Self { capability, target })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMemory {
    AllowForSession,
    DenyPermanently,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    Approved,
    Rejected,
    TimedOut,
    Interrupted,
}

impl ApprovalOutcome {
    pub fn allows_execution(self) -> bool {
        matches!(self, Self::Approved)
    }

    pub fn permission_decision(self) -> PermissionDecision {
        if self.allows_execution() {
            PermissionDecision::Allow
        } else {
            PermissionDecision::Deny
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub key: PermissionKey,
    pub risk: RiskLevel,
    pub requested_at_ms: i64,
    pub resolved_at_ms: i64,
    pub outcome: ApprovalOutcome,
    pub memory: Option<PermissionMemory>,
}

impl ApprovalRecord {
    pub fn new(
        approval_id: impl Into<String>,
        key: PermissionKey,
        risk: RiskLevel,
        requested_at_ms: i64,
        resolved_at_ms: i64,
        outcome: ApprovalOutcome,
        memory: Option<PermissionMemory>,
    ) -> Result<Self, SupervisorError> {
        let approval_id = approval_id.into();
        if approval_id.trim().is_empty() {
            return Err(SupervisorError::InvalidApproval(
                "approval id is empty".into(),
            ));
        }
        if resolved_at_ms < requested_at_ms {
            return Err(SupervisorError::InvalidApproval(
                "approval resolved before it was requested".into(),
            ));
        }
        match (outcome, memory) {
            (ApprovalOutcome::Approved, Some(PermissionMemory::DenyPermanently)) => {
                return Err(SupervisorError::InvalidApproval(
                    "approved outcome cannot create a permanent denial".into(),
                ));
            }
            (ApprovalOutcome::Rejected, Some(PermissionMemory::AllowForSession)) => {
                return Err(SupervisorError::InvalidApproval(
                    "rejected outcome cannot create a session allow".into(),
                ));
            }
            (ApprovalOutcome::TimedOut, Some(_)) | (ApprovalOutcome::Interrupted, Some(_)) => {
                return Err(SupervisorError::InvalidApproval(
                    "timeout/interruption cannot mutate permission memory".into(),
                ));
            }
            _ => {}
        }
        Ok(Self {
            approval_id,
            key,
            risk,
            requested_at_ms,
            resolved_at_ms,
            outcome,
            memory,
        })
    }

    fn apply(&self, permissions: &mut PermissionLedger) -> PermissionDecision {
        if let Some(memory) = self.memory {
            permissions.remember(self.key.clone(), memory);
        }
        self.outcome.permission_decision()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApprovalLedger {
    records: BTreeMap<String, ApprovalRecord>,
}

impl ApprovalLedger {
    pub fn record(
        &mut self,
        record: ApprovalRecord,
        permissions: &mut PermissionLedger,
    ) -> Result<PermissionDecision, SupervisorError> {
        if self.records.contains_key(&record.approval_id) {
            return Err(SupervisorError::ApprovalExists(record.approval_id));
        }
        let decision = record.apply(permissions);
        self.records.insert(record.approval_id.clone(), record);
        Ok(decision)
    }

    pub fn get(&self, approval_id: &str) -> Option<&ApprovalRecord> {
        self.records.get(approval_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ApprovalRecord> {
        self.records.values()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionLedger {
    session_allows: BTreeSet<PermissionKey>,
    permanent_denials: BTreeSet<PermissionKey>,
}

impl PermissionLedger {
    pub fn remember(&mut self, key: PermissionKey, memory: PermissionMemory) {
        match memory {
            PermissionMemory::AllowForSession => {
                self.permanent_denials.remove(&key);
                self.session_allows.insert(key);
            }
            PermissionMemory::DenyPermanently => {
                self.session_allows.remove(&key);
                self.permanent_denials.insert(key);
            }
            PermissionMemory::Clear => {
                self.session_allows.remove(&key);
                self.permanent_denials.remove(&key);
            }
        }
    }

    pub fn evaluate(
        &self,
        policy: &ExecutionPolicy,
        key: &PermissionKey,
        risk: RiskLevel,
    ) -> PermissionDecision {
        if self.permanent_denials.contains(key) || policy_forbids(policy, key.capability) {
            return PermissionDecision::Deny;
        }
        if self.session_allows.contains(key) {
            return PermissionDecision::Allow;
        }
        let above_unattended = risk_score(risk) > risk_score(policy.max_unattended_risk);
        match policy.approval_mode {
            ApprovalMode::Never if above_unattended => PermissionDecision::Deny,
            ApprovalMode::Never => PermissionDecision::Allow,
            ApprovalMode::OnRisk if above_unattended => PermissionDecision::Ask,
            ApprovalMode::OnRisk => PermissionDecision::Allow,
            ApprovalMode::Always => PermissionDecision::Ask,
        }
    }
}

fn policy_forbids(policy: &ExecutionPolicy, capability: Capability) -> bool {
    match capability {
        Capability::Network | Capability::WebSearch => !policy.allow_network,
        Capability::Process | Capability::Git => !policy.allow_process,
        Capability::FilesystemWrite | Capability::Workspace => !policy.allow_workspace_writes,
        _ => false,
    }
}

fn risk_score(risk: RiskLevel) -> u8 {
    match risk {
        RiskLevel::ReadOnly => 0,
        RiskLevel::WorkspaceWrite => 1,
        RiskLevel::SystemWrite => 2,
        RiskLevel::ExternalSideEffect => 3,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub version: u32,
    pub revision: u64,
    pub session_id: String,
    pub backend_id: String,
    pub supervisor: TaskSupervisor,
    pub permissions: PermissionLedger,
    #[serde(default)]
    pub approvals: ApprovalLedger,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl SessionSnapshot {
    pub fn new(
        session_id: impl Into<String>,
        backend_id: impl Into<String>,
        created_at_ms: i64,
    ) -> Result<Self, SupervisorError> {
        let session_id = session_id.into();
        let backend_id = backend_id.into();
        if session_id.trim().is_empty() || backend_id.trim().is_empty() {
            return Err(SupervisorError::InvalidSnapshot(
                "session_id and backend_id are required".into(),
            ));
        }
        Ok(Self {
            version: SNAPSHOT_VERSION,
            revision: 0,
            session_id,
            backend_id,
            supervisor: TaskSupervisor::default(),
            permissions: PermissionLedger::default(),
            approvals: ApprovalLedger::default(),
            created_at_ms,
            updated_at_ms: created_at_ms,
        })
    }

    pub fn validate(&self) -> Result<(), SupervisorError> {
        if self.version != SNAPSHOT_VERSION {
            return Err(SupervisorError::UnsupportedSnapshotVersion(self.version));
        }
        if self.session_id.trim().is_empty() || self.backend_id.trim().is_empty() {
            return Err(SupervisorError::InvalidSnapshot(
                "session_id and backend_id are required".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryJournal {
    snapshots: BTreeMap<String, SessionSnapshot>,
}

impl RecoveryJournal {
    pub fn load(&self, session_id: &str) -> Option<&SessionSnapshot> {
        self.snapshots.get(session_id)
    }

    pub fn save(
        &mut self,
        mut snapshot: SessionSnapshot,
        expected_revision: u64,
        updated_at_ms: i64,
    ) -> Result<u64, SupervisorError> {
        snapshot.validate()?;
        let current_revision = self
            .snapshots
            .get(&snapshot.session_id)
            .map(|current| current.revision)
            .unwrap_or(0);
        if current_revision != expected_revision {
            return Err(SupervisorError::RevisionConflict {
                expected: expected_revision,
                actual: current_revision,
            });
        }
        snapshot.revision = current_revision.saturating_add(1);
        snapshot.updated_at_ms = updated_at_ms;
        let revision = snapshot.revision;
        self.snapshots.insert(snapshot.session_id.clone(), snapshot);
        Ok(revision)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SupervisorError {
    #[error("invalid task: {0}")]
    InvalidTask(String),
    #[error("task already exists: {0}")]
    TaskExists(String),
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("dependency not found: {0}")]
    DependencyNotFound(String),
    #[error("task is not ready: {0}")]
    TaskNotReady(String),
    #[error("invalid task transition for {task}: {from:?} -> {to:?}")]
    InvalidTransition {
        task: String,
        from: TaskState,
        to: TaskState,
    },
    #[error("invalid verification oracle: {0}")]
    InvalidOracle(String),
    #[error("verification oracle already exists: {0}")]
    OracleExists(String),
    #[error("verification oracle not found: {0}")]
    OracleNotFound(String),
    #[error("verification blocked: {0}")]
    VerificationBlocked(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
    #[error("invalid loop policy")]
    InvalidLoopPolicy,
    #[error("invalid attempt: {0}")]
    InvalidAttempt(String),
    #[error("attempt not found: {0}")]
    AttemptNotFound(String),
    #[error("permission target is empty")]
    InvalidPermissionTarget,
    #[error("invalid approval: {0}")]
    InvalidApproval(String),
    #[error("approval already recorded: {0}")]
    ApprovalExists(String),
    #[error("invalid recovery snapshot: {0}")]
    InvalidSnapshot(String),
    #[error("unsupported snapshot version: {0}")]
    UnsupportedSnapshotVersion(u32),
    #[error("snapshot revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, deps: &[&str], mode: VerificationMode) -> TaskRecord {
        TaskRecord::new(
            id,
            format!("complete {id}"),
            deps.iter().map(|value| (*value).to_owned()),
            mode,
        )
        .expect("task")
    }

    #[test]
    fn dependency_readiness_is_deterministic() {
        let mut supervisor = TaskSupervisor::default();
        supervisor
            .add_task(task("a", &[], VerificationMode::BestEffort))
            .unwrap();
        supervisor
            .add_task(task("b", &["a"], VerificationMode::BestEffort))
            .unwrap();
        assert_eq!(supervisor.ready_task_ids(), vec!["a"]);
        supervisor.start_task("a").unwrap();
        supervisor.complete_task("a").unwrap();
        assert_eq!(supervisor.ready_task_ids(), vec!["b"]);
    }

    #[test]
    fn objective_tasks_cannot_claim_success_without_evidence() {
        let mut supervisor = TaskSupervisor::default();
        let mut record = task("release", &[], VerificationMode::ObjectiveRequired);
        record
            .add_oracle(VerificationOracle::required("ci").unwrap())
            .unwrap();
        supervisor.add_task(record).unwrap();
        supervisor.start_task("release").unwrap();
        supervisor.begin_verification("release").unwrap();
        assert!(matches!(
            supervisor.complete_task("release"),
            Err(SupervisorError::VerificationBlocked(_))
        ));
        supervisor
            .task_mut("release")
            .unwrap()
            .record_oracle("ci", OracleStatus::Passed, Some("run:325".into()))
            .unwrap();
        supervisor.complete_task("release").unwrap();
    }

    #[test]
    fn repeated_actions_warn_then_interrupt() {
        let policy = LoopPolicy::default();
        let mut state = LoopState::default();
        assert_eq!(state.observe("same", policy), LoopDisposition::Allow);
        assert_eq!(state.observe("same", policy), LoopDisposition::Warn);
        assert_eq!(state.observe("same", policy), LoopDisposition::Warn);
        assert_eq!(state.observe("same", policy), LoopDisposition::Interrupt);
        assert_eq!(state.consecutive_occurrences(), 4);
    }

    #[test]
    fn permanent_denial_beats_session_allow_and_policy() {
        let key = PermissionKey::new(Capability::Network, "example.com").unwrap();
        let mut ledger = PermissionLedger::default();
        ledger.remember(key.clone(), PermissionMemory::AllowForSession);
        assert_eq!(
            ledger.evaluate(
                &ExecutionPolicy::interactive_default(),
                &key,
                RiskLevel::ReadOnly
            ),
            PermissionDecision::Allow
        );
        ledger.remember(key.clone(), PermissionMemory::DenyPermanently);
        assert_eq!(
            ledger.evaluate(
                &ExecutionPolicy::interactive_default(),
                &key,
                RiskLevel::ReadOnly
            ),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn approval_terminal_outcomes_are_distinct_and_fail_closed() {
        assert!(ApprovalOutcome::Approved.allows_execution());
        for outcome in [
            ApprovalOutcome::Rejected,
            ApprovalOutcome::TimedOut,
            ApprovalOutcome::Interrupted,
        ] {
            assert!(!outcome.allows_execution());
            assert_eq!(outcome.permission_decision(), PermissionDecision::Deny);
        }
        assert_eq!(
            serde_json::to_string(&ApprovalOutcome::TimedOut).unwrap(),
            "\"timed_out\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalOutcome::Interrupted).unwrap(),
            "\"interrupted\""
        );
    }

    #[test]
    fn approval_memory_cannot_override_terminal_outcome() {
        let key = PermissionKey::new(Capability::Network, "api.example.com").unwrap();
        let mut permissions = PermissionLedger::default();
        let mut approvals = ApprovalLedger::default();
        let mut policy = ExecutionPolicy::interactive_default();
        policy.approval_mode = ApprovalMode::Always;

        let approved = ApprovalRecord::new(
            "approval-1",
            key.clone(),
            RiskLevel::ExternalSideEffect,
            10,
            11,
            ApprovalOutcome::Approved,
            Some(PermissionMemory::AllowForSession),
        )
        .unwrap();
        assert_eq!(
            approvals.record(approved, &mut permissions).unwrap(),
            PermissionDecision::Allow
        );
        assert_eq!(
            permissions.evaluate(&policy, &key, RiskLevel::ExternalSideEffect),
            PermissionDecision::Allow
        );

        let rejected = ApprovalRecord::new(
            "approval-2",
            key.clone(),
            RiskLevel::ExternalSideEffect,
            12,
            13,
            ApprovalOutcome::Rejected,
            Some(PermissionMemory::DenyPermanently),
        )
        .unwrap();
        assert_eq!(
            approvals.record(rejected, &mut permissions).unwrap(),
            PermissionDecision::Deny
        );
        assert_eq!(
            permissions.evaluate(&policy, &key, RiskLevel::ReadOnly),
            PermissionDecision::Deny
        );
        assert_eq!(
            approvals.get("approval-2").unwrap().outcome,
            ApprovalOutcome::Rejected
        );
    }

    #[test]
    fn timeout_and_interruption_cannot_create_permission_memory() {
        let key = PermissionKey::new(Capability::Process, "cargo test").unwrap();
        assert!(matches!(
            ApprovalRecord::new(
                "approval-timeout",
                key.clone(),
                RiskLevel::SystemWrite,
                1,
                2,
                ApprovalOutcome::TimedOut,
                Some(PermissionMemory::AllowForSession),
            ),
            Err(SupervisorError::InvalidApproval(_))
        ));
        assert!(matches!(
            ApprovalRecord::new(
                "approval-interrupted",
                key,
                RiskLevel::SystemWrite,
                1,
                2,
                ApprovalOutcome::Interrupted,
                Some(PermissionMemory::DenyPermanently),
            ),
            Err(SupervisorError::InvalidApproval(_))
        ));
    }

    #[test]
    fn recovery_is_provider_neutral_and_revision_checked() {
        let snapshot = SessionSnapshot::new("session-1", "mahayana-native", 10).unwrap();
        let encoded = serde_json::to_string(&snapshot).unwrap().to_lowercase();
        assert!(!encoded.contains("codex"));
        assert!(!encoded.contains("grok"));
        let mut journal = RecoveryJournal::default();
        assert_eq!(journal.save(snapshot.clone(), 0, 11).unwrap(), 1);
        assert_eq!(
            journal.save(snapshot, 0, 12),
            Err(SupervisorError::RevisionConflict {
                expected: 0,
                actual: 1
            })
        );
    }

    #[test]
    fn recovery_snapshot_preserves_approval_terminal_state() {
        let mut snapshot = SessionSnapshot::new("session-2", "mahayana-native", 20).unwrap();
        let key = PermissionKey::new(Capability::FilesystemWrite, "src/lib.rs").unwrap();
        let record = ApprovalRecord::new(
            "approval-timeout",
            key,
            RiskLevel::WorkspaceWrite,
            20,
            25,
            ApprovalOutcome::TimedOut,
            None,
        )
        .unwrap();
        assert_eq!(
            snapshot
                .approvals
                .record(record, &mut snapshot.permissions)
                .unwrap(),
            PermissionDecision::Deny
        );
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: SessionSnapshot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            decoded.approvals.get("approval-timeout").unwrap().outcome,
            ApprovalOutcome::TimedOut
        );
    }
}
