//! Mahayana-owned coding Agent engine.
//!
//! The engine owns the Agent loop, session state, policy/approval boundary,
//! workspace tools, checkpoints, memory, workflows, prompt queue, and
//! subagents. Model inference is injected through `mahayana-model`; Codex and
//! Grok Build are not protocol dependencies of this crate.

use async_trait::async_trait;
use mahayana_kernel::supervisor::{
    ApprovalLedger, ApprovalOutcome, ApprovalRecord, LoopDisposition, LoopPolicy, LoopState,
    PermissionDecision, PermissionKey, PermissionLedger, PermissionMemory,
};
use mahayana_kernel::telemetry::{RuntimeMetricsSnapshot, RuntimeTelemetry};
use mahayana_kernel::{
    ApprovalResolution, BackendDescriptor, Capability, CapabilitySet, EngineBackend,
    ExecutionPolicy, KernelError, KernelEvent, OpenSessionRequest, OperationId,
    ResumeOperationRequest, RiskLevel, RunRequest, SessionId,
    SessionSnapshot as KernelSessionSnapshot, SharedKernelEventSink, SuspendOperationRequest,
};
use mahayana_model::{
    ModelError, ModelEvent, ModelEventSink, ModelRequest, ModelRuntime, ModelUsage,
    SharedModelEventSink,
};
use mahayana_orchestrator::{
    HookEffect, HookPoint, HookRegistry, MemoryStore, PromptEntry, PromptPriority, PromptQueue,
    SubagentScheduler, Workflow,
};
use mahayana_workspace_engine::WorkspaceEngine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use uuid::Uuid;

mod web_research;
use web_research::{WebResearchClient, WebResearchConfig};

const MAIN_ASSISTANT_CONVERSATION_ID: &str = "mahayana-ai:agent:assistant";
const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_MODEL_TURNS: usize = 16;
const DEFAULT_APPROVAL_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Clone, Default)]
pub enum ProcessExecution {
    #[default]
    Host,
    LocalDocker {
        docker_path: PathBuf,
        image: String,
    },
}

#[derive(Debug, Clone)]
pub struct NativeEngineConfig {
    pub model: String,
    pub system_instructions: String,
    pub max_model_turns: usize,
    pub enable_process_tools: bool,
    pub approval_timeout_ms: u64,
    pub process_execution: ProcessExecution,
    pub session_state_path: Option<PathBuf>,
}

impl NativeEngineConfig {
    pub fn desktop(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_instructions: default_system_instructions(),
            max_model_turns: DEFAULT_MAX_MODEL_TURNS,
            enable_process_tools: true,
            approval_timeout_ms: DEFAULT_APPROVAL_TIMEOUT_MS,
            process_execution: ProcessExecution::Host,
            session_state_path: None,
        }
    }

    pub fn embedded(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_instructions: default_system_instructions(),
            max_model_turns: DEFAULT_MAX_MODEL_TURNS,
            enable_process_tools: false,
            approval_timeout_ms: DEFAULT_APPROVAL_TIMEOUT_MS,
            process_execution: ProcessExecution::Host,
            session_state_path: None,
        }
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.model.trim().is_empty() {
            return Err(KernelError::BackendUnavailable(
                "Mahayana native engine model must not be empty".into(),
            ));
        }
        if self.max_model_turns == 0 {
            return Err(KernelError::BackendUnavailable(
                "Mahayana native engine max_model_turns must be at least one".into(),
            ));
        }
        if self.approval_timeout_ms == 0 {
            return Err(KernelError::BackendUnavailable(
                "Mahayana native engine approval timeout must be positive".into(),
            ));
        }
        if let ProcessExecution::LocalDocker { docker_path, image } = &self.process_execution {
            if docker_path.as_os_str().is_empty() {
                return Err(KernelError::BackendUnavailable(
                    "Local Docker executable path must not be empty".into(),
                ));
            }
            if !is_pinned_container_image(image) {
                return Err(KernelError::BackendUnavailable(
                    "Local Docker image must be pinned by sha256 digest".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationAttemptState {
    Running,
    Suspended,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperationAttempt {
    id: String,
    operation_id: String,
    prompt_id: String,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    state: OperationAttemptState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeSession {
    workspace_root: Option<PathBuf>,
    history: Vec<Value>,
    prompt_queue: PromptQueue,
    #[serde(default)]
    active_prompt: Option<PromptEntry>,
    #[serde(default)]
    permissions: PermissionLedger,
    #[serde(default)]
    approvals: ApprovalLedger,
    #[serde(default)]
    loop_state: LoopState,
    #[serde(default)]
    attempts: Vec<OperationAttempt>,
    #[serde(default)]
    updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeSnapshotState {
    session: NativeSession,
    memory: MemoryStore,
    workflows: HashMap<String, Workflow>,
    subagents: SubagentScheduler,
    hooks: HookRegistry,
}

struct ApprovalWaiter {
    operation_id: String,
    sender: oneshot::Sender<ApprovalResolution>,
}

#[derive(Debug, Default)]
struct OperationControl {
    interrupted: AtomicBool,
    suspended: AtomicBool,
}

pub struct NativeEngine {
    model: Arc<dyn ModelRuntime>,
    config: NativeEngineConfig,
    sessions: Mutex<HashMap<String, Arc<AsyncMutex<NativeSession>>>>,
    active_operations: Mutex<HashMap<String, Arc<OperationControl>>>,
    approvals: Mutex<HashMap<String, ApprovalWaiter>>,
    memory: Mutex<MemoryStore>,
    workflows: Mutex<HashMap<String, Workflow>>,
    subagents: Mutex<SubagentScheduler>,
    hooks: Mutex<HookRegistry>,
    telemetry: Arc<RuntimeTelemetry>,
    persisted_sessions: Mutex<HashMap<String, PathBuf>>,
    web_research: Option<WebResearchClient>,
}

impl NativeEngine {
    pub fn new(
        model: Arc<dyn ModelRuntime>,
        config: NativeEngineConfig,
    ) -> Result<Self, KernelError> {
        let web_config = WebResearchConfig::tinyfish_from_env();
        Self::new_with_web_config(model, config, web_config)
    }

    fn new_with_web_config(
        model: Arc<dyn ModelRuntime>,
        config: NativeEngineConfig,
        web_config: Option<WebResearchConfig>,
    ) -> Result<Self, KernelError> {
        config.validate()?;
        let web_research = web_config.map(WebResearchClient::new).transpose()?;
        Ok(Self {
            model,
            config,
            sessions: Mutex::new(HashMap::new()),
            active_operations: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            memory: Mutex::new(MemoryStore::default()),
            workflows: Mutex::new(HashMap::new()),
            subagents: Mutex::new(
                SubagentScheduler::new(4)
                    .map_err(|error| KernelError::Backend(error.to_string()))?,
            ),
            hooks: Mutex::new(HookRegistry::default()),
            telemetry: Arc::new(RuntimeTelemetry::default()),
            persisted_sessions: Mutex::new(HashMap::new()),
            web_research,
        })
    }

    pub fn metrics_snapshot(&self) -> RuntimeMetricsSnapshot {
        self.telemetry.snapshot()
    }

    /// Clears all account-bound native Agent state. The active operation flags
    /// are set before the registries are cleared so in-flight work exits at
    /// its next cancellation checkpoint instead of crossing an account
    /// boundary.
    pub fn reset_session(&self) -> Result<(), KernelError> {
        {
            let operations = self
                .active_operations
                .lock()
                .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?;
            for control in operations.values() {
                control.interrupted.store(true, Ordering::SeqCst);
            }
        }
        self.active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .clear();
        self.approvals
            .lock()
            .map_err(|_| KernelError::Backend("approval registry poisoned".into()))?
            .clear();
        self.sessions
            .lock()
            .map_err(|_| KernelError::Backend("native session registry poisoned".into()))?
            .clear();
        self.persisted_sessions
            .lock()
            .map_err(|_| KernelError::Backend("persisted session registry poisoned".into()))?
            .clear();
        *self
            .memory
            .lock()
            .map_err(|_| KernelError::Backend("memory store poisoned".into()))? =
            MemoryStore::default();
        self.workflows
            .lock()
            .map_err(|_| KernelError::Backend("workflow store poisoned".into()))?
            .clear();
        *self
            .subagents
            .lock()
            .map_err(|_| KernelError::Backend("subagent scheduler poisoned".into()))? =
            SubagentScheduler::default();
        if let Some(path) = self.config.session_state_path.as_deref() {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(KernelError::Backend(error.to_string())),
            }
        }
        Ok(())
    }

    pub fn register_hook(
        &self,
        point: HookPoint,
        priority: i32,
        matcher: Option<String>,
        effect: HookEffect,
    ) -> Result<String, KernelError> {
        Ok(self
            .hooks
            .lock()
            .map_err(|_| KernelError::Backend("hook registry poisoned".into()))?
            .register(point, priority, matcher, effect))
    }

    pub async fn set_permission_memory(
        &self,
        session_id: &SessionId,
        key: PermissionKey,
        memory: PermissionMemory,
    ) -> Result<(), KernelError> {
        let session = self.session(session_id)?;
        session.lock().await.permissions.remember(key, memory);
        Ok(())
    }

    fn capabilities(&self) -> CapabilitySet {
        let mut capabilities = vec![
            Capability::Model,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::Workspace,
            Capability::Checkpoint,
            Capability::Worktree,
            Capability::CodebaseGraph,
            Capability::Memory,
            Capability::Workflow,
            Capability::PromptQueue,
            Capability::Subagent,
            Capability::Hooks,
            Capability::ToolProtocol,
        ];
        if !self.model.is_local() || self.web_research.is_some() {
            capabilities.push(Capability::Network);
        }
        if self.web_research.is_some() {
            capabilities.push(Capability::WebSearch);
        }
        if self.config.enable_process_tools {
            capabilities.push(Capability::Process);
            capabilities.push(Capability::Git);
        }
        CapabilitySet::new(capabilities)
    }

    fn session(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<AsyncMutex<NativeSession>>, KernelError> {
        self.sessions
            .lock()
            .map_err(|_| KernelError::Backend("native session registry poisoned".into()))?
            .get(session_id.as_str())
            .cloned()
            .ok_or_else(|| KernelError::SessionNotFound(session_id.as_str().to_string()))
    }

    fn register_operation(
        &self,
        operation_id: &OperationId,
    ) -> Result<Arc<OperationControl>, KernelError> {
        let control = Arc::new(OperationControl::default());
        self.active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .insert(operation_id.as_str().to_string(), Arc::clone(&control));
        Ok(control)
    }

    fn finish_operation(&self, operation_id: &OperationId) -> Result<(), KernelError> {
        self.active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .remove(operation_id.as_str());
        Ok(())
    }

    fn cancel_operation_approvals(&self, operation_id: &OperationId) -> Result<(), KernelError> {
        self.approvals
            .lock()
            .map_err(|_| KernelError::Backend("approval registry poisoned".into()))?
            .retain(|_, waiter| waiter.operation_id != operation_id.as_str());
        Ok(())
    }

    async fn persist_session_if_configured(
        &self,
        session_id: &SessionId,
    ) -> Result<(), KernelError> {
        let path = self
            .persisted_sessions
            .lock()
            .map_err(|_| KernelError::Backend("persisted session registry poisoned".into()))?
            .get(session_id.as_str())
            .cloned();
        let Some(path) = path else {
            return Ok(());
        };
        let snapshot = self.snapshot_session(session_id).await?;
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|error| KernelError::Backend(error.to_string()))?;
        let parent = path
            .parent()
            .ok_or_else(|| KernelError::Backend("persisted session path has no parent".into()))?;
        std::fs::create_dir_all(parent).map_err(|error| KernelError::Backend(error.to_string()))?;
        let temporary = path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
        write_private_file(&temporary, &bytes)?;
        replace_file(&temporary, &path)?;
        Ok(())
    }

    fn apply_hooks(
        &self,
        point: HookPoint,
        subject: &str,
        mut session: Option<&mut NativeSession>,
        operation_id: &OperationId,
        events: &SharedKernelEventSink,
    ) -> Result<(), KernelError> {
        let hooks = self
            .hooks
            .lock()
            .map_err(|_| KernelError::Backend("hook registry poisoned".into()))?
            .dispatch(point, subject);
        for hook in hooks {
            events.emit(KernelEvent::Activity {
                operation_id: operation_id.clone(),
                kind: "hook".into(),
                title: format!("Mahayana hook {}", hook.id),
                detail: hook.effect.block_reason.clone(),
                metadata: json!({"point": format!("{point:?}"), "hookId": hook.id}),
            })?;
            if let Some(reason) = hook.effect.block_reason {
                return Err(KernelError::PolicyDenied(format!(
                    "hook blocked {subject}: {reason}"
                )));
            }
            if hook.effect.require_approval {
                return Err(KernelError::PolicyDenied(format!(
                    "hook requires explicit approval before {subject}"
                )));
            }
            if let Some(context) = hook.effect.inject_context
                && let Some(active_session) = session.as_deref_mut()
            {
                active_session
                    .history
                    .push(json!({"role":"system", "content": context, "source":"mahayana_hook"}));
            }
        }
        Ok(())
    }

    async fn run_prompt(
        &self,
        session: &mut NativeSession,
        operation_id: &OperationId,
        prompt: String,
        append_user_prompt: bool,
        policy: &ExecutionPolicy,
        control: &OperationControl,
        events: SharedKernelEventSink,
    ) -> Result<String, KernelError> {
        if append_user_prompt {
            session
                .history
                .push(json!({"role": "user", "content": prompt}));
        }

        // Some OpenAI-compatible providers silently turn an explicit tool
        // request into prose even when tools are present in the request. Keep
        // the execution contract on the Agent side: when the user names
        // concrete Mahayana tools, derive a small ordered plan and only use
        // it when the model omitted the corresponding function call. The plan
        // still goes through authorization, execute_tool, loop protection,
        // and function_call_output history, so the UI reflects real work.
        let mut explicit_tool_plan = explicit_tool_request_plan(&prompt);
        let mut last_workflow_id: Option<String> = None;

        for turn in 0..self.config.max_model_turns {
            ensure_operation_active(control)?;
            if !self.model.is_local() && !policy.allow_network {
                return Err(KernelError::PolicyDenied(
                    "remote model inference is disabled by Mahayana policy".into(),
                ));
            }

            self.apply_hooks(
                HookPoint::BeforeModel,
                &self.config.model,
                Some(session),
                operation_id,
                &events,
            )?;
            events.emit(KernelEvent::Activity {
                operation_id: operation_id.clone(),
                kind: "model".into(),
                title: format!("Mahayana reasoning turn {}", turn + 1),
                detail: None,
                metadata: json!({"engine": "mahayana-native"}),
            })?;

            let collector = Arc::new(ModelCollector::streaming(
                Arc::clone(&events),
                operation_id.clone(),
            ));
            let sink: SharedModelEventSink = collector.clone();
            let started = Instant::now();
            let inference = self
                .model
                .infer(
                    ModelRequest {
                        model: self.config.model.clone(),
                        input: Value::Array(session.history.clone()),
                        metadata: json!({
                            "instructions": self.config.system_instructions,
                            "tools": tool_definitions(
                                self.config.enable_process_tools,
                                self.web_research.is_some(),
                            ),
                            "tool_choice": "auto",
                            "parallel_tool_calls": false,
                        }),
                    },
                    sink,
                )
                .await;
            self.telemetry
                .model_finished(started.elapsed(), inference.is_ok());
            inference.map_err(model_error)?;
            ensure_operation_active(control)?;
            self.apply_hooks(
                HookPoint::AfterModel,
                &self.config.model,
                Some(session),
                operation_id,
                &events,
            )?;

            if let Some(usage) = collector.usage()? {
                events.emit(KernelEvent::UsageUpdated {
                    operation_id: operation_id.clone(),
                    total_tokens: usage.total_tokens,
                    input_tokens: usage.input_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    output_tokens: usage.output_tokens,
                    reasoning_output_tokens: usage.reasoning_output_tokens,
                })?;
            }
            let payload = collector.output()?.ok_or_else(|| {
                KernelError::Backend("model runtime completed without a payload".into())
            })?;
            append_model_output(&mut session.history, &payload);
            let mut calls = extract_function_calls(&payload)?;
            if calls.is_empty()
                && let Some(call) = explicit_tool_plan.first().cloned()
            {
                explicit_tool_plan.remove(0);
                calls.push(call);
            }
            if !calls.is_empty() {
                let called_names = calls
                    .iter()
                    .map(|call| call.name.as_str())
                    .collect::<std::collections::HashSet<_>>();
                explicit_tool_plan.retain(|planned| !called_names.contains(planned.name.as_str()));
            }
            if calls.is_empty() {
                let text = mahayana_model::responses::extract_output_text(&payload)
                    .or_else(|| collector.text().ok().filter(|text| !text.is_empty()))
                    .ok_or_else(|| {
                        KernelError::Backend(
                            "model completed without assistant text or tool calls".into(),
                        )
                    })?;
                events.emit(KernelEvent::MessageDelta {
                    operation_id: operation_id.clone(),
                    delta: text.clone(),
                })?;
                events.emit(KernelEvent::MessageCompleted {
                    operation_id: operation_id.clone(),
                    text: text.clone(),
                })?;
                return Ok(text);
            }

            for call in calls {
                ensure_operation_active(control)?;
                let mut call = call;
                if call.name == "workflow_status"
                    && call.arguments.get("workflow_id").and_then(Value::as_str)
                        == Some("$last_workflow_id")
                    && let Some(workflow_id) = last_workflow_id.clone()
                {
                    call.arguments["workflow_id"] = Value::String(workflow_id);
                }
                let fingerprint = tool_fingerprint(&call);
                match session
                    .loop_state
                    .observe(&fingerprint, LoopPolicy::default())
                {
                    LoopDisposition::Allow => {}
                    LoopDisposition::Warn => {
                        events.emit(KernelEvent::Activity {
                            operation_id: operation_id.clone(),
                            kind: "loop_warning".into(),
                            title: format!("Repeated tool action: {}", call.name),
                            detail: Some("Mahayana detected a repeated action and will interrupt if it continues".into()),
                            metadata: json!({"fingerprint": fingerprint}),
                        })?;
                    }
                    LoopDisposition::Interrupt => {
                        return Err(KernelError::PolicyDenied(format!(
                            "Mahayana loop protection interrupted repeated tool action {}",
                            call.name
                        )));
                    }
                }

                self.apply_hooks(
                    HookPoint::BeforeTool,
                    &call.name,
                    Some(session),
                    operation_id,
                    &events,
                )?;
                self.telemetry.tool_started();
                events.emit(KernelEvent::ToolStarted {
                    operation_id: operation_id.clone(),
                    tool: call.name.clone(),
                    arguments: call.arguments.clone(),
                })?;
                let output = self
                    .execute_tool(
                        session,
                        operation_id,
                        &call,
                        policy,
                        control,
                        Arc::clone(&events),
                    )
                    .await;
                match output {
                    Ok(output) => {
                        if call.name == "workflow_create" {
                            last_workflow_id = output
                                .get("workflow_id")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                        }
                        self.telemetry.tool_completed(true);
                        events.emit(KernelEvent::ToolCompleted {
                            operation_id: operation_id.clone(),
                            tool: call.name.clone(),
                            output: output.clone(),
                            success: true,
                        })?;
                        session.history.push(json!({
                            "type": "function_call_output",
                            "call_id": call.call_id,
                            "output": serde_json::to_string(&output)
                                .unwrap_or_else(|_| "null".into()),
                        }));
                    }
                    Err(error) => {
                        self.telemetry.tool_completed(false);
                        let message = error.to_string();
                        events.emit(KernelEvent::ToolCompleted {
                            operation_id: operation_id.clone(),
                            tool: call.name.clone(),
                            output: json!({"error": message}),
                            success: false,
                        })?;
                        session.history.push(json!({
                            "type": "function_call_output",
                            "call_id": call.call_id,
                            "output": serde_json::to_string(&json!({"error": message}))
                                .unwrap_or_else(|_| "null".into()),
                        }));
                    }
                }
                self.apply_hooks(
                    HookPoint::AfterTool,
                    &call.name,
                    Some(session),
                    operation_id,
                    &events,
                )?;
            }
        }

        Err(KernelError::Backend(format!(
            "native Agent exceeded {} model turns",
            self.config.max_model_turns
        )))
    }

    fn execute_tool<'a>(
        &'a self,
        session: &'a mut NativeSession,
        operation_id: &'a OperationId,
        call: &'a FunctionCall,
        policy: &'a ExecutionPolicy,
        control: &'a OperationControl,
        events: SharedKernelEventSink,
    ) -> Pin<Box<dyn Future<Output = Result<Value, KernelError>> + Send + 'a>> {
        Box::pin(async move {
            let risk = tool_risk(&call.name);
            self.authorize_tool(
                session,
                operation_id,
                call,
                risk,
                policy,
                Arc::clone(&events),
            )
            .await?;
            ensure_operation_active(control)?;

            match call.name.as_str() {
                "send_message" => {
                    let message = string_arg(&call.arguments, "message")?.trim();
                    if message.is_empty() {
                        return Err(KernelError::Backend(
                            "send_message requires a non-empty message".into(),
                        ));
                    }
                    events.emit(KernelEvent::MessageDelta {
                        operation_id: operation_id.clone(),
                        delta: message.to_string(),
                    })?;
                    events.emit(KernelEvent::MessageCompleted {
                        operation_id: operation_id.clone(),
                        text: message.to_string(),
                    })?;
                    Ok(json!({"delivered": true, "characters": message.chars().count()}))
                }
                "workspace_read" => {
                    let root = workspace_root(session)?;
                    let path = string_arg(&call.arguments, "path")?;
                    let path = safe_join(root, Path::new(path))?;
                    let content = std::fs::read_to_string(&path)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    Ok(json!({"path": relative_display(root, &path), "content": content}))
                }
                "workspace_write" => {
                    if !policy.allow_workspace_writes {
                        return Err(KernelError::PolicyDenied(
                            "workspace writes are disabled by Mahayana policy".into(),
                        ));
                    }
                    self.apply_hooks(
                        HookPoint::BeforeCheckpoint,
                        "workspace_write",
                        None,
                        operation_id,
                        &events,
                    )?;
                    let root = workspace_root(session)?;
                    let relative = string_arg(&call.arguments, "path")?;
                    let content = string_arg(&call.arguments, "content")?;
                    let engine = WorkspaceEngine::open(root)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    let checkpoint = engine
                        .create_checkpoint(Some(format!("before write {relative}")))
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    events.emit(KernelEvent::CheckpointCreated {
                        operation_id: operation_id.clone(),
                        checkpoint_id: checkpoint.id,
                        label: checkpoint.label,
                    })?;
                    self.apply_hooks(
                        HookPoint::AfterCheckpoint,
                        "workspace_write",
                        None,
                        operation_id,
                        &events,
                    )?;
                    let path = safe_join(root, Path::new(relative))?;
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|error| KernelError::Backend(error.to_string()))?;
                    }
                    std::fs::write(&path, content)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    Ok(json!({"path": relative_display(root, &path), "bytes": content.len()}))
                }
                "workspace_search" => {
                    let root = workspace_root(session)?;
                    let query = string_arg(&call.arguments, "query")?;
                    let limit = call
                        .arguments
                        .get("limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(50)
                        .clamp(1, 200) as usize;
                    let matches = search_workspace(root, query, limit)?;
                    Ok(json!({"query": query, "matches": matches}))
                }
                "workspace_checkpoint" => {
                    self.apply_hooks(
                        HookPoint::BeforeCheckpoint,
                        "workspace_checkpoint",
                        None,
                        operation_id,
                        &events,
                    )?;
                    let root = workspace_root(session)?;
                    let label = call
                        .arguments
                        .get("label")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let checkpoint = WorkspaceEngine::open(root)
                        .and_then(|engine| engine.create_checkpoint(label))
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    events.emit(KernelEvent::CheckpointCreated {
                        operation_id: operation_id.clone(),
                        checkpoint_id: checkpoint.id.clone(),
                        label: checkpoint.label.clone(),
                    })?;
                    self.apply_hooks(
                        HookPoint::AfterCheckpoint,
                        "workspace_checkpoint",
                        None,
                        operation_id,
                        &events,
                    )?;
                    serde_json::to_value(checkpoint)
                        .map_err(|error| KernelError::Backend(error.to_string()))
                }
                "workspace_restore" => {
                    if !policy.allow_workspace_writes {
                        return Err(KernelError::PolicyDenied(
                            "workspace writes are disabled by Mahayana policy".into(),
                        ));
                    }
                    self.apply_hooks(
                        HookPoint::BeforeCheckpoint,
                        "workspace_restore",
                        None,
                        operation_id,
                        &events,
                    )?;
                    let root = workspace_root(session)?;
                    let checkpoint_id = string_arg(&call.arguments, "checkpoint_id")?;
                    let engine = WorkspaceEngine::open(root)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    let safety = engine
                        .create_checkpoint(Some(format!("before restore {checkpoint_id}")))
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    events.emit(KernelEvent::CheckpointCreated {
                        operation_id: operation_id.clone(),
                        checkpoint_id: safety.id,
                        label: safety.label,
                    })?;
                    self.apply_hooks(
                        HookPoint::AfterCheckpoint,
                        "workspace_restore",
                        None,
                        operation_id,
                        &events,
                    )?;
                    let restored = engine
                        .restore_checkpoint(checkpoint_id)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    Ok(json!({"restored": restored.id, "files": restored.files.len()}))
                }
                "workspace_worktree" => {
                    let root = workspace_root(session)?;
                    let checkpoint_id = call.arguments.get("checkpoint_id").and_then(Value::as_str);
                    let worktree = WorkspaceEngine::open(root)
                        .and_then(|engine| engine.create_worktree(checkpoint_id))
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    serde_json::to_value(worktree)
                        .map_err(|error| KernelError::Backend(error.to_string()))
                }
                "codebase_graph" => {
                    let root = workspace_root(session)?;
                    let graph = WorkspaceEngine::open(root)
                        .and_then(|engine| engine.build_codebase_graph())
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    serde_json::to_value(graph)
                        .map_err(|error| KernelError::Backend(error.to_string()))
                }
                "code_symbols" => {
                    let root = workspace_root(session)?;
                    let symbols = WorkspaceEngine::open(root)
                        .and_then(|engine| engine.index_symbols())
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    serde_json::to_value(symbols)
                        .map_err(|error| KernelError::Backend(error.to_string()))
                }
                "memory_put" => {
                    let namespace = string_arg(&call.arguments, "namespace")?;
                    let key = string_arg(&call.arguments, "key")?;
                    let value = call.arguments.get("value").cloned().unwrap_or(Value::Null);
                    let tags = call
                        .arguments
                        .get("tags")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    let record = self
                        .memory
                        .lock()
                        .map_err(|_| KernelError::Backend("memory store poisoned".into()))?
                        .upsert(namespace, key, value, tags, None)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    serde_json::to_value(record)
                        .map_err(|error| KernelError::Backend(error.to_string()))
                }
                "memory_get" => {
                    let namespace = string_arg(&call.arguments, "namespace")?;
                    let key = string_arg(&call.arguments, "key")?;
                    let record = self
                        .memory
                        .lock()
                        .map_err(|_| KernelError::Backend("memory store poisoned".into()))?
                        .get(namespace, key);
                    Ok(json!({"record": record}))
                }
                "memory_search" => {
                    let query = call.arguments.get("query").and_then(Value::as_str);
                    let namespace = call.arguments.get("namespace").and_then(Value::as_str);
                    let tags = call
                        .arguments
                        .get("tags")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    let records = self
                        .memory
                        .lock()
                        .map_err(|_| KernelError::Backend("memory store poisoned".into()))?
                        .search(namespace, query, &tags, 50);
                    Ok(json!({"records": records}))
                }
                "workflow_create" => {
                    let title = string_arg(&call.arguments, "title")?;
                    let mut workflow = Workflow::new(title)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    if let Some(tasks) = call.arguments.get("tasks").and_then(Value::as_array) {
                        for task in tasks {
                            let id = task.get("id").and_then(Value::as_str).ok_or_else(|| {
                                KernelError::Backend("workflow task id is required".into())
                            })?;
                            let task_title =
                                task.get("title").and_then(Value::as_str).unwrap_or(id);
                            let dependencies = task
                                .get("depends_on")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect::<Vec<_>>();
                            workflow
                                .add_task(id, task_title, dependencies, Value::Null)
                                .map_err(|error| KernelError::Backend(error.to_string()))?;
                        }
                    }
                    let id = workflow.id.clone();
                    let snapshot = serde_json::to_value(&workflow)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    self.workflows
                        .lock()
                        .map_err(|_| KernelError::Backend("workflow store poisoned".into()))?
                        .insert(id.clone(), workflow);
                    Ok(json!({"workflow_id": id, "workflow": snapshot}))
                }
                "workflow_status" => {
                    let id = string_arg(&call.arguments, "workflow_id")?;
                    let workflows = self
                        .workflows
                        .lock()
                        .map_err(|_| KernelError::Backend("workflow store poisoned".into()))?;
                    let workflow = workflows
                        .get(id)
                        .ok_or_else(|| KernelError::Backend(format!("workflow not found: {id}")))?;
                    serde_json::to_value(workflow)
                        .map_err(|error| KernelError::Backend(error.to_string()))
                }
                "subagent_run" => {
                    let name = call
                        .arguments
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("subagent");
                    let goal = string_arg(&call.arguments, "goal")?;
                    let task_id = {
                        let mut scheduler = self.subagents.lock().map_err(|_| {
                            KernelError::Backend("subagent scheduler poisoned".into())
                        })?;
                        let task_id = scheduler
                            .spawn(
                                None,
                                name,
                                goal,
                                CapabilitySet::new([Capability::Model]),
                                Value::Null,
                            )
                            .map_err(|error| KernelError::Backend(error.to_string()))?;
                        scheduler
                            .start(&task_id)
                            .map_err(|error| KernelError::Backend(error.to_string()))?;
                        task_id
                    };
                    let result = self
                        .run_subagent(goal, control, Arc::clone(&events), operation_id)
                        .await;
                    match result {
                        Ok(text) => {
                            self.subagents
                                .lock()
                                .map_err(|_| {
                                    KernelError::Backend("subagent scheduler poisoned".into())
                                })?
                                .complete(&task_id, json!({"text": text}))
                                .map_err(|error| KernelError::Backend(error.to_string()))?;
                            Ok(json!({"task_id": task_id, "text": text}))
                        }
                        Err(error) => {
                            let message = error.to_string();
                            let _ = self.subagents.lock().ok().and_then(|mut scheduler| {
                                scheduler.fail(&task_id, message.clone()).ok()
                            });
                            Err(error)
                        }
                    }
                }
                "web_search" => {
                    if !policy.allow_network {
                        return Err(KernelError::PolicyDenied(
                            "web search is disabled by Mahayana network policy".into(),
                        ));
                    }
                    let query = string_arg(&call.arguments, "query")?;
                    let limit = call
                        .arguments
                        .get("limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(10)
                        .clamp(1, 10) as usize;
                    self.web_research
                        .as_ref()
                        .ok_or_else(|| {
                            KernelError::CapabilityUnavailable(
                                "web research provider is not configured".into(),
                            )
                        })?
                        .search(query, limit)
                        .await
                }
                "web_fetch" => {
                    if !policy.allow_network {
                        return Err(KernelError::PolicyDenied(
                            "web fetch is disabled by Mahayana network policy".into(),
                        ));
                    }
                    let urls = call
                        .arguments
                        .get("urls")
                        .and_then(Value::as_array)
                        .ok_or_else(|| KernelError::Backend("web_fetch urls are required".into()))?
                        .iter()
                        .map(|value| {
                            value.as_str().map(str::to_owned).ok_or_else(|| {
                                KernelError::Backend(
                                    "web_fetch urls must contain only strings".into(),
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let format = call
                        .arguments
                        .get("format")
                        .and_then(Value::as_str)
                        .unwrap_or("markdown");
                    self.web_research
                        .as_ref()
                        .ok_or_else(|| {
                            KernelError::CapabilityUnavailable(
                                "web research provider is not configured".into(),
                            )
                        })?
                        .fetch(&urls, format)
                        .await
                }
                "process_exec" => {
                    if !self.config.enable_process_tools || !policy.allow_process {
                        return Err(KernelError::PolicyDenied(
                            "process execution is disabled by Mahayana policy".into(),
                        ));
                    }
                    let root = workspace_root(session)?;
                    let program = string_arg(&call.arguments, "program")?;
                    let args = call
                        .arguments
                        .get("args")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    run_process(&self.config.process_execution, root, program, &args)
                }
                "git_status" => {
                    if !self.config.enable_process_tools || !policy.allow_process {
                        return Err(KernelError::PolicyDenied(
                            "Git process execution is disabled by Mahayana policy".into(),
                        ));
                    }
                    run_process(
                        &self.config.process_execution,
                        workspace_root(session)?,
                        "git",
                        &["status".into(), "--short".into()],
                    )
                }
                "git_diff" => {
                    if !self.config.enable_process_tools || !policy.allow_process {
                        return Err(KernelError::PolicyDenied(
                            "Git process execution is disabled by Mahayana policy".into(),
                        ));
                    }
                    run_process(
                        &self.config.process_execution,
                        workspace_root(session)?,
                        "git",
                        &["diff".into(), "--".into()],
                    )
                }
                other => Err(KernelError::CapabilityUnavailable(format!(
                    "native tool {other} is not registered"
                ))),
            }
        })
    }

    async fn run_subagent(
        &self,
        goal: &str,
        control: &OperationControl,
        events: SharedKernelEventSink,
        operation_id: &OperationId,
    ) -> Result<String, KernelError> {
        ensure_operation_active(control)?;
        let collector = Arc::new(ModelCollector::default());
        let sink: SharedModelEventSink = collector.clone();
        let started = Instant::now();
        let inference = self
            .model
            .infer(
                ModelRequest {
                    model: self.config.model.clone(),
                    input: json!([{"role":"user", "content": goal}]),
                    metadata: json!({
                        "instructions": "You are a focused Mahayana subagent. Solve only the delegated goal and return a concise result. Do not claim tools you were not given."
                    }),
                },
                sink,
            )
            .await;
        self.telemetry
            .model_finished(started.elapsed(), inference.is_ok());
        inference.map_err(model_error)?;
        ensure_operation_active(control)?;
        if let Some(usage) = collector.usage()? {
            events.emit(KernelEvent::UsageUpdated {
                operation_id: operation_id.clone(),
                total_tokens: usage.total_tokens,
                input_tokens: usage.input_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                output_tokens: usage.output_tokens,
                reasoning_output_tokens: usage.reasoning_output_tokens,
            })?;
        }
        let payload = collector
            .output()?
            .ok_or_else(|| KernelError::Backend("subagent returned no model payload".into()))?;
        mahayana_model::responses::extract_output_text(&payload)
            .or_else(|| collector.text().ok().filter(|text| !text.is_empty()))
            .ok_or_else(|| KernelError::Backend("subagent returned no output text".into()))
    }

    async fn authorize_tool(
        &self,
        session: &mut NativeSession,
        operation_id: &OperationId,
        call: &FunctionCall,
        risk: RiskLevel,
        policy: &ExecutionPolicy,
        events: SharedKernelEventSink,
    ) -> Result<(), KernelError> {
        let key = PermissionKey::new(tool_capability(&call.name), permission_target(call))
            .map_err(|error| KernelError::Backend(error.to_string()))?;
        match session.permissions.evaluate(policy, &key, risk) {
            PermissionDecision::Allow => return Ok(()),
            PermissionDecision::Deny => {
                return Err(KernelError::PolicyDenied(format!(
                    "{} is denied for {}",
                    call.name, key.target
                )));
            }
            PermissionDecision::Ask => {}
        }

        let approval_id = format!("approval:{}", Uuid::new_v4());
        let requested_at_ms = now_ms();
        let (sender, receiver) = oneshot::channel();
        self.approvals
            .lock()
            .map_err(|_| KernelError::Backend("approval registry poisoned".into()))?
            .insert(
                approval_id.clone(),
                ApprovalWaiter {
                    operation_id: operation_id.as_str().to_owned(),
                    sender,
                },
            );
        self.telemetry.approval_requested();
        events.emit(KernelEvent::ApprovalRequested {
            operation_id: operation_id.clone(),
            approval_id: approval_id.clone(),
            title: format!("Allow {}", call.name),
            risk,
            details: json!({
                "tool": call.name,
                "capability": format!("{:?}", key.capability),
                "target": key.target,
                "engine": "mahayana-native"
            }),
        })?;

        let result = tokio::time::timeout(
            Duration::from_millis(self.config.approval_timeout_ms),
            receiver,
        )
        .await;
        let resolved_at_ms = now_ms();
        let (outcome, memory) = match result {
            Ok(Ok(resolution)) => {
                let memory = permission_memory_from_metadata(&resolution.metadata)?;
                if resolution.approved {
                    self.telemetry.approval_approved();
                    (ApprovalOutcome::Approved, memory)
                } else {
                    self.telemetry.approval_rejected();
                    (ApprovalOutcome::Rejected, memory)
                }
            }
            Ok(Err(_)) => {
                self.telemetry.approval_interrupted();
                (ApprovalOutcome::Interrupted, None)
            }
            Err(_) => {
                self.approvals
                    .lock()
                    .map_err(|_| KernelError::Backend("approval registry poisoned".into()))?
                    .remove(&approval_id);
                self.telemetry.approval_timed_out();
                (ApprovalOutcome::TimedOut, None)
            }
        };

        let record = ApprovalRecord::new(
            approval_id,
            key,
            risk,
            requested_at_ms,
            resolved_at_ms,
            outcome,
            memory,
        )
        .map_err(|error| KernelError::Backend(error.to_string()))?;
        let decision = {
            let NativeSession {
                approvals,
                permissions,
                ..
            } = session;
            approvals
                .record(record, permissions)
                .map_err(|error| KernelError::Backend(error.to_string()))?
        };
        if decision == PermissionDecision::Allow {
            Ok(())
        } else {
            Err(KernelError::PolicyDenied(format!(
                "approval did not allow {}",
                call.name
            )))
        }
    }

    fn start_attempt(
        session: &mut NativeSession,
        operation_id: &OperationId,
        prompt_id: &str,
    ) -> String {
        let id = format!("native-attempt:{}", Uuid::new_v4());
        session.attempts.push(OperationAttempt {
            id: id.clone(),
            operation_id: operation_id.as_str().to_owned(),
            prompt_id: prompt_id.to_owned(),
            started_at_ms: now_ms(),
            finished_at_ms: None,
            state: OperationAttemptState::Running,
        });
        id
    }

    fn finish_attempt(session: &mut NativeSession, id: &str, state: OperationAttemptState) {
        if let Some(attempt) = session.attempts.iter_mut().find(|attempt| attempt.id == id) {
            attempt.finished_at_ms = Some(now_ms());
            attempt.state = state;
        }
    }

    async fn execute_active_prompt(
        &self,
        session: &mut NativeSession,
        operation_id: &OperationId,
        prompt: PromptEntry,
        append_user_prompt: bool,
        policy: &ExecutionPolicy,
        control: &OperationControl,
        events: SharedKernelEventSink,
    ) -> Result<(), KernelError> {
        let attempt_id = Self::start_attempt(session, operation_id, &prompt.id);
        let result = self
            .run_prompt(
                session,
                operation_id,
                prompt.text.clone(),
                append_user_prompt,
                policy,
                control,
                Arc::clone(&events),
            )
            .await;
        if control.suspended.load(Ordering::SeqCst) {
            Self::finish_attempt(session, &attempt_id, OperationAttemptState::Suspended);
            self.telemetry.operation_suspended();
            events.emit(KernelEvent::Activity {
                operation_id: operation_id.clone(),
                kind: "operation_suspended".into(),
                title: "Mahayana operation suspended".into(),
                detail: None,
                metadata: json!({"promptId": prompt.id}),
            })?;
            return Ok(());
        }
        match result {
            Ok(_) => {
                session
                    .prompt_queue
                    .complete(&prompt.id)
                    .map_err(|error| KernelError::Backend(error.to_string()))?;
                session.active_prompt = None;
                Self::finish_attempt(session, &attempt_id, OperationAttemptState::Completed);
                self.telemetry.operation_completed();
                events.emit(KernelEvent::OperationCompleted {
                    operation_id: operation_id.clone(),
                })?;
                Ok(())
            }
            Err(error) => {
                session
                    .prompt_queue
                    .cancel(&prompt.id)
                    .map_err(|queue_error| KernelError::Backend(queue_error.to_string()))?;
                session.active_prompt = None;
                let interrupted = control.interrupted.load(Ordering::SeqCst);
                Self::finish_attempt(
                    session,
                    &attempt_id,
                    if interrupted {
                        OperationAttemptState::Interrupted
                    } else {
                        OperationAttemptState::Failed
                    },
                );
                self.telemetry.operation_failed();
                events.emit(KernelEvent::OperationFailed {
                    operation_id: operation_id.clone(),
                    message: error.to_string(),
                    retryable: interrupted,
                })?;
                Err(error)
            }
        }
    }
}

#[async_trait]
impl EngineBackend for NativeEngine {
    fn descriptor(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "mahayana-native".into(),
            display_name: "Mahayana Native Engine".into(),
            native: true,
            capabilities: self.capabilities(),
        }
    }

    fn reset_session(&self) -> Result<(), KernelError> {
        NativeEngine::reset_session(self)
    }

    async fn open_session(&self, request: OpenSessionRequest) -> Result<SessionId, KernelError> {
        let persisted_path = request
            .metadata
            .get("conversationId")
            .and_then(Value::as_str)
            .filter(|conversation_id| *conversation_id == MAIN_ASSISTANT_CONVERSATION_ID)
            .and(self.config.session_state_path.clone());
        if let Some(path) = persisted_path.as_ref()
            && let Ok(bytes) = std::fs::read(path)
            && let Ok(snapshot) = serde_json::from_slice::<KernelSessionSnapshot>(&bytes)
        {
            let snapshot_updated_at_ms = snapshot
                .metadata
                .get("updatedAtMs")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let transcript_updated_at_ms = request
                .metadata
                .get("transcriptUpdatedAtMs")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if snapshot_updated_at_ms >= transcript_updated_at_ms {
                let session_id = self.restore_session(snapshot).await?;
                self.persisted_sessions
                    .lock()
                    .map_err(|_| {
                        KernelError::Backend("persisted session registry poisoned".into())
                    })?
                    .insert(session_id.as_str().to_owned(), path.clone());
                self.telemetry.session_opened();
                return Ok(session_id);
            }
        }
        let workspace_root = request
            .workspace_root
            .as_deref()
            .map(PathBuf::from)
            .map(|path| {
                path.canonicalize()
                    .map_err(|error| KernelError::Backend(error.to_string()))
            })
            .transpose()?;
        if let Some(root) = workspace_root.as_deref()
            && !root.is_dir()
        {
            return Err(KernelError::BackendUnavailable(format!(
                "workspace root is not a directory: {}",
                root.display()
            )));
        }
        let session_id = SessionId::new();
        self.sessions
            .lock()
            .map_err(|_| KernelError::Backend("native session registry poisoned".into()))?
            .insert(
                session_id.as_str().to_string(),
                Arc::new(AsyncMutex::new(NativeSession {
                    workspace_root,
                    history: native_bootstrap_history(&request.metadata),
                    prompt_queue: PromptQueue::default(),
                    active_prompt: None,
                    permissions: PermissionLedger::default(),
                    approvals: ApprovalLedger::default(),
                    loop_state: LoopState::default(),
                    attempts: Vec::new(),
                    updated_at_ms: request
                        .metadata
                        .get("transcriptUpdatedAtMs")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                })),
            );
        self.telemetry.session_opened();
        if let Some(path) = persisted_path {
            self.persisted_sessions
                .lock()
                .map_err(|_| KernelError::Backend("persisted session registry poisoned".into()))?
                .insert(session_id.as_str().to_owned(), path);
        }
        Ok(session_id)
    }

    async fn run(
        &self,
        request: RunRequest,
        events: SharedKernelEventSink,
    ) -> Result<(), KernelError> {
        if !self
            .capabilities()
            .supports_all(&request.required_capabilities)
        {
            return Err(KernelError::CapabilityUnavailable(
                "native engine does not satisfy the requested capability set".into(),
            ));
        }
        let session = self.session(&request.session_id)?;
        let control = self.register_operation(&request.operation_id)?;
        self.telemetry.operation_started();
        let result = async {
            let mut session = session.lock().await;
            if session.active_prompt.is_some() {
                return Err(KernelError::Backend(
                    "session already has a suspended or running prompt; resume it before enqueuing another user prompt".into(),
                ));
            }
            let prompt_id = session
                .prompt_queue
                .enqueue(
                    request.input,
                    PromptPriority::UserBlocking,
                    request
                        .metadata
                        .get("clientMessageId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    request.metadata,
                )
                .map_err(|error| KernelError::Backend(error.to_string()))?;
            let prompt = session
                .prompt_queue
                .take_next()
                .ok_or_else(|| KernelError::Backend("prompt queue unexpectedly empty".into()))?;
            if prompt.id != prompt_id {
                return Err(KernelError::Backend(
                    "prompt queue selected a different blocking prompt".into(),
                ));
            }
            session.active_prompt = Some(prompt.clone());
            self.execute_active_prompt(
                &mut session,
                &request.operation_id,
                prompt,
                true,
                &request.policy,
                control.as_ref(),
                events,
            )
            .await
        }
        .await;
        self.finish_operation(&request.operation_id)?;
        self.session(&request.session_id)?
            .lock()
            .await
            .updated_at_ms = now_ms();
        self.persist_session_if_configured(&request.session_id)
            .await?;
        result
    }

    async fn interrupt(&self, operation_id: &OperationId) -> Result<(), KernelError> {
        let control = self
            .active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .get(operation_id.as_str())
            .cloned()
            .ok_or_else(|| KernelError::OperationNotFound(operation_id.as_str().to_string()))?;
        control.interrupted.store(true, Ordering::SeqCst);
        self.cancel_operation_approvals(operation_id)?;
        Ok(())
    }

    async fn resolve_approval(&self, resolution: ApprovalResolution) -> Result<(), KernelError> {
        let waiter = self
            .approvals
            .lock()
            .map_err(|_| KernelError::Backend("approval registry poisoned".into()))?
            .remove(&resolution.approval_id)
            .ok_or_else(|| KernelError::ApprovalNotFound(resolution.approval_id.clone()))?;
        waiter
            .sender
            .send(resolution)
            .map_err(|resolution| KernelError::ApprovalNotFound(resolution.approval_id))
    }

    async fn snapshot_session(
        &self,
        session_id: &SessionId,
    ) -> Result<KernelSessionSnapshot, KernelError> {
        let session = self.session(session_id)?;
        let session = session.lock().await.clone();
        let updated_at_ms = session.updated_at_ms;
        let state = NativeSnapshotState {
            session,
            memory: self
                .memory
                .lock()
                .map_err(|_| KernelError::Backend("memory store poisoned".into()))?
                .clone(),
            workflows: self
                .workflows
                .lock()
                .map_err(|_| KernelError::Backend("workflow store poisoned".into()))?
                .clone(),
            subagents: self
                .subagents
                .lock()
                .map_err(|_| KernelError::Backend("subagent scheduler poisoned".into()))?
                .clone(),
            hooks: self
                .hooks
                .lock()
                .map_err(|_| KernelError::Backend("hook registry poisoned".into()))?
                .clone(),
        };
        Ok(KernelSessionSnapshot {
            session_id: session_id.clone(),
            backend_id: "mahayana-native".into(),
            state: serde_json::to_value(state)
                .map_err(|error| KernelError::Backend(error.to_string()))?,
            metadata: json!({"snapshotVersion": 1, "updatedAtMs": updated_at_ms}),
        })
    }

    async fn restore_session(
        &self,
        snapshot: KernelSessionSnapshot,
    ) -> Result<SessionId, KernelError> {
        if snapshot.backend_id != "mahayana-native" {
            return Err(KernelError::BackendUnavailable(format!(
                "snapshot belongs to backend {}",
                snapshot.backend_id
            )));
        }
        let state: NativeSnapshotState = serde_json::from_value(snapshot.state)
            .map_err(|error| KernelError::Backend(format!("invalid native snapshot: {error}")))?;
        *self
            .memory
            .lock()
            .map_err(|_| KernelError::Backend("memory store poisoned".into()))? = state.memory;
        *self
            .workflows
            .lock()
            .map_err(|_| KernelError::Backend("workflow store poisoned".into()))? = state.workflows;
        *self
            .subagents
            .lock()
            .map_err(|_| KernelError::Backend("subagent scheduler poisoned".into()))? =
            state.subagents;
        *self
            .hooks
            .lock()
            .map_err(|_| KernelError::Backend("hook registry poisoned".into()))? = state.hooks;
        self.sessions
            .lock()
            .map_err(|_| KernelError::Backend("native session registry poisoned".into()))?
            .insert(
                snapshot.session_id.as_str().to_owned(),
                Arc::new(AsyncMutex::new(state.session)),
            );
        Ok(snapshot.session_id)
    }

    async fn suspend_operation(&self, request: SuspendOperationRequest) -> Result<(), KernelError> {
        let control = self
            .active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .get(request.operation_id.as_str())
            .cloned()
            .ok_or_else(|| KernelError::OperationNotFound(request.operation_id.as_str().into()))?;
        let has_running_descendants = self
            .subagents
            .lock()
            .map_err(|_| KernelError::Backend("subagent scheduler poisoned".into()))?
            .running_count()
            > 0;
        let cascade = request
            .metadata
            .get("cascade")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if has_running_descendants && !cascade {
            return Err(KernelError::PolicyDenied(
                "cannot suspend operation while live subagents exist without cascade=true".into(),
            ));
        }
        control.suspended.store(true, Ordering::SeqCst);
        self.cancel_operation_approvals(&request.operation_id)?;
        Ok(())
    }

    async fn resume_operation(
        &self,
        request: ResumeOperationRequest,
        events: SharedKernelEventSink,
    ) -> Result<(), KernelError> {
        if !self
            .capabilities()
            .supports_all(&request.required_capabilities)
        {
            return Err(KernelError::CapabilityUnavailable(
                "native engine does not satisfy the requested capability set".into(),
            ));
        }
        let session = self.session(&request.session_id)?;
        let control = self.register_operation(&request.operation_id)?;
        self.telemetry.operation_started();
        self.telemetry.operation_resumed();
        let result = async {
            let mut session = session.lock().await;
            let prompt = session.active_prompt.clone().ok_or_else(|| {
                KernelError::OperationNotFound(format!(
                    "{} has no suspended prompt in session {}",
                    request.operation_id.as_str(),
                    request.session_id.as_str()
                ))
            })?;
            events.emit(KernelEvent::Activity {
                operation_id: request.operation_id.clone(),
                kind: "operation_resumed".into(),
                title: "Mahayana operation resumed".into(),
                detail: None,
                metadata: json!({"promptId": prompt.id}),
            })?;
            self.execute_active_prompt(
                &mut session,
                &request.operation_id,
                prompt,
                false,
                &request.policy,
                control.as_ref(),
                events,
            )
            .await
        }
        .await;
        self.finish_operation(&request.operation_id)?;
        result
    }
}

#[derive(Debug, Clone)]
struct FunctionCall {
    call_id: String,
    name: String,
    arguments: Value,
}

fn explicit_tool_request_plan(prompt: &str) -> Vec<FunctionCall> {
    const TOOL_NAMES: [&str; 13] = [
        "workspace_read",
        "workspace_write",
        "workspace_search",
        "workspace_checkpoint",
        "workspace_restore",
        "workspace_worktree",
        "codebase_graph",
        "code_symbols",
        "memory_put",
        "memory_get",
        "memory_search",
        "workflow_create",
        "workflow_status",
    ];

    let mut requested = TOOL_NAMES
        .into_iter()
        .filter_map(|name| prompt.find(name).map(|position| (position, name)))
        .collect::<Vec<_>>();
    if requested.is_empty() {
        return Vec::new();
    }
    requested.sort_by_key(|(position, _)| *position);

    requested
        .into_iter()
        .enumerate()
        .map(|(index, (_, name))| FunctionCall {
            call_id: format!("planned-call:{}", index + 1),
            name: name.to_string(),
            arguments: explicit_tool_arguments(prompt, name),
        })
        .collect()
}

fn explicit_tool_arguments(prompt: &str, tool: &str) -> Value {
    match tool {
        "workspace_read" => json!({
            "path": explicit_path(prompt).unwrap_or_else(|| "README.md".to_string()),
        }),
        "workspace_search" => json!({
            "query": quoted_value_after(prompt, "workspace_search")
                .unwrap_or_else(|| "Mahayana".to_string()),
            "limit": number_after(prompt, &["limit", "限制返回"]).unwrap_or(50),
        }),
        "workflow_create" => json!({
            "title": quoted_value_after(prompt, "workflow_create")
                .unwrap_or_else(|| "Mahayana workflow".to_string()),
            "tasks": explicit_workflow_tasks(prompt),
        }),
        "workflow_status" => json!({"workflow_id": "$last_workflow_id"}),
        _ => json!({}),
    }
}

fn explicit_path(prompt: &str) -> Option<String> {
    ["package.json", "Cargo.toml", "README.md"]
        .iter()
        .find(|candidate| prompt.contains(*candidate))
        .map(|candidate| (*candidate).to_string())
}

fn quoted_value_after(prompt: &str, marker: &str) -> Option<String> {
    let start = prompt.find(marker)? + marker.len();
    let tail = &prompt[start..];
    for (open, close) in [('“', '”'), ('"', '"'), ('`', '`'), ('\'', '\'')] {
        let Some(open_index) = tail.find(open) else {
            continue;
        };
        let value_start = open_index + open.len_utf8();
        let Some(close_index) = tail[value_start..].find(close) else {
            continue;
        };
        let value = tail[value_start..value_start + close_index].trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn number_after(prompt: &str, markers: &[&str]) -> Option<u64> {
    markers.iter().find_map(|marker| {
        let start = prompt.find(marker)? + marker.len();
        prompt[start..]
            .chars()
            .skip_while(|character| !character.is_ascii_digit())
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    })
}

fn explicit_workflow_tasks(prompt: &str) -> Vec<Value> {
    let known = ["verify-read", "verify-search"];
    let mut task_ids = known
        .iter()
        .filter(|task_id| prompt.contains(*task_id))
        .copied()
        .collect::<Vec<_>>();
    if task_ids.is_empty() {
        return Vec::new();
    }
    task_ids.dedup();
    task_ids
        .into_iter()
        .map(|task_id| {
            json!({
                "id": task_id,
                "title": task_id,
                "depends_on": if task_id == "verify-search"
                    && prompt.contains("verify-read")
                {
                    json!(["verify-read"])
                } else {
                    json!([])
                },
            })
        })
        .collect()
}

fn extract_function_calls(payload: &Value) -> Result<Vec<FunctionCall>, KernelError> {
    let mut calls = Vec::new();
    for item in payload
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        if !matches!(item_type, "function_call" | "tool_call") {
            continue;
        }
        let name = item
            .get("name")
            .or_else(|| item.pointer("/function/name"))
            .and_then(Value::as_str)
            .ok_or_else(|| KernelError::Backend("tool call is missing a name".into()))?;
        let call_id = item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("call:{}", Uuid::new_v4()));
        let arguments = match item
            .get("arguments")
            .or_else(|| item.pointer("/function/arguments"))
        {
            Some(Value::String(arguments)) => serde_json::from_str(arguments).map_err(|error| {
                KernelError::Backend(format!("invalid tool arguments for {name}: {error}"))
            })?,
            Some(arguments) => arguments.clone(),
            None => json!({}),
        };
        calls.push(FunctionCall {
            call_id,
            name: name.to_string(),
            arguments,
        });
    }
    Ok(calls)
}

fn append_model_output(history: &mut Vec<Value>, payload: &Value) {
    if let Some(output) = payload.get("output").and_then(Value::as_array) {
        history.extend(output.iter().cloned());
    } else if let Some(text) = mahayana_model::responses::extract_output_text(payload) {
        history.push(json!({"role": "assistant", "content": text}));
    }
}

#[derive(Default)]
struct ModelCollector {
    output: Mutex<Option<Value>>,
    text: Mutex<String>,
    usage: Mutex<Option<ModelUsage>>,
    streaming_events: Option<(SharedKernelEventSink, OperationId)>,
}

impl ModelCollector {
    fn streaming(events: SharedKernelEventSink, operation_id: OperationId) -> Self {
        Self {
            streaming_events: Some((events, operation_id)),
            ..Self::default()
        }
    }

    fn output(&self) -> Result<Option<Value>, KernelError> {
        self.output
            .lock()
            .map(|output| output.clone())
            .map_err(|_| KernelError::Backend("model output collector poisoned".into()))
    }

    fn text(&self) -> Result<String, KernelError> {
        self.text
            .lock()
            .map(|text| text.clone())
            .map_err(|_| KernelError::Backend("model text collector poisoned".into()))
    }

    fn usage(&self) -> Result<Option<ModelUsage>, KernelError> {
        self.usage
            .lock()
            .map(|usage| usage.clone())
            .map_err(|_| KernelError::Backend("model usage collector poisoned".into()))
    }
}

impl ModelEventSink for ModelCollector {
    fn emit(&self, event: ModelEvent) -> Result<(), ModelError> {
        match event {
            ModelEvent::OutputTextDelta(delta) => {
                if let Some((events, operation_id)) = self.streaming_events.as_ref() {
                    events
                        .emit(KernelEvent::MessageDelta {
                            operation_id: operation_id.clone(),
                            delta: delta.clone(),
                        })
                        .map_err(|error| ModelError::Inference(error.to_string()))?;
                }
                self.text
                    .lock()
                    .map_err(|_| ModelError::EventConsumerClosed)?
                    .push_str(&delta);
            }
            ModelEvent::Usage(usage) => {
                *self
                    .usage
                    .lock()
                    .map_err(|_| ModelError::EventConsumerClosed)? = Some(usage);
            }
            ModelEvent::Completed { output } => {
                *self
                    .output
                    .lock()
                    .map_err(|_| ModelError::EventConsumerClosed)? = Some(output);
            }
            ModelEvent::Failed { code, message } => {
                return Err(ModelError::Inference(format!("{code}: {message}")));
            }
        }
        Ok(())
    }
}

fn workspace_root(session: &NativeSession) -> Result<&Path, KernelError> {
    session.workspace_root.as_deref().ok_or_else(|| {
        KernelError::CapabilityUnavailable("this session has no workspace root".into())
    })
}

fn safe_join(root: &Path, relative: &Path) -> Result<PathBuf, KernelError> {
    if relative.is_absolute() {
        return Err(KernelError::PolicyDenied(
            "absolute workspace paths are not allowed".into(),
        ));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| KernelError::Backend(error.to_string()))?;
    let mut safe = canonical_root.clone();
    for component in relative.components() {
        match component {
            Component::Normal(segment) => {
                safe.push(segment);
                if safe.exists() {
                    let metadata = std::fs::symlink_metadata(&safe)
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    if metadata.file_type().is_symlink() {
                        return Err(KernelError::PolicyDenied(format!(
                            "workspace path crosses a symbolic link: {}",
                            safe.display()
                        )));
                    }
                    let canonical = safe
                        .canonicalize()
                        .map_err(|error| KernelError::Backend(error.to_string()))?;
                    if !canonical.starts_with(&canonical_root) {
                        return Err(KernelError::PolicyDenied(
                            "workspace path escapes the active root".into(),
                        ));
                    }
                }
            }
            Component::CurDir => {}
            _ => {
                return Err(KernelError::PolicyDenied(
                    "workspace path traversal is not allowed".into(),
                ));
            }
        }
    }
    Ok(safe)
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn search_workspace(root: &Path, query: &str, limit: usize) -> Result<Vec<Value>, KernelError> {
    if query.is_empty() {
        return Err(KernelError::Backend(
            "search query must not be empty".into(),
        ));
    }
    let mut matches = Vec::new();
    search_directory(root, root, query, limit, &mut matches)?;
    Ok(matches)
}

fn search_directory(
    root: &Path,
    directory: &Path,
    query: &str,
    limit: usize,
    matches: &mut Vec<Value>,
) -> Result<(), KernelError> {
    if matches.len() >= limit {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(directory).map_err(|error| KernelError::Backend(error.to_string()))?
    {
        let entry = entry.map_err(|error| KernelError::Backend(error.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| KernelError::Backend(error.to_string()))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            let name = entry.file_name();
            if matches!(
                name.to_str(),
                Some(".git" | ".mahayana" | "target" | "node_modules" | "dist" | "build")
            ) {
                continue;
            }
            search_directory(root, &path, query, limit, matches)?;
        } else if file_type.is_file() {
            let metadata = entry
                .metadata()
                .map_err(|error| KernelError::Backend(error.to_string()))?;
            if metadata.len() > 2 * 1024 * 1024 {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(content) => content,
                Err(_) => continue,
            };
            for (index, line) in content.lines().enumerate() {
                if line.contains(query) {
                    matches.push(json!({
                        "path": relative_display(root, &path),
                        "line": index + 1,
                        "text": line,
                    }));
                    if matches.len() >= limit {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

fn native_bootstrap_history(metadata: &Value) -> Vec<Value> {
    metadata
        .get("bootstrapHistory")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .take(200)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|message| {
            let role = message.get("role").and_then(Value::as_str)?;
            if !matches!(role, "user" | "assistant") {
                return None;
            }
            let content = message.get("content").and_then(Value::as_str)?;
            Some(json!({"role": role, "content": content}))
        })
        .collect()
}

fn run_process(
    execution: &ProcessExecution,
    root: &Path,
    program: &str,
    args: &[String],
) -> Result<Value, KernelError> {
    if program.trim().is_empty() || program.contains(['\r', '\n']) {
        return Err(KernelError::PolicyDenied("invalid process program".into()));
    }
    let output = match execution {
        ProcessExecution::Host => Command::new(program).args(args).current_dir(root).output(),
        ProcessExecution::LocalDocker { docker_path, image } => {
            let canonical_root = root
                .canonicalize()
                .map_err(|error| KernelError::Backend(error.to_string()))?;
            let mount = format!("{}:/workspace:rw", canonical_root.display());
            let temporary = format!(
                "type=tmpfs,destination=/tmp,tmpfs-size={}",
                256 * 1024 * 1024
            );
            Command::new(docker_path)
                .args([
                    "run",
                    "--rm",
                    "--network",
                    "none",
                    "--read-only",
                    "--cap-drop",
                    "ALL",
                    "--security-opt",
                    "no-new-privileges",
                    "--pids-limit",
                    "256",
                    "--memory",
                    "1g",
                    "--cpus",
                    "2",
                    "--label",
                    "com.fabushi.owner=mahayana-native-engine",
                    "--mount",
                    &temporary,
                    "--volume",
                    &mount,
                    "--workdir",
                    "/workspace",
                    image,
                    program,
                ])
                .args(args)
                .output()
        }
    }
    .map_err(|error| KernelError::Backend(error.to_string()))?;
    let stdout = truncate_bytes(&output.stdout);
    let stderr = truncate_bytes(&output.stderr);
    Ok(json!({
        "success": output.status.success(),
        "code": output.status.code(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

fn is_pinned_container_image(image: &str) -> bool {
    let Some((name, digest)) = image.rsplit_once("@sha256:") else {
        return false;
    };
    !name.trim().is_empty()
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), KernelError> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| KernelError::Backend(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| KernelError::Backend(error.to_string()))
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

fn truncate_bytes(bytes: &[u8]) -> String {
    let end = bytes.len().min(MAX_TOOL_OUTPUT_BYTES);
    let mut text = String::from_utf8_lossy(&bytes[..end]).to_string();
    if bytes.len() > end {
        text.push_str("\n...[truncated]");
    }
    text
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, KernelError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| KernelError::Backend(format!("tool argument {key} is required")))
}

fn tool_risk(tool: &str) -> RiskLevel {
    match tool {
        "workspace_write" | "workspace_restore" => RiskLevel::WorkspaceWrite,
        "process_exec" => RiskLevel::SystemWrite,
        _ => RiskLevel::ReadOnly,
    }
}

fn tool_capability(tool: &str) -> Capability {
    match tool {
        "workspace_read" | "workspace_search" | "codebase_graph" | "code_symbols" => {
            Capability::FilesystemRead
        }
        "workspace_write" | "workspace_restore" => Capability::FilesystemWrite,
        "workspace_checkpoint" => Capability::Checkpoint,
        "workspace_worktree" => Capability::Worktree,
        "memory_put" | "memory_get" | "memory_search" => Capability::Memory,
        "workflow_create" | "workflow_status" => Capability::Workflow,
        "subagent_run" => Capability::Subagent,
        "web_search" | "web_fetch" => Capability::WebSearch,
        "process_exec" => Capability::Process,
        "git_status" | "git_diff" => Capability::Git,
        _ => Capability::ToolProtocol,
    }
}

fn permission_target(call: &FunctionCall) -> String {
    let target = match call.name.as_str() {
        "workspace_read" | "workspace_write" => call.arguments.get("path"),
        "workspace_restore" | "workspace_worktree" => call.arguments.get("checkpoint_id"),
        "memory_get" | "memory_put" => call.arguments.get("key"),
        "workflow_status" => call.arguments.get("workflow_id"),
        "process_exec" => call.arguments.get("program"),
        "web_search" => call.arguments.get("query"),
        "web_fetch" => call
            .arguments
            .get("urls")
            .and_then(Value::as_array)
            .and_then(|urls| urls.first()),
        _ => None,
    };
    target
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("{}:{value}", call.name))
        .unwrap_or_else(|| call.name.clone())
}

fn tool_fingerprint(call: &FunctionCall) -> String {
    format!("{}:{}", call.name, call.arguments)
}

fn permission_memory_from_metadata(
    metadata: &Value,
) -> Result<Option<PermissionMemory>, KernelError> {
    let Some(value) = metadata
        .get("permissionMemory")
        .or_else(|| metadata.get("permission_memory"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    match value {
        "allow_for_session" => Ok(Some(PermissionMemory::AllowForSession)),
        "deny_permanently" => Ok(Some(PermissionMemory::DenyPermanently)),
        "clear" => Ok(Some(PermissionMemory::Clear)),
        other => Err(KernelError::Backend(format!(
            "unknown permission memory directive: {other}"
        ))),
    }
}

fn ensure_operation_active(control: &OperationControl) -> Result<(), KernelError> {
    if control.suspended.load(Ordering::SeqCst) {
        return Err(KernelError::Backend("operation suspended".into()));
    }
    if control.interrupted.load(Ordering::SeqCst) {
        return Err(KernelError::Backend("operation interrupted".into()));
    }
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn model_error(error: ModelError) -> KernelError {
    KernelError::Backend(error.to_string())
}

fn tool_definitions(enable_process_tools: bool, enable_web_research: bool) -> Vec<Value> {
    let mut tools = vec![
        function_tool(
            "send_message",
            "Send a concise user-visible progress update or answer as a separate message bubble. Use this for meaningful milestones, confirmations, and the final answer in a multi-step task. Do not invent progress; only report work that has happened or is about to happen.",
            json!({"type":"object","properties":{"message":{"type":"string","description":"The concise message to show the user."}},"required":["message"],"additionalProperties":false}),
        ),
        function_tool(
            "workspace_read",
            "Read a UTF-8 text file inside the active workspace.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
        ),
        function_tool(
            "workspace_write",
            "Write a UTF-8 text file inside the workspace. Mahayana automatically checkpoints before writing.",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}),
        ),
        function_tool(
            "workspace_search",
            "Search text across the workspace while skipping generated and dependency directories.",
            json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":200}},"required":["query"],"additionalProperties":false}),
        ),
        function_tool(
            "workspace_checkpoint",
            "Create a restorable Mahayana workspace checkpoint.",
            json!({"type":"object","properties":{"label":{"type":"string"}},"additionalProperties":false}),
        ),
        function_tool(
            "workspace_restore",
            "Restore a Mahayana workspace checkpoint. A safety checkpoint is created first.",
            json!({"type":"object","properties":{"checkpoint_id":{"type":"string"}},"required":["checkpoint_id"],"additionalProperties":false}),
        ),
        function_tool(
            "workspace_worktree",
            "Create an isolated Mahayana logical worktree from the workspace or a checkpoint.",
            json!({"type":"object","properties":{"checkpoint_id":{"type":"string"}},"additionalProperties":false}),
        ),
        function_tool(
            "codebase_graph",
            "Build the Mahayana cross-language codebase reference graph.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
        function_tool(
            "code_symbols",
            "Index symbols in common workspace programming languages.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
        function_tool(
            "memory_put",
            "Store durable structured Mahayana memory.",
            json!({"type":"object","properties":{"namespace":{"type":"string"},"key":{"type":"string"},"value":{},"tags":{"type":"array","items":{"type":"string"}}},"required":["namespace","key","value"],"additionalProperties":false}),
        ),
        function_tool(
            "memory_get",
            "Read one durable Mahayana memory record.",
            json!({"type":"object","properties":{"namespace":{"type":"string"},"key":{"type":"string"}},"required":["namespace","key"],"additionalProperties":false}),
        ),
        function_tool(
            "memory_search",
            "Search durable Mahayana memory by namespace, text, and tags.",
            json!({"type":"object","properties":{"namespace":{"type":"string"},"query":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}),
        ),
        function_tool(
            "workflow_create",
            "Create a dependency-validated Mahayana workflow DAG.",
            json!({"type":"object","properties":{"title":{"type":"string"},"tasks":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"title":{"type":"string"},"depends_on":{"type":"array","items":{"type":"string"}}},"required":["id"],"additionalProperties":false}}},"required":["title"],"additionalProperties":false}),
        ),
        function_tool(
            "workflow_status",
            "Read the current state of a Mahayana workflow.",
            json!({"type":"object","properties":{"workflow_id":{"type":"string"}},"required":["workflow_id"],"additionalProperties":false}),
        ),
        function_tool(
            "subagent_run",
            "Delegate a focused reasoning task to an isolated Mahayana subagent.",
            json!({"type":"object","properties":{"name":{"type":"string"},"goal":{"type":"string"}},"required":["goal"],"additionalProperties":false}),
        ),
    ];
    if enable_web_research {
        tools.extend([
            function_tool(
                "web_search",
                "Search the live public web for current or external information. Use this when the task depends on information outside the workspace or may have changed since model training.",
                json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":10}},"required":["query"],"additionalProperties":false}),
            ),
            function_tool(
                "web_fetch",
                "Fetch and extract readable content from up to 10 public HTTP(S) URLs returned by web search or provided by the user. Prefer markdown for research and verify important claims from source content rather than snippets alone.",
                json!({"type":"object","properties":{"urls":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"string"}},"format":{"type":"string","enum":["markdown","text"]}},"required":["urls"],"additionalProperties":false}),
            ),
        ]);
    }
    if enable_process_tools {
        tools.extend([
            function_tool(
                "process_exec",
                "Run an explicitly approved process in the active workspace.",
                json!({"type":"object","properties":{"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}}},"required":["program"],"additionalProperties":false}),
            ),
            function_tool(
                "git_status",
                "Read git status for the active workspace.",
                json!({"type":"object","properties":{},"additionalProperties":false}),
            ),
            function_tool(
                "git_diff",
                "Read the current git diff for the active workspace.",
                json!({"type":"object","properties":{},"additionalProperties":false}),
            ),
        ]);
    }
    tools
}

fn function_tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "name": name,
        "description": description,
        "parameters": parameters,
    })
}

fn default_system_instructions() -> String {
    "You are Mahayana, a product-owned coding and automation Agent. Inspect before editing; prefer minimal, reversible changes; use checkpoints before risky workspace mutations; use workflows for dependent tasks; delegate focused analysis to subagents; use web_search when live or external information is needed and web_fetch to inspect strong sources before drawing conclusions. When the user names an available tool or requests a verifiable multi-step operation, make the actual function call, wait for its result, and continue the Agent loop until the requested work is complete; do not replace an executable tool call with a prose claim. For a multi-step task, use send_message to publish short, human-readable milestone updates and the final answer as separate user-visible messages; keep internal reasoning private, never fabricate progress, and do not merge all milestones into one long response. Never claim a tool succeeded unless its result says so; respect Mahayana approval and platform policy."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mahayana_model::ModelProviderMode;
    use std::collections::VecDeque;

    #[test]
    fn web_research_capability_and_tools_follow_provider_configuration() {
        let model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::new()),
        });
        let without_web = NativeEngine::new_with_web_config(
            model.clone(),
            NativeEngineConfig::embedded("model"),
            None,
        )
        .expect("create engine without web");
        assert!(!without_web.capabilities().contains(Capability::WebSearch));
        assert!(!tool_definitions(false, false).iter().any(|tool| matches!(
            tool.get("name").and_then(Value::as_str),
            Some("web_search" | "web_fetch")
        )));

        let with_web = NativeEngine::new_with_web_config(
            model,
            NativeEngineConfig::embedded("model"),
            Some(WebResearchConfig::for_test(
                "http://127.0.0.1:9/search",
                "http://127.0.0.1:9/fetch",
                "MAHAYANA_WEB_RESEARCH_TEST_KEY",
            )),
        )
        .expect("create engine with web");
        assert!(with_web.capabilities().contains(Capability::WebSearch));
        assert!(with_web.capabilities().contains(Capability::Network));
        let names = tool_definitions(false, true)
            .into_iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect::<Vec<_>>();
        assert!(names.contains(&"web_search".to_string()));
        assert!(names.contains(&"web_fetch".to_string()));
        assert_eq!(tool_capability("web_search"), Capability::WebSearch);
        assert_eq!(tool_capability("web_fetch"), Capability::WebSearch);
    }

    #[test]
    fn local_docker_requires_an_immutable_image_digest() {
        assert!(!is_pinned_container_image("example.test/fabushi:latest"));
        assert!(is_pinned_container_image(&format!(
            "example.test/fabushi@sha256:{}",
            "a".repeat(64)
        )));

        let mut config = NativeEngineConfig::desktop("model");
        config.process_execution = ProcessExecution::LocalDocker {
            docker_path: PathBuf::from("docker"),
            image: "example.test/fabushi:latest".into(),
        };
        assert!(config.validate().is_err());
    }

    struct FakeModel {
        outputs: Mutex<VecDeque<Value>>,
    }

    #[async_trait]
    impl ModelRuntime for FakeModel {
        async fn infer(
            &self,
            _request: ModelRequest,
            events: SharedModelEventSink,
        ) -> Result<(), ModelError> {
            let output = self
                .outputs
                .lock()
                .map_err(|_| ModelError::Inference("fake model poisoned".into()))?
                .pop_front()
                .ok_or_else(|| ModelError::Inference("fake model exhausted".into()))?;
            events.emit(ModelEvent::Usage(ModelUsage {
                total_tokens: 3,
                input_tokens: 2,
                cached_input_tokens: 0,
                output_tokens: 1,
                reasoning_output_tokens: 0,
            }))?;
            events.emit(ModelEvent::Completed { output })
        }

        fn provider_mode(&self) -> ModelProviderMode {
            ModelProviderMode::LocalModel
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<KernelEvent>>);

    impl mahayana_kernel::KernelEventSink for Events {
        fn emit(&self, event: KernelEvent) -> Result<(), KernelError> {
            self.0
                .lock()
                .map_err(|_| KernelError::EventConsumerClosed)?
                .push(event);
            Ok(())
        }
    }

    fn temp_workspace() -> PathBuf {
        let root = std::env::temp_dir().join(format!("mahayana-native-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        root
    }

    #[test]
    fn explicit_tool_request_plan_preserves_order_and_dependencies() {
        let plan = explicit_tool_request_plan(
            "请按顺序调用 workspace_read 读取 package.json，workspace_search 搜索 “Mahayana” 限制返回 3 个结果，workflow_create 创建名为“CI 多步骤验证”的工作流并包含 verify-read、verify-search，最后 workflow_status 查询刚刚创建的 workflow_id。",
        );
        assert_eq!(
            plan.iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "workspace_read",
                "workspace_search",
                "workflow_create",
                "workflow_status"
            ]
        );
        assert_eq!(plan[0].arguments["path"], "package.json");
        assert_eq!(plan[1].arguments["query"], "Mahayana");
        assert_eq!(plan[1].arguments["limit"], 3);
        assert_eq!(plan[2].arguments["title"], "CI 多步骤验证");
        assert_eq!(
            plan[2].arguments["tasks"][1]["depends_on"],
            json!(["verify-read"])
        );
        assert_eq!(plan[3].arguments["workflow_id"], "$last_workflow_id");
    }

    #[tokio::test]
    async fn completes_direct_model_response_and_records_telemetry() {
        let model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::from([json!({
                "output": [{"type":"message", "content":[{"type":"output_text", "text":"done"}]}]
            })])),
        });
        let engine =
            NativeEngine::new(model, NativeEngineConfig::embedded("model")).expect("create engine");
        let session = engine
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::MobileEmbedded,
                workspace_root: None,
                model: None,
                metadata: Value::Null,
            })
            .await
            .expect("open session");
        let events = Arc::new(Events::default());
        engine
            .run(
                RunRequest {
                    session_id: session,
                    operation_id: OperationId::new(),
                    input: "hello".into(),
                    policy: ExecutionPolicy::mobile_default(),
                    required_capabilities: CapabilitySet::new([Capability::Model]),
                    metadata: Value::Null,
                },
                events.clone(),
            )
            .await
            .expect("run engine");
        assert!(
            events
                .0
                .lock()
                .expect("events")
                .iter()
                .any(|event| matches!(
                    event,
                    KernelEvent::MessageCompleted { text, .. } if text == "done"
                ))
        );
        let metrics = engine.metrics_snapshot();
        assert_eq!(metrics.sessions_opened, 1);
        assert_eq!(metrics.operations_started, 1);
        assert_eq!(metrics.operations_completed, 1);
        assert_eq!(metrics.model_calls, 1);
    }

    #[tokio::test]
    async fn executes_explicit_multi_step_tool_request_when_model_returns_prose() {
        let workspace = temp_workspace();
        std::fs::write(
            workspace.join("package.json"),
            r#"{"name":"mahayana-test"}"#,
        )
        .expect("seed package manifest");
        let prose = |text| {
            json!({
                "output": [{"type":"message", "content":[{"type":"output_text", "text":text}]}]
            })
        };
        let model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::from([
                prose("我会执行这些步骤。"),
                prose("已读取，继续搜索。"),
                prose("已搜索，继续创建工作流。"),
                prose("已创建，继续查询状态。"),
                prose("全部完成。"),
            ])),
        });
        let engine =
            NativeEngine::new(model, NativeEngineConfig::desktop("model")).expect("create engine");
        let session = engine
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::DesktopFull,
                workspace_root: Some(workspace.to_string_lossy().to_string()),
                model: None,
                metadata: Value::Null,
            })
            .await
            .expect("open session");
        let events = Arc::new(Events::default());
        engine
            .run(
                RunRequest {
                    session_id: session,
                    operation_id: OperationId::new(),
                    input: "请严格按顺序实际调用 workspace_read 读取 package.json，workspace_search 搜索 “Mahayana” 限制返回 3 个结果，workflow_create 创建名为“CI 多步骤验证”的工作流并包含 verify-read、verify-search，最后 workflow_status 查询刚刚创建的 workflow_id。".into(),
                    policy: ExecutionPolicy::interactive_default(),
                    required_capabilities: CapabilitySet::new([Capability::Model]),
                    metadata: Value::Null,
                },
                events.clone(),
            )
            .await
            .expect("run explicit tool request");
        let completed_tools = events
            .0
            .lock()
            .expect("events")
            .iter()
            .filter_map(|event| match event {
                KernelEvent::ToolCompleted { tool, success, .. } if *success => Some(tool.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            completed_tools,
            vec![
                "workspace_read".to_string(),
                "workspace_search".to_string(),
                "workflow_create".to_string(),
                "workflow_status".to_string()
            ]
        );
        assert!(
            events
                .0
                .lock()
                .expect("events")
                .iter()
                .any(|event| matches!(
                    event,
                    KernelEvent::MessageCompleted { text, .. } if text == "全部完成。"
                ))
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn executes_workspace_write_with_checkpoint() {
        let workspace = temp_workspace();
        std::fs::write(workspace.join("existing.txt"), "before").expect("seed workspace");
        let call = json!({
            "output": [{
                "type": "function_call",
                "call_id": "call-1",
                "name": "workspace_write",
                "arguments": "{\"path\":\"new.txt\",\"content\":\"hello\"}"
            }]
        });
        let done = json!({
            "output": [{"type":"message", "content":[{"type":"output_text", "text":"written"}]}]
        });
        let model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::from([call, done])),
        });
        let engine =
            NativeEngine::new(model, NativeEngineConfig::desktop("model")).expect("create engine");
        let session = engine
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::DesktopFull,
                workspace_root: Some(workspace.to_string_lossy().to_string()),
                model: None,
                metadata: Value::Null,
            })
            .await
            .expect("open session");
        let events = Arc::new(Events::default());
        let mut policy = ExecutionPolicy::interactive_default();
        policy.max_unattended_risk = RiskLevel::WorkspaceWrite;
        engine
            .run(
                RunRequest {
                    session_id: session,
                    operation_id: OperationId::new(),
                    input: "write the file".into(),
                    policy,
                    required_capabilities: CapabilitySet::new([
                        Capability::Model,
                        Capability::FilesystemWrite,
                    ]),
                    metadata: Value::Null,
                },
                events.clone(),
            )
            .await
            .expect("run engine");
        assert_eq!(
            std::fs::read_to_string(workspace.join("new.txt")).expect("read result"),
            "hello"
        );
        assert!(
            events
                .0
                .lock()
                .expect("events")
                .iter()
                .any(|event| matches!(event, KernelEvent::CheckpointCreated { .. }))
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn network_policy_blocks_web_search_before_provider_request() {
        let call = json!({
            "output": [{
                "type":"function_call",
                "call_id":"call-web-denied",
                "name":"web_search",
                "arguments":"{\"query\":\"current topic\"}"
            }]
        });
        let done = json!({
            "output": [{"type":"message", "content":[{"type":"output_text", "text":"network denied"}]}]
        });
        let model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::from([call, done])),
        });
        let engine = NativeEngine::new_with_web_config(
            model,
            NativeEngineConfig::embedded("model"),
            Some(WebResearchConfig::for_test(
                "http://127.0.0.1:9/search",
                "http://127.0.0.1:9/fetch",
                "MAHAYANA_WEB_POLICY_TEST_KEY",
            )),
        )
        .expect("create web engine");
        let session = engine
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::MobileEmbedded,
                workspace_root: None,
                model: None,
                metadata: Value::Null,
            })
            .await
            .expect("open session");
        let events = Arc::new(Events::default());
        let mut policy = ExecutionPolicy::mobile_default();
        policy.allow_network = false;
        engine
            .run(
                RunRequest {
                    session_id: session,
                    operation_id: OperationId::new(),
                    input: "search the web".into(),
                    policy,
                    required_capabilities: CapabilitySet::new([Capability::WebSearch]),
                    metadata: Value::Null,
                },
                events.clone(),
            )
            .await
            .expect("model recovers from denied search");
        assert!(
            events
                .0
                .lock()
                .expect("events")
                .iter()
                .any(|event| matches!(
                    event,
                    KernelEvent::ToolCompleted { tool, output, success: false, .. }
                        if tool == "web_search" && output.to_string().contains("denied")
                ))
        );
    }

    #[tokio::test]
    async fn approval_timeout_is_fail_closed() {
        let workspace = temp_workspace();
        let call = json!({
            "output": [{
                "type":"function_call",
                "call_id":"call-timeout",
                "name":"workspace_write",
                "arguments":"{\"path\":\"blocked.txt\",\"content\":\"no\"}"
            }]
        });
        let done = json!({
            "output": [{"type":"message", "content":[{"type":"output_text", "text":"denied"}]}]
        });
        let model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::from([call, done])),
        });
        let mut config = NativeEngineConfig::desktop("model");
        config.approval_timeout_ms = 1;
        let engine = NativeEngine::new(model, config).expect("create engine");
        let session = engine
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::DesktopFull,
                workspace_root: Some(workspace.to_string_lossy().to_string()),
                model: None,
                metadata: Value::Null,
            })
            .await
            .expect("open session");
        engine
            .run(
                RunRequest {
                    session_id: session,
                    operation_id: OperationId::new(),
                    input: "try write".into(),
                    policy: ExecutionPolicy::interactive_default(),
                    required_capabilities: CapabilitySet::new([Capability::FilesystemWrite]),
                    metadata: Value::Null,
                },
                Arc::new(Events::default()),
            )
            .await
            .expect("model can recover from denied tool");
        assert!(!workspace.join("blocked.txt").exists());
        assert_eq!(engine.metrics_snapshot().approvals_timed_out, 1);
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn snapshot_round_trip_preserves_native_orchestration_state() {
        let model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::from([json!({
                "output": [{"type":"message", "content":[{"type":"output_text", "text":"remembered"}]}]
            })])),
        });
        let engine =
            NativeEngine::new(model, NativeEngineConfig::embedded("model")).expect("create engine");
        let session = engine
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::Headless,
                workspace_root: None,
                model: None,
                metadata: Value::Null,
            })
            .await
            .expect("open session");
        engine
            .run(
                RunRequest {
                    session_id: session.clone(),
                    operation_id: OperationId::new(),
                    input: "persist me".into(),
                    policy: ExecutionPolicy::default(),
                    required_capabilities: CapabilitySet::new([Capability::Model]),
                    metadata: Value::Null,
                },
                Arc::new(Events::default()),
            )
            .await
            .expect("run");
        let snapshot = engine
            .snapshot_session(&session)
            .await
            .expect("snapshot native session");
        let history = snapshot
            .state
            .pointer("/session/history")
            .and_then(Value::as_array)
            .expect("history in snapshot");
        assert!(
            history
                .iter()
                .any(|item| item.to_string().contains("persist me"))
        );
        let restored = engine
            .restore_session(snapshot)
            .await
            .expect("restore snapshot");
        assert_eq!(restored, session);
    }

    #[tokio::test]
    async fn main_assistant_history_survives_provider_engine_recreation() {
        let root =
            std::env::temp_dir().join(format!("mahayana-provider-session-{}", Uuid::new_v4()));
        let state_path = root.join("assistant.json");
        let first_model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::from([json!({
                "output": [{"type":"message", "content":[{"type":"output_text", "text":"first provider reply"}]}]
            })])),
        });
        let mut first_config = NativeEngineConfig::desktop("first-model");
        first_config.session_state_path = Some(state_path.clone());
        let first = NativeEngine::new(first_model, first_config).expect("create first engine");
        let session = first
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::DesktopFull,
                workspace_root: None,
                model: None,
                metadata: json!({"conversationId": MAIN_ASSISTANT_CONVERSATION_ID}),
            })
            .await
            .expect("open first session");
        first
            .run(
                RunRequest {
                    session_id: session,
                    operation_id: OperationId::new(),
                    input: "remember across providers".into(),
                    policy: ExecutionPolicy::interactive_default(),
                    required_capabilities: CapabilitySet::new([Capability::Model]),
                    metadata: Value::Null,
                },
                Arc::new(Events::default()),
            )
            .await
            .expect("run first provider");
        assert!(state_path.is_file());

        let second_model = Arc::new(FakeModel {
            outputs: Mutex::new(VecDeque::new()),
        });
        let mut second_config = NativeEngineConfig::desktop("second-model");
        second_config.session_state_path = Some(state_path);
        let second = NativeEngine::new(second_model, second_config).expect("create second engine");
        let restored = second
            .open_session(OpenSessionRequest {
                profile: mahayana_kernel::RuntimeProfile::DesktopFull,
                workspace_root: None,
                model: None,
                metadata: json!({"conversationId": MAIN_ASSISTANT_CONVERSATION_ID}),
            })
            .await
            .expect("restore provider-neutral session");
        let snapshot = second
            .snapshot_session(&restored)
            .await
            .expect("snapshot restored session");
        assert!(
            snapshot
                .state
                .to_string()
                .contains("remember across providers")
        );
        assert!(snapshot.state.to_string().contains("first provider reply"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
