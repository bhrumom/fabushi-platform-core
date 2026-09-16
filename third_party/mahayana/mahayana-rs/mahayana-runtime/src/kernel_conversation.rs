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
    KernelEvent, KernelEventSink, OpenSessionRequest, OperationId as KernelOperationId, RunRequest,
    RuntimeProfile, SessionId, SharedKernelEventSink,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as AsyncMutex;

// FeatureHost's explicit conversation.open contract requests 200 messages. The
// background search/index path asks for 2,000 and Runtime clamps that request to
// 500. Keep read acknowledgement tied to the explicit-open request so a
// background history scan can never clear a real unread assistant reply.
const OPEN_CONVERSATION_HISTORY_LIMIT: u32 = 200;

struct ConversationState {
    history: Vec<Message>,
    read_through_by_conversation: BTreeMap<String, usize>,
}

impl ConversationState {
    fn new(history: Vec<Message>) -> Self {
        let mut read_through_by_conversation = BTreeMap::new();
        for message in &history {
            *read_through_by_conversation
                .entry(message.conversation_id.as_str().to_string())
                .or_insert(0) += 1;
        }
        Self {
            history,
            read_through_by_conversation,
        }
    }

    fn unread_count(&self, conversation_id: &ConversationId) -> u32 {
        let read_through = self
            .read_through_by_conversation
            .get(conversation_id.as_str())
            .copied()
            .unwrap_or_default();
        self.history
            .iter()
            .filter(|message| &message.conversation_id == conversation_id)
            .skip(read_through)
            .filter(|message| message.role == MessageRole::Assistant)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    fn mark_read(&mut self, conversation_id: &ConversationId) {
        let visible_message_count = self
            .history
            .iter()
            .filter(|message| &message.conversation_id == conversation_id)
            .count();
        self.read_through_by_conversation
            .insert(conversation_id.as_str().to_string(), visible_message_count);
    }

    fn clear(&mut self) {
        self.history.clear();
        self.read_through_by_conversation.clear();
    }

    fn record_assistant_completion(&mut self, message: Message, hidden: bool) -> bool {
        if hidden {
            return false;
        }
        self.history.push(message);
        true
    }
}

fn history_request_marks_read(limit: u32) -> bool {
    limit == OPEN_CONVERSATION_HISTORY_LIMIT
}

pub struct KernelConversationProvider {
    backend: Arc<dyn EngineBackend>,
    profile: BuildProfile,
    workspace_root: Option<String>,
    model: Option<String>,
    session_id: AsyncMutex<Option<SessionId>>,
    state: Arc<Mutex<ConversationState>>,
    history_path: Option<PathBuf>,
}

impl KernelConversationProvider {
    pub fn new(
        backend: Arc<dyn EngineBackend>,
        profile: BuildProfile,
        workspace_root: Option<String>,
        model: Option<String>,
        history_path: Option<PathBuf>,
    ) -> Self {
        let history = history_path
            .as_deref()
            .map(load_history)
            .unwrap_or_default();
        Self {
            backend,
            profile,
            workspace_root,
            model,
            session_id: AsyncMutex::new(None),
            state: Arc::new(Mutex::new(ConversationState::new(history))),
            history_path,
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
        let history = self
            .state
            .lock()
            .map_err(|_| ConversationError::Provider("kernel conversation state mutex poisoned".into()))?
            .history
            .iter()
            .filter(|message| &message.conversation_id == conversation_id)
            .map(|message| json!({
                "id": message.id.as_str(),
                "role": match &message.role { MessageRole::Assistant => "assistant", _ => "user" },
                "content": message.text.as_str(),
                "createdAtMs": message.created_at_ms,
            }))
            .collect::<Vec<_>>();
        let transcript_updated_at_ms = history
            .last()
            .and_then(|message| message.get("createdAtMs"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let created = self
            .backend
            .open_session(OpenSessionRequest {
                profile: runtime_profile(self.profile),
                workspace_root: self.workspace_root.clone(),
                model: self.model.clone(),
                metadata: json!({
                    "conversationId": conversation_id.as_str(),
                    "bootstrapHistory": history,
                    "transcriptUpdatedAtMs": transcript_updated_at_ms,
                }),
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
        let conversation_id =
            ConversationId(mahayana_core::MAHAYANA_AI_CONVERSATION_ID.to_string());
        let unread_count = self
            .state
            .lock()
            .map_err(|_| {
                ConversationError::Provider("kernel conversation state mutex poisoned".into())
            })?
            .unread_count(&conversation_id);
        let mut conversation = Conversation::mahayana_assistant();
        conversation.unread_count = unread_count;
        Ok(vec![conversation])
    }

    async fn history(
        &self,
        conversation_id: &ConversationId,
        limit: u32,
    ) -> Result<Vec<Message>, ConversationError> {
        let mut state = self.state.lock().map_err(|_| {
            ConversationError::Provider("kernel conversation state mutex poisoned".into())
        })?;
        let matching = state
            .history
            .iter()
            .filter(|message| &message.conversation_id == conversation_id)
            .cloned()
            .collect::<Vec<_>>();
        let start = matching.len().saturating_sub(limit as usize);
        let messages = matching[start..].to_vec();
        if history_request_marks_read(limit) {
            state.mark_read(conversation_id);
        }
        Ok(messages)
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
            self.state
                .lock()
                .map_err(|_| {
                    ConversationError::Provider("kernel conversation state mutex poisoned".into())
                })?
                .history
                .push(user_message);
            persist_history(&self.state, self.history_path.as_deref()).map_err(kernel_error)?;
        }

        let kernel_operation_id = KernelOperationId::from_string(request.operation_id.as_str());
        let sink: SharedKernelEventSink = Arc::new(RuntimeKernelEventBridge {
            conversation_id: request.conversation_id,
            operation_id: request.operation_id,
            events,
            state: Arc::clone(&self.state),
            history_path: self.history_path.clone(),
            hidden: request.hidden,
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

    async fn reset_session(&self) -> Result<(), ConversationError> {
        self.backend.reset_session().map_err(kernel_error)?;
        *self.session_id.lock().await = None;
        {
            let mut state = self.state.lock().map_err(|_| {
                ConversationError::Provider("kernel conversation state mutex poisoned".into())
            })?;
            state.clear();
        }
        persist_history(&self.state, self.history_path.as_deref()).map_err(kernel_error)
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
    state: Arc<Mutex<ConversationState>>,
    history_path: Option<PathBuf>,
    hidden: bool,
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
                let should_persist = self
                    .state
                    .lock()
                    .map_err(|_| {
                        KernelError::Backend("kernel conversation state mutex poisoned".into())
                    })?
                    .record_assistant_completion(message.clone(), self.hidden);
                if should_persist {
                    persist_history(&self.state, self.history_path.as_deref())?;
                }
                self.emit_runtime(RuntimeEvent::MessageCompleted {
                    operation_id: self.operation_id.clone(),
                    message,
                })
            }
            KernelEvent::UsageUpdated {
                total_tokens,
                input_tokens,
                cached_input_tokens,
                output_tokens,
                reasoning_output_tokens,
                ..
            } => self.emit_runtime(RuntimeEvent::ModelUsageUpdated {
                operation_id: self.operation_id.clone(),
                usage: ModelTokenUsageSnapshot {
                    total: None,
                    last: ModelTokenUsage {
                        total_tokens: i64::try_from(total_tokens).unwrap_or(i64::MAX),
                        input_tokens: i64::try_from(input_tokens).unwrap_or(i64::MAX),
                        cached_input_tokens: i64::try_from(cached_input_tokens).unwrap_or(i64::MAX),
                        output_tokens: i64::try_from(output_tokens).unwrap_or(i64::MAX),
                        reasoning_output_tokens: i64::try_from(reasoning_output_tokens)
                            .unwrap_or(i64::MAX),
                    },
                    model_context_window: None,
                },
            }),
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

fn load_history(path: &Path) -> Vec<Message> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<Message>>(&bytes).unwrap_or_default()
}

fn persist_history(
    state: &Arc<Mutex<ConversationState>>,
    path: Option<&Path>,
) -> Result<(), KernelError> {
    let Some(path) = path else {
        return Ok(());
    };
    let bytes = {
        let state = state
            .lock()
            .map_err(|_| KernelError::Backend("kernel conversation state mutex poisoned".into()))?;
        let start = state.history.len().saturating_sub(1_000);
        serde_json::to_vec(&state.history[start..])
            .map_err(|error| KernelError::Backend(error.to_string()))?
    };
    let parent = path
        .parent()
        .ok_or_else(|| KernelError::Backend("kernel history path has no parent".into()))?;
    std::fs::create_dir_all(parent).map_err(|error| KernelError::Backend(error.to_string()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temporary = path.with_extension(format!("json.{}.{nonce}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| KernelError::Backend(error.to_string()))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| KernelError::Backend(error.to_string()))?;
    replace_file(&temporary, path)
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<(), KernelError> {
    match std::fs::rename(temporary, destination) {
        Ok(()) => Ok(()),
        Err(_error) if destination.exists() => {
            std::fs::remove_file(destination)
                .map_err(|remove_error| KernelError::Backend(remove_error.to_string()))?;
            std::fs::rename(temporary, destination)
                .map_err(|rename_error| KernelError::Backend(rename_error.to_string()))
        }
        Err(error) => Err(KernelError::Backend(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(id: &str) -> ConversationId {
        ConversationId(id.to_string())
    }

    fn message(conversation_id: &ConversationId, role: MessageRole, text: &str) -> Message {
        Message {
            id: MessageId::generated("test-message"),
            conversation_id: conversation_id.clone(),
            role,
            text: text.to_string(),
            created_at_ms: 1,
            metadata: Value::Null,
        }
    }

    #[test]
    fn unread_counts_only_assistant_messages_after_per_conversation_read_boundary() {
        let assistant = conversation(mahayana_core::MAHAYANA_AI_CONVERSATION_ID);
        let research = conversation("codex:agent:research");
        let mut state = ConversationState::new(Vec::new());

        state
            .history
            .push(message(&assistant, MessageRole::User, "hello"));
        state
            .history
            .push(message(&assistant, MessageRole::Assistant, "reply one"));
        state
            .history
            .push(message(&research, MessageRole::Assistant, "research reply"));
        state
            .history
            .push(message(&assistant, MessageRole::Assistant, "reply two"));

        assert_eq!(state.unread_count(&assistant), 2);
        assert_eq!(state.unread_count(&research), 1);

        state.mark_read(&research);
        assert_eq!(state.unread_count(&assistant), 2);
        assert_eq!(state.unread_count(&research), 0);

        state.mark_read(&assistant);
        assert_eq!(state.unread_count(&assistant), 0);
    }

    #[test]
    fn persisted_history_starts_read_for_every_existing_conversation() {
        let assistant = conversation(mahayana_core::MAHAYANA_AI_CONVERSATION_ID);
        let research = conversation("codex:agent:research");
        let mut state = ConversationState::new(vec![
            message(&assistant, MessageRole::Assistant, "persisted assistant"),
            message(&research, MessageRole::Assistant, "persisted research"),
        ]);
        assert_eq!(state.unread_count(&assistant), 0);
        assert_eq!(state.unread_count(&research), 0);

        state
            .history
            .push(message(&assistant, MessageRole::User, "new prompt"));
        state
            .history
            .push(message(&assistant, MessageRole::Assistant, "fresh reply"));
        assert_eq!(state.unread_count(&assistant), 1);
        assert_eq!(state.unread_count(&research), 0);
    }

    #[test]
    fn hidden_assistant_completion_stays_out_of_visible_history_and_unread() {
        let assistant = conversation(mahayana_core::MAHAYANA_AI_CONVERSATION_ID);
        let mut state = ConversationState::new(Vec::new());

        assert!(!state.record_assistant_completion(
            message(&assistant, MessageRole::Assistant, "hidden reply"),
            true,
        ));
        assert!(state.history.is_empty());
        assert_eq!(state.unread_count(&assistant), 0);

        assert!(state.record_assistant_completion(
            message(&assistant, MessageRole::Assistant, "visible reply"),
            false,
        ));
        assert_eq!(state.history.len(), 1);
        assert_eq!(state.unread_count(&assistant), 1);
    }

    #[test]
    fn only_explicit_open_history_contract_marks_read() {
        assert!(history_request_marks_read(OPEN_CONVERSATION_HISTORY_LIMIT));
        assert!(!history_request_marks_read(500));
    }

    #[test]
    fn reset_clears_history_and_all_conversation_boundaries() {
        let assistant = conversation(mahayana_core::MAHAYANA_AI_CONVERSATION_ID);
        let research = conversation("codex:agent:research");
        let mut state = ConversationState::new(vec![
            message(&assistant, MessageRole::Assistant, "persisted assistant"),
            message(&research, MessageRole::Assistant, "persisted research"),
        ]);
        state.clear();
        assert!(state.history.is_empty());
        assert!(state.read_through_by_conversation.is_empty());
        assert_eq!(state.unread_count(&assistant), 0);
        assert_eq!(state.unread_count(&research), 0);
    }
}
