//! Mahayana-owned coding Agent engine.
//!
//! The engine owns the Agent loop, session state, policy/approval boundary,
//! workspace tools, checkpoints, memory, workflows, prompt queue, and
//! subagents. Model inference is injected through `mahayana-model`; Codex and
//! Grok Build are not protocol dependencies of this crate.

use async_trait::async_trait;
use mahayana_kernel::{
    ApprovalMode, ApprovalResolution, BackendDescriptor, Capability, CapabilitySet, EngineBackend,
    ExecutionPolicy, KernelError, KernelEvent, OpenSessionRequest, OperationId, RiskLevel,
    RunRequest, SessionId, SharedKernelEventSink,
};
use mahayana_model::{
    ModelError, ModelEvent, ModelEventSink, ModelRequest, ModelRuntime, ModelUsage,
    SharedModelEventSink,
};
use mahayana_orchestrator::{
    MemoryStore, PromptPriority, PromptQueue, SubagentScheduler, Workflow,
};
use mahayana_workspace_engine::WorkspaceEngine;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use uuid::Uuid;

const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_MODEL_TURNS: usize = 16;

#[derive(Debug, Clone)]
pub struct NativeEngineConfig {
    pub model: String,
    pub system_instructions: String,
    pub max_model_turns: usize,
    pub enable_process_tools: bool,
}

impl NativeEngineConfig {
    pub fn desktop(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_instructions: default_system_instructions(),
            max_model_turns: DEFAULT_MAX_MODEL_TURNS,
            enable_process_tools: true,
        }
    }

    pub fn embedded(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_instructions: default_system_instructions(),
            max_model_turns: DEFAULT_MAX_MODEL_TURNS,
            enable_process_tools: false,
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
        Ok(())
    }
}

struct NativeSession {
    workspace_root: Option<PathBuf>,
    history: Vec<Value>,
    prompt_queue: PromptQueue,
}

struct ApprovalWaiter {
    sender: oneshot::Sender<bool>,
}

pub struct NativeEngine {
    model: Arc<dyn ModelRuntime>,
    config: NativeEngineConfig,
    sessions: Mutex<HashMap<String, Arc<AsyncMutex<NativeSession>>>>,
    active_operations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    approvals: Mutex<HashMap<String, ApprovalWaiter>>,
    memory: Mutex<MemoryStore>,
    workflows: Mutex<HashMap<String, Workflow>>,
    subagents: Mutex<SubagentScheduler>,
}

impl NativeEngine {
    pub fn new(
        model: Arc<dyn ModelRuntime>,
        config: NativeEngineConfig,
    ) -> Result<Self, KernelError> {
        config.validate()?;
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
        })
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
        if !self.model.is_local() {
            capabilities.push(Capability::Network);
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
    ) -> Result<Arc<AtomicBool>, KernelError> {
        let interrupted = Arc::new(AtomicBool::new(false));
        self.active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .insert(operation_id.as_str().to_string(), Arc::clone(&interrupted));
        Ok(interrupted)
    }

    fn finish_operation(&self, operation_id: &OperationId) -> Result<(), KernelError> {
        self.active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .remove(operation_id.as_str());
        Ok(())
    }

    async fn run_prompt(
        &self,
        session: &mut NativeSession,
        operation_id: &OperationId,
        prompt: String,
        policy: &ExecutionPolicy,
        interrupted: &AtomicBool,
        events: SharedKernelEventSink,
    ) -> Result<String, KernelError> {
        session
            .history
            .push(json!({"role": "user", "content": prompt}));

        for turn in 0..self.config.max_model_turns {
            ensure_not_interrupted(interrupted)?;
            if !self.model.is_local() && !policy.allow_network {
                return Err(KernelError::PolicyDenied(
                    "remote model inference is disabled by Mahayana policy".into(),
                ));
            }

            events.emit(KernelEvent::Activity {
                operation_id: operation_id.clone(),
                kind: "model".into(),
                title: format!("Mahayana reasoning turn {}", turn + 1),
                detail: None,
                metadata: json!({"engine": "mahayana-native"}),
            })?;

            let collector = Arc::new(ModelCollector::default());
            let sink: SharedModelEventSink = collector.clone();
            self.model
                .infer(
                    ModelRequest {
                        model: self.config.model.clone(),
                        input: Value::Array(session.history.clone()),
                        metadata: json!({
                            "instructions": self.config.system_instructions,
                            "tools": tool_definitions(self.config.enable_process_tools),
                            "tool_choice": "auto",
                            "parallel_tool_calls": false,
                        }),
                    },
                    sink,
                )
                .await
                .map_err(model_error)?;
            ensure_not_interrupted(interrupted)?;

            if let Some(usage) = collector.usage()? {
                events.emit(KernelEvent::UsageUpdated {
                    operation_id: operation_id.clone(),
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                })?;
            }
            let payload = collector.output()?.ok_or_else(|| {
                KernelError::Backend("model runtime completed without a payload".into())
            })?;
            append_model_output(&mut session.history, &payload);
            let calls = extract_function_calls(&payload)?;
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
                ensure_not_interrupted(interrupted)?;
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
                        interrupted,
                        Arc::clone(&events),
                    )
                    .await;
                match output {
                    Ok(output) => {
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
        interrupted: &'a AtomicBool,
        events: SharedKernelEventSink,
    ) -> Pin<Box<dyn Future<Output = Result<Value, KernelError>> + Send + 'a>> {
        Box::pin(async move {
            let risk = tool_risk(&call.name);
            self.authorize_tool(operation_id, &call.name, risk, policy, Arc::clone(&events))
                .await?;
            ensure_not_interrupted(interrupted)?;

            match call.name.as_str() {
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
                    serde_json::to_value(checkpoint)
                        .map_err(|error| KernelError::Backend(error.to_string()))
                }
                "workspace_restore" => {
                    if !policy.allow_workspace_writes {
                        return Err(KernelError::PolicyDenied(
                            "workspace writes are disabled by Mahayana policy".into(),
                        ));
                    }
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
                        .run_subagent(goal, interrupted, Arc::clone(&events), operation_id)
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
                    run_process(root, program, &args)
                }
                "git_status" => {
                    if !self.config.enable_process_tools || !policy.allow_process {
                        return Err(KernelError::PolicyDenied(
                            "Git process execution is disabled by Mahayana policy".into(),
                        ));
                    }
                    run_process(
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
        interrupted: &AtomicBool,
        events: SharedKernelEventSink,
        operation_id: &OperationId,
    ) -> Result<String, KernelError> {
        ensure_not_interrupted(interrupted)?;
        let collector = Arc::new(ModelCollector::default());
        let sink: SharedModelEventSink = collector.clone();
        self.model
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
            .await
            .map_err(model_error)?;
        ensure_not_interrupted(interrupted)?;
        if let Some(usage) = collector.usage()? {
            events.emit(KernelEvent::UsageUpdated {
                operation_id: operation_id.clone(),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
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
        operation_id: &OperationId,
        tool: &str,
        risk: RiskLevel,
        policy: &ExecutionPolicy,
        events: SharedKernelEventSink,
    ) -> Result<(), KernelError> {
        if matches!(risk, RiskLevel::WorkspaceWrite) && !policy.allow_workspace_writes {
            return Err(KernelError::PolicyDenied(format!(
                "{tool} requires workspace writes"
            )));
        }
        if matches!(risk, RiskLevel::SystemWrite | RiskLevel::ExternalSideEffect)
            && !policy.allow_process
            && matches!(tool, "process_exec")
        {
            return Err(KernelError::PolicyDenied(format!(
                "{tool} requires process execution"
            )));
        }

        let above_unattended = risk_score(risk) > risk_score(policy.max_unattended_risk);
        let needs_approval = match policy.approval_mode {
            ApprovalMode::Never => {
                if above_unattended {
                    return Err(KernelError::PolicyDenied(format!(
                        "{tool} exceeds unattended risk policy"
                    )));
                }
                false
            }
            ApprovalMode::OnRisk => above_unattended,
            ApprovalMode::Always => true,
        };
        if !needs_approval {
            return Ok(());
        }

        let approval_id = format!("approval:{}", Uuid::new_v4());
        let (sender, receiver) = oneshot::channel();
        self.approvals
            .lock()
            .map_err(|_| KernelError::Backend("approval registry poisoned".into()))?
            .insert(approval_id.clone(), ApprovalWaiter { sender });
        events.emit(KernelEvent::ApprovalRequested {
            operation_id: operation_id.clone(),
            approval_id: approval_id.clone(),
            title: format!("Allow {tool}"),
            risk,
            details: json!({"tool": tool, "engine": "mahayana-native"}),
        })?;
        match receiver.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(KernelError::PolicyDenied(format!("user declined {tool}"))),
            Err(_) => Err(KernelError::ApprovalNotFound(approval_id)),
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

    async fn open_session(&self, request: OpenSessionRequest) -> Result<SessionId, KernelError> {
        let workspace_root = request
            .workspace_root
            .as_deref()
            .map(PathBuf::from)
            .map(|path| {
                path.canonicalize()
                    .map_err(|error| KernelError::Backend(error.to_string()))
            })
            .transpose()?;
        if let Some(root) = workspace_root.as_deref() {
            if !root.is_dir() {
                return Err(KernelError::BackendUnavailable(format!(
                    "workspace root is not a directory: {}",
                    root.display()
                )));
            }
        }
        let session_id = SessionId::new();
        self.sessions
            .lock()
            .map_err(|_| KernelError::Backend("native session registry poisoned".into()))?
            .insert(
                session_id.as_str().to_string(),
                Arc::new(AsyncMutex::new(NativeSession {
                    workspace_root,
                    history: Vec::new(),
                    prompt_queue: PromptQueue::default(),
                })),
            );
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
        let interrupted = self.register_operation(&request.operation_id)?;
        let result = async {
            let mut session = session.lock().await;
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
                .next()
                .ok_or_else(|| KernelError::Backend("prompt queue unexpectedly empty".into()))?;
            let result = self
                .run_prompt(
                    &mut session,
                    &request.operation_id,
                    prompt.text,
                    &request.policy,
                    interrupted.as_ref(),
                    events,
                )
                .await;
            match &result {
                Ok(_) => session
                    .prompt_queue
                    .complete(&prompt_id)
                    .map_err(|error| KernelError::Backend(error.to_string()))?,
                Err(_) => session
                    .prompt_queue
                    .cancel(&prompt_id)
                    .map_err(|error| KernelError::Backend(error.to_string()))?,
            }
            result.map(|_| ())
        }
        .await;
        self.finish_operation(&request.operation_id)?;
        result
    }

    async fn interrupt(&self, operation_id: &OperationId) -> Result<(), KernelError> {
        let interrupted = self
            .active_operations
            .lock()
            .map_err(|_| KernelError::Backend("native operation registry poisoned".into()))?
            .get(operation_id.as_str())
            .cloned()
            .ok_or_else(|| KernelError::OperationNotFound(operation_id.as_str().to_string()))?;
        interrupted.store(true, Ordering::SeqCst);
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
            .send(resolution.approved)
            .map_err(|_| KernelError::ApprovalNotFound(resolution.approval_id))
    }
}

#[derive(Debug)]
struct FunctionCall {
    call_id: String,
    name: String,
    arguments: Value,
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
}

impl ModelCollector {
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
            ModelEvent::OutputTextDelta(delta) => self
                .text
                .lock()
                .map_err(|_| ModelError::EventConsumerClosed)?
                .push_str(&delta),
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

fn run_process(root: &Path, program: &str, args: &[String]) -> Result<Value, KernelError> {
    if program.trim().is_empty() || program.contains(['\r', '\n']) {
        return Err(KernelError::PolicyDenied("invalid process program".into()));
    }
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
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

fn risk_score(risk: RiskLevel) -> u8 {
    match risk {
        RiskLevel::ReadOnly => 0,
        RiskLevel::WorkspaceWrite => 1,
        RiskLevel::SystemWrite => 2,
        RiskLevel::ExternalSideEffect => 3,
    }
}

fn ensure_not_interrupted(interrupted: &AtomicBool) -> Result<(), KernelError> {
    if interrupted.load(Ordering::SeqCst) {
        return Err(KernelError::Backend("operation interrupted".into()));
    }
    Ok(())
}

fn model_error(error: ModelError) -> KernelError {
    KernelError::Backend(error.to_string())
}

fn tool_definitions(enable_process_tools: bool) -> Vec<Value> {
    let mut tools = vec![
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
    "You are Mahayana, a product-owned coding and automation Agent. Inspect before editing; prefer minimal, reversible changes; use checkpoints before risky workspace mutations; use workflows for dependent tasks; delegate focused analysis to subagents; never claim a tool succeeded unless its result says so; respect Mahayana approval and platform policy."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mahayana_model::ModelProviderMode;
    use std::collections::VecDeque;

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

    #[tokio::test]
    async fn completes_direct_model_response() {
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
}
