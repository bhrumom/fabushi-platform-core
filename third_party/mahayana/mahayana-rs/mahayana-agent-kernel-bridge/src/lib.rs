//! Transitional bridge from the existing Mahayana `AgentBackend` contract to
//! the provider-neutral sovereign kernel.
//!
//! The bridge intentionally depends on Mahayana contracts only. A Codex-backed
//! `AgentBackend` can sit behind it today, while native Mahayana backends can
//! implement `EngineBackend` directly and bypass this crate entirely.

use async_trait::async_trait;
use mahayana_agent::{
    AgentActivityStatus, AgentBackend, AgentError, AgentEvent, AgentEventSink, AgentMessageRequest,
    ApprovalResolution as AgentApprovalResolution, SharedAgentEventSink, StartThreadRequest,
};
use mahayana_core::{
    ApprovalDecision, ApprovalId, ConversationId, OperationId as AgentOperationId,
};
use mahayana_kernel::{
    ApprovalResolution, BackendDescriptor, Capability, EngineBackend, KernelError, KernelEvent,
    OpenSessionRequest, OperationId, RunRequest, SessionId, SharedKernelEventSink,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct LegacySession {
    thread_id: mahayana_core::AgentThreadId,
    conversation_id: ConversationId,
}

pub struct LegacyAgentKernelBridge {
    backend: Arc<dyn AgentBackend>,
    descriptor: BackendDescriptor,
    sessions: Mutex<HashMap<String, LegacySession>>,
}

impl LegacyAgentKernelBridge {
    pub fn new(backend: Arc<dyn AgentBackend>, descriptor: BackendDescriptor) -> Self {
        Self {
            backend,
            descriptor,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn agent_operation_id(operation_id: &OperationId) -> Result<AgentOperationId, KernelError> {
        AgentOperationId::new(operation_id.as_str())
            .map_err(|error| KernelError::Backend(error.to_string()))
    }

    fn map_agent_error(error: AgentError) -> KernelError {
        match error {
            AgentError::ThreadNotFound(id) => KernelError::SessionNotFound(id.to_string()),
            AgentError::OperationNotFound(id) => KernelError::OperationNotFound(id.to_string()),
            AgentError::ApprovalNotFound(id) => KernelError::ApprovalNotFound(id.to_string()),
            AgentError::Unavailable(message) => KernelError::BackendUnavailable(message),
            AgentError::EventConsumerClosed => KernelError::EventConsumerClosed,
            other => KernelError::Backend(other.to_string()),
        }
    }

    fn validate_policy(request: &RunRequest) -> Result<(), KernelError> {
        let required = &request.required_capabilities;
        let policy = &request.policy;

        if required.contains(Capability::Network) && !policy.allow_network {
            return Err(KernelError::PolicyDenied(
                "network capability is disabled by Mahayana policy".into(),
            ));
        }
        if required.contains(Capability::Process) && !policy.allow_process {
            return Err(KernelError::PolicyDenied(
                "process execution is disabled by Mahayana policy".into(),
            ));
        }
        if required.contains(Capability::FilesystemWrite) && !policy.allow_workspace_writes {
            return Err(KernelError::PolicyDenied(
                "workspace writes are disabled by Mahayana policy".into(),
            ));
        }

        Ok(())
    }
}

struct EventBridge {
    operation_id: OperationId,
    sink: SharedKernelEventSink,
}

impl EventBridge {
    fn emit_kernel(&self, event: KernelEvent) -> Result<(), AgentError> {
        self.sink
            .emit(event)
            .map_err(|error| AgentError::Backend(error.to_string()))
    }
}

impl AgentEventSink for EventBridge {
    fn emit(&self, event: AgentEvent) -> Result<(), AgentError> {
        match event {
            AgentEvent::MessageDelta { delta } => self.emit_kernel(KernelEvent::MessageDelta {
                operation_id: self.operation_id.clone(),
                delta,
            }),
            AgentEvent::MessageCompleted { message } => {
                self.emit_kernel(KernelEvent::MessageCompleted {
                    operation_id: self.operation_id.clone(),
                    text: message.text,
                })
            }
            AgentEvent::TokenUsageUpdated { usage } => {
                let usage = usage.last;
                self.emit_kernel(KernelEvent::UsageUpdated {
                    operation_id: self.operation_id.clone(),
                    input_tokens: usage.input_tokens.max(0) as u64,
                    output_tokens: usage.output_tokens.max(0) as u64,
                })
            }
            AgentEvent::ToolProgress { message } => self.emit_kernel(KernelEvent::Activity {
                operation_id: self.operation_id.clone(),
                kind: "tool_progress".to_string(),
                title: "Tool progress".to_string(),
                detail: Some(message),
                metadata: Value::Null,
            }),
            AgentEvent::Activity { activity } => {
                let status = match activity.status {
                    AgentActivityStatus::Running => "running",
                    AgentActivityStatus::Completed => "completed",
                    AgentActivityStatus::Failed => "failed",
                };
                self.emit_kernel(KernelEvent::Activity {
                    operation_id: self.operation_id.clone(),
                    kind: activity.kind,
                    title: activity.title,
                    detail: activity.detail,
                    metadata: json!({
                        "stepId": activity.step_id,
                        "status": status,
                        "provider": activity.metadata,
                    }),
                })
            }
            AgentEvent::ApprovalRequested {
                approval_id,
                title,
                details,
            } => self.emit_kernel(KernelEvent::ApprovalRequested {
                operation_id: self.operation_id.clone(),
                approval_id: approval_id.to_string(),
                title,
                risk: mahayana_kernel::RiskLevel::ExternalSideEffect,
                details,
            }),
        }
    }
}

#[async_trait]
impl EngineBackend for LegacyAgentKernelBridge {
    fn descriptor(&self) -> BackendDescriptor {
        self.descriptor.clone()
    }

    async fn open_session(&self, request: OpenSessionRequest) -> Result<SessionId, KernelError> {
        let session_id = SessionId::new();
        let conversation_id = request
            .metadata
            .get("conversationId")
            .and_then(Value::as_str)
            .map(ConversationId::new)
            .transpose()
            .map_err(|error| KernelError::Backend(error.to_string()))?
            .unwrap_or_else(|| ConversationId::generated("mahayana-kernel"));

        let thread_id = self
            .backend
            .start_thread(StartThreadRequest {
                conversation_id: conversation_id.clone(),
            })
            .await
            .map_err(Self::map_agent_error)?;

        self.sessions
            .lock()
            .map_err(|_| KernelError::Backend("legacy session registry poisoned".into()))?
            .insert(
                session_id.as_str().to_string(),
                LegacySession {
                    thread_id,
                    conversation_id,
                },
            );

        Ok(session_id)
    }

    async fn run(
        &self,
        request: RunRequest,
        events: SharedKernelEventSink,
    ) -> Result<(), KernelError> {
        Self::validate_policy(&request)?;

        if !self
            .descriptor
            .capabilities
            .supports_all(&request.required_capabilities)
        {
            return Err(KernelError::CapabilityUnavailable(
                "legacy backend does not satisfy the requested capability set".into(),
            ));
        }

        let session = self
            .sessions
            .lock()
            .map_err(|_| KernelError::Backend("legacy session registry poisoned".into()))?
            .get(request.session_id.as_str())
            .cloned()
            .ok_or_else(|| KernelError::SessionNotFound(request.session_id.as_str().into()))?;

        let agent_operation_id = Self::agent_operation_id(&request.operation_id)?;
        let event_sink: SharedAgentEventSink = Arc::new(EventBridge {
            operation_id: request.operation_id.clone(),
            sink: events.clone(),
        });

        self.backend
            .send_message(
                AgentMessageRequest {
                    thread_id: session.thread_id,
                    conversation_id: session.conversation_id,
                    operation_id: agent_operation_id,
                    text: request.input,
                    client_message_id: request
                        .metadata
                        .get("clientMessageId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                },
                event_sink,
            )
            .await
            .map_err(Self::map_agent_error)?;

        events
            .emit(KernelEvent::OperationCompleted {
                operation_id: request.operation_id,
            })
            .map_err(|_| KernelError::EventConsumerClosed)
    }

    async fn interrupt(&self, operation_id: &OperationId) -> Result<(), KernelError> {
        self.backend
            .interrupt(&Self::agent_operation_id(operation_id)?)
            .await
            .map_err(Self::map_agent_error)
    }

    async fn resolve_approval(&self, resolution: ApprovalResolution) -> Result<(), KernelError> {
        let approval_id = ApprovalId::new(resolution.approval_id)
            .map_err(|error| KernelError::Backend(error.to_string()))?;
        self.backend
            .resolve_approval(AgentApprovalResolution {
                approval_id,
                decision: if resolution.approved {
                    ApprovalDecision::Accept
                } else {
                    ApprovalDecision::Decline
                },
                payload: resolution.metadata,
            })
            .await
            .map_err(Self::map_agent_error)
    }
}
