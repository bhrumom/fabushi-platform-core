//! Model inference boundary for native, mobile, and Web runtimes.

use async_trait::async_trait;
pub use mahayana_core::ModelProviderMode;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

pub mod context;
pub mod responses;

pub use context::CompactionPlan;
pub use context::CompactionRequest;
pub use context::CompactionResult;
pub use context::ContextBudget;
pub use context::ContextError;
pub use context::estimate_history_tokens;
pub use context::estimate_json_tokens;
pub use context::plan_compaction;
pub use context::prepare_compaction;
pub use responses::ModelCredentialResolver;
pub use responses::ResponsesModelConfig;
pub use responses::ResponsesModelRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFeature {
    StreamingText,
    ToolCalls,
    ParallelToolCalls,
    UsageAccounting,
    ReasoningUsage,
    StructuredOutput,
    ImageInput,
    AudioInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub features: BTreeSet<ModelFeature>,
    pub max_context_tokens: Option<u64>,
}

impl ModelCapabilities {
    pub fn new(features: impl IntoIterator<Item = ModelFeature>) -> Self {
        Self {
            features: features.into_iter().collect(),
            max_context_tokens: None,
        }
    }

    pub fn baseline() -> Self {
        Self::new([
            ModelFeature::StreamingText,
            ModelFeature::ToolCalls,
            ModelFeature::UsageAccounting,
        ])
    }

    pub fn contains(&self, feature: ModelFeature) -> bool {
        self.features.contains(&feature)
    }

    pub fn supports_all(&self, required: &ModelCapabilities) -> bool {
        required
            .features
            .iter()
            .all(|feature| self.features.contains(feature))
            && match (self.max_context_tokens, required.max_context_tokens) {
                (_, None) => true,
                (Some(available), Some(required)) => available >= required,
                (None, Some(_)) => false,
            }
    }

    pub fn require(&self, required: &ModelCapabilities) -> Result<(), ModelError> {
        if self.supports_all(required) {
            return Ok(());
        }
        let missing = required
            .features
            .iter()
            .filter(|feature| !self.features.contains(feature))
            .map(|feature| format!("{feature:?}"))
            .collect::<Vec<_>>();
        Err(ModelError::CapabilityUnavailable(if missing.is_empty() {
            "required context window is larger than the runtime declares".into()
        } else {
            missing.join(", ")
        }))
    }
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self::baseline()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    pub model: String,
    pub input: Value,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelEvent {
    OutputTextDelta(String),
    Usage(ModelUsage),
    Completed { output: Value },
    Failed { code: String, message: String },
}

/// Receives streaming model events. Mobile and Web implementations should
/// forward from their native token callbacks without blocking the inference
/// thread.
pub trait ModelEventSink: Send + Sync {
    fn emit(&self, event: ModelEvent) -> Result<(), ModelError>;
}

pub type SharedModelEventSink = Arc<dyn ModelEventSink>;

/// Model inference implementation. The Agent loop is independent from whether
/// inference is a local library, loopback service, or explicitly enabled
/// first-party/user endpoint.
#[async_trait]
pub trait ModelRuntime: Send + Sync {
    async fn infer(
        &self,
        request: ModelRequest,
        events: SharedModelEventSink,
    ) -> Result<(), ModelError>;

    fn provider_mode(&self) -> ModelProviderMode;

    /// Provider-neutral feature declaration. Existing runtimes inherit the
    /// historical Mahayana baseline; specialized runtimes should override it
    /// when they support or intentionally omit optional features.
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities::baseline()
    }

    fn is_local(&self) -> bool {
        matches!(
            self.provider_mode(),
            ModelProviderMode::LocalModel | ModelProviderMode::LocalLoopback
        )
    }

    /// Negotiate required features before executing a request.  This keeps
    /// feature mismatches out of vendor-specific error paths and gives all
    /// product surfaces one Mahayana-owned fail-fast contract.
    async fn infer_negotiated(
        &self,
        required: &ModelCapabilities,
        request: ModelRequest,
        events: SharedModelEventSink,
    ) -> Result<(), ModelError> {
        self.capabilities().require(required)?;
        self.infer(request, events).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("model runtime is unavailable: {0}")]
    Unavailable(String),
    #[error("model capability is unavailable: {0}")]
    CapabilityUnavailable(String),
    #[error("model request is invalid: {0}")]
    InvalidRequest(String),
    #[error("model inference failed: {0}")]
    Inference(String),
    #[error("model event consumer is closed")]
    EventConsumerClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_negotiation_fails_before_provider_execution() {
        let available = ModelCapabilities::new([ModelFeature::StreamingText]);
        let required =
            ModelCapabilities::new([ModelFeature::StreamingText, ModelFeature::ToolCalls]);
        assert!(matches!(
            available.require(&required),
            Err(ModelError::CapabilityUnavailable(_))
        ));
    }
}
