//! In-process Codex app-server adapter for the Mahayana Agent contract.

use async_trait::async_trait;
use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessAppServerRequestHandle;
use codex_app_server_client::InProcessClientStartArgs;
use codex_app_server_client::InProcessServerEvent;
use codex_app_server_protocol::AppsListParams;
use codex_app_server_protocol::AppsListResponse;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::CodexErrorInfo;
use codex_app_server_protocol::DynamicToolCallOutputContentItem;
use codex_app_server_protocol::DynamicToolCallParams;
use codex_app_server_protocol::DynamicToolCallResponse;
use codex_app_server_protocol::DynamicToolFunctionSpec;
use codex_app_server_protocol::DynamicToolNamespaceSpec;
use codex_app_server_protocol::DynamicToolNamespaceTool;
use codex_app_server_protocol::DynamicToolSpec;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ListMcpServerStatusParams;
use codex_app_server_protocol::ListMcpServerStatusResponse;
use codex_app_server_protocol::McpResourceReadParams;
use codex_app_server_protocol::McpResourceReadResponse;
use codex_app_server_protocol::McpServerOauthLoginParams;
use codex_app_server_protocol::McpServerOauthLoginResponse;
use codex_app_server_protocol::McpServerRefreshResponse;
use codex_app_server_protocol::McpServerStatusDetail;
use codex_app_server_protocol::McpServerToolCallParams;
use codex_app_server_protocol::McpServerToolCallResponse;
use codex_app_server_protocol::PluginInstallParams;
use codex_app_server_protocol::PluginInstallResponse;
use codex_app_server_protocol::PluginInstalledParams;
use codex_app_server_protocol::PluginInstalledResponse;
use codex_app_server_protocol::PluginReadParams;
use codex_app_server_protocol::PluginReadResponse;
use codex_app_server_protocol::PluginRuntimePlatform as CodexPluginRuntimePlatform;
use codex_app_server_protocol::PluginSource;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::ThreadTokenUsage;
use codex_app_server_protocol::TokenUsageBreakdown;
use codex_app_server_protocol::TurnError;
use codex_app_server_protocol::TurnInterruptParams;
use codex_app_server_protocol::TurnInterruptResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_arg0::Arg0DispatchPaths;
use codex_config::CloudConfigBundleLoader;
use codex_config::LoaderOverrides;
use codex_config::McpServerConfig;
use codex_config::McpServerTransportConfig;
use codex_core::McpManager;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_core::config::find_codex_home;
use codex_core::config::load_global_mcp_servers;
use codex_core::plugin_workbench::mcp_app_orchestration_profile;
use codex_core_plugins::PluginsManager;
use codex_core_plugins::loader::load_plugin_mcp_servers;
use codex_exec_server::EnvironmentManager;
use codex_exec_server::ExecServerRuntimePaths;
use codex_feedback::CodexFeedback;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::SessionSource;
use codex_rmcp_client::delete_oauth_tokens;
use mahayana_agent::AgentActivity;
use mahayana_agent::AgentActivityStatus;
use mahayana_agent::AgentBackend;
use mahayana_agent::AgentError;
use mahayana_agent::AgentEvent;
use mahayana_agent::AgentMessageRequest;
use mahayana_agent::ApprovalResolution;
use mahayana_agent::McpAppSession;
use mahayana_agent::OpenMcpAppRequest;
use mahayana_agent::SharedAgentEventSink;
use mahayana_agent::StartThreadRequest;
use mahayana_conversation::ConversationProvider;
use mahayana_conversation::provider_key_for_conversation_id;
use mahayana_core::AgentThreadId;
use mahayana_core::ApprovalDecision;
use mahayana_core::ApprovalId;
use mahayana_core::ConversationId;
use mahayana_core::Message;
use mahayana_core::MessageId;
use mahayana_core::MessageRole;
use mahayana_core::ModelTokenUsage;
use mahayana_core::ModelTokenUsageSnapshot;
use mahayana_core::OperationId;
use mahayana_host_protocol::ComputerAction;
use mahayana_host_protocol::ComputerControlOrigin;
use mahayana_host_protocol::LocalToolPermission;
use mahayana_platform_core::HostPlatform;
use mahayana_plugin_host::LocalPlugin;
use mahayana_plugin_host::PluginRuntimeVariant as MahayanaPluginRuntimeVariant;
use mahayana_plugin_host::select_runtime_with_availability;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::sync::oneshot;

const PROVIDER_ID: &str = "dacheng-deepseek";

fn shared_installed_plugin_overrides(
    runtime_codex_home: &Path,
) -> Result<Vec<(String, toml::Value)>, AgentError> {
    let Ok(shared_codex_home) = find_codex_home() else {
        return Ok(Vec::new());
    };
    if shared_codex_home.as_path() == runtime_codex_home {
        return Ok(Vec::new());
    }
    let config_path = shared_codex_home.join("config.toml");
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(AgentError::Backend(error.to_string())),
    };
    let config = toml::from_str::<toml::Value>(&contents)
        .map_err(|error| AgentError::Backend(error.to_string()))?;
    Ok(shared_installed_plugin_overrides_from_config(
        &config,
        shared_codex_home.as_path(),
    ))
}

fn shared_installed_plugin_roots(runtime_codex_home: &Path) -> Result<Vec<PathBuf>, AgentError> {
    let Ok(shared_codex_home) = find_codex_home() else {
        return Ok(Vec::new());
    };
    if shared_codex_home.as_path() == runtime_codex_home {
        return Ok(Vec::new());
    }
    let contents = match std::fs::read_to_string(shared_codex_home.join("config.toml")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(AgentError::Backend(error.to_string())),
    };
    let config = toml::from_str::<toml::Value>(&contents)
        .map_err(|error| AgentError::Backend(error.to_string()))?;
    Ok(shared_installed_plugin_roots_from_config(
        &config,
        shared_codex_home.as_path(),
    ))
}

fn shared_installed_plugin_roots_from_config(
    config: &toml::Value,
    shared_codex_home: &Path,
) -> Vec<PathBuf> {
    let local_marketplaces = config
        .get("marketplaces")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|marketplaces| marketplaces.iter())
        .filter_map(|(name, marketplace)| {
            let table = marketplace.as_table()?;
            (table.get("source_type").and_then(toml::Value::as_str) == Some("local"))
                .then_some(())?;
            let source = PathBuf::from(table.get("source")?.as_str()?);
            let source = if source.is_absolute() {
                source
            } else {
                shared_codex_home.join(source)
            };
            Some((name.clone(), source))
        })
        .collect::<HashMap<_, _>>();
    let mut roots = Vec::new();
    for (key, plugin) in config
        .get("plugins")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|plugins| plugins.iter())
    {
        if plugin.get("enabled").and_then(toml::Value::as_bool) != Some(true) {
            continue;
        }
        let Some((plugin_name, marketplace_name)) = key.rsplit_once('@') else {
            continue;
        };
        let Some(marketplace_root) = local_marketplaces.get(marketplace_name) else {
            continue;
        };
        let manifest_path = [
            marketplace_root.join("marketplace.json"),
            marketplace_root.join(".agents/plugins/marketplace.json"),
        ]
        .into_iter()
        .find(|path| path.is_file());
        let Some(manifest_path) = manifest_path else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Value>(&contents) else {
            continue;
        };
        let Some(source_path) = manifest
            .get("plugins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| entry.get("name").and_then(Value::as_str) == Some(plugin_name))
            .and_then(|entry| entry.get("source"))
            .filter(|source| source.get("source").and_then(Value::as_str) == Some("local"))
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let source_path = PathBuf::from(source_path);
        let plugin_root = if source_path.is_absolute() {
            source_path
        } else {
            manifest_path
                .parent()
                .unwrap_or(marketplace_root)
                .join(source_path)
        };
        if plugin_root.join(".codex-plugin/plugin.json").is_file() {
            roots.push(plugin_root);
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

fn safe_config_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'@'))
}

fn shared_installed_plugin_overrides_from_config(
    config: &toml::Value,
    shared_codex_home: &Path,
) -> Vec<(String, toml::Value)> {
    let mut overrides = Vec::new();
    let mut imported_marketplaces = HashSet::new();
    let mut marketplace_overrides = toml::map::Map::new();
    let mut plugin_overrides = toml::map::Map::new();
    let mut mcp_server_overrides = toml::map::Map::new();
    if let Some(marketplaces) = config.get("marketplaces").and_then(toml::Value::as_table) {
        for (name, marketplace) in marketplaces {
            if !safe_config_key(name) {
                continue;
            }
            let Some(table) = marketplace.as_table() else {
                continue;
            };
            if table.get("source_type").and_then(toml::Value::as_str) != Some("local") {
                continue;
            }
            let Some(source) = table.get("source").and_then(toml::Value::as_str) else {
                continue;
            };
            let source = PathBuf::from(source);
            let source = if source.is_absolute() {
                source
            } else {
                shared_codex_home.join(source)
            };
            if !source.join("marketplace.json").is_file()
                && !source.join(".agents/plugins/marketplace.json").is_file()
            {
                continue;
            }
            imported_marketplaces.insert(name.clone());
            marketplace_overrides.insert(name.clone(), marketplace.clone());
        }
    }

    if let Some(plugins) = config.get("plugins").and_then(toml::Value::as_table) {
        for (key, plugin) in plugins {
            if !safe_config_key(key)
                || plugin.get("enabled").and_then(toml::Value::as_bool) != Some(true)
            {
                continue;
            }
            let Some((_, marketplace)) = key.rsplit_once('@') else {
                continue;
            };
            if !imported_marketplaces.contains(marketplace) {
                continue;
            }
            plugin_overrides.insert(key.clone(), plugin.clone());
        }
    }

    if let Some(mcp_servers) = config.get("mcp_servers").and_then(toml::Value::as_table) {
        for (name, server) in mcp_servers {
            if safe_config_key(name) {
                mcp_server_overrides.insert(name.clone(), server.clone());
            }
        }
    }
    if !marketplace_overrides.is_empty() {
        overrides.push((
            "marketplaces".into(),
            toml::Value::Table(marketplace_overrides),
        ));
    }
    if !plugin_overrides.is_empty() {
        overrides.push(("plugins".into(), toml::Value::Table(plugin_overrides)));
    }
    if !mcp_server_overrides.is_empty() {
        overrides.push((
            "mcp_servers".into(),
            toml::Value::Table(mcp_server_overrides),
        ));
    }
    if !imported_marketplaces.is_empty() {
        overrides.push(("features.plugins".into(), toml::Value::Boolean(true)));
    }
    overrides
}

fn debug_plugin_inheritance(label: &str, names: impl Iterator<Item = String>) {
    if std::env::var("MAHAYANA_DEBUG_PLUGIN_INHERITANCE").as_deref() != Ok("1") {
        return;
    }
    let mut names = names.collect::<Vec<_>>();
    names.sort();
    names.dedup();
    eprintln!("mahayana plugin inheritance {label}: {}", names.join(","));
}

fn debug_mcp_runtime_configs(configs: &HashMap<String, McpServerConfig>) {
    if std::env::var("MAHAYANA_DEBUG_PLUGIN_INHERITANCE").as_deref() != Ok("1") {
        return;
    }
    let mut names = configs.keys().cloned().collect::<Vec<_>>();
    names.sort();
    for name in names {
        let Some(config) = configs.get(&name) else {
            continue;
        };
        if let McpServerTransportConfig::Stdio { command, cwd, .. } = &config.transport {
            eprintln!(
                "mahayana MCP config {name}: enabled={} disabled_reason={} available={} command={command} cwd={}",
                config.enabled,
                config.disabled_reason.is_some(),
                mcp_runtime_available(config),
                cwd.as_ref().map(|cwd| cwd.as_str()).unwrap_or("")
            );
        }
    }
}

#[derive(Clone)]
pub struct CodexAgentConfig {
    pub codex_home: PathBuf,
    /// Reuse enabled local Codex plugins and MCP servers from the user's
    /// standard Codex installation without sharing auth or conversation state.
    pub inherit_installed_plugins: bool,
    pub cwd: PathBuf,
    pub workspace_roots: Vec<PathBuf>,
    pub model: String,
    pub responses_base_url: String,
    /// Developer/CLI self-test mode that reuses the user's existing Codex
    /// account provider instead of the first-party Dacheng Responses adapter.
    pub use_codex_account: bool,
    /// Optional Fabushi product session token. Logged-out users use the
    /// first-party anonymous allowance; logged-in users get their account
    /// quota. The token is injected only into the in-memory provider and is
    /// never written to Codex auth files.
    pub product_session_token: Option<String>,
    pub sandbox_mode: SandboxMode,
    pub approval_policy: AskForApproval,
    /// Mahayana CLI executable used for Codex's hidden process helper modes.
    /// Embedded SDK hosts omit it and use the in-process `mahayana` workspace
    /// tools instead, because a mobile application executable cannot implement
    /// desktop argv dispatch.
    pub codex_executable_path: Option<PathBuf>,
    /// Non-Agent providers owned by this same Mahayana Runtime. Codex receives
    /// bounded read-only tools for inspecting their conversations.
    pub conversation_providers: Vec<Arc<dyn ConversationProvider>>,
}

impl CodexAgentConfig {
    pub fn validate(&self) -> Result<(), AgentError> {
        if self
            .product_session_token
            .as_deref()
            .is_some_and(|token| token.trim().is_empty() || token.contains(['\r', '\n']))
        {
            return Err(AgentError::Unavailable(
                "Mahayana product session is invalid".into(),
            ));
        }
        if !self.use_codex_account && self.model.trim().is_empty() {
            return Err(AgentError::Backend(
                "Dacheng DeepSeek model configuration is incomplete".into(),
            ));
        }
        if !self.use_codex_account && !responses_endpoint_is_secure(&self.responses_base_url) {
            return Err(AgentError::Backend(
                "Dacheng Responses endpoint must use HTTPS or loopback HTTP".into(),
            ));
        }
        Ok(())
    }
}

fn responses_endpoint_is_secure(endpoint: &str) -> bool {
    if endpoint.contains(['\r', '\n']) {
        return false;
    }
    endpoint.starts_with("https://")
        || endpoint.starts_with("http://127.0.0.1:")
        || endpoint.starts_with("http://localhost:")
        || endpoint.starts_with("http://[::1]:")
}

struct ActiveOperation {
    thread_id: String,
    turn_id: String,
    conversation_id: mahayana_core::ConversationId,
    events: SharedAgentEventSink,
    assistant_text: String,
    completion: oneshot::Sender<Result<(), AgentError>>,
}

struct PendingApproval {
    request_id: RequestId,
    response_kind: ApprovalResponseKind,
}

enum ApprovalResponseKind {
    CommandExecution,
    FileChange,
    Permissions { requested: Value },
    LegacyExec,
    LegacyPatch,
    ComputerDynamic { params: DynamicToolCallParams },
}

struct CodexAgentInner {
    config: Arc<codex_core::config::Config>,
    requests: InProcessAppServerRequestHandle,
    shared_installed_plugin_roots: Vec<PathBuf>,
    next_request_id: AtomicI64,
    operations: Mutex<HashMap<OperationId, ActiveOperation>>,
    approvals: Mutex<HashMap<ApprovalId, PendingApproval>>,
    computer_session_approved: AtomicBool,
    conversation_providers: Vec<Arc<dyn ConversationProvider>>,
}

impl CodexAgentInner {
    fn request_id(&self) -> RequestId {
        RequestId::Integer(self.next_request_id.fetch_add(1, Ordering::Relaxed))
    }

    async fn reject_server_request(
        &self,
        request_id: RequestId,
        message: impl Into<String>,
    ) -> Result<(), AgentError> {
        self.requests
            .reject_server_request(
                request_id,
                JSONRPCErrorError {
                    code: -32601,
                    message: message.into(),
                    data: None,
                },
            )
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))
    }

    fn operation_sink(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> Result<Option<SharedAgentEventSink>, AgentError> {
        let operations = self
            .operations
            .lock()
            .map_err(|_| AgentError::Backend("operation mutex poisoned".into()))?;
        Ok(operations
            .values()
            .find(|operation| {
                operation.thread_id == thread_id
                    && turn_id.is_none_or(|turn_id| operation.turn_id == turn_id)
            })
            .map(|operation| Arc::clone(&operation.events)))
    }

    fn remember_approval(
        &self,
        request_id: RequestId,
        response_kind: ApprovalResponseKind,
        thread_id: &str,
        turn_id: Option<&str>,
        title: &str,
        details: Value,
    ) -> Result<(), AgentError> {
        let events = self
            .operation_sink(thread_id, turn_id)?
            .ok_or_else(|| AgentError::Backend("approval has no active Codex turn".into()))?;
        let approval_id = ApprovalId::generated("codex-approval");
        self.approvals
            .lock()
            .map_err(|_| AgentError::Backend("approval mutex poisoned".into()))?
            .insert(
                approval_id.clone(),
                PendingApproval {
                    request_id,
                    response_kind,
                },
            );
        events.emit(AgentEvent::ApprovalRequested {
            approval_id,
            title: title.into(),
            details,
        })
    }

    async fn remember_or_reject(
        &self,
        request_id: RequestId,
        response_kind: ApprovalResponseKind,
        thread_id: &str,
        turn_id: Option<&str>,
        title: &str,
        details: Value,
    ) -> Result<(), AgentError> {
        match self.remember_approval(
            request_id.clone(),
            response_kind,
            thread_id,
            turn_id,
            title,
            details,
        ) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.reject_server_request(request_id, error.to_string())
                    .await
            }
        }
    }

    async fn resolve_dynamic_tool_response(
        &self,
        request_id: RequestId,
        response: DynamicToolCallResponse,
    ) -> Result<(), AgentError> {
        let response = serde_json::to_value(response)
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        self.requests
            .resolve_server_request(request_id, response)
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))
    }

    async fn handle_dynamic_tool_request(
        &self,
        request_id: RequestId,
        params: DynamicToolCallParams,
    ) -> Result<(), AgentError> {
        if params.namespace.as_deref() == Some("mahayana") && params.tool == "computer" {
            let policy = mahayana_computer::control_policy();
            if !policy.local_execution_enabled || !policy.ai_control_enabled {
                return self
                    .resolve_dynamic_tool_response(
                        request_id,
                        dynamic_tool_error("AI computer control is disabled on this computer"),
                    )
                    .await;
            }
            if policy.local_tool_permission == LocalToolPermission::Never {
                return self
                    .resolve_dynamic_tool_response(
                        request_id,
                        dynamic_tool_error("AI local-tool permission is set to Never"),
                    )
                    .await;
            }
            let session_approved = self.computer_session_approved.load(Ordering::Relaxed);
            if policy.local_tool_permission == LocalToolPermission::Ask && !session_approved {
                let action = params
                    .arguments
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("computer");
                let description = params
                    .arguments
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("AI wants to interact with the user's computer");
                let details = json!({
                    "kind": "local-tool",
                    "capability": "computer.control",
                    "reason": description,
                    "subject": format!("Computer · {action}"),
                    "detail": params.arguments.to_string(),
                    "location": "local",
                    "proposedRule": "Allow AI computer control on this computer",
                });
                return self
                    .remember_or_reject(
                        request_id,
                        ApprovalResponseKind::ComputerDynamic {
                            params: params.clone(),
                        },
                        &params.thread_id,
                        Some(&params.turn_id),
                        "AI 请求控制这台电脑",
                        details,
                    )
                    .await;
            }
        }
        let response = self.execute_dynamic_tool(params).await;
        self.resolve_dynamic_tool_response(request_id, response)
            .await
    }

    async fn handle_server_request(&self, request: ServerRequest) -> Result<(), AgentError> {
        match request {
            ServerRequest::DynamicToolCall { request_id, params } => {
                self.handle_dynamic_tool_request(request_id, params).await
            }
            ServerRequest::CommandExecutionRequestApproval { request_id, params } => {
                let details = serde_json::to_value(&params)
                    .map_err(|error| AgentError::Backend(error.to_string()))?;
                self.remember_or_reject(
                    request_id,
                    ApprovalResponseKind::CommandExecution,
                    &params.thread_id,
                    Some(&params.turn_id),
                    "Codex 请求执行命令",
                    details,
                )
                .await
            }
            ServerRequest::FileChangeRequestApproval { request_id, params } => {
                let details = serde_json::to_value(&params)
                    .map_err(|error| AgentError::Backend(error.to_string()))?;
                self.remember_or_reject(
                    request_id,
                    ApprovalResponseKind::FileChange,
                    &params.thread_id,
                    Some(&params.turn_id),
                    "Codex 请求修改文件",
                    details,
                )
                .await
            }
            ServerRequest::PermissionsRequestApproval { request_id, params } => {
                let requested = serde_json::to_value(&params.permissions)
                    .map_err(|error| AgentError::Backend(error.to_string()))?;
                let details = serde_json::to_value(&params)
                    .map_err(|error| AgentError::Backend(error.to_string()))?;
                self.remember_or_reject(
                    request_id,
                    ApprovalResponseKind::Permissions { requested },
                    &params.thread_id,
                    Some(&params.turn_id),
                    "Codex 请求扩展权限",
                    details,
                )
                .await
            }
            ServerRequest::ExecCommandApproval { request_id, params } => {
                let thread_id = params.conversation_id.to_string();
                let details = serde_json::to_value(&params)
                    .map_err(|error| AgentError::Backend(error.to_string()))?;
                self.remember_or_reject(
                    request_id,
                    ApprovalResponseKind::LegacyExec,
                    &thread_id,
                    None,
                    "Codex 请求执行命令",
                    details,
                )
                .await
            }
            ServerRequest::ApplyPatchApproval { request_id, params } => {
                let thread_id = params.conversation_id.to_string();
                let details = serde_json::to_value(&params)
                    .map_err(|error| AgentError::Backend(error.to_string()))?;
                self.remember_or_reject(
                    request_id,
                    ApprovalResponseKind::LegacyPatch,
                    &thread_id,
                    None,
                    "Codex 请求应用补丁",
                    details,
                )
                .await
            }
            request => {
                self.reject_server_request(
                    request.id().clone(),
                    "this embedded Mahayana surface does not support the requested interaction",
                )
                .await
            }
        }
    }

    fn emit_computer_activity(
        &self,
        params: &DynamicToolCallParams,
        status: AgentActivityStatus,
        detail: Option<String>,
    ) {
        let Ok(Some(events)) = self.operation_sink(&params.thread_id, Some(&params.turn_id)) else {
            return;
        };
        let action = params
            .arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("computer");
        let _ = events.emit(AgentEvent::Activity {
            activity: AgentActivity {
                step_id: params.call_id.clone(),
                kind: "computer".into(),
                title: format!("Computer · {action}"),
                detail,
                status,
                metadata: Some(json!({
                    "type": "computerUse",
                    "origin": "ai",
                    "arguments": params.arguments,
                })),
            },
        });
    }

    async fn execute_computer_dynamic_tool(
        &self,
        params: &DynamicToolCallParams,
    ) -> DynamicToolCallResponse {
        let mut primary_value = params.arguments.clone();
        if let Some(object) = primary_value.as_object_mut() {
            object.remove("then");
        }
        let primary = match serde_json::from_value::<ComputerAction>(primary_value) {
            Ok(action) => action,
            Err(error) => return dynamic_tool_error(&format!("invalid Computer action: {error}")),
        };
        let follow_ups = match params.arguments.get("then") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => match serde_json::from_value::<Vec<ComputerAction>>(value.clone()) {
                Ok(actions) => actions,
                Err(error) => {
                    return dynamic_tool_error(&format!("invalid Computer then sequence: {error}"));
                }
            },
        };
        let mut actions = Vec::with_capacity(1 + follow_ups.len());
        actions.push(primary);
        actions.extend(follow_ups);
        self.emit_computer_activity(
            params,
            AgentActivityStatus::Running,
            Some(params.arguments.to_string()),
        );
        let execute_actions = actions.clone();
        let result = match tokio::task::spawn_blocking(move || {
            mahayana_computer::execute(&execute_actions, ComputerControlOrigin::Ai)
        })
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                self.emit_computer_activity(
                    params,
                    AgentActivityStatus::Failed,
                    Some(error.to_string()),
                );
                return dynamic_tool_error(&error.to_string());
            }
            Err(error) => {
                self.emit_computer_activity(
                    params,
                    AgentActivityStatus::Failed,
                    Some(error.to_string()),
                );
                return dynamic_tool_error(&format!("Computer executor stopped: {error}"));
            }
        };
        self.emit_computer_activity(
            params,
            AgentActivityStatus::Completed,
            Some(format!("{} action(s) completed", result.actions_executed)),
        );
        DynamicToolCallResponse {
            content_items: vec![
                DynamicToolCallOutputContentItem::InputText {
                    text: json!({
                        "ok": true,
                        "actionsExecuted": result.actions_executed,
                        "capturedAtMs": result.snapshot.captured_at_ms,
                        "width": result.snapshot.width,
                        "height": result.snapshot.height,
                        "message": "Computer action ran on this user's computer. The image is the resulting screen."
                    })
                    .to_string(),
                },
                DynamicToolCallOutputContentItem::InputImage {
                    image_url: result.snapshot.data_url,
                },
            ],
            success: true,
        }
    }

    async fn execute_dynamic_tool(&self, params: DynamicToolCallParams) -> DynamicToolCallResponse {
        if params.namespace.as_deref() != Some("mahayana") {
            return dynamic_tool_error("不支持的工具命名空间");
        }
        let result = match params.tool.as_str() {
            "computer" => return self.execute_computer_dynamic_tool(&params).await,
            "list_conversations" => {
                let mut conversations = Vec::new();
                for provider in &self.conversation_providers {
                    match provider.list_conversations().await {
                        Ok(items) => conversations.extend(items),
                        Err(error) => {
                            return dynamic_tool_error(&format!(
                                "{} 联系人读取失败：{error}",
                                provider.key()
                            ));
                        }
                    }
                }
                conversations.sort_by(|left, right| {
                    right
                        .updated_at_ms
                        .cmp(&left.updated_at_ms)
                        .then_with(|| left.id.cmp(&right.id))
                });
                serde_json::to_value(conversations)
            }
            "conversation_history" => {
                let Some(conversation_id) = params
                    .arguments
                    .get("conversationId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                else {
                    return dynamic_tool_error("conversationId 不能为空");
                };
                let conversation_id = ConversationId(conversation_id.to_string());
                let provider_key = match provider_key_for_conversation_id(&conversation_id) {
                    Ok(key) => key,
                    Err(error) => return dynamic_tool_error(&error.to_string()),
                };
                let Some(provider) = self
                    .conversation_providers
                    .iter()
                    .find(|provider| provider.key() == provider_key)
                else {
                    return dynamic_tool_error("该会话不属于当前大乘 Runtime");
                };
                let limit = params
                    .arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 100) as u32;
                match provider.history(&conversation_id, limit).await {
                    Ok(messages) => serde_json::to_value(messages),
                    Err(error) => return dynamic_tool_error(&error.to_string()),
                }
            }
            "read_workspace_file" => {
                let Some(path) = required_string_argument(&params.arguments, "path") else {
                    return dynamic_tool_error("path 不能为空");
                };
                match self.read_workspace_file(path).await {
                    Ok((relative_path, contents)) => Ok(json!({
                        "path": relative_path,
                        "contents": contents,
                    })),
                    Err(error) => return dynamic_tool_error(&error),
                }
            }
            "write_workspace_file" => {
                let Some(path) = required_string_argument(&params.arguments, "path") else {
                    return dynamic_tool_error("path 不能为空");
                };
                let Some(contents) = params.arguments.get("contents").and_then(Value::as_str)
                else {
                    return dynamic_tool_error("contents 必须是字符串");
                };
                match self.write_workspace_file(path, contents).await {
                    Ok((relative_path, bytes)) => Ok(json!({
                        "path": relative_path,
                        "bytesWritten": bytes,
                    })),
                    Err(error) => return dynamic_tool_error(&error),
                }
            }
            "list_workspace_files" => match self.list_workspace_files().await {
                Ok(files) => Ok(json!({"files": files})),
                Err(error) => return dynamic_tool_error(&error),
            },
            _ => return dynamic_tool_error("不支持的大乘工具"),
        };
        match result {
            Ok(value) => dynamic_tool_success(value),
            Err(error) => dynamic_tool_error(&error.to_string()),
        }
    }

    fn workspace_root(&self) -> Result<PathBuf, String> {
        let root = self
            .config
            .workspace_roots
            .first()
            .map(|root| root.to_path_buf())
            .unwrap_or_else(|| self.config.cwd.to_path_buf());
        std::fs::canonicalize(&root)
            .map_err(|error| format!("无法访问大乘工作区 {}：{error}", root.display()))
    }

    fn relative_workspace_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err("工作区路径不能为空".into());
        }
        let path = Path::new(trimmed);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("工作区路径必须是安全的相对路径，不能包含 .、.. 或绝对路径".into());
        }
        Ok(path.to_path_buf())
    }

    fn checked_workspace_path(
        &self,
        raw_path: &str,
        must_exist: bool,
    ) -> Result<(PathBuf, PathBuf), String> {
        let relative_path = self.relative_workspace_path(raw_path)?;
        let root = self.workspace_root()?;
        let candidate = root.join(&relative_path);
        let checked_path = if must_exist || candidate.exists() {
            std::fs::canonicalize(&candidate).map_err(|error| {
                format!("无法访问工作区文件 {}：{error}", relative_path.display())
            })?
        } else {
            let parent = candidate
                .parent()
                .ok_or_else(|| "工作区文件缺少父目录".to_string())?;
            let checked_parent = std::fs::canonicalize(parent).map_err(|error| {
                format!("工作区父目录不存在 {}：{error}", relative_path.display())
            })?;
            checked_parent.join(
                candidate
                    .file_name()
                    .ok_or_else(|| "工作区文件名无效".to_string())?,
            )
        };
        if !checked_path.starts_with(&root) {
            return Err("工作区路径越过了应用沙箱边界".into());
        }
        Ok((relative_path, checked_path))
    }

    async fn read_workspace_file(&self, raw_path: &str) -> Result<(String, String), String> {
        let (relative_path, path) = self.checked_workspace_path(raw_path, true)?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|error| format!("无法读取文件元数据：{error}"))?;
        if !metadata.is_file() {
            return Err("目标不是普通文件".into());
        }
        if metadata.len() > 512 * 1024 {
            return Err("单个工作区文本文件不能超过 512 KiB".into());
        }
        let contents = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| format!("无法读取 UTF-8 文本文件：{error}"))?;
        Ok((relative_path.to_string_lossy().into_owned(), contents))
    }

    async fn write_workspace_file(
        &self,
        raw_path: &str,
        contents: &str,
    ) -> Result<(String, usize), String> {
        if contents.len() > 512 * 1024 {
            return Err("单个工作区文本文件不能超过 512 KiB".into());
        }
        let (relative_path, path) = self.checked_workspace_path(raw_path, false)?;
        tokio::fs::write(path, contents)
            .await
            .map_err(|error| format!("无法写入工作区文件：{error}"))?;
        Ok((relative_path.to_string_lossy().into_owned(), contents.len()))
    }

    async fn list_workspace_files(&self) -> Result<Vec<Value>, String> {
        let root = self.workspace_root()?;
        let mut entries = tokio::fs::read_dir(root)
            .await
            .map_err(|error| format!("无法列出大乘工作区：{error}"))?;
        let mut files = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| format!("无法读取大乘工作区条目：{error}"))?
        {
            if files.len() >= 200 {
                break;
            }
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| format!("无法读取工作区文件类型：{error}"))?;
            files.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "kind": if file_type.is_file() {
                    "file"
                } else if file_type.is_dir() {
                    "directory"
                } else {
                    "other"
                },
            }));
        }
        files.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(files)
    }

    fn update_delta(
        &self,
        thread_id: &str,
        turn_id: &str,
        delta: String,
    ) -> Result<(), AgentError> {
        let events = {
            let mut operations = self
                .operations
                .lock()
                .map_err(|_| AgentError::Backend("operation mutex poisoned".into()))?;
            let Some(operation) = operations
                .values_mut()
                .find(|operation| operation.thread_id == thread_id && operation.turn_id == turn_id)
            else {
                return Ok(());
            };
            operation.assistant_text.push_str(&delta);
            Arc::clone(&operation.events)
        };
        events.emit(AgentEvent::MessageDelta { delta })
    }

    fn update_token_usage(
        &self,
        thread_id: &str,
        turn_id: &str,
        usage: ThreadTokenUsage,
    ) -> Result<(), AgentError> {
        let Some(events) = self.operation_sink(thread_id, Some(turn_id))? else {
            return Ok(());
        };
        events.emit(AgentEvent::TokenUsageUpdated {
            usage: ModelTokenUsageSnapshot {
                total: Some(model_token_usage(usage.total)),
                last: model_token_usage(usage.last),
                model_context_window: usage.model_context_window,
            },
        })
    }

    fn update_tool_progress(
        &self,
        thread_id: &str,
        turn_id: &str,
        message: String,
    ) -> Result<(), AgentError> {
        let Some(events) = self.operation_sink(thread_id, Some(turn_id))? else {
            return Ok(());
        };
        events.emit(AgentEvent::ToolProgress { message })
    }

    fn update_item_activity(
        &self,
        thread_id: &str,
        turn_id: &str,
        item: &ThreadItem,
        status: AgentActivityStatus,
    ) -> Result<(), AgentError> {
        let Some(activity) = thread_item_activity(item, status) else {
            return Ok(());
        };
        let Some(events) = self.operation_sink(thread_id, Some(turn_id))? else {
            return Ok(());
        };
        events.emit(AgentEvent::Activity { activity })
    }

    fn replace_completed_text(
        &self,
        thread_id: &str,
        turn_id: &str,
        text: String,
    ) -> Result<(), AgentError> {
        if let Some(operation) = self
            .operations
            .lock()
            .map_err(|_| AgentError::Backend("operation mutex poisoned".into()))?
            .values_mut()
            .find(|operation| operation.thread_id == thread_id && operation.turn_id == turn_id)
        {
            merge_completed_agent_text(&mut operation.assistant_text, text);
        }
        Ok(())
    }

    fn take_operation(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<Option<ActiveOperation>, AgentError> {
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| AgentError::Backend("operation mutex poisoned".into()))?;
        let operation_id = operations.iter().find_map(|(operation_id, operation)| {
            (operation.thread_id == thread_id && operation.turn_id == turn_id)
                .then(|| operation_id.clone())
        });
        Ok(operation_id.and_then(|operation_id| operations.remove(&operation_id)))
    }

    fn complete_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
        status: TurnStatus,
        turn_error: Option<TurnError>,
    ) -> Result<(), AgentError> {
        if status == TurnStatus::InProgress {
            return Ok(());
        }
        let Some(operation) = self.take_operation(thread_id, turn_id)? else {
            return Ok(());
        };
        let result = match status {
            TurnStatus::Completed => operation
                .events
                .emit(AgentEvent::MessageCompleted {
                    message: Message {
                        id: MessageId::generated("message"),
                        conversation_id: operation.conversation_id,
                        role: MessageRole::Assistant,
                        text: operation.assistant_text,
                        created_at_ms: now_ms(),
                        metadata: json!({
                            "codexThreadId": thread_id,
                            "codexTurnId": turn_id,
                            "embedded": true,
                        }),
                    },
                })
                .map_err(|error| AgentError::Backend(error.to_string())),
            TurnStatus::Interrupted => Err(AgentError::Backend(
                "Codex turn was interrupted".to_string(),
            )),
            TurnStatus::Failed => Err(turn_error
                .map(agent_error_from_turn)
                .unwrap_or_else(|| AgentError::Backend("Codex turn failed".to_string()))),
            TurnStatus::InProgress => unreachable!(),
        };
        let _ = operation.completion.send(result);
        Ok(())
    }

    fn fail_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
        error: AgentError,
    ) -> Result<(), AgentError> {
        if let Some(operation) = self.take_operation(thread_id, turn_id)? {
            let _ = operation.completion.send(Err(error));
        }
        Ok(())
    }

    fn fail_all(&self, message: &str) {
        if let Ok(mut operations) = self.operations.lock() {
            for (_, operation) in operations.drain() {
                let _ = operation
                    .completion
                    .send(Err(AgentError::Backend(message.to_string())));
            }
        }
    }

    async fn handle_event(&self, event: InProcessServerEvent) -> Result<(), AgentError> {
        match event {
            InProcessServerEvent::Lagged { skipped } => {
                self.fail_all(&format!(
                    "in-process Codex event stream lagged by {skipped} events"
                ));
                Ok(())
            }
            InProcessServerEvent::ServerRequest(request) => {
                self.handle_server_request(request).await
            }
            InProcessServerEvent::ServerNotification(notification) => match notification {
                ServerNotification::AgentMessageDelta(delta) => {
                    self.update_delta(&delta.thread_id, &delta.turn_id, delta.delta)
                }
                ServerNotification::ItemStarted(started) => self.update_item_activity(
                    &started.thread_id,
                    &started.turn_id,
                    &started.item,
                    AgentActivityStatus::Running,
                ),
                ServerNotification::ItemCompleted(completed) => {
                    self.update_item_activity(
                        &completed.thread_id,
                        &completed.turn_id,
                        &completed.item,
                        AgentActivityStatus::Completed,
                    )?;
                    if let ThreadItem::AgentMessage { text, .. } = completed.item {
                        self.replace_completed_text(
                            &completed.thread_id,
                            &completed.turn_id,
                            text,
                        )?;
                    }
                    Ok(())
                }
                ServerNotification::ThreadTokenUsageUpdated(updated) => self.update_token_usage(
                    &updated.thread_id,
                    &updated.turn_id,
                    updated.token_usage,
                ),
                ServerNotification::McpToolCallProgress(progress) => self.update_tool_progress(
                    &progress.thread_id,
                    &progress.turn_id,
                    progress.message,
                ),
                ServerNotification::McpServerStatusUpdated(update) => {
                    if std::env::var("MAHAYANA_DEBUG_PLUGIN_INHERITANCE").as_deref() == Ok("1") {
                        eprintln!(
                            "mahayana MCP startup {}: {:?}{}",
                            update.name,
                            update.status,
                            update
                                .error
                                .as_deref()
                                .map(|error| format!(" ({error})"))
                                .unwrap_or_default()
                        );
                    }
                    Ok(())
                }
                ServerNotification::TurnCompleted(completed) => self.complete_turn(
                    &completed.thread_id,
                    &completed.turn.id,
                    completed.turn.status,
                    completed.turn.error,
                ),
                ServerNotification::Error(error) if !error.will_retry => self.fail_turn(
                    &error.thread_id,
                    &error.turn_id,
                    agent_error_from_turn(error.error),
                ),
                _ => Ok(()),
            },
        }
    }
}

fn thread_item_activity(
    item: &ThreadItem,
    requested_status: AgentActivityStatus,
) -> Option<AgentActivity> {
    let (kind, title, detail) = match item {
        ThreadItem::UserMessage { .. }
        | ThreadItem::HookPrompt { .. }
        | ThreadItem::AgentMessage { .. } => return None,
        ThreadItem::Plan { text, .. } => ("plan", "更新执行计划".to_string(), Some(text.clone())),
        ThreadItem::Reasoning { summary, .. } => (
            "reasoning",
            "分析与推理".to_string(),
            (!summary.is_empty()).then(|| summary.join("\n")),
        ),
        ThreadItem::CommandExecution {
            command,
            cwd,
            aggregated_output,
            exit_code,
            ..
        } => (
            "shell",
            command.clone(),
            Some(format!(
                "工作目录：{cwd}\n{}{}",
                aggregated_output.as_deref().unwrap_or(""),
                exit_code
                    .map(|code| format!("\n退出码：{code}"))
                    .unwrap_or_default()
            )),
        ),
        ThreadItem::FileChange { changes, .. } => (
            "edit",
            format!("修改 {} 个文件", changes.len()),
            Some(format!("{changes:#?}")),
        ),
        ThreadItem::McpToolCall {
            server,
            tool,
            arguments,
            error,
            ..
        } => (
            "mcp",
            format!("{server} · {tool}"),
            Some(match error {
                Some(error) => format!("参数：{arguments}\n错误：{error:?}"),
                None => format!("参数：{arguments}"),
            }),
        ),
        ThreadItem::DynamicToolCall {
            namespace,
            tool,
            arguments,
            ..
        } => (
            "tool",
            namespace
                .as_deref()
                .map(|namespace| format!("{namespace} · {tool}"))
                .unwrap_or_else(|| tool.clone()),
            Some(format!("参数：{arguments}")),
        ),
        ThreadItem::CollabAgentToolCall {
            tool,
            receiver_thread_ids,
            prompt,
            ..
        } => (
            "subagent",
            format!("子 Agent · {tool:?}"),
            Some(format!(
                "目标：{}{}",
                receiver_thread_ids.join(", "),
                prompt
                    .as_deref()
                    .map(|prompt| format!("\n任务：{prompt}"))
                    .unwrap_or_default()
            )),
        ),
        ThreadItem::SubAgentActivity {
            kind, agent_path, ..
        } => (
            "subagent",
            format!("子 Agent · {agent_path}"),
            Some(format!("{kind:?}")),
        ),
        ThreadItem::WebSearch(item) => (
            "web",
            "网页搜索".to_string(),
            serde_json::to_string(item).ok(),
        ),
        ThreadItem::ImageView { path, .. } => {
            ("image", "查看图片".to_string(), Some(path.to_string()))
        }
        ThreadItem::Sleep { duration_ms, .. } => (
            "wait",
            "等待任务".to_string(),
            Some(format!("{} ms", duration_ms)),
        ),
        ThreadItem::ImageGeneration(item) => (
            "image",
            "生成图片".to_string(),
            serde_json::to_string(item).ok(),
        ),
        ThreadItem::EnteredReviewMode { review, .. } => {
            ("review", "进入代码审查".to_string(), Some(review.clone()))
        }
        ThreadItem::ExitedReviewMode { review, .. } => {
            ("review", "完成代码审查".to_string(), Some(review.clone()))
        }
        ThreadItem::ContextCompaction { .. } => ("context", "压缩会话上下文".to_string(), None),
    };
    let serialized = serde_json::to_value(item).unwrap_or(Value::Null);
    let failed = serialized.get("status").and_then(Value::as_str) == Some("failed")
        || serialized.get("success").and_then(Value::as_bool) == Some(false)
        || serialized
            .get("error")
            .is_some_and(|error| !error.is_null());
    Some(AgentActivity {
        step_id: item.id().to_string(),
        kind: kind.into(),
        title: truncate_activity(title),
        detail: detail.map(truncate_activity),
        status: if failed {
            AgentActivityStatus::Failed
        } else {
            requested_status
        },
        metadata: Some(serialized),
    })
}

fn truncate_activity(value: String) -> String {
    const LIMIT: usize = 4_000;
    if value.chars().count() <= LIMIT {
        return value;
    }
    let mut truncated = value.chars().take(LIMIT).collect::<String>();
    truncated.push_str("\n…内容已截断");
    truncated
}

fn model_token_usage(usage: TokenUsageBreakdown) -> ModelTokenUsage {
    ModelTokenUsage {
        total_tokens: usage.total_tokens,
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_output_tokens: usage.reasoning_output_tokens,
    }
}

fn agent_error_from_turn(error: TurnError) -> AgentError {
    if error.codex_error_info.as_ref() == Some(&CodexErrorInfo::UsageLimitExceeded) {
        AgentError::UsageLimitExceeded(error.message)
    } else {
        AgentError::Backend(error.message)
    }
}

async fn dispatch_events(mut client: InProcessAppServerClient, inner: Weak<CodexAgentInner>) {
    while let Some(event) = client.next_event().await {
        let Some(inner) = inner.upgrade() else {
            return;
        };
        if let Err(error) = inner.handle_event(event).await {
            inner.fail_all(&error.to_string());
        }
    }
    if let Some(inner) = inner.upgrade() {
        inner.fail_all("in-process Codex event stream closed");
    }
}

/// A long-lived, in-process Codex runtime. One background dispatcher owns the
/// app-server event receiver and routes concurrent turns to their own sinks.
/// Requests, interrupts, and approvals use the cloneable in-process handle;
/// no `codex` executable, socket, or cloud Agent is involved.
pub struct CodexAgentBackend {
    inner: Arc<CodexAgentInner>,
}

impl CodexAgentBackend {
    pub async fn start(settings: CodexAgentConfig) -> Result<Self, AgentError> {
        settings.validate()?;
        std::fs::create_dir_all(&settings.codex_home)
            .map_err(|error| AgentError::Backend(error.to_string()))?;

        let loader_overrides = LoaderOverrides {
            ignore_user_config: true,
            ignore_user_and_project_exec_policy_rules: true,
            ..LoaderOverrides::default()
        };
        let inherited_plugin_roots = if settings.inherit_installed_plugins {
            shared_installed_plugin_roots(&settings.codex_home)?
        } else {
            Vec::new()
        };
        let mut cli_overrides = Vec::new();
        if settings.inherit_installed_plugins {
            cli_overrides.extend(shared_installed_plugin_overrides(&settings.codex_home)?);
        }
        let mut config = ConfigBuilder::default()
            .codex_home(settings.codex_home)
            .cli_overrides(cli_overrides.clone())
            .loader_overrides(loader_overrides.clone())
            .harness_overrides(ConfigOverrides {
                model: (!settings.use_codex_account).then(|| settings.model.clone()),
                cwd: Some(settings.cwd),
                approval_policy: Some(settings.approval_policy),
                sandbox_mode: Some(settings.sandbox_mode),
                workspace_roots: Some(
                    settings
                        .workspace_roots
                        .iter()
                        .map(|root| {
                            codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(root)
                                .map_err(|error| AgentError::Backend(error.to_string()))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                ..ConfigOverrides::default()
            })
            .build()
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        debug_plugin_inheritance(
            "startup MCP servers",
            config.mcp_servers.get().keys().cloned(),
        );

        if !settings.use_codex_account {
            let provider = ModelProviderInfo {
                name: "大乘 DeepSeek Responses".into(),
                base_url: Some(settings.responses_base_url),
                env_key: None,
                env_key_instructions: None,
                experimental_bearer_token: settings.product_session_token,
                auth: None,
                aws: None,
                wire_api: WireApi::Responses,
                query_params: None,
                http_headers: None,
                env_http_headers: None,
                request_max_retries: Some(2),
                stream_max_retries: Some(2),
                stream_idle_timeout_ms: Some(300_000),
                websocket_connect_timeout_ms: None,
                requires_openai_auth: false,
                supports_websockets: false,
            };
            config.model = Some(settings.model);
            config.model_provider_id = PROVIDER_ID.into();
            config.model_provider = provider.clone();
            config
                .model_providers
                .insert(PROVIDER_ID.into(), provider.clone());

            // App-server derives a fresh Config for every thread. Keep the
            // first-party provider in its in-memory CLI override layer as well as
            // the startup Config so thread/start and resume see the exact same
            // provider without writing credentials to config.toml.
            let provider_override = toml::Value::try_from(&provider)
                .map_err(|error| AgentError::Backend(error.to_string()))?;
            cli_overrides.push((format!("model_providers.{PROVIDER_ID}"), provider_override));
        }

        let config = Arc::new(config);
        let state_db = codex_core::init_state_db(config.as_ref()).await;
        let (arg0_paths, environment_manager) =
            if let Some(codex_executable_path) = settings.codex_executable_path {
                let arg0_paths = Arg0DispatchPaths {
                    codex_self_exe: Some(codex_executable_path),
                    ..Arg0DispatchPaths::default()
                };
                let local_runtime_paths = ExecServerRuntimePaths::from_optional_paths(
                    arg0_paths.codex_self_exe.clone(),
                    arg0_paths.codex_linux_sandbox_exe.clone(),
                )
                .map_err(|error| AgentError::Backend(error.to_string()))?;
                let environment_manager = EnvironmentManager::from_env(Some(local_runtime_paths))
                    .await
                    .map_err(|error| AgentError::Backend(error.to_string()))?;
                (arg0_paths, environment_manager)
            } else {
                (
                    Arg0DispatchPaths::default(),
                    EnvironmentManager::without_environments(),
                )
            };
        let config_warnings = config
            .startup_warnings
            .iter()
            .map(
                |summary| codex_app_server_protocol::ConfigWarningNotification {
                    summary: summary.clone(),
                    details: None,
                    path: None,
                    range: None,
                },
            )
            .collect();
        let client = InProcessAppServerClient::start(InProcessClientStartArgs {
            arg0_paths,
            config: Arc::clone(&config),
            cli_overrides,
            loader_overrides,
            strict_config: false,
            cloud_config_bundle: CloudConfigBundleLoader::default(),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db,
            environment_manager: Arc::new(environment_manager),
            config_warnings,
            session_source: SessionSource::Exec,
            enable_codex_api_key_env: settings.use_codex_account,
            client_name: "mahayana-runtime".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            experimental_api: true,
            mcp_server_openai_form_elicitation: false,
            opt_out_notification_methods: Vec::new(),
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await
        .map_err(|error| AgentError::Backend(error.to_string()))?;
        let inner = Arc::new(CodexAgentInner {
            config,
            requests: client.request_handle(),
            shared_installed_plugin_roots: inherited_plugin_roots,
            next_request_id: AtomicI64::new(1),
            operations: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            computer_session_approved: AtomicBool::new(false),
            conversation_providers: settings.conversation_providers,
        });
        tokio::spawn(dispatch_events(client, Arc::downgrade(&inner)));
        Ok(Self { inner })
    }
}

fn mcp_custom_instructions_path(codex_home: &Path) -> PathBuf {
    codex_home.join("fabushi-mcp-custom-instructions.json")
}

fn load_mcp_custom_instructions(codex_home: &Path) -> Result<HashMap<String, String>, AgentError> {
    let path = mcp_custom_instructions_path(codex_home);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => {
            return Err(AgentError::Backend(format!(
                "failed to read MCP custom instructions: {error}"
            )));
        }
    };
    let mut instructions =
        serde_json::from_str::<HashMap<String, String>>(&contents).map_err(|error| {
            AgentError::Backend(format!("invalid MCP custom instructions: {error}"))
        })?;
    instructions
        .retain(|server, instruction| !server.trim().is_empty() && !instruction.trim().is_empty());
    Ok(instructions)
}

fn save_mcp_custom_instructions(
    codex_home: &Path,
    instructions: &HashMap<String, String>,
) -> Result<(), AgentError> {
    std::fs::create_dir_all(codex_home).map_err(|error| {
        AgentError::Backend(format!("failed to create MCP settings directory: {error}"))
    })?;
    let path = mcp_custom_instructions_path(codex_home);
    let temporary = path.with_extension("json.tmp");
    let serialized = serde_json::to_vec_pretty(instructions).map_err(|error| {
        AgentError::Backend(format!("serialize MCP custom instructions: {error}"))
    })?;
    std::fs::write(&temporary, serialized)
        .map_err(|error| AgentError::Backend(format!("write MCP custom instructions: {error}")))?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| AgentError::Backend(format!("commit MCP custom instructions: {error}")))?;
    Ok(())
}

#[async_trait]
impl AgentBackend for CodexAgentBackend {
    async fn start_thread(
        &self,
        _request: StartThreadRequest,
    ) -> Result<AgentThreadId, AgentError> {
        let response: ThreadStartResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::ThreadStart {
                request_id: self.inner.request_id(),
                params: ThreadStartParams {
                    model: self.inner.config.model.clone(),
                    model_provider: Some(self.inner.config.model_provider_id.clone()),
                    cwd: Some(self.inner.config.cwd.to_string_lossy().into_owned()),
                    runtime_workspace_roots: Some(self.inner.config.workspace_roots.clone()),
                    approval_policy: Some(
                        self.inner.config.permissions.approval_policy.value().into(),
                    ),
                    ephemeral: Some(false),
                    dynamic_tools: Some(mahayana_dynamic_tools()),
                    ..ThreadStartParams::default()
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        AgentThreadId::new(response.thread.id)
            .map_err(|error| AgentError::Backend(error.to_string()))
    }

    async fn send_message(
        &self,
        request: AgentMessageRequest,
        events: SharedAgentEventSink,
    ) -> Result<(), AgentError> {
        let thread_id = request.thread_id.to_string();
        let response: TurnStartResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::TurnStart {
                request_id: self.inner.request_id(),
                params: TurnStartParams {
                    thread_id: thread_id.clone(),
                    client_user_message_id: request.client_message_id,
                    input: vec![UserInput::Text {
                        text: request.text,
                        text_elements: Vec::new(),
                    }],
                    cwd: Some(self.inner.config.cwd.to_path_buf()),
                    runtime_workspace_roots: Some(self.inner.config.workspace_roots.clone()),
                    ..TurnStartParams::default()
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        let turn_id = response.turn.id;
        let (completion, result) = oneshot::channel();
        self.inner
            .operations
            .lock()
            .map_err(|_| AgentError::Backend("operation mutex poisoned".into()))?
            .insert(
                request.operation_id,
                ActiveOperation {
                    thread_id,
                    turn_id,
                    conversation_id: request.conversation_id,
                    events,
                    assistant_text: String::new(),
                    completion,
                },
            );
        result
            .await
            .map_err(|_| AgentError::Backend("Codex turn dispatcher stopped".into()))?
    }

    async fn interrupt(&self, operation_id: &OperationId) -> Result<(), AgentError> {
        let operation = self
            .inner
            .operations
            .lock()
            .map_err(|_| AgentError::Backend("operation mutex poisoned".into()))?
            .get(operation_id)
            .map(|operation| (operation.thread_id.clone(), operation.turn_id.clone()))
            .ok_or_else(|| AgentError::OperationNotFound(operation_id.clone()))?;
        let _: TurnInterruptResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::TurnInterrupt {
                request_id: self.inner.request_id(),
                params: TurnInterruptParams {
                    thread_id: operation.0,
                    turn_id: operation.1,
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        Ok(())
    }

    async fn resolve_approval(&self, resolution: ApprovalResolution) -> Result<(), AgentError> {
        let pending = self
            .inner
            .approvals
            .lock()
            .map_err(|_| AgentError::Backend("approval mutex poisoned".into()))?
            .remove(&resolution.approval_id)
            .ok_or_else(|| AgentError::ApprovalNotFound(resolution.approval_id.clone()))?;
        match pending.response_kind {
            ApprovalResponseKind::ComputerDynamic { params } => {
                let response = match resolution.decision {
                    ApprovalDecision::Accept | ApprovalDecision::AcceptForSession => {
                        if resolution.decision == ApprovalDecision::AcceptForSession {
                            self.inner
                                .computer_session_approved
                                .store(true, Ordering::Relaxed);
                        }
                        self.inner.execute_computer_dynamic_tool(&params).await
                    }
                    ApprovalDecision::Decline | ApprovalDecision::Cancel => {
                        dynamic_tool_error("User declined AI computer control")
                    }
                };
                self.inner
                    .resolve_dynamic_tool_response(pending.request_id, response)
                    .await
            }
            response_kind => {
                let response = approval_response(response_kind, resolution.decision);
                self.inner
                    .requests
                    .resolve_server_request(pending.request_id, response)
                    .await
                    .map_err(|error| AgentError::Backend(error.to_string()))
            }
        }
    }

    async fn list_mcp_servers(&self) -> Result<Vec<Value>, AgentError> {
        let response: ListMcpServerStatusResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::McpServerStatusList {
                request_id: self.inner.request_id(),
                params: ListMcpServerStatusParams {
                    cursor: None,
                    limit: None,
                    detail: Some(McpServerStatusDetail::ToolsAndAuthOnly),
                    thread_id: None,
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        response
            .data
            .into_iter()
            .map(|status| {
                serde_json::to_value(status).map_err(|error| AgentError::Backend(error.to_string()))
            })
            .collect()
    }

    async fn list_connector_apps(&self) -> Result<Vec<Value>, AgentError> {
        let response: AppsListResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::AppsList {
                request_id: self.inner.request_id(),
                params: AppsListParams {
                    cursor: None,
                    limit: Some(200),
                    thread_id: None,
                    force_refetch: false,
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        response
            .data
            .into_iter()
            .map(|app| {
                serde_json::to_value(app).map_err(|error| AgentError::Backend(error.to_string()))
            })
            .collect()
    }

    async fn mcp_oauth_login(&self, server: &str) -> Result<String, AgentError> {
        let server = server.trim();
        if server.is_empty() {
            return Err(AgentError::Backend(
                "MCP server name must not be empty".into(),
            ));
        }
        let response: McpServerOauthLoginResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::McpServerOauthLogin {
                request_id: self.inner.request_id(),
                params: McpServerOauthLoginParams {
                    name: server.to_string(),
                    thread_id: None,
                    scopes: None,
                    timeout_secs: Some(15 * 60),
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        Ok(response.authorization_url)
    }

    async fn mcp_oauth_logout(&self, server: &str) -> Result<bool, AgentError> {
        let server = server.trim();
        if server.is_empty() {
            return Err(AgentError::Backend(
                "MCP server name must not be empty".into(),
            ));
        }
        let manager = McpManager::new(Arc::new(PluginsManager::new(
            self.inner.config.codex_home.to_path_buf(),
        )));
        let servers = manager.configured_servers(&self.inner.config).await;
        let config = servers
            .get(server)
            .ok_or_else(|| AgentError::Backend(format!("No MCP server named '{server}' found.")))?;
        let url = match &config.transport {
            McpServerTransportConfig::StreamableHttp { url, .. } => url,
            _ => {
                return Err(AgentError::Backend(
                    "OAuth logout is only supported for streamable HTTP MCP servers.".into(),
                ));
            }
        };
        delete_oauth_tokens(
            server,
            url,
            self.inner.config.mcp_oauth_credentials_store_mode,
            self.inner.config.auth_keyring_backend_kind(),
        )
        .map_err(|error| {
            AgentError::Backend(format!("failed to delete OAuth credentials: {error}"))
        })
    }

    async fn mcp_custom_instructions(&self) -> Result<HashMap<String, String>, AgentError> {
        load_mcp_custom_instructions(&self.inner.config.codex_home)
    }

    async fn set_mcp_custom_instructions(
        &self,
        server: &str,
        instructions: &str,
    ) -> Result<(), AgentError> {
        let server = server.trim();
        if server.is_empty() || server.len() > 200 {
            return Err(AgentError::Backend("invalid MCP server name".into()));
        }
        let instructions = instructions.trim();
        if instructions.len() > 20_000 {
            return Err(AgentError::Backend(
                "MCP custom instructions exceed 20000 characters".into(),
            ));
        }
        let mut settings = load_mcp_custom_instructions(&self.inner.config.codex_home)?;
        if instructions.is_empty() {
            settings.remove(server);
        } else {
            settings.insert(server.to_string(), instructions.to_string());
        }
        save_mcp_custom_instructions(&self.inner.config.codex_home, &settings)
    }

    async fn set_mcp_tool_disabled(
        &self,
        server: &str,
        tool: &str,
        disabled: bool,
    ) -> Result<Vec<String>, AgentError> {
        let server = server.trim();
        let tool = tool.trim();
        if server.is_empty() || server.len() > 200 || tool.is_empty() || tool.len() > 240 {
            return Err(AgentError::Backend(
                "invalid MCP server or tool name".into(),
            ));
        }
        let codex_home = self.inner.config.codex_home.to_path_buf();
        let mut servers = load_global_mcp_servers(&codex_home)
            .await
            .map_err(|error| AgentError::Backend(format!("failed to load MCP servers: {error}")))?;
        let config = servers.get_mut(server).ok_or_else(|| {
            AgentError::Backend(format!(
                "MCP server is not a user-managed global configuration: {server}"
            ))
        })?;
        let mut disabled_tools = config.disabled_tools.clone().unwrap_or_default();
        if disabled {
            if !disabled_tools.iter().any(|existing| existing == tool) {
                disabled_tools.push(tool.to_string());
            }
        } else {
            disabled_tools.retain(|existing| existing != tool);
        }
        disabled_tools.sort();
        disabled_tools.dedup();
        config.disabled_tools = (!disabled_tools.is_empty()).then(|| disabled_tools.clone());
        ConfigEditsBuilder::new(&codex_home)
            .replace_mcp_servers(&servers)
            .apply()
            .await
            .map_err(|error| {
                AgentError::Backend(format!("failed to persist MCP tool filter: {error}"))
            })?;
        self.refresh_mcp_servers().await?;
        Ok(disabled_tools)
    }

    async fn remove_mcp_server(&self, server: &str) -> Result<bool, AgentError> {
        let server = server.trim();
        if server.is_empty() {
            return Err(AgentError::Backend(
                "MCP server name must not be empty".into(),
            ));
        }
        let codex_home = self.inner.config.codex_home.to_path_buf();
        let mut servers = load_global_mcp_servers(&codex_home)
            .await
            .map_err(|error| AgentError::Backend(format!("failed to load MCP servers: {error}")))?;
        let Some(removed_config) = servers.remove(server) else {
            return Ok(false);
        };
        ConfigEditsBuilder::new(&codex_home)
            .replace_mcp_servers(&servers)
            .apply()
            .await
            .map_err(|error| {
                AgentError::Backend(format!("failed to persist MCP removal: {error}"))
            })?;
        if let McpServerTransportConfig::StreamableHttp { url, .. } = &removed_config.transport {
            let _ = delete_oauth_tokens(
                server,
                url,
                self.inner.config.mcp_oauth_credentials_store_mode,
                self.inner.config.auth_keyring_backend_kind(),
            );
        }
        self.refresh_mcp_servers().await?;
        Ok(true)
    }

    async fn refresh_mcp_servers(&self) -> Result<(), AgentError> {
        let _: McpServerRefreshResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::McpServerRefresh {
                request_id: self.inner.request_id(),
                params: None,
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        Ok(())
    }

    async fn call_mcp_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, AgentError> {
        let server = server.trim();
        let tool = tool.trim();
        if server.is_empty() || tool.is_empty() {
            return Err(AgentError::Backend(
                "MCP server and tool are required".into(),
            ));
        }

        // First-party connector actions must not depend on the connector also
        // being represented as a marketplace plugin. In particular, ChatGPT
        // Apps are exposed by Codex through the built-in `codex_apps` MCP
        // server and therefore have no PluginRead record. Start an ephemeral,
        // read-only thread and invoke exactly the requested MCP tool on it.
        // No model turn is started, shell/web/dynamic tools are disabled, and
        // configured standalone MCP servers are narrowed to the requested
        // server whenever Codex has an explicit config for it.
        let mut config = HashMap::new();
        if let Some(server_config) = self.inner.config.mcp_servers.get().get(server) {
            let mut server_config = server_config.clone();
            resolve_relative_mcp_command(&mut server_config);
            let mut value = serde_json::to_value(server_config)
                .map_err(|error| AgentError::Backend(error.to_string()))?;
            remove_null_values(&mut value);
            config.insert("mcp_servers".into(), json!({ server: value }));
        }
        config.insert("features".into(), json!({ "shell_tool": false }));
        config.insert("web_search".into(), Value::String("disabled".into()));

        let response: ThreadStartResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::ThreadStart {
                request_id: self.inner.request_id(),
                params: ThreadStartParams {
                    model: self.inner.config.model.clone(),
                    model_provider: Some(self.inner.config.model_provider_id.clone()),
                    cwd: Some(self.inner.config.cwd.to_string_lossy().into_owned()),
                    runtime_workspace_roots: Some(Vec::new()),
                    approval_policy: Some(
                        self.inner.config.permissions.approval_policy.value().into(),
                    ),
                    sandbox: Some(SandboxMode::ReadOnly.into()),
                    config: Some(config),
                    base_instructions: Some(
                        "Connector action host. Do not run a model turn; only the host may call the selected MCP tool."
                            .into(),
                    ),
                    developer_instructions: Some(
                        "No shell, filesystem mutation, web search, or unrelated connector actions are permitted."
                            .into(),
                    ),
                    ephemeral: Some(true),
                    environments: Some(Vec::new()),
                    dynamic_tools: Some(Vec::new()),
                    ..ThreadStartParams::default()
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        let thread_id = AgentThreadId::new(response.thread.id)
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        let tools = self.list_mcp_app_tools(&thread_id, server).await?;
        if !tools
            .iter()
            .any(|descriptor| descriptor.get("name").and_then(Value::as_str) == Some(tool))
        {
            return Err(AgentError::Backend(format!(
                "MCP tool not available: {server}/{tool}"
            )));
        }
        self.call_mcp_app_tool(&thread_id, server, tool, arguments)
            .await
    }

    async fn open_mcp_app(&self, request: OpenMcpAppRequest) -> Result<McpAppSession, AgentError> {
        let installed: PluginInstalledResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::PluginInstalled {
                request_id: self.inner.request_id(),
                params: PluginInstalledParams {
                    cwds: Some(vec![self.inner.config.cwd.clone()]),
                    install_suggestion_plugin_names: None,
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        let mut installed_local_plugin_paths = self.inner.shared_installed_plugin_roots.clone();
        installed_local_plugin_paths.extend(
            installed
                .marketplaces
                .iter()
                .flat_map(|marketplace| marketplace.plugins.iter())
                .filter(|plugin| plugin.installed && plugin.enabled)
                .filter_map(|plugin| match &plugin.source {
                    PluginSource::Local { path } => Some(path.as_path().to_path_buf()),
                    PluginSource::Git { .. } | PluginSource::Npm { .. } | PluginSource::Remote => {
                        None
                    }
                }),
        );
        installed_local_plugin_paths.sort();
        installed_local_plugin_paths.dedup();
        let (marketplace_path, remote_marketplace_name) = installed
            .marketplaces
            .iter()
            .find_map(|marketplace| {
                marketplace
                    .plugins
                    .iter()
                    .any(|plugin| {
                        plugin.installed
                            && plugin_identity_matches(&plugin.id, &plugin.name, &request.plugin_id)
                    })
                    .then(|| {
                        let remote_name =
                            marketplace.path.is_none().then(|| marketplace.name.clone());
                        (marketplace.path.clone(), remote_name)
                    })
            })
            .ok_or_else(|| {
                AgentError::Backend(format!(
                    "plugin `{}` is not installed in a Codex marketplace",
                    request.plugin_id
                ))
            })?;
        let detail: PluginReadResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::PluginRead {
                request_id: self.inner.request_id(),
                params: PluginReadParams {
                    marketplace_path,
                    remote_marketplace_name,
                    plugin_name: request.plugin_id.clone(),
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        if !detail.plugin.summary.enabled {
            return Err(AgentError::Backend(format!(
                "plugin `{}` is disabled",
                request.plugin_id
            )));
        }
        let mut mcp_configs = self.inner.config.mcp_servers.get().clone();
        if let PluginSource::Local { path } = &detail.plugin.summary.source {
            mcp_configs.extend(load_plugin_mcp_servers(path.as_path(), None).await);
        }
        debug_plugin_inheritance("workspace thread MCP servers", mcp_configs.keys().cloned());
        let runtime_variants = detail
            .plugin
            .runtime_variants
            .iter()
            .map(|variant| MahayanaPluginRuntimeVariant {
                id: variant.id.clone(),
                server: variant.server.clone(),
                platforms: variant
                    .platforms
                    .iter()
                    .map(|platform| match platform {
                        CodexPluginRuntimePlatform::Cli => HostPlatform::Cli,
                        CodexPluginRuntimePlatform::Desktop => HostPlatform::Desktop,
                        CodexPluginRuntimePlatform::Mobile => HostPlatform::Mobile,
                        CodexPluginRuntimePlatform::Web => HostPlatform::Web,
                    })
                    .collect(),
                priority: i64::from(variant.priority),
            })
            .collect::<Vec<_>>();
        let selected = select_runtime_with_availability(
            request.platform,
            &detail.plugin.mcp_servers,
            &runtime_variants,
            |server| mcp_configs.get(server).is_some_and(mcp_runtime_available),
        )
        .map_err(|error| AgentError::Backend(error.to_string()))?;
        let orchestration = mcp_app_orchestration_profile(&request.plugin_id, &selected.server);
        if orchestration.workspace_builder {
            for path in installed_local_plugin_paths {
                for (name, config) in load_plugin_mcp_servers(path.as_path(), None).await {
                    let replace = mcp_configs
                        .get(&name)
                        .is_none_or(|existing| !mcp_runtime_available(existing));
                    if replace {
                        mcp_configs.insert(name, config);
                    }
                }
            }
        }
        debug_plugin_inheritance(
            "workspace thread MCP servers after standalone plugin loading",
            mcp_configs.keys().cloned(),
        );
        debug_mcp_runtime_configs(&mcp_configs);
        let (mut command_tools, tool_gates) = match &detail.plugin.summary.source {
            PluginSource::Local { path } => LocalPlugin::load(path.as_path())
                .map_err(|error| AgentError::Backend(error.to_string()))?
                .mahayana
                .map(|manifest| {
                    let mut commands = HashMap::new();
                    for command in manifest.commands {
                        commands.insert(command.name, command.tool.clone());
                        for alias in command.aliases {
                            commands.insert(alias, command.tool.clone());
                        }
                    }
                    let gates = manifest
                        .gates
                        .into_iter()
                        .filter_map(|gate| {
                            gate.target
                                .strip_prefix("tool:")
                                .map(|tool| (tool.to_string(), gate.entitlement))
                        })
                        .collect();
                    (commands, gates)
                })
                .unwrap_or_default(),
            PluginSource::Git { .. } | PluginSource::Npm { .. } | PluginSource::Remote => {
                (HashMap::new(), HashMap::new())
            }
        };
        let server_config = mcp_configs.get(&selected.server).ok_or_else(|| {
            AgentError::Backend(format!(
                "plugin `{}` selected MCP server `{}`, but Codex did not load its configuration",
                request.plugin_id, selected.server
            ))
        })?;
        let thread_mcp_configs = if orchestration.workspace_builder {
            mcp_configs
                .iter()
                .map(|(name, server)| {
                    let mut server = server.clone();
                    resolve_relative_mcp_command(&mut server);
                    if matches!(server.transport, McpServerTransportConfig::Stdio { .. })
                        && mcp_runtime_available(&server)
                    {
                        server.required = true;
                    }
                    let mut value = serde_json::to_value(server)
                        .map_err(|error| AgentError::Backend(error.to_string()))?;
                    remove_null_values(&mut value);
                    Ok((name.clone(), value))
                })
                .collect::<Result<HashMap<_, _>, AgentError>>()?
        } else {
            let mut value = serde_json::to_value(server_config)
                .map_err(|error| AgentError::Backend(error.to_string()))?;
            remove_null_values(&mut value);
            HashMap::from([(selected.server.clone(), value)])
        };
        debug_plugin_inheritance(
            "required workspace MCP servers",
            thread_mcp_configs
                .iter()
                .filter(|(_, server)| server.get("required").and_then(Value::as_bool) == Some(true))
                .map(|(name, _)| name.clone()),
        );
        let mut config = HashMap::new();
        config.insert(
            "mcp_servers".into(),
            serde_json::to_value(thread_mcp_configs)
                .map_err(|error| AgentError::Backend(error.to_string()))?,
        );
        config.insert(
            "features".into(),
            json!({ "shell_tool": orchestration.workspace_builder }),
        );
        config.insert("web_search".into(), Value::String("disabled".into()));
        let response: ThreadStartResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::ThreadStart {
                request_id: self.inner.request_id(),
                params: ThreadStartParams {
                    model: self.inner.config.model.clone(),
                    model_provider: Some(self.inner.config.model_provider_id.clone()),
                    cwd: Some(self.inner.config.cwd.to_string_lossy().into_owned()),
                    runtime_workspace_roots: Some(if orchestration.workspace_builder {
                        self.inner.config.workspace_roots.clone()
                    } else {
                        Vec::new()
                    }),
                    approval_policy: Some(
                        self.inner.config.permissions.approval_policy.value().into(),
                    ),
                    sandbox: Some(if orchestration.workspace_builder {
                        SandboxMode::WorkspaceWrite.into()
                    } else {
                        SandboxMode::ReadOnly.into()
                    }),
                    config: Some(config),
                    base_instructions: Some(orchestration.base_instructions),
                    developer_instructions: Some(orchestration.developer_instructions),
                    ephemeral: Some(false),
                    environments: Some(Vec::new()),
                    dynamic_tools: Some(if orchestration.workspace_builder {
                        mahayana_dynamic_tools()
                    } else {
                        Vec::new()
                    }),
                    ..ThreadStartParams::default()
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        let thread_id = AgentThreadId::new(response.thread.id)
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        let tools = self
            .list_mcp_app_tools(&thread_id, &selected.server)
            .await?;
        // A plugin may expose a richer local desktop command set than its
        // account HTTP runtime. Negotiate the commands available on this
        // concrete server instead of rejecting the entire MiniApp session
        // when one platform-specific tool is absent.
        command_tools.retain(|_, tool| {
            tools
                .iter()
                .any(|descriptor| descriptor.get("name").and_then(Value::as_str) == Some(tool))
        });
        let has_home = tools
            .iter()
            .any(|tool| tool.get("name").and_then(Value::as_str) == Some("home"));
        let home_result = if has_home {
            self.call_mcp_app_tool(&thread_id, &selected.server, "home", json!({}))
                .await?
        } else {
            Value::Null
        };
        let resource_uri = home_result
            .pointer("/_meta/ui~1resourceUri")
            .or_else(|| home_result.pointer("/meta/ui~1resourceUri"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let ui_resources = if let Some(uri) = resource_uri {
            let resources: McpResourceReadResponse = self
                .inner
                .requests
                .request_typed(ClientRequest::McpResourceRead {
                    request_id: self.inner.request_id(),
                    params: McpResourceReadParams {
                        thread_id: Some(thread_id.to_string()),
                        server: selected.server.clone(),
                        uri,
                    },
                })
                .await
                .map_err(|error| AgentError::Backend(error.to_string()))?;
            resources
                .contents
                .into_iter()
                .map(|content| {
                    serde_json::to_value(content)
                        .map_err(|error| AgentError::Backend(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        Ok(McpAppSession {
            thread_id,
            plugin_id: request.plugin_id,
            server: selected.server,
            command_tools,
            tool_gates,
            tools,
            home_result,
            ui_resources,
        })
    }

    async fn list_mcp_app_tools(
        &self,
        thread_id: &AgentThreadId,
        server: &str,
    ) -> Result<Vec<Value>, AgentError> {
        let response: ListMcpServerStatusResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::McpServerStatusList {
                request_id: self.inner.request_id(),
                params: ListMcpServerStatusParams {
                    cursor: None,
                    limit: None,
                    detail: Some(McpServerStatusDetail::Full),
                    thread_id: Some(thread_id.to_string()),
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        debug_plugin_inheritance(
            "initialized MCP tools",
            response.data.iter().map(|status| {
                let mut tools = status.tools.keys().cloned().collect::<Vec<_>>();
                tools.sort();
                format!("{}[{}]", status.name, tools.join("|"))
            }),
        );
        let status = response
            .data
            .into_iter()
            .find(|status| status.name == server)
            .ok_or_else(|| {
                AgentError::Backend(format!("MCP App server `{server}` was not initialized"))
            })?;
        let mut tools = status
            .tools
            .into_values()
            .map(|tool| {
                serde_json::to_value(tool).map_err(|error| AgentError::Backend(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        tools.sort_by(|left, right| {
            left.get("name")
                .and_then(Value::as_str)
                .cmp(&right.get("name").and_then(Value::as_str))
        });
        Ok(tools)
    }

    async fn read_mcp_app_resource(
        &self,
        thread_id: &AgentThreadId,
        server: &str,
        uri: &str,
    ) -> Result<Vec<Value>, AgentError> {
        let resources: McpResourceReadResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::McpResourceRead {
                request_id: self.inner.request_id(),
                params: McpResourceReadParams {
                    thread_id: Some(thread_id.to_string()),
                    server: server.to_string(),
                    uri: uri.to_string(),
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        resources
            .contents
            .into_iter()
            .map(|content| {
                serde_json::to_value(content)
                    .map_err(|error| AgentError::Backend(error.to_string()))
            })
            .collect()
    }

    async fn call_mcp_app_tool(
        &self,
        thread_id: &AgentThreadId,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, AgentError> {
        let response: McpServerToolCallResponse = self
            .inner
            .requests
            .request_typed(ClientRequest::McpServerToolCall {
                request_id: self.inner.request_id(),
                params: McpServerToolCallParams {
                    thread_id: thread_id.to_string(),
                    server: server.into(),
                    tool: tool.into(),
                    arguments: Some(arguments),
                    meta: None,
                },
            })
            .await
            .map_err(|error| AgentError::Backend(error.to_string()))?;
        serde_json::to_value(response).map_err(|error| AgentError::Backend(error.to_string()))
    }

    fn name(&self) -> &'static str {
        "codex-app-server-in-process"
    }
}

fn plugin_identity_matches(installed_id: &str, installed_name: &str, requested: &str) -> bool {
    installed_name == requested || installed_id == requested
}

fn merge_completed_agent_text(current: &mut String, completed: String) {
    if !completed.is_empty() || current.is_empty() {
        *current = completed;
    }
}

fn remove_null_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, value| !value.is_null());
            for value in map.values_mut() {
                remove_null_values(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                remove_null_values(value);
            }
        }
        _ => {}
    }
}

fn mcp_runtime_available(config: &McpServerConfig) -> bool {
    if !config.enabled || config.disabled_reason.is_some() {
        return false;
    }
    match &config.transport {
        McpServerTransportConfig::StreamableHttp { .. } => true,
        McpServerTransportConfig::Stdio { command, cwd, .. } => {
            command_available(command, cwd.as_ref().map(|cwd| cwd.as_str()))
        }
    }
}

fn resolve_relative_mcp_command(config: &mut McpServerConfig) {
    let McpServerTransportConfig::Stdio { command, cwd, .. } = &mut config.transport else {
        return;
    };
    let command_path = Path::new(command);
    if command_path.is_absolute() || command_path.components().count() <= 1 {
        return;
    }
    let Some(cwd) = cwd.as_ref().map(|cwd| Path::new(cwd.as_str())) else {
        return;
    };
    if !cwd.is_absolute() {
        return;
    }
    let relative_command = command_path.strip_prefix(".").unwrap_or(command_path);
    let resolved = cwd.join(relative_command);
    if executable_file(&resolved) {
        *command = resolved.to_string_lossy().into_owned();
    }
}

fn command_available(command: &str, cwd: Option<&str>) -> bool {
    let path = Path::new(command);
    if path.is_absolute() || path.components().count() > 1 {
        if executable_file(path) {
            return true;
        }
        return path.is_relative()
            && cwd
                .map(Path::new)
                .filter(|cwd| cwd.is_absolute())
                .is_some_and(|cwd| executable_file(&cwd.join(path)));
    }
    std::env::var_os("PATH").is_some_and(|search_path| {
        std::env::split_paths(&search_path).any(|directory| {
            let candidate = directory.join(command);
            if executable_file(&candidate) {
                return true;
            }
            #[cfg(windows)]
            {
                std::env::var_os("PATHEXT").is_some_and(|extensions| {
                    extensions.to_string_lossy().split(';').any(|extension| {
                        executable_file(
                            &candidate.with_extension(extension.trim_start_matches('.')),
                        )
                    })
                })
            }
            #[cfg(not(windows))]
            false
        })
    })
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

fn computer_action_core_schema(include_screenshot: bool) -> Value {
    let actions = if include_screenshot {
        json!([
            "screenshot",
            "click",
            "move",
            "drag",
            "type",
            "key",
            "scroll",
            "wait"
        ])
    } else {
        json!(["click", "move", "drag", "type", "key", "scroll", "wait"])
    };
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": actions,
                "description": "What to do on this user's computer. Every call returns one fresh screenshot after all actions finish."
            },
            "x": {"type": "integer", "description": "X pixel in display space, origin top-left. Omit for click/move/scroll to use the current cursor."},
            "y": {"type": "integer", "description": "Y pixel in display space, origin top-left. Omit for click/move/scroll to use the current cursor."},
            "x2": {"type": "integer", "description": "Drag end X when path is omitted."},
            "y2": {"type": "integer", "description": "Drag end Y when path is omitted."},
            "path": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"x": {"type": "integer"}, "y": {"type": "integer"}},
                    "required": ["x", "y"],
                    "additionalProperties": false
                },
                "description": "Ordered drag path. At least two points are required when used."
            },
            "text": {"type": "string", "description": "Unicode text for type."},
            "key": {"type": "string", "description": "Key or chord such as Return, ctrl+a, command+l, Alt+Left."},
            "button": {"type": "string", "enum": ["left", "right", "middle"], "description": "Mouse button for click or drag, default left."},
            "count": {"type": "integer", "minimum": 1, "maximum": 3, "description": "Click count: 1, 2, or 3."},
            "direction": {"type": "string", "enum": ["up", "down", "left", "right"], "description": "Scroll direction; runtime default down."},
            "amount": {"type": "integer", "description": "Scroll amount in clicks, default 3."},
            "durationMs": {"type": "integer", "minimum": 0, "maximum": 30000, "description": "Wait duration in milliseconds; runtime default 1000 and max 30000."},
            "description": {"type": "string", "description": "Concise intended UI target and purpose, especially for click and drag."}
        },
        "required": ["action"],
        "additionalProperties": false
    })
}

fn computer_tool_schema() -> Value {
    let mut schema = computer_action_core_schema(true);
    let follow_up = computer_action_core_schema(false);
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert(
            "then".into(),
            json!({
                "type": "array",
                "minItems": 1,
                "maxItems": 9,
                "items": follow_up,
                "description": "Up to 9 known follow-up actions executed in order in this same call. Use separate calls when a later step depends on seeing the previous rendered screen."
            }),
        );
    }
    schema
}

fn mahayana_dynamic_tools() -> Vec<DynamicToolSpec> {
    vec![DynamicToolSpec::Namespace(DynamicToolNamespaceSpec {
        name: "mahayana".into(),
        description: "操作用户这台电脑、App 内置大乘 Runtime 的本地工作区与联系人会话。".into(),
        tools: vec![
            DynamicToolNamespaceTool::Function(DynamicToolFunctionSpec {
                name: "computer".into(),
                description: "控制安装 Fabushi 的用户电脑。支持 screenshot、click、move、drag、type、key、scroll、wait；坐标原点在屏幕左上角。每次调用在全部动作结束后返回最终屏幕截图。只有在用户启用 AI 电脑控制并通过本机审批策略后才能执行。".into(),
                input_schema: computer_tool_schema(),
                defer_loading: false,
            }),
            DynamicToolNamespaceTool::Function(DynamicToolFunctionSpec {
                name: "list_conversations".into(),
                description: "列出当前大乘 Runtime 中可供 Codex 接入的联系人会话。".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                defer_loading: false,
            }),
            DynamicToolNamespaceTool::Function(DynamicToolFunctionSpec {
                name: "conversation_history".into(),
                description: "读取一个 Telegram 或法布施联系人会话的最近历史。".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "conversationId": {"type": "string"},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                    },
                    "required": ["conversationId"],
                    "additionalProperties": false
                }),
                defer_loading: false,
            }),
            DynamicToolNamespaceTool::Function(DynamicToolFunctionSpec {
                name: "read_workspace_file".into(),
                description: "读取 App 私有大乘工作区内的 UTF-8 文本文件。".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "工作区内的相对路径"}
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                defer_loading: false,
            }),
            DynamicToolNamespaceTool::Function(DynamicToolFunctionSpec {
                name: "write_workspace_file".into(),
                description: "创建或覆盖 App 私有大乘工作区内的 UTF-8 文本文件。".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "工作区内的相对路径"},
                        "contents": {"type": "string", "description": "完整文件内容"}
                    },
                    "required": ["path", "contents"],
                    "additionalProperties": false
                }),
                defer_loading: false,
            }),
            DynamicToolNamespaceTool::Function(DynamicToolFunctionSpec {
                name: "list_workspace_files".into(),
                description: "列出 App 私有大乘工作区根目录中的文件。".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                defer_loading: false,
            }),
        ],
    })]
}

fn required_string_argument<'a>(arguments: &'a Value, name: &str) -> Option<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn dynamic_tool_success(value: Value) -> DynamicToolCallResponse {
    match serde_json::to_string(&value) {
        Ok(text) if text.len() <= 32_000 => DynamicToolCallResponse {
            content_items: vec![DynamicToolCallOutputContentItem::InputText { text }],
            success: true,
        },
        _ => dynamic_tool_error("会话数据超过单次 Codex 上下文上限，请缩小查询范围"),
    }
}

fn dynamic_tool_error(message: &str) -> DynamicToolCallResponse {
    DynamicToolCallResponse {
        content_items: vec![DynamicToolCallOutputContentItem::InputText {
            text: json!({"error": message}).to_string(),
        }],
        success: false,
    }
}

fn approval_response(kind: ApprovalResponseKind, decision: ApprovalDecision) -> Value {
    match kind {
        ApprovalResponseKind::CommandExecution | ApprovalResponseKind::FileChange => json!({
            "decision": match decision {
                ApprovalDecision::Accept => "accept",
                ApprovalDecision::AcceptForSession => "acceptForSession",
                ApprovalDecision::Decline => "decline",
                ApprovalDecision::Cancel => "cancel",
            }
        }),
        ApprovalResponseKind::Permissions { requested } => match decision {
            ApprovalDecision::Accept | ApprovalDecision::AcceptForSession => json!({
                "permissions": requested,
                "scope": if matches!(decision, ApprovalDecision::AcceptForSession) {
                    "session"
                } else {
                    "turn"
                }
            }),
            ApprovalDecision::Decline | ApprovalDecision::Cancel => json!({
                "permissions": {},
                "scope": "turn"
            }),
        },
        ApprovalResponseKind::LegacyExec | ApprovalResponseKind::LegacyPatch => json!({
            "decision": match decision {
                ApprovalDecision::Accept => "approved",
                ApprovalDecision::AcceptForSession => "approved_for_session",
                ApprovalDecision::Decline => "denied",
                ApprovalDecision::Cancel => "abort",
            }
        }),
        ApprovalResponseKind::ComputerDynamic { .. } => {
            unreachable!("Computer dynamic approvals resolve by executing the suspended tool call")
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_product_approval_decisions_to_app_server_responses() {
        assert_eq!(
            approval_response(
                ApprovalResponseKind::CommandExecution,
                ApprovalDecision::AcceptForSession,
            ),
            json!({"decision": "acceptForSession"})
        );
        assert_eq!(
            approval_response(ApprovalResponseKind::LegacyExec, ApprovalDecision::Cancel),
            json!({"decision": "abort"})
        );
    }

    #[test]
    fn computer_tool_schema_preserves_batching_and_field_names() {
        let schema = computer_tool_schema();
        let properties = schema["properties"].as_object().expect("properties");
        assert!(properties.contains_key("count"));
        assert!(properties.contains_key("durationMs"));
        assert!(!properties.contains_key("clickCount"));
        assert!(!properties.contains_key("waitMs"));
        let primary = properties["action"]["enum"]
            .as_array()
            .expect("primary actions");
        assert!(primary.iter().any(|value| value == "screenshot"));
        let follow_up = &properties["then"];
        assert_eq!(follow_up["maxItems"], 9);
        let follow_actions = follow_up["items"]["properties"]["action"]["enum"]
            .as_array()
            .expect("follow-up actions");
        assert!(!follow_actions.iter().any(|value| value == "screenshot"));
        assert_eq!(follow_actions.len(), 7);
    }

    #[test]
    fn matches_marketplace_qualified_plugin_ids_by_short_name() {
        assert!(plugin_identity_matches(
            "bot-father@fabushi-official",
            "bot-father",
            "bot-father"
        ));
        assert!(plugin_identity_matches(
            "bot-father@fabushi-official",
            "bot-father",
            "bot-father@fabushi-official"
        ));
        assert!(!plugin_identity_matches(
            "mahayana-assistant@fabushi-official",
            "mahayana-assistant",
            "bot-father"
        ));
    }

    #[test]
    fn empty_completed_item_does_not_erase_agent_output() {
        let mut text = "DeepSeek tool result".to_string();

        merge_completed_agent_text(&mut text, String::new());

        assert_eq!(text, "DeepSeek tool result");
    }

    #[test]
    fn removes_nulls_before_thread_config_conversion() {
        let mut value = json!({
            "tool_timeout_sec": null,
            "nested": {"keep": 3, "drop": null},
            "items": [null, {"drop": null, "keep": true}]
        });
        remove_null_values(&mut value);
        assert_eq!(
            value,
            json!({
                "nested": {"keep": 3},
                "items": [null, {"keep": true}]
            })
        );
    }

    #[test]
    fn accepts_anonymous_product_session_for_first_party_allowance() {
        let config = CodexAgentConfig {
            codex_home: PathBuf::from("/tmp/mahayana-test"),
            inherit_installed_plugins: false,
            cwd: PathBuf::from("/tmp"),
            workspace_roots: Vec::new(),
            model: "deepseek-chat".into(),
            responses_base_url: "https://example.test/v1".into(),
            use_codex_account: false,
            product_session_token: None,
            sandbox_mode: SandboxMode::ReadOnly,
            approval_policy: AskForApproval::OnRequest,
            codex_executable_path: None,
            conversation_providers: Vec::new(),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_non_https_provider_and_header_injection() {
        let mut config = CodexAgentConfig {
            codex_home: PathBuf::from("/tmp/mahayana-test"),
            inherit_installed_plugins: false,
            cwd: PathBuf::from("/tmp"),
            workspace_roots: Vec::new(),
            model: "deepseek-chat".into(),
            responses_base_url: "http://example.test/v1".into(),
            use_codex_account: false,
            product_session_token: Some("secret".into()),
            sandbox_mode: SandboxMode::ReadOnly,
            approval_policy: AskForApproval::OnRequest,
            codex_executable_path: None,
            conversation_providers: Vec::new(),
        };
        assert!(config.validate().is_err());
        config.responses_base_url = "https://example.test/v1".into();
        config.product_session_token = Some("secret\nInjected: yes".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn permits_only_explicit_loopback_http_for_local_cli_testing() {
        assert!(responses_endpoint_is_secure("http://127.0.0.1:8788/v1"));
        assert!(responses_endpoint_is_secure("http://localhost:8788/v1"));
        assert!(responses_endpoint_is_secure("http://[::1]:8788/v1"));
        assert!(!responses_endpoint_is_secure("http://192.168.1.10:8788/v1"));
        assert!(!responses_endpoint_is_secure(
            "http://localhost.evil.test:8788/v1"
        ));
        assert!(!responses_endpoint_is_secure(
            "https://example.test\nInjected: yes"
        ));
    }

    #[test]
    fn shared_local_plugins_and_mcp_servers_are_projected_without_auth_state() {
        let marketplace =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../.agents/plugins");
        let config = toml::from_str::<toml::Value>(&format!(
            r#"
                [marketplaces.shared-test]
                source_type = "local"
                source = "{}"

                [marketplaces.remote-test]
                source_type = "git"
                source = "https://example.test/plugins.git"

                [plugins."global-dharma@shared-test"]
                enabled = true

                [plugins."ignored@remote-test"]
                enabled = true

                [mcp_servers.node_repl]
                command = "/tmp/node_repl"
                args = []
            "#,
            marketplace.display()
        ))
        .expect("shared config");

        let overrides =
            shared_installed_plugin_overrides_from_config(&config, Path::new("/tmp/shared-codex"));
        let layer = codex_config::build_cli_overrides_layer(&overrides);
        assert!(layer["marketplaces"]["shared-test"].is_table());
        assert!(layer["plugins"]["global-dharma@shared-test"].is_table());
        assert!(layer["mcp_servers"]["node_repl"].is_table());
        assert!(layer["mcp_servers"].get("\"node_repl\"").is_none());
        assert!(
            layer["marketplaces"].get("remote-test").is_none()
                && layer["plugins"].get("ignored@remote-test").is_none()
        );
    }

    #[test]
    fn shared_enabled_local_plugin_roots_follow_marketplace_layout() {
        let marketplace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../");
        let config = toml::from_str::<toml::Value>(&format!(
            r#"
                [marketplaces.shared-test]
                source_type = "local"
                source = "{}"

                [plugins."global-dharma@shared-test"]
                enabled = true
            "#,
            marketplace_root.display()
        ))
        .expect("shared config");

        let roots =
            shared_installed_plugin_roots_from_config(&config, Path::new("/tmp/shared-codex"));
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with(".agents/plugins/plugins/global-dharma"));
        assert!(roots[0].join(".codex-plugin/plugin.json").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn relative_plugin_mcp_commands_resolve_against_their_configured_cwd() {
        let mut config = serde_json::from_value::<McpServerConfig>(json!({
            "command": "./echo",
            "args": [],
            "cwd": "/bin"
        }))
        .expect("relative stdio MCP config");

        assert!(mcp_runtime_available(&config));
        resolve_relative_mcp_command(&mut config);
        let McpServerTransportConfig::Stdio { command, .. } = config.transport else {
            panic!("expected stdio MCP config");
        };
        assert_eq!(command, "/bin/echo");
    }
}
