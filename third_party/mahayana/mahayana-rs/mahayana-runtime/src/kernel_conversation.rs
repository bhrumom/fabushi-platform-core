use async_trait::async_trait;
use mahayana_conversation::{
    ConversationError, ConversationProvider, MAHAYANA_AI_PROVIDER_KEY, ResolveApprovalRequest,
    SendMessageRequest, SharedConversationEventSink,
};
use mahayana_core::{
    ApprovalDecision, ApprovalId, BuildProfile, Conversation, ConversationId, Message, MessageId,
    MessageRole, ModelTokenUsage, ModelTokenUsageSnapshot, OperationId, RuntimeActivityStatus,
    RuntimeEvent,
};
use mahayana_kernel::{
    ApprovalResolution, Capability, CapabilitySet, EngineBackend, ExecutionPolicy, KernelError,
    KernelEvent, KernelEventSink, OpenSessionRequest, OperationId as KernelOperationId, RiskLevel,
    RunRequest, RuntimeProfile, SessionId, SharedKernelEventSink,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as AsyncMutex;

pub const MAHAYANA_AI_CONVERSATION_ID: &str = "mahayana-ai:agent:assistant";

pub struct KernelConversationProvider {
    backend: Arc<dyn EngineBackend>,
    profile: BuildProfile,
    workspace_root: Option<String>,
    model: Option<String>,
    session_id: AsyncMutex<Option<SessionId>>,
    history: Arc<Mutex<Vec<Message>>>,
}

impl KernelConversationProvider {
    pub fn new(
        backend: Arc<dyn EngineBackend>,
        profile: BuildProfile,
        workspace_root: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            backend,
            profile,
            workspace_root,
            model,
            session_id: AsyncMutex::new(None),
            history: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn session_id(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<SessionId, ConversationError> {
        let mut session_id = self.session_id.lock().await;
        if let Some(session_id) = session_id.as_ref() {
            return Ok(session_id.clone());
        }
        let created = self
            .backend
            .open_session(OpenSessionRequest {
                profile: runtime_profile(self.profile),
                workspace_root: self.workspace_root.clone(),
                model: self.model.clone(),
                metadata: json!({"conversationId": conversation_id.as_str()}),
            })
            .await
            .map_err(kernel_error)?;
        *session_id = Some(created.clone());
        Ok(created)
    }
}

#[async_trait]
impl ConversationProvider for KernelConversationProvider {
    fn key(&self) -> &'static str {
        MAHAYANA_AI_PROVIDER_KEY
    }

    async fn list_conversations(&self) -> Result<Vec<Conversation>, ConversationError> {
        Ok(vec![Conversation::mahayana_assistant()])
    }

    async fn history(
        &self,
        conversation_id: &ConversationId,
        limit: u32,
    ) -> Result<Vec<Message>, ConversationError> {
        let history = self
            .history
            .lock()
            .map_err(|_| ConversationError::Provider("kernel history mutex poisoned".into()))?;
        let matching = history
            .iter()
            .filter(|message| &message.conversation_id == conversation_id)
            .cloned()
            .collect::<Vec<_>>();
        let start = matching.len().saturating_sub(limit as usize);
        Ok(matching[start..].to_vec())
    }

    async fn send_message(
        &self,
        request: SendMessageRequest,
        events: SharedConversationEventSink,
    ) -> Result<(), ConversationError> {
        let session_id = self.session_id(&request.conversation_id).await?;
        let user_message = Message {
            id: request
                .client_message_id
                .as_deref()
                .and_then(|id| MessageId::new(id).ok())
                .unwrap_or_else(|| MessageId::generated("message")),
            conversation_id: request.conversation_id.clone(),
            role: MessageRole::User,
            text: request.text.clone(),
            created_at_ms: now_ms(),
            metadata: json!({"runtime": "mahayana-kernel"}),
        };
        if !request.hidden {
            self.history
                .lock()
                .map_err(|_| ConversationError::Provider("kernel history mutex poisoned".into()))?
                .push(user_message);
        }

        let kernel_operation_id = KernelOperationId::from_string(request.operation_id.as_str());
        let sink: SharedKernelEventSink = Arc::new(RuntimeKernelEventBridge {
            conversation_id: request.conversation_id,
            operation_id: request.operation_id,
            events,
            history: Arc::clone(&self.history),
        });
        self.backend
            .run(
                RunRequest {
                    session_id,
                    operation_id: kernel_operation_id,
                    input: request.text,
                    policy: execution_policy(self.profile),
                    required_capabilities: CapabilitySet::new([Capability::Model]),
                    metadata: json!({"clientMessageId": request.client_message_id}),
                },
                sink,
            )
            .await
            .map_err(kernel_error)
    }

    async fn interrupt(&self, operation_id: &OperationId) -> Result<(), ConversationError> {
        self.backend
            .interrupt(&KernelOperationId::from_string(operation_id.as_str()))
            .await
            .map_err(kernel_error)
    }

    async fn resolve_approval(
        &self,
        request: ResolveApprovalRequest,
    ) -> Result<(), ConversationError> {
        self.backend
            .resolve_approval(ApprovalResolution {
                approval_id: request.approval_id.to_string(),
                approved: matches!(
                    request.decision,
                    ApprovalDecision::Accept | ApprovalDecision::AcceptForSession
                ),
                metadata: request.payload,
            })
            .await
            .map_err(kernel_error)
    }
}

struct RuntimeKernelEventBridge {
    conversation_id: ConversationId,
    operation_id: OperationId,
    events: SharedConversationEventSink,
    history: Arc<Mutex<Vec<Message>>>,
}

impl RuntimeKernelEventBridge {
    fn emit_runtime(&self, event: RuntimeEvent) -> Result<(), KernelError> {
        self.events
            .emit(event)
            .map_err(|error| KernelError::Backend(error.to_string()))
    }

    fn activity(
        &self,
        step_id: String,
        kind: String,
        title: String,
        detail: Option<String>,
        status: RuntimeActivityStatus,
        metadata: Option<Value>,
    ) -> Result<(), KernelError> {
        self.emit_runtime(RuntimeEvent::AgentActivity {
            operation_id: self.operation_id.clone(),
            step_id,
            kind,
            title,
            detail,
            status,
            metadata,
        })
    }
}

impl KernelEventSink for RuntimeKernelEventBridge {
    fn emit(&self, event: KernelEvent) -> Result<(), KernelError> {
        match event {
            KernelEvent::MessageDelta { delta, .. } => {
                self.emit_runtime(RuntimeEvent::MessageDelta {
                    operation_id: self.operation_id.clone(),
                    conversation_id: self.conversation_id.clone(),
                    delta,
                })
            }
            KernelEvent::MessageCompleted { text, .. } => {
                let message = Message {
                    id: MessageId::generated("message"),
                    conversation_id: self.conversation_id.clone(),
                    role: MessageRole::Assistant,
                    text,
                    created_at_ms: now_ms(),
                    metadata: json!({"runtime": "mahayana-kernel"}),
                };
                self.history
                    .lock()
                    .map_err(|_| KernelError::Backend("kernel history mutex poisoned".into()))?
                    .push(message.clone());
                self.emit_runtime(RuntimeEvent::MessageCompleted {
                    operation_id: self.operation_id.clone(),
                    message,
                })
            }
            KernelEvent::UsageUpdated {
                input_tokens,
                output_tokens,
                ..
            } => {
                let total_tokens = input_tokens.saturating_add(output_tokens);
                self.emit_runtime(RuntimeEvent::ModelUsageUpdated {
                    operation_id: self.operation_id.clone(),
                    usage: ModelTokenUsageSnapshot {
                        total: None,
                        last: ModelTokenUsage {
                            total_tokens: i64::try_from(total_tokens).unwrap_or(i64::MAX),
                            input_tokens: i64::try_from(input_tokens).unwrap_or(i64::MAX),
                            cached_input_tokens: 0,
                            output_tokens: i64::try_from(output_tokens).unwrap_or(i64::MAX),
                            reasoning_output_tokens: 0,
                        },
                        model_context_window: None,
                    },
                })
            }
            KernelEvent::Activity {
                kind,
                title,
                detail,
                metadata,
                ..
            } => {
                let step_id = metadata
                    .get("stepId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("kernel-step:{}", self.operation_id));
                let status = metadata
                    .get("status")
                    .and_then(Value::as_str)
                    .map(runtime_activity_status)
                    .unwrap_or(RuntimeActivityStatus::Running);
                self.activity(step_id, kind, title, detail, status, Some(metadata))
            }
            KernelEvent::ToolStarted {
                tool, arguments, ..
            } => self.activity(
                format!("tool:{tool}"),
                "tool".into(),
                format!("Running {tool}"),
                None,
                RuntimeActivityStatus::Running,
                Some(json!({"tool": tool, "arguments": arguments})),
            ),
            KernelEvent::ToolCompleted {
                tool,
                output,
                success,
                ..
            } => self.activity(
                format!("tool:{tool}"),
                "tool".into(),
                format!("Completed {tool}"),
                None,
                if success {
                    RuntimeActivityStatus::Completed
                } else {
                    RuntimeActivityStatus::Failed
                },
                Some(json!({"tool": tool, "output": output, "success": success})),
            ),
            KernelEvent::ApprovalRequested {
                approval_id,
                title,
                risk,
                details,
                ..
            } => {
                let approval_id = ApprovalId::new(approval_id)
                    .map_err(|error| KernelError::Backend(error.to_string()))?;
                self.emit_runtime(RuntimeEvent::ApprovalRequested {
                    operation_id: self.operation_id.clone(),
                    approval_id,
                    title,
                    details: json!({"risk": risk, "details": details}),
                })
            }
            KernelEvent::CheckpointCreated {
                checkpoint_id,
                label,
                ..
            } => self.activity(
                format!("checkpoint:{checkpoint_id}"),
                "checkpoint".into(),
                label.unwrap_or_else(|| "Workspace checkpoint".into()),
                Some(checkpoint_id.clone()),
                RuntimeActivityStatus::Completed,
                Some(json!({"checkpointId": checkpoint_id})),
            ),
            KernelEvent::OperationCompleted { .. } => Ok(()),
            KernelEvent::OperationFailed {
                message, retryable, ..
            } => self.activity(
                format!("operation:{}", self.operation_id),
                "operation".into(),
                "Operation failed".into(),
                Some(message),
                RuntimeActivityStatus::Failed,
                Some(json!({"retryable": retryable})),
            ),
        }
    }
}

fn runtime_profile(profile: BuildProfile) -> RuntimeProfile {
    match profile {
        BuildProfile::DesktopFull => RuntimeProfile::DesktopFull,
        BuildProfile::MobileEmbedded => RuntimeProfile::MobileEmbedded,
        BuildProfile::WebWasm => RuntimeProfile::WebWasm,
    }
}

fn execution_policy(profile: BuildProfile) -> ExecutionPolicy {
    match profile {
        BuildProfile::DesktopFull => ExecutionPolicy::interactive_default(),
        BuildProfile::MobileEmbedded | BuildProfile::WebWasm => ExecutionPolicy::mobile_default(),
    }
}

fn runtime_activity_status(value: &str) -> RuntimeActivityStatus {
    match value {
        "completed" => RuntimeActivityStatus::Completed,
        "failed" => RuntimeActivityStatus::Failed,
        _ => RuntimeActivityStatus::Running,
    }
}

fn kernel_error(error: KernelError) -> ConversationError {
    match error {
        KernelError::OperationNotFound(id) => ConversationError::OperationNotFound(
            OperationId::new(id).unwrap_or_else(|_| OperationId::generated("operation")),
        ),
        KernelError::ApprovalNotFound(id) => ConversationError::ApprovalNotFound(
            ApprovalId::new(id).unwrap_or_else(|_| ApprovalId::generated("approval")),
        ),
        other => ConversationError::Provider(other.to_string()),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
