//! Product-owned execution kernel for Mahayana.
//!
//! This crate is deliberately provider-neutral.  Public product surfaces must
//! depend on these contracts instead of Codex, Grok Build, or model-provider
//! protocol types.  External engines are adapters behind [`EngineBackend`].

pub mod resilience;
pub mod sandbox;
pub mod supervisor;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(String);

impl OperationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Model,
    FilesystemRead,
    FilesystemWrite,
    Process,
    Git,
    Network,
    WebSearch,
    ComputerUse,
    ToolProtocol,
    Mcp,
    Skills,
    Plugins,
    Memory,
    Workspace,
    Checkpoint,
    Worktree,
    CodebaseGraph,
    Workflow,
    PromptQueue,
    Subagent,
    Hooks,
    Voice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    values: BTreeSet<Capability>,
}

impl CapabilitySet {
    pub fn new(values: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub fn empty() -> Self {
        Self {
            values: BTreeSet::new(),
        }
    }

    pub fn contains(&self, capability: Capability) -> bool {
        self.values.contains(&capability)
    }

    pub fn supports_all(&self, required: &CapabilitySet) -> bool {
        required
            .values
            .iter()
            .all(|value| self.values.contains(value))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.values.iter()
    }
}

impl Default for CapabilitySet {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfile {
    DesktopFull,
    MobileEmbedded,
    WebWasm,
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    ReadOnly,
    WorkspaceWrite,
    SystemWrite,
    ExternalSideEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Never,
    OnRisk,
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub approval_mode: ApprovalMode,
    pub max_unattended_risk: RiskLevel,
    pub allow_network: bool,
    pub allow_process: bool,
    pub allow_workspace_writes: bool,
}

impl ExecutionPolicy {
    pub fn interactive_default() -> Self {
        Self {
            approval_mode: ApprovalMode::OnRisk,
            max_unattended_risk: RiskLevel::ReadOnly,
            allow_network: true,
            allow_process: true,
            allow_workspace_writes: true,
        }
    }

    pub fn mobile_default() -> Self {
        Self {
            approval_mode: ApprovalMode::OnRisk,
            max_unattended_risk: RiskLevel::ReadOnly,
            allow_network: true,
            allow_process: false,
            allow_workspace_writes: true,
        }
    }
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self::interactive_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenSessionRequest {
    pub profile: RuntimeProfile,
    pub workspace_root: Option<String>,
    pub model: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub session_id: SessionId,
    pub operation_id: OperationId,
    pub input: String,
    pub policy: ExecutionPolicy,
    pub required_capabilities: CapabilitySet,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolution {
    pub approval_id: String,
    pub approved: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelEvent {
    MessageDelta {
        operation_id: OperationId,
        delta: String,
    },
    MessageCompleted {
        operation_id: OperationId,
        text: String,
    },
    Activity {
        operation_id: OperationId,
        kind: String,
        title: String,
        detail: Option<String>,
        metadata: Value,
    },
    ToolStarted {
        operation_id: OperationId,
        tool: String,
        arguments: Value,
    },
    ToolCompleted {
        operation_id: OperationId,
        tool: String,
        output: Value,
        success: bool,
    },
    ApprovalRequested {
        operation_id: OperationId,
        approval_id: String,
        title: String,
        risk: RiskLevel,
        details: Value,
    },
    CheckpointCreated {
        operation_id: OperationId,
        checkpoint_id: String,
        label: Option<String>,
    },
    UsageUpdated {
        operation_id: OperationId,
        input_tokens: u64,
        output_tokens: u64,
    },
    OperationCompleted {
        operation_id: OperationId,
    },
    OperationFailed {
        operation_id: OperationId,
        message: String,
        retryable: bool,
    },
}

pub trait KernelEventSink: Send + Sync {
    fn emit(&self, event: KernelEvent) -> Result<(), KernelError>;
}

pub type SharedKernelEventSink = Arc<dyn KernelEventSink>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendDescriptor {
    pub id: String,
    pub display_name: String,
    pub native: bool,
    pub capabilities: CapabilitySet,
}

#[async_trait]
pub trait EngineBackend: Send + Sync {
    fn descriptor(&self) -> BackendDescriptor;

    async fn open_session(&self, request: OpenSessionRequest) -> Result<SessionId, KernelError>;

    async fn run(
        &self,
        request: RunRequest,
        events: SharedKernelEventSink,
    ) -> Result<(), KernelError>;

    async fn interrupt(&self, operation_id: &OperationId) -> Result<(), KernelError>;

    async fn resolve_approval(&self, resolution: ApprovalResolution) -> Result<(), KernelError>;
}

/// Product-owned backend registry and capability router.
///
/// Native Mahayana engines and temporary compatibility adapters are treated
/// uniformly, while callers can explicitly prefer native implementations.
pub struct Kernel {
    backends: HashMap<String, Arc<dyn EngineBackend>>,
}

impl Kernel {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
        }
    }

    pub fn register(&mut self, backend: Arc<dyn EngineBackend>) -> Option<Arc<dyn EngineBackend>> {
        let id = backend.descriptor().id;
        self.backends.insert(id, backend)
    }

    pub fn backend(&self, id: &str) -> Option<Arc<dyn EngineBackend>> {
        self.backends.get(id).cloned()
    }

    pub fn descriptors(&self) -> Vec<BackendDescriptor> {
        let mut descriptors = self
            .backends
            .values()
            .map(|backend| backend.descriptor())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.id.cmp(&right.id));
        descriptors
    }

    pub fn resolve(
        &self,
        required: &CapabilitySet,
        prefer_native: bool,
    ) -> Result<Arc<dyn EngineBackend>, KernelError> {
        let mut candidates = self
            .backends
            .values()
            .filter(|backend| backend.descriptor().capabilities.supports_all(required))
            .cloned()
            .collect::<Vec<_>>();

        candidates.sort_by_key(|backend| {
            let descriptor = backend.descriptor();
            (prefer_native && !descriptor.native, descriptor.id)
        });

        candidates.into_iter().next().ok_or_else(|| {
            KernelError::CapabilityUnavailable(
                required
                    .iter()
                    .map(|capability| format!("{capability:?}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("backend not found: {0}")]
    BackendNotFound(String),
    #[error("required capability is unavailable: {0}")]
    CapabilityUnavailable(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("operation not found: {0}")]
    OperationNotFound(String),
    #[error("approval not found: {0}")]
    ApprovalNotFound(String),
    #[error("operation denied by policy: {0}")]
    PolicyDenied(String),
    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),
    #[error("backend failed: {0}")]
    Backend(String),
    #[error("event consumer is closed")]
    EventConsumerClosed,
}

#[cfg(test)]
mod tests {
    use super::{Capability, CapabilitySet, ExecutionPolicy, SessionId};

    #[test]
    fn identifiers_are_product_owned_and_unique() {
        assert_ne!(SessionId::new(), SessionId::new());
    }

    #[test]
    fn capability_sets_support_subset_routing() {
        let backend = CapabilitySet::new([
            Capability::Model,
            Capability::Workspace,
            Capability::Checkpoint,
        ]);
        let required = CapabilitySet::new([Capability::Model, Capability::Workspace]);
        assert!(backend.supports_all(&required));
    }

    #[test]
    fn mobile_policy_disables_process_execution() {
        assert!(!ExecutionPolicy::mobile_default().allow_process);
    }
}
