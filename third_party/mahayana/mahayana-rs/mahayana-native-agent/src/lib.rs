//! Product-owned `AgentBackend` implementation for Mahayana.
//!
//! This adapter preserves the existing MiniApp/host contract while replacing
//! the default Codex implementation with the sovereign native engine and MCP
//! transport. Codex can remain behind an explicit compatibility feature.

use async_trait::async_trait;
use mahayana_agent::{
    AgentActivity, AgentActivityStatus, AgentBackend, AgentError, AgentEvent, AgentMessageRequest,
    ApprovalResolution, McpAppSession, OpenMcpAppRequest, SharedAgentEventSink, StartThreadRequest,
};
use mahayana_core::{
    AgentThreadId, ApprovalDecision, ConversationId, Message, MessageId, MessageRole,
    ModelTokenUsage, ModelTokenUsageSnapshot, OperationId,
};
use mahayana_kernel::{
    ApprovalResolution as KernelApprovalResolution, Capability, CapabilitySet, EngineBackend,
    ExecutionPolicy, KernelError, KernelEvent, KernelEventSink, OpenSessionRequest,
    OperationId as KernelOperationId, RunRequest, RuntimeProfile, SessionId, SharedKernelEventSink,
};
use mahayana_mcp_runtime::{NativeMcpRegistry, ResolvedMcpPlugin};
use mahayana_native_engine::NativeEngine;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct NativeAgentConfig {
    pub profile: RuntimeProfile,
    pub workspace_root: Option<PathBuf>,
    pub mcp_registry: NativeMcpRegistry,
}

#[derive(Clone)]
struct NativeThread {
    session_id: SessionId,
    conversation_id: ConversationId,
}

#[derive(Clone)]
struct NativeMcpSession {
    plugin: ResolvedMcpPlugin,
    tools: Vec<Value>,
}

pub struct NativeAgentBackend {
    engine: Arc<NativeEngine>,
    config: NativeAgentConfig,
    threads: Mutex<HashMap<String, NativeThread>>,
    mcp_sessions: Mutex<HashMap<String, NativeMcpSession>>,
    disabled_tools: Mutex<HashMap<String, HashSet<String>>>,
}

impl NativeAgentBackend {
    pub fn new(engine: Arc<NativeEngine>, config: NativeAgentConfig) -> Self {
        Self {
            engine,
            config,
            threads: Mutex::new(HashMap::new()),
            mcp_sessions: Mutex::new(HashMap::new()),
            disabled_tools: Mutex::new(HashMap::new()),
        }
    }

    async fn create_thread(
        &self,
        conversation_id: ConversationId,
    ) -> Result<(AgentThreadId, NativeThread), AgentError> {
        let session_id = self
            .engine
            .open_session(OpenSessionRequest {
                profile: self.config.profile,
                workspace_root: self
                    .config
                    .workspace_root
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string()),
                model: None,
                metadata: json!({"conversationId": conversation_id.as_str()}),
            })
            .await
            .map_err(kernel_error)?;
        let thread_id = AgentThreadId::generated("mahayana-native-thread");
        let thread = NativeThread {
            session_id,
            conversation_id,
        };
        self.threads
            .lock()
            .map_err(|_| AgentError::Backend("native thread registry poisoned".into()))?
            .insert(thread_id.to_string(), thread.clone());
        Ok((thread_id, thread))
    }

    fn thread(&self, id: &AgentThreadId) -> Result<NativeThread, AgentError> {
        self.threads
            .lock()
            .map_err(|_| AgentError::Backend("native thread registry poisoned".into()))?
            .get(&id.to_string())
            .cloned()
            .ok_or_else(|| AgentError::ThreadNotFound(id.clone()))
    }

    fn mcp_session(&self, id: &AgentThreadId) -> Result<NativeMcpSession, AgentError> {
        self.mcp_sessions
            .lock()
            .map_err(|_| AgentError::Backend("native MCP session registry poisoned".into()))?
            .get(&id.to_string())
            .cloned()
            .ok_or_else(|| AgentError::ThreadNotFound(id.clone()))
    }

    fn is_tool_disabled(&self, server: &str, tool: &str) -> Result<bool, AgentError> {
        Ok(self
            .disabled_tools
            .lock()
            .map_err(|_| AgentError::Backend("MCP tool policy registry poisoned".into()))?
            .get(server)
            .is_some_and(|tools| tools.contains(tool)))
    }

    async fn mcp_call(
        &self,
        session: NativeMcpSession,
        tool: String,
        arguments: Value,
    ) -> Result<Value, AgentError> {
        if self.is_tool_disabled(&session.plugin.server_name, &tool)? {
            return Err(AgentError::Unavailable(format!(
                "MCP tool `{tool}` is disabled by Mahayana policy"
            )));
        }
        tokio::task::spawn_blocking(move || session.plugin.client().call_tool(&tool, arguments))
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?
            .map_err(|error| AgentError::Backend(error.to_string()))
    }
}

struct NativeEventBridge {
    operation_id: OperationId,
    conversation_id: ConversationId,
    sink: SharedAgentEventSink,
}

impl KernelEventSink for NativeEventBridge {
    fn emit(&self, event: KernelEvent) -> Result<(), KernelError> {
        let mapped = match event {
            KernelEvent::MessageDelta { delta, .. } => AgentEvent::MessageDelta { delta },
            KernelEvent::MessageCompleted { text, .. } => AgentEvent::MessageCompleted {
                message: Message {
                    id: MessageId::generated("mahayana-native-message"),
                    conversation_id: self.conversation_id.clone(),
                    role: MessageRole::Assistant,
                    text,
                    created_at_ms: now_ms(),
                    metadata: json!({"engine":"mahayana-native"}),
                },
            },
            KernelEvent::UsageUpdated {
                total_tokens,
                input_tokens,
                cached_input_tokens,
                output_tokens,
                reasoning_output_tokens,
                ..
            } => AgentEvent::TokenUsageUpdated {
                usage: ModelTokenUsageSnapshot {
                    total: None,
                    last: ModelTokenUsage {
                        total_tokens: to_i64(total_tokens),
                        input_tokens: to_i64(input_tokens),
                        cached_input_tokens: to_i64(cached_input_tokens),
                        output_tokens: to_i64(output_tokens),
                        reasoning_output_tokens: to_i64(reasoning_output_tokens),
                    },
                    model_context_window: None,
                },
            },
            KernelEvent::ApprovalRequested {
                approval_id,
                title,
                risk,
                details,
                ..
            } => AgentEvent::ApprovalRequested {
                approval_id: mahayana_core::ApprovalId::new(approval_id)
                    .map_err(|error| KernelError::Backend(error.to_string()))?,
                title,
                details: json!({"risk":risk,"details":details}),
            },
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
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("mahayana-native:{kind}:{title}"));
                AgentEvent::Activity {
                    activity: AgentActivity {
                        step_id,
                        kind,
                        title,
                        detail,
                        status: activity_status(&metadata),
                        metadata: Some(metadata),
                    },
                }
            }
            KernelEvent::ToolStarted { tool, .. } if tool == "send_message" => return Ok(()),
            KernelEvent::ToolStarted {
                tool, arguments, ..
            } => {
                let (title, detail) = tool_activity_copy(&tool, false, true);
                AgentEvent::Activity {
                    activity: AgentActivity {
                        step_id: format!("tool:{tool}"),
                        kind: "tool".into(),
                        title,
                        detail,
                        status: AgentActivityStatus::Running,
                        metadata: Some(json!({"tool":tool,"arguments":arguments})),
                    },
                }
            }
            KernelEvent::ToolCompleted { tool, .. } if tool == "send_message" => return Ok(()),
            KernelEvent::ToolCompleted {
                tool,
                output,
                success,
                ..
            } => {
                let (title, detail) = tool_activity_copy(&tool, true, success);
                AgentEvent::Activity {
                    activity: AgentActivity {
                        step_id: format!("tool:{tool}"),
                        kind: "tool".into(),
                        title,
                        detail,
                        status: if success {
                            AgentActivityStatus::Completed
                        } else {
                            AgentActivityStatus::Failed
                        },
                        metadata: Some(json!({"tool":tool,"output":output,"success":success})),
                    },
                }
            }
            KernelEvent::CheckpointCreated {
                checkpoint_id,
                label,
                ..
            } => AgentEvent::Activity {
                activity: AgentActivity {
                    step_id: format!("checkpoint:{checkpoint_id}"),
                    kind: "checkpoint".into(),
                    title: label.unwrap_or_else(|| "Workspace checkpoint".into()),
                    detail: Some(checkpoint_id),
                    status: AgentActivityStatus::Completed,
                    metadata: None,
                },
            },
            KernelEvent::OperationFailed {
                message, retryable, ..
            } => AgentEvent::Activity {
                activity: AgentActivity {
                    step_id: format!("operation:{}", self.operation_id),
                    kind: "operation".into(),
                    title: "Operation failed".into(),
                    detail: Some(message),
                    status: AgentActivityStatus::Failed,
                    metadata: Some(json!({"retryable":retryable})),
                },
            },
            KernelEvent::OperationCompleted { .. } => return Ok(()),
        };
        self.sink
            .emit(mapped)
            .map_err(|error| KernelError::Backend(error.to_string()))
    }
}

#[async_trait]
impl AgentBackend for NativeAgentBackend {
    async fn start_thread(&self, request: StartThreadRequest) -> Result<AgentThreadId, AgentError> {
        self.create_thread(request.conversation_id)
            .await
            .map(|(thread_id, _)| thread_id)
    }

    async fn send_message(
        &self,
        request: AgentMessageRequest,
        events: SharedAgentEventSink,
    ) -> Result<(), AgentError> {
        let thread = self.thread(&request.thread_id)?;
        if let Ok(mcp) = self.mcp_session(&request.thread_id)
            && mcp
                .tools
                .iter()
                .any(|tool| tool.get("name").and_then(Value::as_str) == Some("chat"))
        {
            let result = self
                .mcp_call(
                    mcp,
                    "chat".into(),
                    json!({"message":request.text,"surface":"agent"}),
                )
                .await?;
            return events.emit(AgentEvent::MessageCompleted {
                message: Message {
                    id: MessageId::generated("mcp-chat"),
                    conversation_id: request.conversation_id,
                    role: MessageRole::Assistant,
                    text: mcp_result_text(&result),
                    created_at_ms: now_ms(),
                    metadata: json!({"mcpResult":result}),
                },
            });
        }

        let sink: SharedKernelEventSink = Arc::new(NativeEventBridge {
            operation_id: request.operation_id.clone(),
            conversation_id: thread.conversation_id,
            sink: events,
        });
        self.engine
            .run(
                RunRequest {
                    session_id: thread.session_id,
                    operation_id: KernelOperationId::from_string(request.operation_id.as_str()),
                    input: request.text,
                    policy: policy_for_profile(self.config.profile),
                    required_capabilities: CapabilitySet::new([Capability::Model]),
                    metadata: json!({"clientMessageId":request.client_message_id}),
                },
                sink,
            )
            .await
            .map_err(kernel_error)
    }

    async fn interrupt(&self, operation_id: &OperationId) -> Result<(), AgentError> {
        self.engine
            .interrupt(&KernelOperationId::from_string(operation_id.as_str()))
            .await
            .map_err(kernel_error)
    }

    async fn resolve_approval(&self, resolution: ApprovalResolution) -> Result<(), AgentError> {
        self.engine
            .resolve_approval(KernelApprovalResolution {
                approval_id: resolution.approval_id.to_string(),
                approved: matches!(
                    resolution.decision,
                    ApprovalDecision::Accept | ApprovalDecision::AcceptForSession
                ),
                metadata: resolution.payload,
            })
            .await
            .map_err(kernel_error)
    }

    fn reset_session(&self) -> Result<(), AgentError> {
        NativeEngine::reset_session(&self.engine).map_err(kernel_error)?;
        self.threads
            .lock()
            .map_err(|_| AgentError::Backend("native thread registry poisoned".into()))?
            .clear();
        self.mcp_sessions
            .lock()
            .map_err(|_| AgentError::Backend("native MCP session registry poisoned".into()))?
            .clear();
        self.disabled_tools
            .lock()
            .map_err(|_| AgentError::Backend("MCP tool policy registry poisoned".into()))?
            .clear();
        Ok(())
    }

    async fn list_mcp_servers(&self) -> Result<Vec<Value>, AgentError> {
        let sessions = self
            .mcp_sessions
            .lock()
            .map_err(|_| AgentError::Backend("native MCP session registry poisoned".into()))?;
        let mut servers = sessions
            .values()
            .map(|session| {
                json!({
                    "name":session.plugin.server_name,
                    "pluginId":session.plugin.plugin_id,
                    "status":"connected",
                    "runtime":"mahayana-native"
                })
            })
            .collect::<Vec<_>>();
        servers.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        servers.dedup_by(|left, right| left["name"] == right["name"]);
        Ok(servers)
    }

    async fn list_connector_apps(&self) -> Result<Vec<Value>, AgentError> {
        Ok(Vec::new())
    }

    async fn remove_mcp_server(&self, _server: &str) -> Result<bool, AgentError> {
        Ok(false)
    }

    async fn mcp_custom_instructions(&self) -> Result<HashMap<String, String>, AgentError> {
        Ok(HashMap::new())
    }

    async fn set_mcp_tool_disabled(
        &self,
        server: &str,
        tool: &str,
        disabled: bool,
    ) -> Result<Vec<String>, AgentError> {
        let mut policies = self
            .disabled_tools
            .lock()
            .map_err(|_| AgentError::Backend("MCP tool policy registry poisoned".into()))?;
        let tools = policies.entry(server.to_string()).or_default();
        if disabled {
            tools.insert(tool.to_string());
        } else {
            tools.remove(tool);
        }
        let mut disabled = tools.iter().cloned().collect::<Vec<_>>();
        disabled.sort();
        Ok(disabled)
    }

    async fn refresh_mcp_servers(&self) -> Result<(), AgentError> {
        Ok(())
    }

    async fn call_mcp_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, AgentError> {
        let session = self
            .mcp_sessions
            .lock()
            .map_err(|_| AgentError::Backend("native MCP session registry poisoned".into()))?
            .values()
            .find(|session| session.plugin.server_name == server)
            .cloned()
            .ok_or_else(|| AgentError::Unavailable(format!("MCP server not open: {server}")))?;
        self.mcp_call(session, tool.to_string(), arguments).await
    }

    async fn open_mcp_app(&self, request: OpenMcpAppRequest) -> Result<McpAppSession, AgentError> {
        let platform = request.platform;
        let registry = self.config.mcp_registry.clone();
        let plugin_id = request.plugin_id.clone();
        let resolved =
            tokio::task::spawn_blocking(move || registry.resolve_plugin(&plugin_id, platform))
                .await
                .map_err(|error| AgentError::Backend(error.to_string()))?
                .map_err(|error| AgentError::Unavailable(error.to_string()))?;
        let client = resolved.client();
        let tools = tokio::task::spawn_blocking({
            let client = client.clone();
            move || client.list_tools()
        })
        .await
        .map_err(|error| AgentError::Backend(error.to_string()))?
        .map_err(|error| AgentError::Backend(error.to_string()))?;
        let resources = tokio::task::spawn_blocking({
            let client = client.clone();
            move || client.list_resources().unwrap_or_default()
        })
        .await
        .map_err(|error| AgentError::Backend(error.to_string()))?;
        let home_result = if tools
            .iter()
            .any(|tool| tool.get("name").and_then(Value::as_str) == Some("home"))
        {
            tokio::task::spawn_blocking({
                let client = client.clone();
                move || client.call_tool("home", json!({}))
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?
            .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let (thread_id, _) = self.create_thread(request.conversation_id).await?;
        self.mcp_sessions
            .lock()
            .map_err(|_| AgentError::Backend("native MCP session registry poisoned".into()))?
            .insert(
                thread_id.to_string(),
                NativeMcpSession {
                    plugin: resolved.clone(),
                    tools: tools.clone(),
                },
            );
        Ok(McpAppSession {
            thread_id,
            plugin_id: request.plugin_id,
            server: resolved.server_name,
            command_tools: command_tools(&tools),
            tool_gates: tool_gates(&tools),
            tools,
            home_result,
            ui_resources: resources,
        })
    }

    async fn list_mcp_app_tools(
        &self,
        thread_id: &AgentThreadId,
        server: &str,
    ) -> Result<Vec<Value>, AgentError> {
        let session = self.mcp_session(thread_id)?;
        if session.plugin.server_name != server {
            return Err(AgentError::Unavailable(format!(
                "MCP session is for `{}`, not `{server}`",
                session.plugin.server_name
            )));
        }
        Ok(session.tools)
    }

    async fn call_mcp_app_tool(
        &self,
        thread_id: &AgentThreadId,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, AgentError> {
        let session = self.mcp_session(thread_id)?;
        if session.plugin.server_name != server {
            return Err(AgentError::Unavailable(format!(
                "MCP session is for `{}`, not `{server}`",
                session.plugin.server_name
            )));
        }
        self.mcp_call(session, tool.to_string(), arguments).await
    }

    async fn read_mcp_app_resource(
        &self,
        thread_id: &AgentThreadId,
        server: &str,
        uri: &str,
    ) -> Result<Vec<Value>, AgentError> {
        let session = self.mcp_session(thread_id)?;
        if session.plugin.server_name != server {
            return Err(AgentError::Unavailable(format!(
                "MCP session is for `{}`, not `{server}`",
                session.plugin.server_name
            )));
        }
        let client = session.plugin.client();
        let uri = uri.to_string();
        tokio::task::spawn_blocking(move || client.read_resource(&uri))
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?
            .map_err(|error| AgentError::Backend(error.to_string()))
    }

    fn name(&self) -> &'static str {
        "mahayana-native"
    }
}

fn command_tools(tools: &[Value]) -> HashMap<String, String> {
    let mut commands = HashMap::new();
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let command = tool
            .pointer("/annotations/command")
            .or_else(|| tool.pointer("/_meta/mahayana/command"))
            .and_then(Value::as_str);
        if let Some(command) = command {
            commands.insert(
                command.trim_start_matches('/').to_string(),
                name.to_string(),
            );
        }
    }
    commands
}

fn tool_gates(tools: &[Value]) -> HashMap<String, String> {
    tools
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            let capability = tool
                .pointer("/annotations/requiresCapability")
                .or_else(|| tool.pointer("/_meta/mahayana/entitlement"))?
                .as_str()?;
            Some((name.to_string(), capability.to_string()))
        })
        .collect()
}

fn mcp_result_text(result: &Value) -> String {
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        let text = content
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }
    result
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            serde_json::to_string_pretty(result).unwrap_or_else(|_| "MCP result".into())
        })
}

fn policy_for_profile(profile: RuntimeProfile) -> ExecutionPolicy {
    match profile {
        RuntimeProfile::DesktopFull | RuntimeProfile::Headless => {
            ExecutionPolicy::interactive_default()
        }
        RuntimeProfile::MobileEmbedded | RuntimeProfile::WebWasm => {
            ExecutionPolicy::mobile_default()
        }
    }
}

fn tool_activity_copy(tool: &str, completed: bool, success: bool) -> (String, Option<String>) {
    let (running, done, detail) = match tool {
        "subagent_run" => (
            "请独立子智能体复核第一版",
            "独立复核已完成",
            "检查完整性、可运行性和明显遗漏",
        ),
        "workspace_read" => ("读取工作区文件", "工作区文件已读取", "基于真实文件继续处理"),
        "workspace_write" => ("写入实现文件", "实现文件已写入", "把变更落到实际工作区"),
        "workspace_search" => ("搜索相关代码", "相关代码搜索完成", "定位需要处理的实现位置"),
        "workspace_checkpoint" => (
            "创建安全检查点",
            "安全检查点已创建",
            "为后续修改保留可恢复状态",
        ),
        "process_exec" => ("运行验证命令", "验证命令已完成", "使用真实执行结果检查实现"),
        "git_status" => ("检查改动状态", "改动状态已检查", "确认当前工作区变更"),
        "git_diff" => ("检查代码差异", "代码差异已检查", "核对实际修改内容"),
        "web_search" => ("搜索外部资料", "外部资料搜索完成", "获取任务需要的最新信息"),
        "web_fetch" => ("读取外部资料", "外部资料读取完成", "核对来源内容"),
        "workflow_create" => ("建立执行步骤", "执行步骤已建立", "按依赖关系组织后续工作"),
        "workflow_status" => ("检查任务进度", "任务进度已更新", "确认各步骤当前状态"),
        _ => ("执行工具步骤", "工具步骤已完成", "继续推进实际任务"),
    };
    if completed {
        if success {
            (done.to_string(), Some(detail.to_string()))
        } else {
            (
                format!("{done}（失败）"),
                Some("这一步没有成功，Agent 会根据错误继续处理".into()),
            )
        }
    } else {
        (running.to_string(), Some(detail.to_string()))
    }
}

fn activity_status(metadata: &Value) -> AgentActivityStatus {
    match metadata.get("status").and_then(Value::as_str) {
        Some("completed") => AgentActivityStatus::Completed,
        Some("failed") => AgentActivityStatus::Failed,
        _ => AgentActivityStatus::Running,
    }
}

fn kernel_error(error: KernelError) -> AgentError {
    match error {
        KernelError::SessionNotFound(id) => AgentError::Backend(format!("session not found: {id}")),
        KernelError::OperationNotFound(id) => AgentError::OperationNotFound(
            OperationId::new(id).unwrap_or_else(|_| OperationId::generated("operation")),
        ),
        KernelError::ApprovalNotFound(id) => AgentError::ApprovalNotFound(
            mahayana_core::ApprovalId::new(id)
                .unwrap_or_else(|_| mahayana_core::ApprovalId::generated("approval")),
        ),
        KernelError::BackendUnavailable(message) | KernelError::CapabilityUnavailable(message) => {
            AgentError::Unavailable(message)
        }
        KernelError::EventConsumerClosed => AgentError::EventConsumerClosed,
        other => AgentError::Backend(other.to_string()),
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

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_and_entitlement_metadata_are_product_owned() {
        let tools = vec![json!({
            "name":"publish",
            "annotations":{"command":"/ship","requiresCapability":"publish.pro"}
        })];
        assert_eq!(
            command_tools(&tools).get("ship").map(String::as_str),
            Some("publish")
        );
        assert_eq!(
            tool_gates(&tools).get("publish").map(String::as_str),
            Some("publish.pro")
        );
    }
}
