//! Product-level feature controller over the direct Mahayana Runtime Host.
//!
//! `HostMode::Test` is a deterministic in-process backend for fast E2E. It uses
//! real Rust commands, state, approvals, event ordering, and lifecycle without
//! network or simulator dependencies. Production provider mappings are added
//! feature-by-feature and must never silently fall back to test behavior.

use base64::Engine as _;
use chrono::Datelike;
use chrono::NaiveDate;
use chrono::TimeZone;
use chrono::Timelike;
use chrono::Utc;

#[cfg(feature = "production")]
use mahayana_core::ApprovalDecision as RuntimeApprovalDecision;
#[cfg(feature = "production")]
use mahayana_core::ApprovalId;
#[cfg(feature = "production")]
use mahayana_core::CODEX_ASSISTANT_CONVERSATION_ID;
#[cfg(feature = "production")]
use mahayana_core::ConversationId;
#[cfg(feature = "production")]
use mahayana_core::MessageRole as RuntimeMessageRole;
#[cfg(feature = "production")]
use mahayana_core::OperationId;
#[cfg(feature = "production")]
use mahayana_core::RuntimeActivityStatus;
#[cfg(feature = "production")]
use mahayana_core::RuntimeCommand;
#[cfg(feature = "production")]
use mahayana_core::RuntimeEvent;
#[cfg(feature = "production")]
use mahayana_core::RuntimeResponse;
#[cfg(feature = "production")]
use mahayana_core::capability::CapabilityAvailability;
#[cfg(feature = "production")]
use mahayana_core::capability::CapabilityKind;
#[cfg(feature = "production")]
use mahayana_host::HostCreateConfig;
#[cfg(feature = "production")]
use mahayana_host::MahayanaHost;
use mahayana_host_protocol::AgentBroadcastResult;
use mahayana_host_protocol::AgentMode;
use mahayana_host_protocol::AgentPeerMessage;
use mahayana_host_protocol::AgentStepStatus;
#[cfg(feature = "production")]
use mahayana_host_protocol::ApprovalDecision;
use mahayana_host_protocol::ApprovalResolution;
use mahayana_host_protocol::AsyncTaskKind;
use mahayana_host_protocol::AsyncTaskStatus;
use mahayana_host_protocol::AsyncTaskSummary;
use mahayana_host_protocol::AttachmentChunkResult;
use mahayana_host_protocol::AttachmentContext;
use mahayana_host_protocol::AttachmentImageResult;
use mahayana_host_protocol::AttachmentStored;
use mahayana_host_protocol::AttachmentTextResult;
use mahayana_host_protocol::AutoReviewBehavior;
use mahayana_host_protocol::AutoReviewRule;
use mahayana_host_protocol::AutomationSummary;
use mahayana_host_protocol::AutomationTrigger;
use mahayana_host_protocol::BotSummary;
use mahayana_host_protocol::CapabilitySummary;
use mahayana_host_protocol::CommandAccepted;
use mahayana_host_protocol::ComputerActionResult;
use mahayana_host_protocol::ComputerControlOrigin;
use mahayana_host_protocol::ComputerSnapshot;
use mahayana_host_protocol::ConnectorAccountSummary;
use mahayana_host_protocol::ConnectorStatus;
use mahayana_host_protocol::ConnectorSummary;
use mahayana_host_protocol::ConnectorToolSummary;
use mahayana_host_protocol::ConnectorTransport;
use mahayana_host_protocol::ConversationMessage;
use mahayana_host_protocol::ConversationSummary;
use mahayana_host_protocol::DraftAction;
use mahayana_host_protocol::DraftSendState;
use mahayana_host_protocol::ErrorTray;
use mahayana_host_protocol::EventCard;
use mahayana_host_protocol::EventField;
use mahayana_host_protocol::FeatureCommand;
use mahayana_host_protocol::GroupMessage;
use mahayana_host_protocol::GroupSpeaker;
use mahayana_host_protocol::GroupSummary;
use mahayana_host_protocol::HOST_PROTOCOL_VERSION;
use mahayana_host_protocol::HostConfig;
use mahayana_host_protocol::HostEvent;
use mahayana_host_protocol::HostInfo;
use mahayana_host_protocol::HostMode;
use mahayana_host_protocol::ListenerIntegrationSummary;
use mahayana_host_protocol::ListenerPlatform;
use mahayana_host_protocol::LocalToolPermission;
use mahayana_host_protocol::MemoryKind;
use mahayana_host_protocol::MemoryRecord;
use mahayana_host_protocol::MessageDraft;
use mahayana_host_protocol::MessageRole;
use mahayana_host_protocol::ProductHostSettings;
use mahayana_host_protocol::SearchMediaMatch;
use mahayana_host_protocol::SearchMessageMatch;
use mahayana_host_protocol::SkillPublishState;
use mahayana_host_protocol::SkillSource;
use mahayana_host_protocol::SkillSummary;
use mahayana_host_protocol::SkillTeamSummary;
use mahayana_host_protocol::SubagentStatus;
use mahayana_host_protocol::SubagentSummary;
use mahayana_host_protocol::SurfacePlatform;
use mahayana_host_protocol::TEACH_MAX_DURATION_MS;
use mahayana_host_protocol::TeachEntryPoint;
use mahayana_host_protocol::TeachRecordingResult;
use mahayana_host_protocol::TeachRecordingStatus;
use mahayana_host_protocol::TranscriptCard;
use mahayana_host_protocol::UpdateState;
use mahayana_host_protocol::WorkflowSource;
use mahayana_host_protocol::WorkflowSummary;
use mahayana_host_protocol::WorkflowTrigger;
#[cfg(feature = "production")]
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use sha2::Digest as _;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::MutexGuard;
#[cfg(feature = "production")]
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum FeatureHostError {
    #[cfg(feature = "production")]
    #[error(transparent)]
    Runtime(#[from] mahayana_host::HostError),
    #[error("production Runtime support is not compiled into this Host")]
    ProductionUnavailable,
    #[error("feature Host state mutex is poisoned")]
    StatePoisoned,
    #[error("feature Host is closed")]
    Closed,
    #[error("{0}")]
    Contract(String),
}

#[cfg_attr(not(feature = "production"), allow(dead_code))]
#[derive(Debug)]
struct PendingApproval {
    mini_app_id: String,
    capability: String,
    runtime_approval_id: Option<String>,
}

const GROUP_MAX_MEMBER_TURNS: usize = 10;
const GROUP_MAX_ROUNDS: usize = 3;
const GROUP_PROMPT_HISTORY_LIMIT: usize = 24;

#[derive(Debug, Clone)]
struct GroupRunState {
    run_id: String,
    round: usize,
    speaker_order: Vec<String>,
    speaker_index: usize,
    total_messages: usize,
    messages_this_round: usize,
}

#[derive(Debug, Clone)]
struct GroupOperationContext {
    run_id: String,
    group_id: String,
    member_id: String,
    member_name: String,
}

#[derive(Debug, Clone)]
struct BackgroundOperationContext {
    agent_id: String,
    agent_name: String,
    source: String,
    teach_artifact: Option<String>,
}

#[derive(Debug, Clone)]
struct RemoteComputerLocalSession {
    device_id: String,
    client_id: String,
    expires_at_seconds: i64,
}

#[derive(Debug)]
struct TeachCaptureProcess {
    agent_id: String,
    entry_point: TeachEntryPoint,
    started_at_ms: i64,
    session_dir: PathBuf,
    video_path: PathBuf,
    child: Option<std::process::Child>,
}

enum MemoryAction {
    List { limit: usize },
    Add { content: String, kind: MemoryKind },
    Remove { id: String },
    Clear,
}

#[derive(Debug)]
struct FeatureState {
    events: VecDeque<HostEvent>,
    installed: BTreeMap<String, String>,
    pending_approvals: BTreeMap<String, PendingApproval>,
    operations: BTreeSet<String>,
    operation_agents: BTreeMap<String, String>,
    background_operations: BTreeMap<String, BackgroundOperationContext>,
    remote_computer_sessions: BTreeMap<String, RemoteComputerLocalSession>,
    remote_computer_device_secrets: BTreeMap<String, String>,
    subagents: BTreeMap<String, SubagentSummary>,
    async_tasks: BTreeMap<String, AsyncTaskSummary>,
    peer_messages: Vec<AgentPeerMessage>,
    settings: ProductHostSettings,
    trays: Vec<ErrorTray>,
    sequence: u64,
    closed: bool,
    session_active: bool,
    auth_user: Option<Value>,
    automations: BTreeMap<String, AutomationSummary>,
    connectors: BTreeMap<String, ConnectorSummary>,
    skills: BTreeMap<String, SkillSummary>,
    bots: BTreeMap<String, BotSummary>,
    groups: BTreeMap<String, GroupSummary>,
    group_runs: BTreeMap<String, GroupRunState>,
    group_operations: BTreeMap<String, GroupOperationContext>,
    listeners: BTreeMap<ListenerPlatform, ListenerIntegrationSummary>,
    update_state: UpdateState,
}

impl Default for FeatureState {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            installed: BTreeMap::new(),
            pending_approvals: BTreeMap::new(),
            operations: BTreeSet::new(),
            operation_agents: BTreeMap::new(),
            background_operations: BTreeMap::new(),
            remote_computer_sessions: BTreeMap::new(),
            remote_computer_device_secrets: BTreeMap::new(),
            subagents: BTreeMap::new(),
            async_tasks: BTreeMap::new(),
            peer_messages: Vec::new(),
            settings: ProductHostSettings::default(),
            trays: Vec::new(),
            sequence: 0,
            closed: false,
            session_active: true,
            auth_user: None,
            automations: BTreeMap::new(),
            connectors: default_connectors(),
            skills: default_skills(),
            bots: default_bots(),
            groups: BTreeMap::new(),
            group_runs: BTreeMap::new(),
            group_operations: BTreeMap::new(),
            listeners: default_listeners(),
            update_state: UpdateState::UpToDate {
                version: env!("CARGO_PKG_VERSION").into(),
            },
        }
    }
}

pub struct FeatureHostController {
    config: HostConfig,
    info: HostInfo,
    #[cfg(feature = "production")]
    runtime: Option<MahayanaHost>,
    automation_path: Option<PathBuf>,
    bot_state_path: Option<PathBuf>,
    group_state_path: Option<PathBuf>,
    peer_messages_path: Option<PathBuf>,
    settings_path: Option<PathBuf>,
    remote_device_state_path: Option<PathBuf>,
    memory_root_path: Option<PathBuf>,
    workflow_root_path: Option<PathBuf>,
    teach_recording: Mutex<Option<TeachCaptureProcess>>,
    state: Mutex<FeatureState>,
}

impl FeatureHostController {
    pub fn create(config: HostConfig, platform: SurfacePlatform) -> Result<Self, FeatureHostError> {
        validate_config(&config)?;
        match config.mode {
            HostMode::Test => Ok(Self::create_test_backend(config, platform)),
            HostMode::Production => {
                #[cfg(feature = "production")]
                {
                    Self::create_with_host_config(config, platform, HostCreateConfig::default())
                }
                #[cfg(not(feature = "production"))]
                {
                    Err(FeatureHostError::ProductionUnavailable)
                }
            }
        }
    }

    fn create_test_backend(config: HostConfig, platform: SurfacePlatform) -> Self {
        let memory_root_path = Some(std::env::temp_dir().join(format!(
            "fabushi-feature-host-memory-{}-{}",
            config.profile_id,
            std::process::id()
        )));
        let workflow_root_path = Some(std::env::temp_dir().join(format!(
            "fabushi-feature-host-workflows-{}-{}",
            config.profile_id,
            std::process::id()
        )));
        let info = HostInfo {
            runtime_version: "mahayana-test-backend".to_string(),
            protocol_version: HOST_PROTOCOL_VERSION.to_string(),
            platform,
        };
        let mut state = FeatureState::default();
        sync_computer_control_policy(&state.settings);
        state.events.push_back(HostEvent::HostReady {
            timestamp: timestamp(),
            info: info.clone(),
        });
        Self {
            config,
            info,
            #[cfg(feature = "production")]
            runtime: None,
            automation_path: None,
            bot_state_path: None,
            group_state_path: None,
            peer_messages_path: None,
            settings_path: None,
            remote_device_state_path: None,
            memory_root_path,
            workflow_root_path,
            teach_recording: Mutex::new(None),
            state: Mutex::new(state),
        }
    }

    #[cfg(feature = "production")]
    pub fn create_with_host_config(
        config: HostConfig,
        platform: SurfacePlatform,
        host_config: HostCreateConfig,
    ) -> Result<Self, FeatureHostError> {
        validate_config(&config)?;
        if config.mode == HostMode::Test {
            return Ok(Self::create_test_backend(config, platform));
        }
        let automation_path = host_config.automation_path.clone().or_else(|| {
            host_config
                .runtime
                .data_dir
                .as_ref()
                .map(|data_dir| data_dir.join("automations.json"))
        });
        let bot_state_path = host_config
            .runtime
            .data_dir
            .as_ref()
            .map(|data_dir| data_dir.join("bots.json"));
        let group_state_path = host_config
            .runtime
            .data_dir
            .as_ref()
            .map(|data_dir| data_dir.join("groups.json"));
        let peer_messages_path = host_config
            .runtime
            .data_dir
            .as_ref()
            .map(|data_dir| data_dir.join("peer-messages.json"));
        let settings_path = host_config
            .runtime
            .data_dir
            .as_ref()
            .map(|data_dir| data_dir.join("settings.json"));
        let remote_device_state_path = host_config
            .runtime
            .data_dir
            .as_ref()
            .map(|data_dir| data_dir.join("remote-computer-device.json"));
        let memory_root_path = host_config
            .runtime
            .data_dir
            .as_ref()
            .map(|data_dir| data_dir.join("agents"));
        let workflow_root_path = host_config
            .runtime
            .data_dir
            .as_ref()
            .map(|data_dir| data_dir.join("workflows"));
        let runtime = MahayanaHost::create(host_config)?;
        let info = HostInfo {
            runtime_version: format!("mahayana-abi-{}", runtime.status().runtime_abi_version),
            protocol_version: HOST_PROTOCOL_VERSION.to_string(),
            platform,
        };
        let mut state = FeatureState::default();
        if let Some(path) = automation_path.as_deref() {
            state.automations = load_automations(path);
        }
        if let Some(path) = bot_state_path.as_deref() {
            for (id, bot) in load_bots(path) {
                state.bots.insert(id, bot);
            }
        }
        if let Some(path) = group_state_path.as_deref() {
            state.groups = load_groups(path);
        }
        if let Some(path) = peer_messages_path.as_deref() {
            state.peer_messages = load_peer_messages(path);
        }
        if let Some(path) = settings_path.as_deref() {
            state.settings = load_product_host_settings(path);
        }
        if let Some(path) = remote_device_state_path.as_deref() {
            state.remote_computer_device_secrets = load_remote_computer_device_secrets(path);
        }
        sync_computer_control_policy(&state.settings);
        state.events.push_back(HostEvent::HostReady {
            timestamp: timestamp(),
            info: info.clone(),
        });
        Ok(Self {
            config,
            info,
            runtime: Some(runtime),
            automation_path,
            bot_state_path,
            group_state_path,
            peer_messages_path,
            settings_path,
            remote_device_state_path,
            memory_root_path,
            workflow_root_path,
            teach_recording: Mutex::new(None),
            state: Mutex::new(state),
        })
    }

    pub fn info(&self) -> HostInfo {
        self.info.clone()
    }

    /// Return UI-safe account state. Credentials stay inside the Rust product
    /// client and are never serialized across the presentation boundary.
    pub fn auth_status(&self) -> Result<Value, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => {
                let state = self.state()?;
                Ok(match state.auth_user.as_ref() {
                    Some(user) => json!({
                        "@type": "mahayana.auth.status",
                        "loggedIn": true,
                        "provider": "test",
                        "user": user,
                    }),
                    None => json!({
                        "@type": "mahayana.auth.status",
                        "loggedIn": false,
                        "provider": "test",
                    }),
                })
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                {
                    // First paint must be local-first. Restoring the UI-safe
                    // account from the Rust-owned session is immediate and
                    // lets an offline macOS launch remain signed in. Network
                    // operations still validate the token at their boundary.
                    if let Ok(session) = self
                        .runtime()?
                        .product_execute("mahayana.auth.session.restore", &json!({}))
                    {
                        return Ok(session);
                    }
                    self.runtime()?
                        .product_execute("mahayana.auth.status", &json!({}))
                        .map_err(FeatureHostError::from)
                }
                #[cfg(not(feature = "production"))]
                Err(FeatureHostError::ProductionUnavailable)
            }
        }
    }

    pub fn password_login(
        &self,
        username: String,
        password: String,
    ) -> Result<Value, FeatureHostError> {
        let username = required(username, "username")?;
        let password = required(password, "password")?;
        #[cfg(not(feature = "production"))]
        let _ = &password;
        match self.config.mode {
            HostMode::Test => {
                let user = json!({
                    "id": "fast-e2e-user",
                    "username": username,
                    "nickname": "本地测试用户",
                });
                self.state()?.auth_user = Some(user.clone());
                Ok(json!({
                    "@type": "mahayana.auth.session",
                    "loggedIn": true,
                    "provider": "test",
                    "sessionStored": true,
                    "user": user,
                }))
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.password.login",
                        &json!({"username": username, "password": password}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn browser_login_start(&self) -> Result<Value, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => Ok(json!({
                "attemptId": "test-browser-login",
                "loginUrl": "about:blank#fabushi-test-browser-login",
                "expiresAt": now_millis() / 1000 + 600,
                "pollAfterMs": 250,
            })),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.browser.start",
                        &json!({"platform": "desktop"}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn browser_login_reopen(&self, attempt_id: String) -> Result<Value, FeatureHostError> {
        let attempt_id = required(attempt_id, "attemptId")?;
        match self.config.mode {
            HostMode::Test => Ok(json!({
                "status": "pending",
                "attemptId": attempt_id,
                "loginUrl": "about:blank#fabushi-test-browser-login",
                "pollAfterMs": 120,
            })),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.browser.reopen",
                        &json!({"attemptId": attempt_id}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn browser_login_cancel(&self, attempt_id: String) -> Result<Value, FeatureHostError> {
        let attempt_id = required(attempt_id, "attemptId")?;
        match self.config.mode {
            HostMode::Test => Ok(json!({"status": "cancelled"})),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.browser.cancel",
                        &json!({"attemptId": attempt_id}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn browser_login_poll(&self, attempt_id: String) -> Result<Value, FeatureHostError> {
        let attempt_id = required(attempt_id, "attemptId")?;
        match self.config.mode {
            HostMode::Test => {
                if attempt_id != "test-browser-login" {
                    return Ok(json!({"status": "expired"}));
                }
                let user = json!({
                    "id": "fast-e2e-browser-user",
                    "email": "browser@example.test",
                    "nickname": "Browser 测试用户",
                });
                self.state()?.auth_user = Some(user.clone());
                Ok(json!({
                    "status": "completed",
                    "provider": "browser",
                    "auth": {
                        "loggedIn": true,
                        "provider": "browser",
                        "user": user,
                    }
                }))
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.browser.poll",
                        &json!({"attemptId": attempt_id}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn auth_providers(&self) -> Result<Value, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => Ok(json!([
                {"id": "google", "displayName": "Google", "enabled": true},
                {"id": "apple", "displayName": "Apple", "enabled": true},
                {"id": "microsoft", "displayName": "Microsoft", "enabled": true},
                {"id": "github", "displayName": "GitHub", "enabled": true}
            ])),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute("mahayana.auth.oauth.providers", &json!({}))
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn oauth_start(&self, provider: String) -> Result<Value, FeatureHostError> {
        let provider = required(provider, "provider")?;
        match self.config.mode {
            HostMode::Test => Ok(json!({
                "attemptId": format!("test-oauth-{provider}"),
                "provider": provider,
                "authorizationUrl": format!("about:blank#fabushi-test-oauth-{provider}"),
            })),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.oauth.start",
                        &json!({"provider": provider, "platform": "macos"}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn oauth_poll(&self, attempt_id: String) -> Result<Value, FeatureHostError> {
        let attempt_id = required(attempt_id, "attemptId")?;
        match self.config.mode {
            HostMode::Test => {
                let user = json!({
                    "id": "fast-e2e-oauth-user",
                    "email": "oauth@example.test",
                    "nickname": "OAuth 测试用户",
                });
                self.state()?.auth_user = Some(user.clone());
                Ok(json!({
                    "attemptId": attempt_id,
                    "status": "completed",
                    "auth": {
                        "loggedIn": true,
                        "provider": "google",
                        "user": user,
                    }
                }))
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.oauth.poll",
                        &json!({"attemptId": attempt_id}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn logout(&self) -> Result<Value, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => {
                self.state()?.auth_user = None;
                Ok(json!({
                    "@type": "mahayana.auth.session",
                    "loggedIn": false,
                    "revoked": true,
                }))
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .clear_session()
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn execute(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        if matches!(
            &command,
            FeatureCommand::AutomationList { .. }
                | FeatureCommand::AutomationUpsert { .. }
                | FeatureCommand::AutomationSetEnabled { .. }
                | FeatureCommand::AutomationDelete { .. }
                | FeatureCommand::AutomationRun { .. }
        ) {
            return self.execute_automation(command);
        }
        if matches!(
            &command,
            FeatureCommand::BotCreate { .. }
                | FeatureCommand::BotUpdate { .. }
                | FeatureCommand::BotClone { .. }
                | FeatureCommand::BotDelete { .. }
                | FeatureCommand::BotSetHidden { .. }
        ) {
            return self.execute_bot_profile(command);
        }
        if matches!(
            &command,
            FeatureCommand::GroupList { .. }
                | FeatureCommand::GroupCreate { .. }
                | FeatureCommand::GroupUpdate { .. }
                | FeatureCommand::GroupDelete { .. }
                | FeatureCommand::GroupSend { .. }
        ) {
            return self.execute_group_chat(command);
        }
        if matches!(
            &command,
            FeatureCommand::AgentSend { .. }
                | FeatureCommand::AgentBroadcast { .. }
                | FeatureCommand::AgentPeerHistory { .. }
        ) {
            return self.execute_agent_messaging(command);
        }
        if matches!(
            &command,
            FeatureCommand::SubagentList { .. } | FeatureCommand::AsyncTaskList { .. }
        ) {
            return self.execute_subagent_observation(command);
        }
        if matches!(
            &command,
            FeatureCommand::TeachStatus { .. }
                | FeatureCommand::TeachStart { .. }
                | FeatureCommand::TeachStop { .. }
        ) {
            return self.execute_teach(command);
        }
        if matches!(
            &command,
            FeatureCommand::ComputerStatus { .. }
                | FeatureCommand::ComputerScreenshot { .. }
                | FeatureCommand::ComputerAction { .. }
        ) {
            return self.execute_computer(command);
        }
        if matches!(
            &command,
            FeatureCommand::RemoteComputerRegister { .. }
                | FeatureCommand::RemoteComputerHeartbeat { .. }
                | FeatureCommand::RemoteComputerClients { .. }
                | FeatureCommand::RemoteComputerClientRevoke { .. }
                | FeatureCommand::RemoteComputerSessions { .. }
                | FeatureCommand::RemoteComputerSessionActivate { .. }
                | FeatureCommand::RemoteComputerSessionClose { .. }
                | FeatureCommand::RemoteComputerSignal { .. }
                | FeatureCommand::RemoteComputerSignalDrain { .. }
        ) {
            return self.execute_remote_computer(command);
        }
        if matches!(
            &command,
            FeatureCommand::MemoryList { .. }
                | FeatureCommand::MemoryAdd { .. }
                | FeatureCommand::MemoryRemove { .. }
                | FeatureCommand::MemoryClear { .. }
        ) {
            return self.execute_memory(command);
        }
        if matches!(
            &command,
            FeatureCommand::TrayList { .. }
                | FeatureCommand::TrayDismiss { .. }
                | FeatureCommand::TrayClear { .. }
                | FeatureCommand::TrayClearForAgent { .. }
        ) {
            return self.execute_tray(command);
        }
        if matches!(
            &command,
            FeatureCommand::WorkflowList { .. }
                | FeatureCommand::WorkflowUpsert { .. }
                | FeatureCommand::WorkflowSetEnabled { .. }
                | FeatureCommand::WorkflowDelete { .. }
                | FeatureCommand::WorkflowRun { .. }
                | FeatureCommand::WorkflowImportMarkdown { .. }
                | FeatureCommand::WorkflowImportLiveSource { .. }
        ) {
            return self.execute_workflow(command);
        }
        if matches!(
            &command,
            FeatureCommand::AttachmentUpload { .. }
                | FeatureCommand::AttachmentReadText { .. }
                | FeatureCommand::AttachmentReadChunk { .. }
                | FeatureCommand::AttachmentReadImage { .. }
        ) {
            return self.execute_attachment(command);
        }
        if matches!(
            &command,
            FeatureCommand::SearchMessages { .. } | FeatureCommand::SearchMedia { .. }
        ) {
            return self.execute_search(command);
        }
        if matches!(
            &command,
            FeatureCommand::McpList { .. }
                | FeatureCommand::McpApps { .. }
                | FeatureCommand::McpOauthLogin { .. }
                | FeatureCommand::McpOauthLogout { .. }
                | FeatureCommand::McpRemove { .. }
                | FeatureCommand::McpSetCustomInstructions { .. }
                | FeatureCommand::McpSetToolDisabled { .. }
                | FeatureCommand::McpRefresh { .. }
                | FeatureCommand::McpToolCall { .. }
        ) {
            return self.execute_mcp(command);
        }
        if matches!(
            &command,
            FeatureCommand::SettingsGet { .. }
                | FeatureCommand::SettingsUpdate { .. }
                | FeatureCommand::AuditList { .. }
        ) {
            return self.execute_settings_and_audit(command);
        }
        if is_product_surface_command(&command) {
            return self.execute_product_surface(command);
        }
        match self.config.mode {
            HostMode::Test => self.execute_test(command),
            HostMode::Production => self.execute_production(command),
        }
    }

    fn execute_automation(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        match command {
            FeatureCommand::AutomationList { agent_id, .. } => {
                let mut automations = self
                    .state()?
                    .automations
                    .values()
                    .filter(|automation| {
                        agent_id.as_ref().is_none_or(|agent_id| {
                            automation.agent_id.as_deref() == Some(agent_id.as_str())
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                automations.sort_by_key(|item| item.created_at_ms);
                self.state()?.events.push_back(HostEvent::AutomationListed {
                    timestamp: timestamp(),
                    automations,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationUpsert {
                id,
                agent_id,
                name,
                prompt,
                schedule,
                trigger,
                enabled,
                ..
            } => {
                let name = required(name, "automation name")?;
                let prompt = required(prompt, "automation prompt")?;
                let trigger = trigger.unwrap_or_else(|| AutomationTrigger::Schedule {
                    schedule: schedule.clone(),
                });
                let (schedule, trigger) = match trigger {
                    AutomationTrigger::Schedule { schedule } => {
                        let schedule = normalize_automation_schedule(&schedule)?;
                        (schedule.clone(), AutomationTrigger::Schedule { schedule })
                    }
                    AutomationTrigger::Event {
                        source,
                        event,
                        filter,
                    } => {
                        let event = required(event, "automation event")?;
                        (
                            format!("event:{}:{event}", listener_platform_slug(source)),
                            AutomationTrigger::Event {
                                source,
                                event,
                                filter: filter.filter(|value| !value.trim().is_empty()),
                            },
                        )
                    }
                };
                let now = now_millis();
                let mut state = self.state()?;
                let id = id
                    .filter(|id| is_safe_automation_id(id))
                    .unwrap_or_else(|| {
                        state.sequence += 1;
                        format!("routine-{}-{}", now, state.sequence)
                    });
                let previous = state.automations.get(&id).cloned();
                let requested_agent_id = match agent_id {
                    Some(agent_id) => Some(required(agent_id, "automation agent id")?),
                    None => None,
                };
                if let Some(agent_id) = requested_agent_id.as_deref() {
                    if !state.bots.contains_key(agent_id) {
                        return Err(FeatureHostError::Contract(format!(
                            "unknown automation agent: {agent_id}"
                        )));
                    }
                }
                if let (Some(previous), Some(agent_id)) =
                    (previous.as_ref(), requested_agent_id.as_deref())
                {
                    ensure_automation_agent_scope(previous, Some(agent_id))?;
                }
                let resolved_agent_id = requested_agent_id
                    .or_else(|| previous.as_ref().and_then(|item| item.agent_id.clone()));
                let action = if previous.is_some() {
                    "updated"
                } else {
                    "created"
                };
                let automation = AutomationSummary {
                    id: id.clone(),
                    agent_id: resolved_agent_id,
                    name,
                    prompt,
                    schedule: schedule.clone(),
                    trigger: Some(trigger.clone()),
                    enabled,
                    created_at_ms: previous.as_ref().map_or(now, |item| item.created_at_ms),
                    last_run_at_ms: previous.as_ref().and_then(|item| item.last_run_at_ms),
                    next_run_at_ms: automation_next_run(&trigger, &schedule, enabled, now),
                };
                state.automations.insert(id, automation.clone());
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: action.into(),
                    automation,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationSetEnabled {
                id,
                agent_id,
                enabled,
                ..
            } => {
                let mut state = self.state()?;
                let automation = state.automations.get_mut(&id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown automation: {id}"))
                })?;
                ensure_automation_agent_scope(automation, agent_id.as_deref())?;
                automation.enabled = enabled;
                let trigger =
                    automation
                        .trigger
                        .clone()
                        .unwrap_or_else(|| AutomationTrigger::Schedule {
                            schedule: automation.schedule.clone(),
                        });
                automation.next_run_at_ms =
                    automation_next_run(&trigger, &automation.schedule, enabled, now_millis());
                let automation = automation.clone();
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: if enabled { "resumed" } else { "paused" }.into(),
                    automation,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationDelete { id, agent_id, .. } => {
                let mut state = self.state()?;
                let existing = state.automations.get(&id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown automation: {id}"))
                })?;
                ensure_automation_agent_scope(existing, agent_id.as_deref())?;
                let automation = state
                    .automations
                    .remove(&id)
                    .expect("automation checked above");
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: "deleted".into(),
                    automation,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationRun { id, agent_id, .. } => {
                let automation =
                    {
                        let mut state = self.state()?;
                        let automation = state.automations.get_mut(&id).ok_or_else(|| {
                            FeatureHostError::Contract(format!("unknown automation: {id}"))
                        })?;
                        ensure_automation_agent_scope(automation, agent_id.as_deref())?;
                        let now = now_millis();
                        automation.last_run_at_ms = Some(now);
                        let trigger = automation.trigger.clone().unwrap_or_else(|| {
                            AutomationTrigger::Schedule {
                                schedule: automation.schedule.clone(),
                            }
                        });
                        automation.next_run_at_ms = automation_next_run(
                            &trigger,
                            &automation.schedule,
                            automation.enabled,
                            now,
                        );
                        let automation = automation.clone();
                        self.persist_automations(&state.automations)?;
                        state.events.push_back(HostEvent::AutomationChanged {
                            timestamp: timestamp(),
                            action: "running".into(),
                            automation: automation.clone(),
                        });
                        automation
                    };
                if let Some(AutomationTrigger::Event {
                    source,
                    event,
                    filter,
                }) = automation.trigger.as_ref()
                {
                    self.state()?.events.push_back(HostEvent::TranscriptCard {
                        timestamp: timestamp(),
                        entry_id: format!("event-{}-{}", automation.id, now_millis()),
                        operation_id: None,
                        card: TranscriptCard::Event {
                            event: EventCard {
                                source: *source,
                                event: event.clone(),
                                title: format!("{} event", listener_platform_display(*source)),
                                summary: format!("{} woke routine “{}”.", event, automation.name),
                                url: None,
                                actor: None,
                                fields: filter.as_ref().map(|filter| {
                                    vec![EventField {
                                        label: "Filter".into(),
                                        value: filter.clone(),
                                    }]
                                }),
                                occurred_at_ms: Some(now_millis()),
                            },
                        },
                    });
                }
                let trigger_context = match automation.trigger.as_ref() {
                    Some(AutomationTrigger::Event { source, event, .. }) => format!(
                        "\n触发事件：{} / {}",
                        listener_platform_display(*source),
                        event
                    ),
                    _ => String::new(),
                };
                let text = format!(
                    "[自动化例程：{}]{}\n这是用户保存的 standing instruction。请立即执行并报告结果。\n\n{}",
                    automation.name, trigger_context, automation.prompt
                );
                let target_agent_id = automation
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "mahayana-assistant".into());
                match self.config.mode {
                    HostMode::Test => self.execute_test(FeatureCommand::ChatSend {
                        request_id,
                        text,
                        agent_id: Some(target_agent_id.clone()),
                        conversation_id: None,
                        mode: AgentMode::Agent,
                        mode_statement: None,
                        model: None,
                        attachments: Vec::new(),
                    }),
                    HostMode::Production => {
                        #[cfg(feature = "production")]
                        return self.production_chat(
                            request_id,
                            text,
                            Some(target_agent_id),
                            None,
                            AgentMode::Agent,
                            None,
                            None,
                            Vec::new(),
                        );
                        #[cfg(not(feature = "production"))]
                        return Err(FeatureHostError::ProductionUnavailable);
                    }
                }
            }
            _ => unreachable!("non-automation command routed to automation executor"),
        }
    }

    fn execute_bot_profile(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let mut state = self.state()?;
        ensure_open(&state)?;
        let (action, bot) = match command {
            FeatureCommand::BotCreate {
                name,
                description,
                title,
                avatar,
                avatar_shape,
                avatar_color,
                ..
            } => {
                let name = clamp_line(&name, 72);
                if name.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "bot name must not be empty".into(),
                    ));
                }
                let id = next_id(&mut state, "agent");
                let bot = BotSummary {
                    id: id.clone(),
                    name,
                    description: clamp_block(&description, 2000),
                    title: title.trim().to_string(),
                    hidden: false,
                    avatar: sanitize_avatar_data_url(avatar)?,
                    avatar_shape: clean_optional_string(avatar_shape),
                    avatar_color: clean_optional_string(avatar_color),
                    notifications_enabled: true,
                    notify_on_updates: true,
                    unread: false,
                    conversation_id: Some(format!("codex:agent:{id}")),
                };
                state.bots.insert(id, bot.clone());
                ("created", bot)
            }
            FeatureCommand::BotUpdate {
                id,
                name,
                description,
                title,
                avatar,
                avatar_shape,
                avatar_color,
                notifications_enabled,
                notify_on_updates,
                unread,
                ..
            } => {
                let bot = state
                    .bots
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown bot: {id}")))?;
                if let Some(name) = name {
                    let name = clamp_line(&name, 72);
                    if name.is_empty() {
                        return Err(FeatureHostError::Contract(
                            "bot name must not be empty".into(),
                        ));
                    }
                    bot.name = name;
                }
                if let Some(description) = description {
                    bot.description = clamp_block(&description, 2000);
                }
                if let Some(title) = title {
                    bot.title = title.trim().to_string();
                }
                if avatar.is_some() {
                    bot.avatar = sanitize_avatar_data_url(avatar)?;
                }
                if avatar_shape.is_some() {
                    bot.avatar_shape = clean_optional_string(avatar_shape);
                }
                if avatar_color.is_some() {
                    bot.avatar_color = clean_optional_string(avatar_color);
                }
                if let Some(enabled) = notifications_enabled {
                    bot.notifications_enabled = enabled;
                }
                if let Some(enabled) = notify_on_updates {
                    bot.notify_on_updates = enabled;
                }
                if let Some(unread) = unread {
                    bot.unread = unread;
                }
                ("updated", bot.clone())
            }
            FeatureCommand::BotClone { id, .. } => {
                let source = state
                    .bots
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown bot: {id}")))?;
                let new_id = next_id(&mut state, "agent");
                let clone_name = clone_agent_display_name(&source.name);
                let bot = BotSummary {
                    id: new_id.clone(),
                    name: clone_name,
                    description: source.description,
                    title: source.title,
                    hidden: false,
                    avatar: source.avatar,
                    avatar_shape: source.avatar_shape,
                    avatar_color: source.avatar_color,
                    notifications_enabled: source.notifications_enabled,
                    notify_on_updates: source.notify_on_updates,
                    unread: false,
                    conversation_id: Some(format!("codex:agent:{new_id}")),
                };
                state.bots.insert(new_id, bot.clone());
                ("cloned", bot)
            }
            FeatureCommand::BotDelete { id, .. } => {
                if id == "mahayana-assistant" {
                    return Err(FeatureHostError::Contract(
                        "the primary Mahayana assistant cannot be deleted".into(),
                    ));
                }
                let bot = state
                    .bots
                    .remove(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown bot: {id}")))?;
                ("deleted", bot)
            }
            FeatureCommand::BotSetHidden { id, hidden, .. } => {
                let bot = state
                    .bots
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown bot: {id}")))?;
                bot.hidden = hidden;
                ("updated", bot.clone())
            }
            _ => unreachable!("non-bot-profile command routed to bot executor"),
        };
        self.persist_bots(&state.bots)?;
        state.events.push_back(HostEvent::BotChanged {
            timestamp: timestamp(),
            action: action.into(),
            bot,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_group_chat(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let mut state = self.state()?;
        ensure_open(&state)?;
        let mut kick_group_id: Option<String> = None;
        match command {
            FeatureCommand::GroupList { .. } => {
                let mut groups = state.groups.values().cloned().collect::<Vec<_>>();
                groups.sort_by_key(|group| group.created_at_ms);
                state.events.push_back(HostEvent::GroupListed {
                    timestamp: timestamp(),
                    groups,
                });
            }
            FeatureCommand::GroupCreate {
                name,
                description,
                member_ids,
                ..
            } => {
                let name = clamp_line(&name, 72);
                if name.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "group name must not be empty".into(),
                    ));
                }
                let member_ids = validate_group_members(&state, member_ids)?;
                let now = now_millis();
                let id = next_id(&mut state, "group");
                let group = GroupSummary {
                    id: id.clone(),
                    name,
                    description: clamp_block(&description, 2000),
                    member_ids,
                    messages: Vec::new(),
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                state.groups.insert(id, group.clone());
                self.persist_groups(&state.groups)?;
                state.events.push_back(HostEvent::GroupChanged {
                    timestamp: timestamp(),
                    action: "created".into(),
                    group,
                });
            }
            FeatureCommand::GroupUpdate {
                id,
                name,
                description,
                member_ids,
                ..
            } => {
                let validated_members = member_ids
                    .map(|member_ids| validate_group_members(&state, member_ids))
                    .transpose()?;
                let group = state
                    .groups
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown group: {id}")))?;
                if let Some(name) = name {
                    let name = clamp_line(&name, 72);
                    if name.is_empty() {
                        return Err(FeatureHostError::Contract(
                            "group name must not be empty".into(),
                        ));
                    }
                    group.name = name;
                }
                if let Some(description) = description {
                    group.description = clamp_block(&description, 2000);
                }
                if let Some(member_ids) = validated_members {
                    group.member_ids = member_ids;
                }
                group.updated_at_ms = now_millis();
                let group = group.clone();
                self.persist_groups(&state.groups)?;
                state.events.push_back(HostEvent::GroupChanged {
                    timestamp: timestamp(),
                    action: "updated".into(),
                    group,
                });
            }
            FeatureCommand::GroupDelete { id, .. } => {
                let group = state
                    .groups
                    .remove(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown group: {id}")))?;
                self.persist_groups(&state.groups)?;
                state.events.push_back(HostEvent::GroupChanged {
                    timestamp: timestamp(),
                    action: "deleted".into(),
                    group,
                });
            }
            FeatureCommand::GroupSend { id, text, .. } => {
                let text = clamp_block(&text, 8000);
                if text.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "group message must not be empty".into(),
                    ));
                }
                let message_id = next_id(&mut state, "group-message");
                let now = now_millis();
                let group = state
                    .groups
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown group: {id}")))?;
                group.messages.push(GroupMessage {
                    id: message_id,
                    speaker: GroupSpeaker::User { name: None },
                    content: text,
                    created_at_ms: now,
                });
                if group.messages.len() > 500 {
                    let overflow = group.messages.len() - 500;
                    group.messages.drain(0..overflow);
                }
                group.updated_at_ms = now;
                let group = group.clone();
                let responder_ids = resolve_group_responders(&group, &state.bots);
                if !responder_ids.is_empty() {
                    let run_id = next_id(&mut state, "group-run");
                    state.group_runs.insert(
                        id.clone(),
                        GroupRunState {
                            run_id,
                            round: 0,
                            speaker_order: order_round_speakers(&responder_ids, 0),
                            speaker_index: 0,
                            total_messages: 0,
                            messages_this_round: 0,
                        },
                    );
                    kick_group_id = Some(id.clone());
                }
                self.persist_groups(&state.groups)?;
                state.events.push_back(HostEvent::GroupChanged {
                    timestamp: timestamp(),
                    action: "message".into(),
                    group,
                });
            }
            _ => unreachable!("non-group command routed to group executor"),
        }
        drop(state);
        let operation_id = match (self.config.mode, kick_group_id) {
            (HostMode::Production, Some(group_id)) => {
                #[cfg(feature = "production")]
                {
                    self.start_next_group_turn(&group_id)?
                }
                #[cfg(not(feature = "production"))]
                {
                    let _ = group_id;
                    None
                }
            }
            _ => None,
        };
        Ok(CommandAccepted {
            request_id,
            operation_id,
        })
    }

    fn execute_teach(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        self.refresh_finished_teach_recording()?;
        match command {
            FeatureCommand::TeachStatus { .. } => {
                let status = {
                    let guard = self
                        .teach_recording
                        .lock()
                        .map_err(|_| FeatureHostError::StatePoisoned)?;
                    teach_recording_status(guard.as_ref())
                };
                self.state()?.events.push_back(HostEvent::TeachChanged {
                    timestamp: timestamp(),
                    status,
                    result: None,
                });
            }
            FeatureCommand::TeachStart {
                agent_id,
                entry_point,
                ..
            } => {
                if !is_safe_memory_agent_id(&agent_id)
                    || !self.state()?.bots.contains_key(&agent_id)
                {
                    return Err(FeatureHostError::Contract(format!(
                        "unknown teach agent: {agent_id}"
                    )));
                }
                let mut guard = self
                    .teach_recording
                    .lock()
                    .map_err(|_| FeatureHostError::StatePoisoned)?;
                if let Some(active) = guard.as_ref() {
                    if active.agent_id == agent_id {
                        let status = teach_recording_status(Some(active));
                        drop(guard);
                        self.state()?.events.push_back(HostEvent::TeachChanged {
                            timestamp: timestamp(),
                            status,
                            result: None,
                        });
                        return Ok(CommandAccepted {
                            request_id,
                            operation_id: None,
                        });
                    }
                    return Err(FeatureHostError::Contract(format!(
                        "teach recording is already active for {}",
                        active.agent_id
                    )));
                }

                let root = self.memory_root_path.as_deref().ok_or_else(|| {
                    FeatureHostError::Contract("teach recording storage is unavailable".into())
                })?;
                let started_at_ms = now_millis();
                let session_dir = root
                    .join(&agent_id)
                    .join("teach-sessions")
                    .join(format!("{started_at_ms}"));
                std::fs::create_dir_all(&session_dir).map_err(|error| {
                    FeatureHostError::Contract(format!("create teach session: {error}"))
                })?;
                let video_path = session_dir.join("demo.mp4");
                let child = if self.config.mode == HostMode::Production {
                    Some(spawn_teach_capture(&video_path)?)
                } else {
                    None
                };
                *guard = Some(TeachCaptureProcess {
                    agent_id: agent_id.clone(),
                    entry_point,
                    started_at_ms,
                    session_dir,
                    video_path,
                    child,
                });
                let status = teach_recording_status(guard.as_ref());
                drop(guard);
                self.state()?.events.push_back(HostEvent::TeachChanged {
                    timestamp: timestamp(),
                    status,
                    result: None,
                });
            }
            FeatureCommand::TeachStop { agent_id, save, .. } => {
                let active = {
                    let mut guard = self
                        .teach_recording
                        .lock()
                        .map_err(|_| FeatureHostError::StatePoisoned)?;
                    let Some(active) = guard.as_ref() else {
                        let status = TeachRecordingStatus::default();
                        drop(guard);
                        self.state()?.events.push_back(HostEvent::TeachChanged {
                            timestamp: timestamp(),
                            status,
                            result: None,
                        });
                        return Ok(CommandAccepted {
                            request_id,
                            operation_id: None,
                        });
                    };
                    if active.agent_id != agent_id {
                        return Err(FeatureHostError::Contract(format!(
                            "teach recording belongs to {}, not {agent_id}",
                            active.agent_id
                        )));
                    }
                    guard.take().expect("teach recording existed")
                };
                let result = self.finalize_teach_capture(active, save)?;
                self.state()?.events.push_back(HostEvent::TeachChanged {
                    timestamp: timestamp(),
                    status: TeachRecordingStatus::default(),
                    result: Some(result),
                });
            }
            _ => unreachable!("non-teach command routed to teach executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn refresh_finished_teach_recording(&self) -> Result<(), FeatureHostError> {
        let finished = {
            let mut guard = self
                .teach_recording
                .lock()
                .map_err(|_| FeatureHostError::StatePoisoned)?;
            let Some(active) = guard.as_mut() else {
                return Ok(());
            };
            let process_finished = match active.child.as_mut() {
                Some(child) => child
                    .try_wait()
                    .map_err(|error| {
                        FeatureHostError::Contract(format!("poll teach capture: {error}"))
                    })?
                    .is_some(),
                None => now_millis() - active.started_at_ms >= TEACH_MAX_DURATION_MS,
            };
            process_finished.then(|| guard.take().expect("teach recording existed"))
        };
        if let Some(active) = finished {
            let result = self.finalize_teach_capture(active, true)?;
            self.state()?.events.push_back(HostEvent::TeachChanged {
                timestamp: timestamp(),
                status: TeachRecordingStatus::default(),
                result: Some(result),
            });
        }
        Ok(())
    }

    fn finalize_teach_capture(
        &self,
        mut active: TeachCaptureProcess,
        save: bool,
    ) -> Result<TeachRecordingResult, FeatureHostError> {
        if let Some(child) = active.child.as_mut() {
            stop_teach_capture(child)?;
        }
        let ended_at_ms = now_millis();
        let duration_ms = (ended_at_ms - active.started_at_ms).clamp(0, TEACH_MAX_DURATION_MS);
        if !save {
            let _ = std::fs::remove_dir_all(&active.session_dir);
            return Ok(TeachRecordingResult {
                agent_id: active.agent_id,
                video_path: active.video_path.to_string_lossy().to_string(),
                started_at_ms: active.started_at_ms,
                ended_at_ms,
                duration_ms,
                saved: false,
            });
        }

        if self.config.mode == HostMode::Test && !active.video_path.exists() {
            std::fs::write(&active.video_path, b"fabushi-test-teach-video").map_err(|error| {
                FeatureHostError::Contract(format!("write teach test fixture: {error}"))
            })?;
        }
        let metadata = std::fs::metadata(&active.video_path).map_err(|error| {
            FeatureHostError::Contract(format!("teach capture did not produce a video: {error}"))
        })?;
        if metadata.len() == 0 {
            return Err(FeatureHostError::Contract(
                "teach capture produced an empty video".into(),
            ));
        }

        let manifest = json!({
            "agentId": active.agent_id,
            "entryPoint": active.entry_point,
            "startedAtMs": active.started_at_ms,
            "endedAtMs": ended_at_ms,
            "durationMs": duration_ms,
            "videoPath": active.video_path,
        });
        std::fs::write(
            active.session_dir.join("session.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|error| {
                FeatureHostError::Contract(format!("serialize teach manifest: {error}"))
            })?,
        )
        .map_err(|error| FeatureHostError::Contract(format!("write teach manifest: {error}")))?;

        if self.config.mode == HostMode::Production {
            let _ = extract_teach_frames(&active.video_path, &active.session_dir.join("frames"));
        }
        let video_path = active.video_path.to_string_lossy().to_string();
        let _ = self.schedule_teach_learning(&active.agent_id, &video_path, &active.session_dir);
        Ok(TeachRecordingResult {
            agent_id: active.agent_id,
            video_path,
            started_at_ms: active.started_at_ms,
            ended_at_ms,
            duration_ms,
            saved: true,
        })
    }

    fn schedule_teach_learning(
        &self,
        agent_id: &str,
        video_path: &str,
        session_dir: &Path,
    ) -> Result<Option<String>, FeatureHostError> {
        let bot = self.state()?.bots.get(agent_id).cloned().ok_or_else(|| {
            FeatureHostError::Contract(format!("unknown teach agent: {agent_id}"))
        })?;
        let frames_dir = session_dir.join("frames").to_string_lossy().to_string();
        let prompt = format!(
            "[teach-recording] The user just demonstrated a repeatable task for you.\nRecording: {video_path}\nExtracted frames (when present): {frames_dir}\n\nStudy the demonstration carefully. Infer the intent, ordered steps, important UI landmarks, decision points, and safety checks. Return a reusable Markdown workflow/skill only: start with a concise # heading, then instructions another future run can follow. Do not merely summarize the recording and do not mention this hidden teach prompt."
        );
        if self.config.mode == HostMode::Test {
            return Ok(None);
        }
        #[cfg(feature = "production")]
        {
            let conversation_id = bot.conversation_id.clone().ok_or_else(|| {
                FeatureHostError::Contract(format!("bot has no conversation: {}", bot.id))
            })?;
            let response = self.runtime()?.execute(RuntimeCommand::SendMessage {
                conversation_id: ConversationId(conversation_id),
                text: prompt,
                client_message_id: Some(format!("teach:{}:{}", bot.id, now_millis())),
                hidden: true,
            })?;
            let operation_id = match response {
                RuntimeResponse::Accepted { operation_id } => operation_id.to_string(),
                other => return Err(unexpected_response("teach.learn", other)),
            };
            let mut state = self.state()?;
            state.background_operations.insert(
                operation_id.clone(),
                BackgroundOperationContext {
                    agent_id: bot.id.clone(),
                    agent_name: bot.name.clone(),
                    source: "teach-recording".into(),
                    teach_artifact: Some(video_path.to_string()),
                },
            );
            state.events.push_back(HostEvent::AgentBackgroundStarted {
                timestamp: timestamp(),
                agent_id: bot.id,
                agent_name: bot.name,
                operation_id: operation_id.clone(),
                source: "teach-recording".into(),
            });
            Ok(Some(operation_id))
        }
        #[cfg(not(feature = "production"))]
        Ok(None)
    }

    fn persist_teach_workflow(
        &self,
        agent_id: &str,
        artifact: &str,
        markdown: &str,
    ) -> Result<WorkflowSummary, FeatureHostError> {
        let workflow_root = self
            .workflow_root_path
            .as_deref()
            .ok_or_else(|| FeatureHostError::Contract("workflow storage is unavailable".into()))?;
        let body = clamp_block(markdown, 100_000);
        if body.is_empty() {
            return Err(FeatureHostError::Contract(
                "teach learning returned an empty workflow".into(),
            ));
        }
        let name = derive_teach_workflow_name(&body);
        let base_id = slugify_teach_workflow_name(&name);
        let mut id = base_id.clone();
        let mut suffix = 2usize;
        while workflow_root.join(&id).exists() {
            id = format!("{base_id}-{suffix}");
            suffix += 1;
            if suffix > 999 {
                id = format!("{base_id}-{}", now_millis());
                break;
            }
        }
        let folder = workflow_root.join(&id);
        std::fs::create_dir_all(&folder).map_err(|error| {
            FeatureHostError::Contract(format!("create learned workflow: {error}"))
        })?;
        let description = "Learned from a recorded demonstration.".to_string();
        let metadata = json!({
            "name": name,
            "description": description,
            "metadata": { "source": artifact },
        });
        let yaml = serde_yaml::to_string(&metadata).map_err(|error| {
            FeatureHostError::Contract(format!("serialize learned workflow frontmatter: {error}"))
        })?;
        let file_path = folder.join("SKILL.md");
        std::fs::write(&file_path, format!("---\n{yaml}---\n{}\n", body.trim())).map_err(
            |error| FeatureHostError::Contract(format!("write learned workflow: {error}")),
        )?;
        let created_at = now_millis();
        let workflow = WorkflowSummary {
            id,
            name,
            description,
            body,
            trigger: None,
            source_ref: Some(artifact.to_string()),
            source: WorkflowSource::Workflow,
            plugin_id: None,
            published_by_current_user: false,
            is_enabled_for_agent: true,
            disable_model_invocation: None,
            schedule_description: None,
            created_at,
            last_run_at: None,
            next_run_at: None,
            helper_scripts: Vec::new(),
            file_path: file_path.to_string_lossy().to_string(),
        };
        if let Some(agent_root) = self.memory_root_path.as_deref() {
            let _ = set_workflow_enabled(agent_root, agent_id, &workflow.id, true);
        }
        Ok(workflow)
    }

    fn execute_subagent_observation(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        match command {
            FeatureCommand::SubagentList { agent_id, .. } => {
                let state = self.state()?;
                ensure_open(&state)?;
                let mut subagents = state
                    .subagents
                    .values()
                    .filter(|subagent| subagent.parent_agent_id == agent_id)
                    .cloned()
                    .collect::<Vec<_>>();
                subagents.sort_by_key(|subagent| subagent.started_at_ms);
                drop(state);
                self.state()?.events.push_back(HostEvent::SubagentListed {
                    timestamp: timestamp(),
                    agent_id,
                    subagents,
                });
            }
            FeatureCommand::AsyncTaskList { agent_id, .. } => {
                let state = self.state()?;
                ensure_open(&state)?;
                let mut tasks = state
                    .async_tasks
                    .values()
                    .filter(|task| task.parent_agent_id == agent_id)
                    .cloned()
                    .collect::<Vec<_>>();
                tasks.sort_by_key(|task| task.started_at_ms);
                drop(state);
                self.state()?.events.push_back(HostEvent::AsyncTaskListed {
                    timestamp: timestamp(),
                    agent_id,
                    tasks,
                });
            }
            _ => unreachable!("non-subagent command routed to subagent observer"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_agent_messaging(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        match command {
            FeatureCommand::AgentPeerHistory {
                agent_id, limit, ..
            } => {
                let state = self.state()?;
                ensure_open(&state)?;
                if !state.bots.contains_key(&agent_id) {
                    return Err(FeatureHostError::Contract(format!(
                        "unknown bot: {agent_id}"
                    )));
                }
                let mut messages = state
                    .peer_messages
                    .iter()
                    .filter(|message| {
                        message.from_agent_id == agent_id || message.target_id == agent_id
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                messages.sort_by_key(|message| message.created_at_ms);
                if messages.len() > limit.min(1000) {
                    let start = messages.len() - limit.min(1000);
                    messages = messages.split_off(start);
                }
                drop(state);
                self.state()?
                    .events
                    .push_back(HostEvent::AgentPeerHistoryListed {
                        timestamp: timestamp(),
                        agent_id,
                        messages,
                    });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AgentSend {
                from_agent_id,
                target_id,
                text,
                priority,
                ..
            } => {
                let text = clamp_block(&text, 8000);
                if text.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "agent message must not be empty".into(),
                    ));
                }
                if from_agent_id == target_id {
                    return Err(FeatureHostError::Contract(
                        "an agent cannot message itself".into(),
                    ));
                }

                let mut kick_group: Option<String> = None;
                let direct_target = {
                    let mut state = self.state()?;
                    ensure_open(&state)?;
                    let sender = state.bots.get(&from_agent_id).cloned().ok_or_else(|| {
                        FeatureHostError::Contract(format!("unknown sender bot: {from_agent_id}"))
                    })?;
                    if let Some(target) = state.bots.get(&target_id).cloned() {
                        let peer = AgentPeerMessage {
                            id: next_id(&mut state, "agent-message"),
                            from_agent_id: sender.id.clone(),
                            from_agent_name: sender.name.clone(),
                            target_id: target.id.clone(),
                            target_name: target.name.clone(),
                            text: text.clone(),
                            priority,
                            created_at_ms: now_millis(),
                        };
                        state.peer_messages.push(peer.clone());
                        if state.peer_messages.len() > 5000 {
                            let overflow = state.peer_messages.len() - 5000;
                            state.peer_messages.drain(0..overflow);
                        }
                        self.persist_peer_messages(&state.peer_messages)?;
                        state.events.push_back(HostEvent::AgentPeerMessageChanged {
                            timestamp: timestamp(),
                            message: peer,
                        });
                        Some((sender, target))
                    } else if let Some(group_snapshot) = state.groups.get(&target_id).cloned() {
                        if !group_snapshot
                            .member_ids
                            .iter()
                            .any(|id| id == &from_agent_id)
                        {
                            return Err(FeatureHostError::Contract(format!(
                                "agent {from_agent_id} is not a member of group {target_id}"
                            )));
                        }
                        let now = now_millis();
                        let message_id = next_id(&mut state, "group-message");
                        let group = state
                            .groups
                            .get_mut(&target_id)
                            .expect("group snapshot existed");
                        group.messages.push(GroupMessage {
                            id: message_id,
                            speaker: GroupSpeaker::Member {
                                id: sender.id.clone(),
                                name: sender.name.clone(),
                            },
                            content: text.clone(),
                            created_at_ms: now,
                        });
                        if group.messages.len() > 500 {
                            let overflow = group.messages.len() - 500;
                            group.messages.drain(0..overflow);
                        }
                        group.updated_at_ms = now;
                        let group = group.clone();
                        let mut responders = group
                            .member_ids
                            .iter()
                            .filter(|id| *id != &from_agent_id)
                            .cloned()
                            .collect::<Vec<_>>();
                        let lower = text.to_lowercase();
                        let mentioned = responders
                            .iter()
                            .filter(|id| {
                                state.bots.get(*id).is_some_and(|bot| {
                                    group_member_handles(&bot.name)
                                        .iter()
                                        .any(|handle| has_group_mention_at(&lower, handle))
                                })
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        if !mentioned.is_empty() && !has_everyone_group_mention(&lower) {
                            responders = mentioned;
                        }
                        if !responders.is_empty() {
                            let run_id = next_id(&mut state, "group-run");
                            state.group_runs.insert(
                                target_id.clone(),
                                GroupRunState {
                                    run_id,
                                    round: 0,
                                    speaker_order: responders,
                                    speaker_index: 0,
                                    total_messages: 0,
                                    messages_this_round: 0,
                                },
                            );
                            kick_group = Some(target_id.clone());
                        }
                        self.persist_groups(&state.groups)?;
                        state.events.push_back(HostEvent::GroupChanged {
                            timestamp: timestamp(),
                            action: "message".into(),
                            group,
                        });
                        None
                    } else {
                        return Err(FeatureHostError::Contract(format!(
                            "unknown agent or group: {target_id}"
                        )));
                    }
                };

                if let Some(group_id) = kick_group {
                    if self.config.mode == HostMode::Production {
                        #[cfg(feature = "production")]
                        {
                            let _ = self.start_next_group_turn(&group_id)?;
                        }
                    }
                    return Ok(CommandAccepted {
                        request_id,
                        operation_id: None,
                    });
                }

                if let Some((sender, target)) = direct_target {
                    let wake_prompt = build_agent_inbound_wake_prompt(&sender, &text, priority);
                    self.schedule_background_agent_turn(
                        &target,
                        if priority {
                            "agent-priority"
                        } else {
                            "agent-message"
                        },
                        wake_prompt,
                        format!("peer:{}:{}", sender.id, request_id),
                    )?;
                }
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AgentBroadcast {
                target_ids,
                message,
                ..
            } => {
                let message = clamp_block(&message, 8000);
                if message.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "broadcast message must not be empty".into(),
                    ));
                }
                let targets = {
                    let state = self.state()?;
                    ensure_open(&state)?;
                    match target_ids {
                        Some(ids) => {
                            let unique = ids.into_iter().collect::<BTreeSet<_>>();
                            unique
                                .into_iter()
                                .filter_map(|id| state.bots.get(&id).cloned())
                                .collect::<Vec<_>>()
                        }
                        None => state.bots.values().cloned().collect::<Vec<_>>(),
                    }
                };
                let total = targets.len();
                let mut scheduled = 0usize;
                for target in targets {
                    let prompt = build_admin_broadcast_wake_prompt(&message);
                    if self
                        .schedule_background_agent_turn(
                            &target,
                            "broadcast",
                            prompt,
                            format!("broadcast:{}:{}", target.id, request_id),
                        )
                        .is_ok()
                    {
                        scheduled += 1;
                    }
                }
                let result = AgentBroadcastResult { total, scheduled };
                self.state()?.events.push_back(HostEvent::AgentBroadcasted {
                    timestamp: timestamp(),
                    result,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            _ => unreachable!("non-agent-messaging command routed to agent messaging executor"),
        }
    }

    fn schedule_background_agent_turn(
        &self,
        target: &BotSummary,
        source: &str,
        prompt: String,
        client_message_id: String,
    ) -> Result<Option<String>, FeatureHostError> {
        if self.config.mode == HostMode::Test {
            let operation_id = format!("background-test-{}-{}", target.id, now_millis());
            let mut state = self.state()?;
            state.events.push_back(HostEvent::AgentBackgroundStarted {
                timestamp: timestamp(),
                agent_id: target.id.clone(),
                agent_name: target.name.clone(),
                operation_id: operation_id.clone(),
                source: source.to_string(),
            });
            state.events.push_back(HostEvent::AgentBackgroundMessage {
                timestamp: timestamp(),
                agent_id: target.id.clone(),
                agent_name: target.name.clone(),
                operation_id: operation_id.clone(),
                source: source.to_string(),
                text: format!("{} received background work.", target.name),
            });
            state.events.push_back(HostEvent::AgentBackgroundFinished {
                timestamp: timestamp(),
                agent_id: target.id.clone(),
                agent_name: target.name.clone(),
                operation_id: operation_id.clone(),
                source: source.to_string(),
                error: None,
            });
            return Ok(Some(operation_id));
        }

        #[cfg(feature = "production")]
        {
            let conversation_id = target.conversation_id.clone().ok_or_else(|| {
                FeatureHostError::Contract(format!("bot has no conversation: {}", target.id))
            })?;
            let response = self.runtime()?.execute(RuntimeCommand::SendMessage {
                conversation_id: ConversationId(conversation_id),
                text: prompt,
                client_message_id: Some(client_message_id),
                hidden: true,
            })?;
            let operation_id = match response {
                RuntimeResponse::Accepted { operation_id } => operation_id.to_string(),
                other => return Err(unexpected_response("agent.background", other)),
            };
            let mut state = self.state()?;
            state.background_operations.insert(
                operation_id.clone(),
                BackgroundOperationContext {
                    agent_id: target.id.clone(),
                    agent_name: target.name.clone(),
                    source: source.to_string(),
                    teach_artifact: None,
                },
            );
            state.events.push_back(HostEvent::AgentBackgroundStarted {
                timestamp: timestamp(),
                agent_id: target.id.clone(),
                agent_name: target.name.clone(),
                operation_id: operation_id.clone(),
                source: source.to_string(),
            });
            Ok(Some(operation_id))
        }
        #[cfg(not(feature = "production"))]
        Err(FeatureHostError::ProductionUnavailable)
    }

    fn execute_computer(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let settings = self.state()?.settings.clone();
        match command {
            FeatureCommand::ComputerStatus { .. } => {
                let status = mahayana_computer::status(
                    settings.local_execution,
                    settings.route_egress_locally,
                    settings.remote_control_enabled,
                    settings.ai_computer_control_enabled,
                );
                self.state()?
                    .events
                    .push_back(HostEvent::ComputerStatusChanged {
                        timestamp: timestamp(),
                        request_id: request_id.clone(),
                        status,
                    });
            }
            FeatureCommand::ComputerScreenshot {
                origin, session_id, ..
            } => {
                self.ensure_computer_origin_allowed(origin, session_id.as_deref(), &settings)?;
                let snapshot = if self.config.mode == HostMode::Test {
                    test_computer_snapshot()
                } else {
                    mahayana_computer::capture_screen()
                        .map_err(|error| FeatureHostError::Contract(error.to_string()))?
                };
                let _ = self.append_action_audit(
                    "mahayana-assistant",
                    session_id.as_deref(),
                    json!({
                        "kind": "computerScreenshot",
                        "origin": computer_origin_label(origin),
                        "sessionId": session_id,
                        "capturedAtMs": snapshot.captured_at_ms,
                    }),
                );
                self.state()?
                    .events
                    .push_back(HostEvent::ComputerSnapshotCaptured {
                        timestamp: timestamp(),
                        request_id: request_id.clone(),
                        origin,
                        snapshot,
                    });
            }
            FeatureCommand::ComputerAction {
                origin,
                agent_id,
                session_id,
                action,
                then,
                ..
            } => {
                self.ensure_computer_origin_allowed(origin, session_id.as_deref(), &settings)?;
                let audit_agent_id = agent_id
                    .as_deref()
                    .filter(|id| is_safe_memory_agent_id(id))
                    .unwrap_or("mahayana-assistant");
                if let Some(agent_id) = agent_id.as_deref() {
                    if !self.state()?.bots.contains_key(agent_id) {
                        return Err(FeatureHostError::Contract(format!(
                            "unknown computer-control agent: {agent_id}"
                        )));
                    }
                }
                let mut actions = Vec::with_capacity(1 + then.len());
                actions.push(action);
                actions.extend(then);
                let result = if self.config.mode == HostMode::Test {
                    for action in &actions {
                        mahayana_computer::validate_action(action)
                            .map_err(|error| FeatureHostError::Contract(error.to_string()))?;
                    }
                    if actions.len() > mahayana_host_protocol::COMPUTER_MAX_ACTIONS_PER_CALL {
                        return Err(FeatureHostError::Contract(format!(
                            "at most {} computer actions may be batched",
                            mahayana_host_protocol::COMPUTER_MAX_ACTIONS_PER_CALL
                        )));
                    }
                    ComputerActionResult {
                        origin,
                        actions_executed: actions.len(),
                        snapshot: test_computer_snapshot(),
                    }
                } else {
                    mahayana_computer::execute(&actions, origin)
                        .map_err(|error| FeatureHostError::Contract(error.to_string()))?
                };
                let serialized_actions = serde_json::to_value(&actions).unwrap_or(Value::Null);
                self.append_action_audit(
                    audit_agent_id,
                    session_id.as_deref(),
                    json!({
                        "kind": "computerUse",
                        "origin": computer_origin_label(origin),
                        "sessionId": session_id,
                        "actionCount": result.actions_executed,
                        "actions": serialized_actions,
                        "status": "success",
                    }),
                )?;
                self.state()?
                    .events
                    .push_back(HostEvent::ComputerActionCompleted {
                        timestamp: timestamp(),
                        request_id: request_id.clone(),
                        result,
                    });
            }
            _ => unreachable!("non-computer command routed to computer executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_remote_computer(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let remote_enabled = self.state()?.settings.remote_control_enabled;
        let (method, action, payload, local_transition) = match command {
            FeatureCommand::RemoteComputerRegister {
                device_id, label, ..
            } => {
                if !remote_enabled {
                    return Err(FeatureHostError::Contract(
                        "enable remote computer control before creating a pairing code".into(),
                    ));
                }
                let device_secret = self.remote_device_secret(&device_id, true)?;
                (
                    "mahayana.remote.computer.register",
                    "registered",
                    json!({"deviceId": device_id, "label": label, "deviceSecret": device_secret}),
                    None,
                )
            }
            FeatureCommand::RemoteComputerHeartbeat { device_id, .. } => {
                if !remote_enabled {
                    return Err(FeatureHostError::Contract(
                        "remote computer control is disabled".into(),
                    ));
                }
                let device_secret = self.remote_device_secret(&device_id, false)?;
                (
                    "mahayana.remote.computer.heartbeat",
                    "heartbeat",
                    json!({"deviceId": device_id, "deviceSecret": device_secret}),
                    None,
                )
            }
            FeatureCommand::RemoteComputerClients { device_id, .. } => (
                "mahayana.remote.computer.clients",
                "clients",
                json!({"deviceId": device_id}),
                None,
            ),
            FeatureCommand::RemoteComputerClientRevoke {
                device_id,
                client_id,
                ..
            } => (
                "mahayana.remote.computer.client.revoke",
                "clientRevoked",
                json!({"deviceId": device_id, "clientId": client_id}),
                Some(("revoke-client".to_string(), client_id)),
            ),
            FeatureCommand::RemoteComputerSessions { device_id, .. } => {
                if !remote_enabled {
                    return Err(FeatureHostError::Contract(
                        "remote computer control is disabled".into(),
                    ));
                }
                let device_secret = self.remote_device_secret(&device_id, false)?;
                (
                    "mahayana.remote.computer.sessions",
                    "sessions",
                    json!({"deviceId": device_id, "deviceSecret": device_secret}),
                    None,
                )
            }
            FeatureCommand::RemoteComputerSessionActivate {
                device_id,
                session_id,
                ..
            } => {
                if !remote_enabled {
                    return Err(FeatureHostError::Contract(
                        "remote computer control is disabled".into(),
                    ));
                }
                let device_secret = self.remote_device_secret(&device_id, false)?;
                (
                    "mahayana.remote.computer.session.activate",
                    "sessionActivated",
                    json!({"deviceId": device_id, "sessionId": session_id, "deviceSecret": device_secret}),
                    Some(("activate".to_string(), session_id)),
                )
            }
            FeatureCommand::RemoteComputerSessionClose {
                device_id,
                session_id,
                ..
            } => {
                let device_secret = self.remote_device_secret(&device_id, false)?;
                (
                    "mahayana.remote.computer.session.close",
                    "sessionClosed",
                    json!({"deviceId": device_id, "sessionId": session_id, "role": "desktop", "deviceSecret": device_secret}),
                    Some(("close".to_string(), session_id)),
                )
            }
            FeatureCommand::RemoteComputerSignal {
                device_id,
                session_id,
                kind,
                payload,
                ..
            } => {
                if !remote_enabled {
                    return Err(FeatureHostError::Contract(
                        "remote computer control is disabled".into(),
                    ));
                }
                let device_secret = self.remote_device_secret(&device_id, false)?;
                (
                    "mahayana.remote.computer.signal",
                    "signal",
                    json!({
                        "deviceId": device_id,
                        "sessionId": session_id,
                        "senderRole": "desktop",
                        "deviceSecret": device_secret,
                        "kind": kind,
                        "payload": payload,
                    }),
                    None,
                )
            }
            FeatureCommand::RemoteComputerSignalDrain {
                device_id,
                session_id,
                after_signal_id,
                ..
            } => {
                if !remote_enabled {
                    return Err(FeatureHostError::Contract(
                        "remote computer control is disabled".into(),
                    ));
                }
                let device_secret = self.remote_device_secret(&device_id, false)?;
                (
                    "mahayana.remote.computer.signals.drain",
                    "signals",
                    json!({
                        "deviceId": device_id,
                        "sessionId": session_id,
                        "receiverRole": "desktop",
                        "deviceSecret": device_secret,
                        "afterSignalId": after_signal_id.max(0),
                    }),
                    None,
                )
            }
            _ => unreachable!("non-remote-computer command routed to remote computer executor"),
        };

        let data = if self.config.mode == HostMode::Test {
            match action {
                "registered" => json!({
                    "deviceId": payload.get("deviceId"),
                    "label": payload.get("label"),
                    "pairingCode": "AB12CD34",
                    "pairingExpiresAt": now_millis() / 1000 + 600,
                }),
                "heartbeat" => json!({"ok": true, "lastSeenAt": now_millis() / 1000}),
                "clients" => json!({"deviceId": payload.get("deviceId"), "clients": []}),
                "sessions" => json!({"deviceId": payload.get("deviceId"), "sessions": []}),
                "sessionActivated" => json!({
                    "sessionId": payload.get("sessionId"),
                    "clientId": "remote-client-test",
                    "expiresAt": now_millis() / 1000 + 7200,
                    "state": "active",
                }),
                "sessionClosed" => {
                    json!({"sessionId": payload.get("sessionId"), "state": "closed"})
                }
                "signals" => {
                    json!({"sessionId": payload.get("sessionId"), "signals": [], "lastSignalId": payload.get("afterSignalId")})
                }
                _ => json!({"ok": true}),
            }
        } else {
            #[cfg(feature = "production")]
            {
                self.runtime()?.product_execute(method, &payload)?
            }
            #[cfg(not(feature = "production"))]
            {
                return Err(FeatureHostError::ProductionUnavailable);
            }
        };

        if let Some((transition, id)) = local_transition {
            match transition.as_str() {
                "activate" => {
                    let client_id = data
                        .get("clientId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            FeatureHostError::Contract(
                                "remote session activation did not return clientId".into(),
                            )
                        })?
                        .to_string();
                    let expires_at_seconds = data
                        .get("expiresAt")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| {
                            FeatureHostError::Contract(
                                "remote session activation did not return expiresAt".into(),
                            )
                        })?;
                    let device_id = payload
                        .get("deviceId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.state()?.remote_computer_sessions.insert(
                        id,
                        RemoteComputerLocalSession {
                            device_id,
                            client_id,
                            expires_at_seconds,
                        },
                    );
                }
                "close" => {
                    self.state()?.remote_computer_sessions.remove(&id);
                }
                "revoke-client" => {
                    self.state()?
                        .remote_computer_sessions
                        .retain(|_, session| session.client_id != id);
                }
                _ => {}
            }
        }

        self.state()?
            .events
            .push_back(HostEvent::RemoteComputerChanged {
                timestamp: timestamp(),
                request_id: request_id.clone(),
                action: action.to_string(),
                data,
            });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn ensure_computer_origin_allowed(
        &self,
        origin: ComputerControlOrigin,
        session_id: Option<&str>,
        settings: &ProductHostSettings,
    ) -> Result<(), FeatureHostError> {
        match origin {
            ComputerControlOrigin::LocalUi => {
                if !settings.local_execution {
                    return Err(FeatureHostError::Contract(
                        "local computer control is disabled in settings".into(),
                    ));
                }
            }
            ComputerControlOrigin::RemoteMobile => {
                if !settings.remote_control_enabled {
                    return Err(FeatureHostError::Contract(
                        "remote computer control is disabled in settings".into(),
                    ));
                }
                let session_id = session_id
                    .filter(|session| !session.trim().is_empty())
                    .ok_or_else(|| {
                        FeatureHostError::Contract(
                            "remote computer control requires an active paired session id".into(),
                        )
                    })?;
                let mut state = self.state()?;
                let now = now_millis() / 1000;
                state
                    .remote_computer_sessions
                    .retain(|_, session| session.expires_at_seconds > now);
                let Some(session) = state.remote_computer_sessions.get(session_id) else {
                    return Err(FeatureHostError::Contract(
                        "remote computer session is not active on this desktop".into(),
                    ));
                };
                if session.device_id.trim().is_empty() || session.client_id.trim().is_empty() {
                    return Err(FeatureHostError::Contract(
                        "remote computer session metadata is invalid".into(),
                    ));
                }
            }
            ComputerControlOrigin::Ai => {
                if !settings.local_execution || !settings.ai_computer_control_enabled {
                    return Err(FeatureHostError::Contract(
                        "AI computer control is disabled in settings".into(),
                    ));
                }
                match settings.local_tool_permission {
                    LocalToolPermission::Never => {
                        return Err(FeatureHostError::Contract(
                            "AI local-tool permission is set to Never".into(),
                        ));
                    }
                    LocalToolPermission::Ask => {
                        return Err(FeatureHostError::Contract(
                            "AI computer control requires an explicit approval while local-tool permission is Ask"
                                .into(),
                        ));
                    }
                    LocalToolPermission::Always => {}
                }
            }
        }
        Ok(())
    }

    fn execute_memory(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let (agent_id, action) = match command {
            FeatureCommand::MemoryList {
                agent_id, limit, ..
            } => (agent_id, MemoryAction::List { limit }),
            FeatureCommand::MemoryAdd {
                agent_id,
                content,
                kind,
                ..
            } => (agent_id, MemoryAction::Add { content, kind }),
            FeatureCommand::MemoryRemove { agent_id, id, .. } => {
                (agent_id, MemoryAction::Remove { id })
            }
            FeatureCommand::MemoryClear { agent_id, .. } => (agent_id, MemoryAction::Clear),
            _ => unreachable!("non-memory command routed to memory executor"),
        };
        {
            let state = self.state()?;
            ensure_open(&state)?;
            if !state.bots.contains_key(&agent_id) {
                return Err(FeatureHostError::Contract(format!(
                    "unknown bot: {agent_id}"
                )));
            }
        }
        if !is_safe_memory_agent_id(&agent_id) {
            return Err(FeatureHostError::Contract(format!(
                "unsafe memory agent id: {agent_id}"
            )));
        }
        let root = self
            .memory_root_path
            .as_deref()
            .ok_or_else(|| FeatureHostError::Contract("memory storage is unavailable".into()))?;
        let memory_dir = root.join(&agent_id).join("memory");
        match action {
            MemoryAction::List { limit } => {
                let memories = list_memories(&memory_dir, limit.min(1000))?;
                let count = count_memories(&memory_dir)?;
                self.state()?.events.push_back(HostEvent::MemoryListed {
                    timestamp: timestamp(),
                    agent_id,
                    memories,
                    count,
                    location: Some(memory_dir.to_string_lossy().into_owned()),
                });
            }
            MemoryAction::Add { content, kind } => {
                let memory = add_memory(&memory_dir, &content, now_millis(), kind)?;
                self.state()?.events.push_back(HostEvent::MemoryChanged {
                    timestamp: timestamp(),
                    agent_id,
                    action: if memory.is_some() {
                        "added"
                    } else {
                        "duplicate"
                    }
                    .into(),
                    memory,
                });
            }
            MemoryAction::Remove { id } => {
                let removed = remove_memory(&memory_dir, &id)?;
                self.state()?.events.push_back(HostEvent::MemoryChanged {
                    timestamp: timestamp(),
                    agent_id,
                    action: if removed { "removed" } else { "notFound" }.into(),
                    memory: None,
                });
            }
            MemoryAction::Clear => {
                if memory_dir.exists() {
                    std::fs::remove_dir_all(&memory_dir).map_err(|error| {
                        FeatureHostError::Contract(format!("clear memory: {error}"))
                    })?;
                }
                self.state()?.events.push_back(HostEvent::MemoryChanged {
                    timestamp: timestamp(),
                    agent_id,
                    action: "cleared".into(),
                    memory: None,
                });
            }
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_tray(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let mut state = self.state()?;
        ensure_open(&state)?;
        match command {
            FeatureCommand::TrayList { .. } => {
                let trays = state.trays.clone();
                state.events.push_back(HostEvent::TrayListed {
                    timestamp: timestamp(),
                    trays,
                });
            }
            FeatureCommand::TrayDismiss { id, .. } => {
                let before = state.trays.len();
                state.trays.retain(|tray| tray.id != id);
                if state.trays.len() != before {
                    state.events.push_back(HostEvent::TrayChanged {
                        timestamp: timestamp(),
                        action: "dismissed".into(),
                        tray: None,
                        id: Some(id),
                    });
                }
            }
            FeatureCommand::TrayClear { .. } => {
                if !state.trays.is_empty() {
                    state.trays.clear();
                    state.events.push_back(HostEvent::TrayChanged {
                        timestamp: timestamp(),
                        action: "cleared".into(),
                        tray: None,
                        id: None,
                    });
                }
            }
            FeatureCommand::TrayClearForAgent { agent_id, .. } => {
                let removed = state
                    .trays
                    .iter()
                    .filter(|tray| tray.agent_id == agent_id)
                    .map(|tray| tray.id.clone())
                    .collect::<Vec<_>>();
                state.trays.retain(|tray| tray.agent_id != agent_id);
                for id in removed {
                    state.events.push_back(HostEvent::TrayChanged {
                        timestamp: timestamp(),
                        action: "dismissed".into(),
                        tray: None,
                        id: Some(id),
                    });
                }
            }
            _ => unreachable!("non-tray command routed to tray executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_workflow(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let workflow_root = self
            .workflow_root_path
            .as_deref()
            .ok_or_else(|| FeatureHostError::Contract("workflow storage is unavailable".into()))?;
        let agent_root = self
            .memory_root_path
            .as_deref()
            .ok_or_else(|| FeatureHostError::Contract("agent storage is unavailable".into()))?;
        let agent_id = match &command {
            FeatureCommand::WorkflowList { agent_id, .. }
            | FeatureCommand::WorkflowUpsert { agent_id, .. }
            | FeatureCommand::WorkflowSetEnabled { agent_id, .. }
            | FeatureCommand::WorkflowDelete { agent_id, .. }
            | FeatureCommand::WorkflowRun { agent_id, .. }
            | FeatureCommand::WorkflowImportMarkdown { agent_id, .. }
            | FeatureCommand::WorkflowImportLiveSource { agent_id, .. } => agent_id.clone(),
            _ => unreachable!("non-workflow command routed to workflow executor"),
        };
        {
            let state = self.state()?;
            ensure_open(&state)?;
            if !state.bots.contains_key(&agent_id) {
                return Err(FeatureHostError::Contract(format!(
                    "unknown bot: {agent_id}"
                )));
            }
        }
        if !is_safe_memory_agent_id(&agent_id) {
            return Err(FeatureHostError::Contract(format!(
                "unsafe workflow agent id: {agent_id}"
            )));
        }

        match command {
            FeatureCommand::WorkflowList { .. } => {
                let mut workflows = list_workflow_summaries(workflow_root, agent_root, &agent_id);
                let automations = {
                    let state = self.state()?;
                    state
                        .automations
                        .values()
                        .filter(|automation| {
                            automation
                                .agent_id
                                .as_deref()
                                .is_none_or(|owner| owner == agent_id.as_str())
                        })
                        .map(workflow_from_automation)
                        .collect::<Vec<_>>()
                };
                workflows.extend(automations);
                workflows.truncate(WORKFLOW_UI_LIMIT);
                self.state()?.events.push_back(HostEvent::WorkflowListed {
                    timestamp: timestamp(),
                    agent_id,
                    workflows,
                });
            }
            FeatureCommand::WorkflowUpsert {
                id,
                name,
                description,
                body,
                trigger,
                source_ref,
                ..
            } => {
                if let Some(mut trigger) = trigger {
                    trigger.schedule = normalize_automation_schedule(&trigger.schedule)?;
                    let now = now_millis();
                    let automation_id = id.clone().unwrap_or_else(|| slugify_workflow_name(&name));
                    let automation = {
                        let mut state = self.state()?;
                        let created_at_ms = state
                            .automations
                            .get(&automation_id)
                            .map(|automation| automation.created_at_ms)
                            .unwrap_or(now);
                        let last_run_at_ms = state
                            .automations
                            .get(&automation_id)
                            .and_then(|automation| automation.last_run_at_ms);
                        let automation = AutomationSummary {
                            id: automation_id.clone(),
                            agent_id: Some(agent_id.clone()),
                            name: clamp_workflow_name(&name),
                            prompt: clamp_workflow_body(&body),
                            schedule: trigger.schedule.clone(),
                            trigger: Some(AutomationTrigger::Schedule {
                                schedule: trigger.schedule.clone(),
                            }),
                            enabled: trigger.is_enabled,
                            created_at_ms,
                            last_run_at_ms,
                            next_run_at_ms: trigger
                                .is_enabled
                                .then(|| next_automation_run(&trigger.schedule, now))
                                .flatten(),
                        };
                        state
                            .automations
                            .insert(automation_id.clone(), automation.clone());
                        self.persist_automations(&state.automations)?;
                        automation
                    };
                    let workflow_dir = workflow_root.join(&automation_id);
                    if workflow_dir.exists() {
                        let _ = std::fs::remove_dir_all(&workflow_dir);
                    }
                    let workflow = workflow_from_automation(&automation);
                    self.state()?.events.push_back(HostEvent::WorkflowChanged {
                        timestamp: timestamp(),
                        agent_id,
                        action: "saved".into(),
                        workflow: Some(workflow),
                        id: None,
                    });
                } else {
                    if let Some(existing_id) = id.as_deref() {
                        let removed_automation = {
                            let mut state = self.state()?;
                            if let Some(existing) = state.automations.get(existing_id) {
                                ensure_automation_agent_scope(existing, Some(agent_id.as_str()))?;
                            }
                            let removed = state.automations.remove(existing_id).is_some();
                            if removed {
                                self.persist_automations(&state.automations)?;
                            }
                            removed
                        };
                        if removed_automation {
                            // Converting a scheduled automation back into a trigger-less workflow.
                        }
                    }
                    let workflow = write_workflow(
                        workflow_root,
                        agent_root,
                        &agent_id,
                        id.as_deref(),
                        &name,
                        &description,
                        &body,
                        None,
                        source_ref.as_deref(),
                    )?;
                    self.state()?.events.push_back(HostEvent::WorkflowChanged {
                        timestamp: timestamp(),
                        agent_id,
                        action: "saved".into(),
                        workflow: Some(workflow),
                        id: None,
                    });
                }
            }
            FeatureCommand::WorkflowSetEnabled { id, enabled, .. } => {
                let automation = {
                    let mut state = self.state()?;
                    if let Some(automation) = state.automations.get_mut(&id) {
                        ensure_automation_agent_scope(automation, Some(agent_id.as_str()))?;
                        automation.enabled = enabled;
                        automation.next_run_at_ms = if enabled {
                            next_automation_run(&automation.schedule, now_millis())
                        } else {
                            None
                        };
                        let automation = automation.clone();
                        self.persist_automations(&state.automations)?;
                        Some(automation)
                    } else {
                        None
                    }
                };
                let workflow = if let Some(automation) = automation {
                    Some(workflow_from_automation(&automation))
                } else {
                    set_workflow_enabled(agent_root, &agent_id, &id, enabled)?;
                    load_workflow_summary(workflow_root, agent_root, &agent_id, &id)
                };
                self.state()?.events.push_back(HostEvent::WorkflowChanged {
                    timestamp: timestamp(),
                    agent_id,
                    action: "enabled".into(),
                    workflow,
                    id: Some(id),
                });
            }
            FeatureCommand::WorkflowDelete { id, .. } => {
                let removed_automation = {
                    let mut state = self.state()?;
                    if let Some(existing) = state.automations.get(&id) {
                        ensure_automation_agent_scope(existing, Some(agent_id.as_str()))?;
                    }
                    let removed = state.automations.remove(&id).is_some();
                    if removed {
                        self.persist_automations(&state.automations)?;
                    }
                    removed
                };
                if !removed_automation {
                    let path = workflow_root.join(&id);
                    if path.exists() {
                        std::fs::remove_dir_all(&path).map_err(|error| {
                            FeatureHostError::Contract(format!("delete workflow: {error}"))
                        })?;
                    }
                    forget_workflow_enablement(agent_root, &agent_id, &id)?;
                }
                self.state()?.events.push_back(HostEvent::WorkflowChanged {
                    timestamp: timestamp(),
                    agent_id,
                    action: "deleted".into(),
                    workflow: None,
                    id: Some(id),
                });
            }
            FeatureCommand::WorkflowRun { id, .. } => {
                if self.state()?.automations.contains_key(&id) {
                    return self.execute_automation(FeatureCommand::AutomationRun {
                        request_id,
                        id,
                        agent_id: Some(agent_id),
                    });
                }
                let workflow = load_workflow_summary(workflow_root, agent_root, &agent_id, &id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown workflow: {id}")))?;
                if !workflow.is_enabled_for_agent {
                    return Err(FeatureHostError::Contract(format!(
                        "workflow is disabled for {agent_id}: {id}"
                    )));
                }
                let visible_text = format!("@{}", workflow.name);
                let recipe = clamp_block(&workflow.body, WORKFLOW_INJECTED_BODY_LIMIT);
                let runtime_text = format!(
                    "[Workflow reference: {}]\nFollow this recipe for the current turn:\n{}\n\n[Visible user message]\n{}",
                    workflow.name, recipe, visible_text
                );
                match self.config.mode {
                    HostMode::Test => {
                        let mut state = self.state()?;
                        state.events.push_back(HostEvent::ChatMessage {
                            timestamp: timestamp(),
                            role: MessageRole::User,
                            text: visible_text,
                            operation_id: None,
                        });
                        state.events.push_back(HostEvent::ChatMessage {
                            timestamp: timestamp(),
                            role: MessageRole::Assistant,
                            text: format!("Running workflow: {}", workflow.name),
                            operation_id: None,
                        });
                        return Ok(CommandAccepted {
                            request_id,
                            operation_id: None,
                        });
                    }
                    HostMode::Production => {
                        #[cfg(feature = "production")]
                        {
                            let conversation_id = self
                                .state()?
                                .bots
                                .get(&agent_id)
                                .and_then(|bot| bot.conversation_id.clone())
                                .ok_or_else(|| {
                                    FeatureHostError::Contract(format!(
                                        "bot has no conversation: {agent_id}"
                                    ))
                                })?;
                            let (provider, model) = match self
                                .runtime()?
                                .execute(RuntimeCommand::Status)?
                            {
                                RuntimeResponse::Status(status) => (
                                    format!("{:?}", status.model_provider).to_lowercase(),
                                    status.model,
                                ),
                                other => return Err(unexpected_response("runtime.status", other)),
                            };
                            let response =
                                self.runtime()?.execute(RuntimeCommand::SendMessage {
                                    conversation_id: ConversationId(conversation_id),
                                    text: runtime_text,
                                    client_message_id: Some(request_id.clone()),
                                    hidden: true,
                                })?;
                            let operation_id = match response {
                                RuntimeResponse::Accepted { operation_id } => {
                                    operation_id.to_string()
                                }
                                other => return Err(unexpected_response("workflow.run", other)),
                            };
                            let mut state = self.state()?;
                            state.operations.insert(operation_id.clone());
                            state
                                .operation_agents
                                .insert(operation_id.clone(), agent_id);
                            state.events.push_back(HostEvent::ModelRouted {
                                timestamp: timestamp(),
                                operation_id: operation_id.clone(),
                                provider,
                                model,
                                mode: AgentMode::Agent,
                            });
                            state.events.push_back(HostEvent::ChatMessage {
                                timestamp: timestamp(),
                                role: MessageRole::User,
                                text: visible_text,
                                operation_id: None,
                            });
                            state.events.push_back(HostEvent::OperationStarted {
                                timestamp: timestamp(),
                                operation_id: operation_id.clone(),
                                label: format!("workflow:{}", workflow.id),
                                interruptible: true,
                            });
                            return Ok(CommandAccepted {
                                request_id,
                                operation_id: Some(operation_id),
                            });
                        }
                        #[cfg(not(feature = "production"))]
                        return Err(FeatureHostError::ProductionUnavailable);
                    }
                }
            }
            FeatureCommand::WorkflowImportMarkdown {
                markdown,
                fallback_name,
                ..
            } => {
                let parsed = parse_workflow_file(&markdown).ok_or_else(|| {
                    FeatureHostError::Contract("workflow markdown is empty".into())
                })?;
                let name = if parsed.name.is_empty() {
                    derive_workflow_name_from_markdown(&parsed.body)
                        .or(fallback_name.map(|name| clamp_workflow_name(&name)))
                        .unwrap_or_default()
                } else {
                    parsed.name.clone()
                };
                if name.is_empty() || parsed.body.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "workflow markdown must include a name and body".into(),
                    ));
                }
                let workflow = if let Some(mut trigger) = parsed.trigger.clone() {
                    trigger.schedule = normalize_automation_schedule(&trigger.schedule)?;
                    let now = now_millis();
                    let id = slugify_workflow_name(&name);
                    let automation = AutomationSummary {
                        id: id.clone(),
                        agent_id: Some(agent_id.clone()),
                        name: name.clone(),
                        prompt: parsed.body.clone(),
                        schedule: trigger.schedule.clone(),
                        trigger: Some(AutomationTrigger::Schedule {
                            schedule: trigger.schedule.clone(),
                        }),
                        enabled: trigger.is_enabled,
                        created_at_ms: now,
                        last_run_at_ms: None,
                        next_run_at_ms: trigger
                            .is_enabled
                            .then(|| next_automation_run(&trigger.schedule, now))
                            .flatten(),
                    };
                    {
                        let mut state = self.state()?;
                        state.automations.insert(id, automation.clone());
                        self.persist_automations(&state.automations)?;
                    }
                    workflow_from_automation(&automation)
                } else {
                    write_workflow(
                        workflow_root,
                        agent_root,
                        &agent_id,
                        None,
                        &name,
                        &parsed.description,
                        &parsed.body,
                        None,
                        parsed.source_ref.as_deref(),
                    )?
                };
                self.state()?.events.push_back(HostEvent::WorkflowChanged {
                    timestamp: timestamp(),
                    agent_id,
                    action: "imported".into(),
                    workflow: Some(workflow),
                    id: None,
                });
            }
            FeatureCommand::WorkflowImportLiveSource {
                source,
                fallback_name,
                ..
            } => {
                let source = source.trim().to_string();
                if source.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "workflow live source must not be empty".into(),
                    ));
                }
                let name = fallback_name
                    .map(|name| clamp_workflow_name(&name))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| derive_workflow_name_from_source(&source));
                let description = build_live_source_description(&name, &source);
                let body = build_live_source_pointer_body(&source);
                let workflow = write_workflow(
                    workflow_root,
                    agent_root,
                    &agent_id,
                    None,
                    &name,
                    &description,
                    &body,
                    None,
                    Some(&source),
                )?;
                self.state()?.events.push_back(HostEvent::WorkflowChanged {
                    timestamp: timestamp(),
                    agent_id,
                    action: "imported".into(),
                    workflow: Some(workflow),
                    id: None,
                });
            }
            _ => unreachable!("non-workflow command routed to workflow executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_attachment(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let agent_root = self.memory_root_path.as_deref().ok_or_else(|| {
            FeatureHostError::Contract("attachment storage is unavailable".into())
        })?;
        match command {
            FeatureCommand::AttachmentUpload {
                agent_id,
                filename,
                mime_type,
                bytes_base64,
                ..
            } => {
                if !is_safe_memory_agent_id(&agent_id)
                    || !self.state()?.bots.contains_key(&agent_id)
                {
                    return Err(FeatureHostError::Contract(format!(
                        "unknown attachment owner: {agent_id}"
                    )));
                }
                let filename = filename.trim();
                if filename.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "attachment filename must not be empty".into(),
                    ));
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bytes_base64.trim())
                    .map_err(|error| {
                        FeatureHostError::Contract(format!("invalid attachment base64: {error}"))
                    })?;
                if bytes.is_empty() {
                    return Err(FeatureHostError::Contract("attachment is empty".into()));
                }
                let limit = attachment_byte_limit_for_name(filename);
                if bytes.len() as u64 > limit {
                    return Err(FeatureHostError::Contract(format!(
                        "attachment exceeds {} bytes",
                        limit
                    )));
                }
                let hash = format!("{:x}", Sha256::digest(&bytes));
                let extension = Path::new(filename)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.to_ascii_lowercase())
                    .filter(|extension| {
                        extension
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric())
                    })
                    .filter(|extension| !extension.is_empty())
                    .unwrap_or_else(|| "bin".into());
                let attachments_dir = agent_root.join(&agent_id).join("attachments");
                std::fs::create_dir_all(&attachments_dir).map_err(|error| {
                    FeatureHostError::Contract(format!("create attachment directory: {error}"))
                })?;
                let path = attachments_dir.join(format!("{hash}.{extension}"));
                if !path.exists() {
                    std::fs::write(&path, &bytes).map_err(|error| {
                        FeatureHostError::Contract(format!("write attachment: {error}"))
                    })?;
                }
                let attachment = AttachmentStored {
                    id: hash.clone(),
                    agent_id,
                    name: filename.to_string(),
                    path: path.to_string_lossy().to_string(),
                    mime_type: clean_optional_string(mime_type)
                        .or_else(|| media_mime_type(filename).map(str::to_string)),
                    size_bytes: bytes.len() as u64,
                    hash,
                };
                self.state()?.events.push_back(HostEvent::AttachmentStored {
                    timestamp: timestamp(),
                    attachment,
                });
            }
            FeatureCommand::AttachmentReadText { agent_id, path, .. } => {
                let resolved = resolve_agent_attachment_path(agent_root, &agent_id, &path)?;
                let metadata = std::fs::metadata(&resolved).map_err(|error| {
                    FeatureHostError::Contract(format!("read attachment metadata: {error}"))
                })?;
                let bytes = metadata.len();
                let preview = read_file_prefix(&resolved, ATTACHMENT_TEXT_PREVIEW_BYTE_CAP)?;
                let binary = !is_text_previewable_name(&resolved) || looks_like_binary(&preview);
                let result = AttachmentTextResult {
                    path: resolved.to_string_lossy().to_string(),
                    kind: if binary { "binary" } else { "text" }.into(),
                    text: (!binary).then(|| String::from_utf8_lossy(&preview).to_string()),
                    truncated: !binary && bytes > ATTACHMENT_TEXT_PREVIEW_BYTE_CAP as u64,
                    bytes,
                };
                self.state()?
                    .events
                    .push_back(HostEvent::AttachmentTextRead {
                        timestamp: timestamp(),
                        result,
                    });
            }
            FeatureCommand::AttachmentReadChunk {
                agent_id,
                path,
                offset,
                length,
                ..
            } => {
                let resolved = resolve_agent_attachment_path(agent_root, &agent_id, &path)?;
                let metadata = std::fs::metadata(&resolved).map_err(|error| {
                    FeatureHostError::Contract(format!("read attachment metadata: {error}"))
                })?;
                let total_size = metadata.len();
                let start = offset.min(total_size);
                let length = length
                    .min(ATTACHMENT_CHUNK_MAX_BYTES as u64)
                    .min(total_size - start);
                let bytes = read_file_range(&resolved, start, length as usize)?;
                let result = AttachmentChunkResult {
                    path: resolved.to_string_lossy().to_string(),
                    bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    total_size,
                    mime: media_mime_type(resolved.to_string_lossy().as_ref()).map(str::to_string),
                };
                self.state()?
                    .events
                    .push_back(HostEvent::AttachmentChunkRead {
                        timestamp: timestamp(),
                        result,
                    });
            }
            FeatureCommand::AttachmentReadImage { agent_id, path, .. } => {
                let resolved = resolve_agent_attachment_path(agent_root, &agent_id, &path)?;
                let mime = media_mime_type(resolved.to_string_lossy().as_ref())
                    .filter(|mime| mime.starts_with("image/"))
                    .ok_or_else(|| {
                        FeatureHostError::Contract("attachment is not a supported image".into())
                    })?;
                let bytes = std::fs::read(&resolved).map_err(|error| {
                    FeatureHostError::Contract(format!("read image attachment: {error}"))
                })?;
                if bytes.len() as u64 > ATTACHMENT_BYTE_LIMIT {
                    return Err(FeatureHostError::Contract(
                        "image attachment exceeds preview limit".into(),
                    ));
                }
                let (width, height) = image_dimensions(&bytes, mime);
                let result = AttachmentImageResult {
                    path: resolved.to_string_lossy().to_string(),
                    data_url: format!(
                        "data:{mime};base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ),
                    width,
                    height,
                };
                self.state()?
                    .events
                    .push_back(HostEvent::AttachmentImageRead {
                        timestamp: timestamp(),
                        result,
                    });
            }
            _ => unreachable!("non-attachment command routed to attachment executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_search(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        {
            let state = self.state()?;
            ensure_open(&state)?;
        }
        match command {
            FeatureCommand::SearchMessages { query, limit, .. } => {
                let query = query.trim().to_lowercase();
                let limit = limit.clamp(1, AGENT_CONTENT_SEARCH_MAX_RESULTS);
                let mut matches = Vec::new();
                if !query.is_empty() && self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    {
                        let conversations =
                            match self.runtime()?.execute(RuntimeCommand::ListConversations)? {
                                RuntimeResponse::Conversations { data } => data,
                                other => {
                                    return Err(unexpected_response(
                                        "search.messages.conversations",
                                        other,
                                    ));
                                }
                            };
                        let bots = self.state()?.bots.values().cloned().collect::<Vec<_>>();
                        let bot_by_conversation = bots
                            .iter()
                            .filter_map(|bot| {
                                bot.conversation_id
                                    .as_ref()
                                    .map(|conversation_id| (conversation_id.clone(), bot))
                            })
                            .collect::<BTreeMap<_, _>>();
                        for conversation in conversations {
                            let response =
                                self.runtime()?
                                    .execute(RuntimeCommand::ConversationHistory {
                                        conversation_id: conversation.id.clone(),
                                        limit: Some(2_000),
                                    });
                            let messages = match response {
                                Ok(RuntimeResponse::History { data }) => data,
                                _ => continue,
                            };
                            let bot = bot_by_conversation.get(&conversation.id.0).copied();
                            let agent_id = bot
                                .map(|bot| bot.id.clone())
                                .unwrap_or_else(|| conversation.id.0.clone());
                            let agent_name = bot
                                .map(|bot| bot.name.clone())
                                .unwrap_or_else(|| conversation.title.clone());
                            let mut per_agent = 0usize;
                            for message in messages.into_iter().rev() {
                                if per_agent >= AGENT_CONTENT_SEARCH_MAX_MATCHES_PER_AGENT {
                                    break;
                                }
                                let Some(snippet) = build_content_snippet(&message.text, &query)
                                else {
                                    continue;
                                };
                                let role = match message.role {
                                    RuntimeMessageRole::User => MessageRole::User,
                                    RuntimeMessageRole::Assistant
                                    | RuntimeMessageRole::Contact
                                    | RuntimeMessageRole::MiniApp
                                    | RuntimeMessageRole::System => MessageRole::Assistant,
                                };
                                matches.push(SearchMessageMatch {
                                    agent_id: agent_id.clone(),
                                    agent_name: agent_name.clone(),
                                    conversation_id: conversation.id.0.clone(),
                                    entry_id: message.id.to_string(),
                                    role,
                                    timestamp_ms: message.created_at_ms,
                                    snippet,
                                });
                                per_agent += 1;
                            }
                        }
                        matches.sort_by_key(|item| std::cmp::Reverse(item.timestamp_ms));
                        matches.truncate(limit);
                    }
                }
                self.state()?
                    .events
                    .push_back(HostEvent::SearchMessagesListed {
                        timestamp: timestamp(),
                        query,
                        matches,
                    });
            }
            FeatureCommand::SearchMedia { query, limit, .. } => {
                let query = query.trim().to_lowercase();
                let limit = limit.clamp(1, AGENT_CONTENT_SEARCH_MAX_RESULTS);
                let bots = self.state()?.bots.values().cloned().collect::<Vec<_>>();
                let mut matches = Vec::new();
                if let Some(agent_root) = self.memory_root_path.as_deref() {
                    for bot in bots {
                        collect_agent_media_matches(
                            &agent_root.join(&bot.id).join("attachments"),
                            &bot.id,
                            &bot.name,
                            &query,
                            &mut matches,
                        );
                    }
                }
                matches.sort_by_key(|item| std::cmp::Reverse(item.timestamp_ms));
                matches.truncate(limit);
                self.state()?
                    .events
                    .push_back(HostEvent::SearchMediaListed {
                        timestamp: timestamp(),
                        query,
                        matches,
                    });
            }
            _ => unreachable!("non-search command routed to search executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_mcp(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        {
            let state = self.state()?;
            ensure_open(&state)?;
        }
        match command {
            FeatureCommand::McpList { .. } => {
                let servers = if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    {
                        match self.runtime()?.execute(RuntimeCommand::McpServers)? {
                            RuntimeResponse::McpServers { data } => data,
                            other => return Err(unexpected_response("mcp.list", other)),
                        }
                    }
                    #[cfg(not(feature = "production"))]
                    Vec::new()
                } else {
                    Vec::new()
                };
                self.state()?.events.push_back(HostEvent::McpListed {
                    timestamp: timestamp(),
                    servers,
                });
            }
            FeatureCommand::McpApps { .. } => {
                let apps = if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    {
                        match self.runtime()?.execute(RuntimeCommand::McpApps)? {
                            RuntimeResponse::McpApps { data } => data,
                            other => return Err(unexpected_response("mcp.apps", other)),
                        }
                    }
                    #[cfg(not(feature = "production"))]
                    Vec::new()
                } else {
                    Vec::new()
                };
                self.state()?.events.push_back(HostEvent::McpAppsListed {
                    timestamp: timestamp(),
                    apps,
                });
            }
            FeatureCommand::McpOauthLogin { server, .. } => {
                let server = required(server, "MCP server")?;
                if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    {
                        let (server, authorization_url, removed) = match self
                            .runtime()?
                            .execute(RuntimeCommand::McpOauthLogin { server })?
                        {
                            RuntimeResponse::McpOauth {
                                server,
                                authorization_url,
                                removed,
                            } => (server, authorization_url, removed),
                            other => return Err(unexpected_response("mcp.oauthLogin", other)),
                        };
                        self.state()?.events.push_back(HostEvent::McpOauthChanged {
                            timestamp: timestamp(),
                            server,
                            authorization_url,
                            removed,
                        });
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                } else {
                    self.state()?.events.push_back(HostEvent::McpOauthChanged {
                        timestamp: timestamp(),
                        server,
                        authorization_url: Some("https://example.test/mcp-oauth".into()),
                        removed: false,
                    });
                }
            }
            FeatureCommand::McpOauthLogout { server, .. } => {
                let server = required(server, "MCP server")?;
                if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    {
                        let (server, authorization_url, removed) = match self
                            .runtime()?
                            .execute(RuntimeCommand::McpOauthLogout { server })?
                        {
                            RuntimeResponse::McpOauth {
                                server,
                                authorization_url,
                                removed,
                            } => (server, authorization_url, removed),
                            other => return Err(unexpected_response("mcp.oauthLogout", other)),
                        };
                        self.state()?.events.push_back(HostEvent::McpOauthChanged {
                            timestamp: timestamp(),
                            server,
                            authorization_url,
                            removed,
                        });
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                } else {
                    self.state()?.events.push_back(HostEvent::McpOauthChanged {
                        timestamp: timestamp(),
                        server,
                        authorization_url: None,
                        removed: true,
                    });
                }
            }
            FeatureCommand::McpRemove { server, .. } => {
                let server = required(server, "MCP server")?;
                if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    {
                        let removed = match self.runtime()?.execute(RuntimeCommand::McpRemove {
                            server: server.clone(),
                        })? {
                            RuntimeResponse::McpRemoved { removed, .. } => removed,
                            other => return Err(unexpected_response("mcp.remove", other)),
                        };
                        if !removed {
                            return Err(FeatureHostError::Contract(format!(
                                "MCP server is not a user-managed configuration: {server}"
                            )));
                        }
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                }
                self.state()?.events.push_back(HostEvent::McpRefreshed {
                    timestamp: timestamp(),
                });
            }
            FeatureCommand::McpSetCustomInstructions {
                server,
                instructions,
                ..
            } => {
                let server = required(server, "MCP server")?;
                let instructions = clamp_block(&instructions, 20_000);
                if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    match self
                        .runtime()?
                        .execute(RuntimeCommand::McpSetCustomInstructions {
                            server: server.clone(),
                            instructions,
                        })? {
                        RuntimeResponse::McpCustomInstructionsUpdated { .. } => {}
                        other => {
                            return Err(unexpected_response("mcp.setCustomInstructions", other));
                        }
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                }
                self.state()?.events.push_back(HostEvent::McpRefreshed {
                    timestamp: timestamp(),
                });
            }
            FeatureCommand::McpSetToolDisabled {
                server,
                tool,
                disabled,
                ..
            } => {
                let server = required(server, "MCP server")?;
                let tool = required(tool, "MCP tool")?;
                if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    match self
                        .runtime()?
                        .execute(RuntimeCommand::McpSetToolDisabled {
                            server: server.clone(),
                            tool,
                            disabled,
                        })? {
                        RuntimeResponse::McpToolDisabledUpdated { .. } => {}
                        other => return Err(unexpected_response("mcp.setToolDisabled", other)),
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                }
                self.state()?.events.push_back(HostEvent::McpRefreshed {
                    timestamp: timestamp(),
                });
            }
            FeatureCommand::McpRefresh { .. } => {
                if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    match self.runtime()?.execute(RuntimeCommand::McpRefresh)? {
                        RuntimeResponse::McpRefreshed => {}
                        other => return Err(unexpected_response("mcp.refresh", other)),
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                }
                self.state()?.events.push_back(HostEvent::McpRefreshed {
                    timestamp: timestamp(),
                });
            }
            FeatureCommand::McpToolCall {
                server,
                tool,
                arguments,
                ..
            } => {
                let server = required(server, "MCP server")?;
                let tool = required(tool, "MCP tool")?;
                let result = if self.config.mode == HostMode::Production {
                    #[cfg(feature = "production")]
                    {
                        match self.runtime()?.execute(RuntimeCommand::McpToolCall {
                            server: server.clone(),
                            tool: tool.clone(),
                            arguments,
                        })? {
                            RuntimeResponse::McpToolResult { result, .. } => result,
                            other => return Err(unexpected_response("mcp.toolCall", other)),
                        }
                    }
                    #[cfg(not(feature = "production"))]
                    Value::Null
                } else {
                    json!({"ok": true, "mock": true})
                };
                let _ = self.append_action_audit(
                    "mahayana-assistant",
                    None,
                    json!({
                        "kind": "mcpToolCall",
                        "serverIdentifier": server,
                        "toolName": tool,
                        "transport": "runtime",
                        "status": "success",
                    }),
                );
                self.state()?.events.push_back(HostEvent::McpToolResult {
                    timestamp: timestamp(),
                    server,
                    tool,
                    result,
                });
            }
            _ => unreachable!("non-MCP command routed to MCP executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn execute_settings_and_audit(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        match command {
            FeatureCommand::SettingsGet { .. } => {
                let settings = self.state()?.settings.clone();
                self.state()?.events.push_back(HostEvent::SettingsChanged {
                    timestamp: timestamp(),
                    settings,
                });
            }
            FeatureCommand::SettingsUpdate { mut settings, .. } => {
                settings.auto_review_rules = sanitize_auto_review_rules(settings.auto_review_rules);
                {
                    let mut state = self.state()?;
                    ensure_open(&state)?;
                    state.settings = settings.clone();
                }
                if let Some(path) = self.settings_path.as_deref() {
                    persist_product_host_settings(path, &settings)?;
                }
                sync_computer_control_policy(&settings);
                if !settings.remote_control_enabled {
                    self.state()?.remote_computer_sessions.clear();
                }
                self.state()?.events.push_back(HostEvent::SettingsChanged {
                    timestamp: timestamp(),
                    settings,
                });
            }
            FeatureCommand::AuditList {
                agent_id, limit, ..
            } => {
                if !is_safe_memory_agent_id(&agent_id)
                    || !self.state()?.bots.contains_key(&agent_id)
                {
                    return Err(FeatureHostError::Contract(format!(
                        "unknown audit agent: {agent_id}"
                    )));
                }
                let records = self
                    .memory_root_path
                    .as_deref()
                    .map(|root| {
                        read_action_audit(
                            &root.join(&agent_id).join("audit.jsonl"),
                            limit.min(1000),
                        )
                    })
                    .unwrap_or_default();
                self.state()?.events.push_back(HostEvent::AuditListed {
                    timestamp: timestamp(),
                    agent_id,
                    records,
                });
            }
            _ => unreachable!("non-settings command routed to settings executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    fn append_action_audit(
        &self,
        agent_id: &str,
        turn_id: Option<&str>,
        action: Value,
    ) -> Result<(), FeatureHostError> {
        if !is_safe_memory_agent_id(agent_id) {
            return Ok(());
        }
        let Some(root) = self.memory_root_path.as_deref() else {
            return Ok(());
        };
        let path = root.join(agent_id).join("audit.jsonl");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                FeatureHostError::Contract(format!("create action audit directory: {error}"))
            })?;
        }
        let record = json!({
            "ts": timestamp(),
            "agentId": agent_id,
            "eventId": format!("audit-{}-{}", now_millis(), std::process::id()),
            "turnId": turn_id.unwrap_or(""),
            "action": action,
        });
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| FeatureHostError::Contract(format!("open action audit: {error}")))?;
        writeln!(file, "{record}")
            .map_err(|error| FeatureHostError::Contract(format!("append action audit: {error}")))
    }

    fn execute_product_surface(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => self.execute_product_surface_test(command),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self.execute_product_surface_production(command);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    fn execute_product_surface_test(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let mut state = self.state()?;
        ensure_open(&state)?;
        match command {
            FeatureCommand::ConnectorList { .. } => {
                let connectors = state.connectors.values().cloned().collect();
                state.events.push_back(HostEvent::ConnectorListed {
                    timestamp: timestamp(),
                    connectors,
                });
            }
            FeatureCommand::ConnectorConnect {
                connector_id,
                account_label,
                ..
            } => {
                let connector_id = required(connector_id, "connectorId")?;
                let account_id = next_id(&mut state, "account");
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                connector.accounts.push(ConnectorAccountSummary {
                    id: account_id,
                    label: account_label
                        .clone()
                        .filter(|label| !label.trim().is_empty())
                        .unwrap_or_else(|| "Personal".into()),
                    status: ConnectorStatus::Connected,
                    email: None,
                    team_managed: Some(false),
                    error: None,
                });
                connector.status = ConnectorStatus::Connected;
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "connected".into(),
                    connector,
                });
                if let Some(platform) = listener_platform_for_connector(&connector_id) {
                    if let Some(integration) = state.listeners.get_mut(&platform) {
                        integration.is_connected = true;
                        integration.account_label = account_label;
                        let integration = integration.clone();
                        state.events.push_back(HostEvent::ListenerChanged {
                            timestamp: timestamp(),
                            integration,
                        });
                    }
                }
            }
            FeatureCommand::ConnectorRenameAccount {
                connector_id,
                account_id,
                label,
                ..
            } => {
                let label = required(label, "account label")?;
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                let account = connector
                    .accounts
                    .iter_mut()
                    .find(|account| account.id == account_id)
                    .ok_or_else(|| {
                        FeatureHostError::Contract(format!(
                            "unknown connector account: {account_id}"
                        ))
                    })?;
                if account.team_managed == Some(true) {
                    return Err(FeatureHostError::Contract(
                        "team-managed accounts cannot be renamed".into(),
                    ));
                }
                account.label = label;
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "updated".into(),
                    connector,
                });
            }
            FeatureCommand::ConnectorRemoveAccount {
                connector_id,
                account_id,
                ..
            } => {
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                if connector
                    .accounts
                    .iter()
                    .any(|account| account.id == account_id && account.team_managed == Some(true))
                {
                    return Err(FeatureHostError::Contract(
                        "team-managed accounts cannot be removed".into(),
                    ));
                }
                let before = connector.accounts.len();
                connector
                    .accounts
                    .retain(|account| account.id != account_id);
                if before == connector.accounts.len() {
                    return Err(FeatureHostError::Contract(format!(
                        "unknown connector account: {account_id}"
                    )));
                }
                if connector.accounts.is_empty() {
                    connector.status = ConnectorStatus::Disconnected;
                }
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "removed".into(),
                    connector,
                });
            }
            FeatureCommand::ConnectorSetToolEnabled {
                connector_id,
                tool_id,
                enabled,
                ..
            } => {
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                let tool = connector
                    .tools
                    .iter_mut()
                    .find(|tool| tool.id == tool_id)
                    .ok_or_else(|| {
                        FeatureHostError::Contract(format!("unknown connector tool: {tool_id}"))
                    })?;
                tool.enabled = enabled;
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "toolChanged".into(),
                    connector,
                });
            }
            FeatureCommand::SkillList { agent_id, .. } => {
                let skills = state
                    .skills
                    .values()
                    .filter(|skill| {
                        agent_id
                            .as_ref()
                            .is_none_or(|agent_id| skill.owner_agent_id.as_ref() == Some(agent_id))
                    })
                    .cloned()
                    .collect();
                state.events.push_back(HostEvent::SkillListed {
                    timestamp: timestamp(),
                    skills,
                    teams: default_skill_teams(),
                });
            }
            FeatureCommand::SkillUpsert {
                id,
                name,
                description,
                use_when,
                instructions,
                owner_agent_id,
                ..
            } => {
                let name = required(name, "skill name")?;
                let use_when = required(use_when, "skill useWhen")?;
                let instructions = required(instructions, "skill instructions")?;
                let id = id.unwrap_or_else(|| next_id(&mut state, "skill"));
                let action = if state.skills.contains_key(&id) {
                    "updated"
                } else {
                    "created"
                };
                let previous = state.skills.get(&id);
                if previous.is_some_and(|skill| skill.read_only == Some(true)) {
                    return Err(FeatureHostError::Contract(
                        "managed skills cannot be edited".into(),
                    ));
                }
                let skill = SkillSummary {
                    id: id.clone(),
                    name,
                    description,
                    use_when,
                    instructions,
                    source: previous.map_or(SkillSource::Private, |skill| skill.source),
                    publish_state: previous
                        .map_or(SkillPublishState::Local, |skill| skill.publish_state),
                    owner_agent_id,
                    team_id: previous.and_then(|skill| skill.team_id.clone()),
                    team_name: previous.and_then(|skill| skill.team_name.clone()),
                    read_only: previous.and_then(|skill| skill.read_only),
                    updated_at_ms: now_millis(),
                };
                state.skills.insert(id, skill.clone());
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: action.into(),
                    skill,
                });
            }
            FeatureCommand::SkillDelete { id, .. } => {
                let skill = state
                    .skills
                    .get(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.read_only == Some(true) || skill.source == SkillSource::Team {
                    return Err(FeatureHostError::Contract(
                        "team-managed skills cannot be deleted".into(),
                    ));
                }
                let skill = state.skills.remove(&id).expect("skill checked above");
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "deleted".into(),
                    skill,
                });
            }
            FeatureCommand::SkillPublish { id, team_id, .. } => {
                let team = default_skill_teams()
                    .into_iter()
                    .find(|team| team.id == team_id)
                    .ok_or_else(|| {
                        FeatureHostError::Contract(format!("unknown skill team: {team_id}"))
                    })?;
                let skill = state
                    .skills
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.description.trim().is_empty() {
                    return Err(FeatureHostError::Contract(
                        "add a skill description before publishing".into(),
                    ));
                }
                skill.source = SkillSource::Team;
                skill.publish_state = SkillPublishState::Published;
                skill.team_id = Some(team.id);
                skill.team_name = Some(team.name);
                skill.updated_at_ms = now_millis();
                let skill = skill.clone();
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "published".into(),
                    skill,
                });
            }
            FeatureCommand::SkillUnpublish { id, .. } => {
                let skill = state
                    .skills
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.publish_state == SkillPublishState::Managed {
                    return Err(FeatureHostError::Contract(
                        "managed skills cannot be unpublished".into(),
                    ));
                }
                skill.source = SkillSource::Private;
                skill.publish_state = SkillPublishState::Local;
                skill.team_id = None;
                skill.team_name = None;
                skill.updated_at_ms = now_millis();
                let skill = skill.clone();
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "unpublished".into(),
                    skill,
                });
            }
            FeatureCommand::SkillSync { id, .. } => {
                let skill = state
                    .skills
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.team_id.is_none() {
                    return Err(FeatureHostError::Contract(
                        "only published skills can be synced".into(),
                    ));
                }
                skill.publish_state = SkillPublishState::Synced;
                skill.updated_at_ms = now_millis();
                let skill = skill.clone();
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "synced".into(),
                    skill,
                });
            }
            FeatureCommand::BotList { .. } => {
                let bots = state.bots.values().cloned().collect();
                state.events.push_back(HostEvent::BotListed {
                    timestamp: timestamp(),
                    bots,
                });
            }
            FeatureCommand::BotSetHidden { id, hidden, .. } => {
                let bot = state
                    .bots
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown bot: {id}")))?;
                bot.hidden = hidden;
                let bot = bot.clone();
                state.events.push_back(HostEvent::BotChanged {
                    timestamp: timestamp(),
                    action: "updated".into(),
                    bot,
                });
            }
            FeatureCommand::DraftResolve { draft, action, .. } => {
                let draft_id = draft.id().to_string();
                match action {
                    DraftAction::Discard => {
                        state.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Discarded,
                            error: None,
                        });
                    }
                    DraftAction::Send => {
                        validate_draft(&draft)?;
                        state.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id: draft_id.clone(),
                            status: DraftSendState::Sending,
                            error: None,
                        });
                        state.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Sent,
                            error: None,
                        });
                    }
                }
            }
            FeatureCommand::SecretProvide {
                secret_request_id,
                value,
                ..
            } => {
                if value.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "secret value must not be empty".into(),
                    ));
                }
                state.events.push_back(HostEvent::SecretProvided {
                    timestamp: timestamp(),
                    secret_request_id,
                });
            }
            FeatureCommand::ListenerList { .. } => {
                let integrations = state.listeners.values().cloned().collect();
                state.events.push_back(HostEvent::ListenerListed {
                    timestamp: timestamp(),
                    integrations,
                });
            }
            FeatureCommand::ListenerConnect { platform, .. } => {
                let integration = state.listeners.get_mut(&platform).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "unsupported listener platform: {platform:?}"
                    ))
                })?;
                integration.is_connected = true;
                integration.error = None;
                let integration = integration.clone();
                state.events.push_back(HostEvent::ListenerChanged {
                    timestamp: timestamp(),
                    integration,
                });
                if let Some(connector_id) = connector_for_listener_platform(platform) {
                    if let Some(connector) = state.connectors.get_mut(connector_id) {
                        connector.status = ConnectorStatus::Connected;
                        let connector = connector.clone();
                        state.events.push_back(HostEvent::ConnectorChanged {
                            timestamp: timestamp(),
                            action: "connected".into(),
                            connector,
                        });
                    }
                }
            }
            FeatureCommand::ListenerDisconnect { platform, .. } => {
                let integration = state.listeners.get_mut(&platform).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "unsupported listener platform: {platform:?}"
                    ))
                })?;
                integration.is_connected = false;
                integration.account_label = None;
                integration.error = None;
                let integration = integration.clone();
                state.events.push_back(HostEvent::ListenerChanged {
                    timestamp: timestamp(),
                    integration,
                });
                if let Some(connector_id) = connector_for_listener_platform(platform) {
                    if let Some(connector) = state.connectors.get_mut(connector_id) {
                        connector.status = ConnectorStatus::Disconnected;
                        connector.accounts.clear();
                        let connector = connector.clone();
                        state.events.push_back(HostEvent::ConnectorChanged {
                            timestamp: timestamp(),
                            action: "disconnected".into(),
                            connector,
                        });
                    }
                }
            }
            FeatureCommand::UpdateStatus { .. } => {
                let update_state = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: update_state,
                });
            }
            FeatureCommand::UpdateCheck { .. } => {
                state.update_state = UpdateState::Checking;
                let checking = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: checking,
                });
                state.update_state = UpdateState::UpToDate {
                    version: env!("CARGO_PKG_VERSION").into(),
                };
                let update_state = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: update_state,
                });
            }
            FeatureCommand::UpdateInstall { .. } => {
                let version = match &state.update_state {
                    UpdateState::Available { version, .. }
                    | UpdateState::Ready { version }
                    | UpdateState::Downloading { version, .. }
                    | UpdateState::Staging { version } => version.clone(),
                    _ => {
                        return Err(FeatureHostError::Contract(
                            "no update is available to install".into(),
                        ));
                    }
                };
                state.update_state = UpdateState::Downloading {
                    version: version.clone(),
                    progress: Some(100),
                };
                let downloading = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: downloading,
                });
                state.update_state = UpdateState::Ready { version };
                let ready = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: ready,
                });
            }
            _ => unreachable!("non-product-surface command routed to product executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_live_connector_sources(
        &self,
    ) -> Result<(Vec<Value>, Vec<Value>), FeatureHostError> {
        let servers = match self.runtime()?.execute(RuntimeCommand::McpServers)? {
            RuntimeResponse::McpServers { data } => data,
            other => return Err(unexpected_response("mahayana.mcp.servers", other)),
        };
        let apps = match self.runtime()?.execute(RuntimeCommand::McpApps) {
            Ok(RuntimeResponse::McpApps { data }) => data,
            Ok(other) => return Err(unexpected_response("mahayana.mcp.apps", other)),
            // Connector directory discovery is feature-gated in some Codex
            // builds. Live MCP tool metadata is still authoritative for linked
            // connectors, so an unavailable directory should not hide them.
            Err(_) => Vec::new(),
        };
        Ok((servers, apps))
    }

    #[cfg(feature = "production")]
    fn production_connector_snapshot(
        &self,
    ) -> Result<
        (
            Vec<ConnectorSummary>,
            BTreeMap<String, LiveConnectorProjection>,
        ),
        FeatureHostError,
    > {
        let payload = json!({"type": "connector.list", "requestId": "connector-snapshot"});
        let response = self
            .runtime()?
            .product_execute("mahayana.connector.list", &payload)?;
        let base: Vec<ConnectorSummary> =
            decode_product_field(response, "connectors", "mahayana.connector.list")?;
        let (servers, apps) = self.production_live_connector_sources()?;
        let live = live_connector_projections(&servers, &apps);
        Ok((merge_live_connectors(base, &live), live))
    }

    #[cfg(feature = "production")]
    fn emit_connector_snapshot_change(
        &self,
        connector_id: &str,
        action: &str,
    ) -> Result<(), FeatureHostError> {
        let (connectors, _) = self.production_connector_snapshot()?;
        let connector = connectors
            .into_iter()
            .find(|connector| connector.id == connector_id)
            .ok_or_else(|| {
                FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
            })?;
        self.state()?.events.push_back(HostEvent::ConnectorChanged {
            timestamp: timestamp(),
            action: action.into(),
            connector,
        });
        Ok(())
    }

    #[cfg(feature = "production")]
    fn execute_live_product_surface_production(
        &self,
        command: &FeatureCommand,
    ) -> Result<Option<CommandAccepted>, FeatureHostError> {
        let request_id = command.request_id().to_string();
        match command {
            FeatureCommand::ConnectorList { .. } => {
                let (connectors, _) = self.production_connector_snapshot()?;
                self.state()?.events.push_back(HostEvent::ConnectorListed {
                    timestamp: timestamp(),
                    connectors,
                });
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ConnectorConnect {
                connector_id,
                account_label: _,
                ..
            } => {
                if connector_id == "git" {
                    let payload = serde_json::to_value(command).map_err(|error| {
                        FeatureHostError::Contract(format!("encode connector.connect: {error}"))
                    })?;
                    self.runtime()?
                        .product_execute("mahayana.connector.connect", &payload)?;
                    self.emit_connector_snapshot_change(connector_id, "connected")?;
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let (_, live) = self.production_connector_snapshot()?;
                let projection = live.get(connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} is not installed or discoverable in the Codex connector runtime; add its plugin from Plugins first"
                    ))
                })?;
                if let Some(url) = projection.install_url.clone() {
                    if !url.starts_with("https://") {
                        return Err(FeatureHostError::Contract(
                            "connector authorization URL must use HTTPS".into(),
                        ));
                    }
                    self.state()?
                        .events
                        .push_back(HostEvent::ConnectorOauthRequested {
                            timestamp: timestamp(),
                            connector_id: connector_id.clone(),
                            authorization_url: url,
                        });
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let server = projection.server_name.clone().ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} does not expose an OAuth-capable MCP server"
                    ))
                })?;
                if projection.status == Some(ConnectorStatus::Connected) {
                    self.emit_connector_snapshot_change(connector_id, "connected")?;
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let authorization_url =
                    match self.runtime()?.execute(RuntimeCommand::McpOauthLogin {
                        server: server.clone(),
                    })? {
                        RuntimeResponse::McpOauth {
                            authorization_url: Some(url),
                            ..
                        } => url,
                        other => {
                            return Err(unexpected_response("mahayana.mcp.oauth.login", other));
                        }
                    };
                self.state()?
                    .events
                    .push_back(HostEvent::ConnectorOauthRequested {
                        timestamp: timestamp(),
                        connector_id: connector_id.clone(),
                        authorization_url,
                    });
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ConnectorRenameAccount { connector_id, .. }
            | FeatureCommand::ConnectorSetToolEnabled { connector_id, .. } => {
                let method = product_surface_method(command);
                let payload = serde_json::to_value(command).map_err(|error| {
                    FeatureHostError::Contract(format!("encode {method}: {error}"))
                })?;
                self.runtime()?.product_execute(method, &payload)?;
                self.emit_connector_snapshot_change(
                    connector_id,
                    if matches!(command, FeatureCommand::ConnectorSetToolEnabled { .. }) {
                        "toolChanged"
                    } else {
                        "updated"
                    },
                )?;
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ConnectorRemoveAccount {
                connector_id,
                account_id,
                ..
            } => {
                let (_, live) = self.production_connector_snapshot()?;
                if let Some(projection) = live.get(connector_id) {
                    if projection.server_name.as_deref() == Some("codex_apps") {
                        if let Some(url) = projection.install_url.clone() {
                            self.state()?
                                .events
                                .push_back(HostEvent::ConnectorOauthRequested {
                                    timestamp: timestamp(),
                                    connector_id: connector_id.clone(),
                                    authorization_url: url,
                                });
                        }
                        return Err(FeatureHostError::Contract(
                            "this linked ChatGPT App account is server-managed; its account page was opened because the current Codex connector API does not expose unlink"
                                .into(),
                        ));
                    }
                    if let Some(server) = projection.server_name.clone() {
                        match self
                            .runtime()?
                            .execute(RuntimeCommand::McpOauthLogout { server })?
                        {
                            RuntimeResponse::McpOauth { .. } => {}
                            other => {
                                return Err(unexpected_response(
                                    "mahayana.mcp.oauth.logout",
                                    other,
                                ));
                            }
                        }
                    }
                }
                let method = product_surface_method(command);
                let payload = serde_json::to_value(command).map_err(|error| {
                    FeatureHostError::Contract(format!("encode {method}: {error}"))
                })?;
                self.runtime()?.product_execute(method, &payload)?;
                let _ = account_id;
                self.emit_connector_snapshot_change(connector_id, "removed")?;
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::DraftResolve { draft, action, .. } => {
                let draft_id = draft.id().to_string();
                if *action == DraftAction::Discard {
                    self.state()?.events.push_back(HostEvent::DraftChanged {
                        timestamp: timestamp(),
                        draft_id,
                        status: DraftSendState::Discarded,
                        error: None,
                    });
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                validate_draft(draft)?;
                let connector_id = match draft {
                    MessageDraft::Email { .. } => "gmail",
                    MessageDraft::Slack { .. } => "slack",
                };
                let (_, live) = self.production_connector_snapshot()?;
                let projection = live.get(connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} connector is not installed; install and authorize it before sending"
                    ))
                })?;
                if projection.status != Some(ConnectorStatus::Connected) {
                    return Err(FeatureHostError::Contract(format!(
                        "{connector_id} connector is not authorized"
                    )));
                }
                let server = projection.server_name.clone().ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} connector does not expose an MCP server"
                    ))
                })?;
                let (tool, schema) =
                    projection_send_tool(projection, connector_id).ok_or_else(|| {
                        FeatureHostError::Contract(format!(
                            "{connector_id} connector has no compatible send tool"
                        ))
                    })?;
                let arguments = draft_tool_arguments(draft, schema)?;
                self.state()?.events.push_back(HostEvent::DraftChanged {
                    timestamp: timestamp(),
                    draft_id: draft_id.clone(),
                    status: DraftSendState::Sending,
                    error: None,
                });
                let result = self.runtime()?.execute(RuntimeCommand::McpToolCall {
                    server,
                    tool: tool.to_string(),
                    arguments,
                });
                match result {
                    Ok(RuntimeResponse::McpToolResult { .. }) => {
                        self.state()?.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Sent,
                            error: None,
                        });
                        Ok(Some(CommandAccepted {
                            request_id,
                            operation_id: None,
                        }))
                    }
                    Ok(other) => Err(unexpected_response("mahayana.mcp.tool.call", other)),
                    Err(error) => {
                        let message = error.to_string();
                        self.state()?.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Failed,
                            error: Some(message.clone()),
                        });
                        Err(error.into())
                    }
                }
            }
            FeatureCommand::ListenerList { .. } => {
                let payload = serde_json::to_value(command).map_err(|error| {
                    FeatureHostError::Contract(format!("encode listener.list: {error}"))
                })?;
                let response = self
                    .runtime()?
                    .product_execute("mahayana.listener.list", &payload)?;
                let mut integrations: Vec<ListenerIntegrationSummary> =
                    decode_product_field(response, "integrations", "mahayana.listener.list")?;
                let (connectors, _) = self.production_connector_snapshot()?;
                for integration in &mut integrations {
                    if integration.platform == ListenerPlatform::Git {
                        continue;
                    }
                    if let Some(connector_id) =
                        connector_for_listener_platform(integration.platform)
                    {
                        if let Some(connector) = connectors
                            .iter()
                            .find(|connector| connector.id == connector_id)
                        {
                            integration.is_connected =
                                connector.status == ConnectorStatus::Connected;
                            integration.account_label = connector
                                .accounts
                                .first()
                                .map(|account| account.label.clone());
                        }
                    }
                }
                self.state()?.events.push_back(HostEvent::ListenerListed {
                    timestamp: timestamp(),
                    integrations,
                });
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ListenerConnect { platform, .. } => {
                if *platform == ListenerPlatform::Git {
                    let payload = serde_json::to_value(command).map_err(|error| {
                        FeatureHostError::Contract(format!("encode listener.connect: {error}"))
                    })?;
                    let response = self
                        .runtime()?
                        .product_execute("mahayana.listener.connect", &payload)?;
                    let integration =
                        decode_product_field(response, "integration", "mahayana.listener.connect")?;
                    self.state()?.events.push_back(HostEvent::ListenerChanged {
                        timestamp: timestamp(),
                        integration,
                    });
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let connector_id = connector_for_listener_platform(*platform).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "no connector exists for listener platform {platform:?}"
                    ))
                })?;
                let synthetic = FeatureCommand::ConnectorConnect {
                    request_id: request_id.clone(),
                    connector_id: connector_id.into(),
                    account_label: None,
                };
                self.execute_live_product_surface_production(&synthetic)?;
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ListenerDisconnect { platform, .. } => {
                if *platform != ListenerPlatform::Git {
                    let connector_id =
                        connector_for_listener_platform(*platform).ok_or_else(|| {
                            FeatureHostError::Contract(format!(
                                "no connector exists for listener platform {platform:?}"
                            ))
                        })?;
                    let (connectors, live) = self.production_connector_snapshot()?;
                    let accounts = connectors
                        .iter()
                        .find(|connector| connector.id == connector_id)
                        .map(|connector| connector.accounts.clone())
                        .unwrap_or_default();
                    if accounts.is_empty() {
                        if let Some(projection) = live.get(connector_id) {
                            if projection.status == Some(ConnectorStatus::Connected) {
                                if let Some(server) = projection.server_name.clone() {
                                    match self
                                        .runtime()?
                                        .execute(RuntimeCommand::McpOauthLogout { server })?
                                    {
                                        RuntimeResponse::McpOauth { .. } => {}
                                        other => {
                                            return Err(unexpected_response(
                                                "mahayana.mcp.oauth.logout",
                                                other,
                                            ));
                                        }
                                    }
                                    self.emit_connector_snapshot_change(connector_id, "removed")?;
                                }
                            }
                        }
                    } else {
                        for account in accounts {
                            let synthetic = FeatureCommand::ConnectorRemoveAccount {
                                request_id: format!("{request_id}:{}", account.id),
                                connector_id: connector_id.into(),
                                account_id: account.id,
                            };
                            self.execute_live_product_surface_production(&synthetic)?;
                        }
                    }
                }
                let payload = serde_json::to_value(command).map_err(|error| {
                    FeatureHostError::Contract(format!("encode listener.disconnect: {error}"))
                })?;
                let response = self
                    .runtime()?
                    .product_execute("mahayana.listener.disconnect", &payload)?;
                let integration =
                    decode_product_field(response, "integration", "mahayana.listener.disconnect")?;
                self.state()?.events.push_back(HostEvent::ListenerChanged {
                    timestamp: timestamp(),
                    integration,
                });
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            _ => Ok(None),
        }
    }

    #[cfg(feature = "production")]
    fn execute_product_surface_production(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        if let Some(accepted) = self.execute_live_product_surface_production(&command)? {
            return Ok(accepted);
        }
        let request_id = command.request_id().to_string();
        let method = product_surface_method(&command);
        let payload = serde_json::to_value(&command).map_err(|error| {
            FeatureHostError::Contract(format!("encode {method} payload: {error}"))
        })?;

        if let FeatureCommand::DraftResolve {
            draft,
            action: DraftAction::Send,
            ..
        } = &command
        {
            validate_draft(draft)?;
            self.state()?.events.push_back(HostEvent::DraftChanged {
                timestamp: timestamp(),
                draft_id: draft.id().to_string(),
                status: DraftSendState::Sending,
                error: None,
            });
        }

        let response = self
            .runtime()?
            .product_execute(method, &payload)
            .map_err(FeatureHostError::from)?;
        let mut state = self.state()?;
        match command {
            FeatureCommand::ConnectorList { .. } => {
                let connectors = decode_product_field(response, "connectors", method)?;
                state.events.push_back(HostEvent::ConnectorListed {
                    timestamp: timestamp(),
                    connectors,
                });
            }
            FeatureCommand::ConnectorConnect { connector_id, .. } => {
                if let Some(url) = response
                    .get("authorizationUrl")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                {
                    state.events.push_back(HostEvent::ConnectorOauthRequested {
                        timestamp: timestamp(),
                        connector_id,
                        authorization_url: url,
                    });
                }
                if response.get("connector").is_some() || response.get("id").is_some() {
                    let connector = decode_product_field(response, "connector", method)?;
                    state.events.push_back(HostEvent::ConnectorChanged {
                        timestamp: timestamp(),
                        action: "connected".into(),
                        connector,
                    });
                }
            }
            FeatureCommand::ConnectorRenameAccount { .. }
            | FeatureCommand::ConnectorRemoveAccount { .. }
            | FeatureCommand::ConnectorSetToolEnabled { .. } => {
                let action = match command {
                    FeatureCommand::ConnectorRemoveAccount { .. } => "removed",
                    FeatureCommand::ConnectorSetToolEnabled { .. } => "toolChanged",
                    _ => "updated",
                };
                let connector = decode_product_field(response, "connector", method)?;
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: action.into(),
                    connector,
                });
            }
            FeatureCommand::SkillList { .. } => {
                let skills = decode_product_field(response.clone(), "skills", method)?;
                let teams = response
                    .get("teams")
                    .cloned()
                    .map(|value| decode_product_value(value, method))
                    .transpose()?
                    .unwrap_or_default();
                state.events.push_back(HostEvent::SkillListed {
                    timestamp: timestamp(),
                    skills,
                    teams,
                });
            }
            FeatureCommand::SkillUpsert { .. }
            | FeatureCommand::SkillDelete { .. }
            | FeatureCommand::SkillPublish { .. }
            | FeatureCommand::SkillUnpublish { .. }
            | FeatureCommand::SkillSync { .. } => {
                let action = match command {
                    FeatureCommand::SkillDelete { .. } => "deleted".to_string(),
                    FeatureCommand::SkillPublish { .. } => "published".to_string(),
                    FeatureCommand::SkillUnpublish { .. } => "unpublished".to_string(),
                    FeatureCommand::SkillSync { .. } => "synced".to_string(),
                    _ => response
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("updated")
                        .to_string(),
                };
                let skill = decode_product_field(response, "skill", method)?;
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action,
                    skill,
                });
            }
            FeatureCommand::BotList { .. } => {
                let mut bots: Vec<BotSummary> = decode_product_field(response, "bots", method)?;
                let mut known = bots
                    .iter()
                    .map(|bot| bot.id.clone())
                    .collect::<BTreeSet<_>>();
                for bot in state.bots.values() {
                    if known.insert(bot.id.clone()) {
                        bots.push(bot.clone());
                    }
                }
                state.events.push_back(HostEvent::BotListed {
                    timestamp: timestamp(),
                    bots,
                });
            }
            FeatureCommand::BotSetHidden { .. } => {
                let bot = decode_product_field(response, "bot", method)?;
                state.events.push_back(HostEvent::BotChanged {
                    timestamp: timestamp(),
                    action: "updated".into(),
                    bot,
                });
            }
            FeatureCommand::DraftResolve { draft, action, .. } => {
                let status = response
                    .get("status")
                    .cloned()
                    .map(|value| decode_product_value(value, method))
                    .transpose()?
                    .unwrap_or(match action {
                        DraftAction::Send => DraftSendState::Sent,
                        DraftAction::Discard => DraftSendState::Discarded,
                    });
                state.events.push_back(HostEvent::DraftChanged {
                    timestamp: timestamp(),
                    draft_id: draft.id().to_string(),
                    status,
                    error: response
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }
            FeatureCommand::SecretProvide {
                secret_request_id, ..
            } => {
                state.events.push_back(HostEvent::SecretProvided {
                    timestamp: timestamp(),
                    secret_request_id,
                });
            }
            FeatureCommand::ListenerList { .. } => {
                let integrations = decode_product_field(response, "integrations", method)?;
                state.events.push_back(HostEvent::ListenerListed {
                    timestamp: timestamp(),
                    integrations,
                });
            }
            FeatureCommand::ListenerConnect { .. } | FeatureCommand::ListenerDisconnect { .. } => {
                let integration = decode_product_field(response, "integration", method)?;
                state.events.push_back(HostEvent::ListenerChanged {
                    timestamp: timestamp(),
                    integration,
                });
            }
            FeatureCommand::UpdateStatus { .. }
            | FeatureCommand::UpdateCheck { .. }
            | FeatureCommand::UpdateInstall { .. } => {
                let update_state = decode_product_field(response, "state", method)?;
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: update_state,
                });
            }
            _ => unreachable!("non-product-surface command routed to product executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    /// Delivers a verified listener event into the automation engine. This is
    /// intentionally not a renderer command: only the backend relay/native
    /// listener bridge may call it, so web content cannot forge trigger events.
    pub fn ingest_listener_event(&self, event: EventCard) -> Result<usize, FeatureHostError> {
        let serialized = serde_json::to_string(&event).map_err(|error| {
            FeatureHostError::Contract(format!("encode listener event: {error}"))
        })?;
        let matching_ids = {
            let state = self.state()?;
            state
                .automations
                .values()
                .filter(|automation| {
                    if !automation.enabled {
                        return false;
                    }
                    let Some(AutomationTrigger::Event {
                        source,
                        event: expected_event,
                        filter,
                    }) = automation.trigger.as_ref()
                    else {
                        return false;
                    };
                    *source == event.source
                        && (expected_event == "*" || expected_event == &event.event)
                        && filter.as_ref().is_none_or(|filter| {
                            serialized
                                .to_ascii_lowercase()
                                .contains(&filter.to_ascii_lowercase())
                        })
                })
                .map(|automation| automation.id.clone())
                .collect::<Vec<_>>()
        };

        for id in &matching_ids {
            let automation = {
                let mut state = self.state()?;
                let automation = state.automations.get_mut(id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown automation: {id}"))
                })?;
                automation.last_run_at_ms = Some(event.occurred_at_ms.unwrap_or_else(now_millis));
                let automation = automation.clone();
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: "triggered".into(),
                    automation: automation.clone(),
                });
                state.events.push_back(HostEvent::TranscriptCard {
                    timestamp: timestamp(),
                    entry_id: format!("relay-{}-{}", automation.id, now_millis()),
                    operation_id: None,
                    card: TranscriptCard::Event {
                        event: event.clone(),
                    },
                });
                automation
            };
            let details = event
                .fields
                .as_ref()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|field| format!("{}: {}", field.label, field.value))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let text = format!(
                "[事件自动化：{}]\n来源：{}\n事件：{}\n标题：{}\n摘要：{}{}\n\n这是用户保存的 standing instruction。请基于上面的真实事件立即执行并报告结果。\n\n{}",
                automation.name,
                listener_platform_display(event.source),
                event.event,
                event.title,
                event.summary,
                if details.is_empty() {
                    String::new()
                } else {
                    format!("\n{details}")
                },
                automation.prompt
            );
            let request_id = format!("listener-{}-{}", automation.id, now_millis());
            let target_agent_id = automation
                .agent_id
                .clone()
                .unwrap_or_else(|| "mahayana-assistant".into());
            match self.config.mode {
                HostMode::Test => {
                    self.execute_test(FeatureCommand::ChatSend {
                        request_id,
                        text,
                        agent_id: Some(target_agent_id.clone()),
                        conversation_id: None,
                        mode: AgentMode::Agent,
                        mode_statement: None,
                        model: None,
                        attachments: Vec::new(),
                    })?;
                }
                HostMode::Production => {
                    #[cfg(feature = "production")]
                    {
                        self.production_chat(
                            request_id,
                            text,
                            Some(target_agent_id),
                            None,
                            AgentMode::Agent,
                            None,
                            None,
                            Vec::new(),
                        )?;
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                }
            }
        }
        Ok(matching_ids.len())
    }

    pub fn receive(&self) -> Result<Option<HostEvent>, FeatureHostError> {
        self.fire_due_automation()?;
        if let Some(event) = self.state()?.events.pop_front() {
            return Ok(Some(event));
        }
        match self.config.mode {
            HostMode::Test => Ok(None),
            HostMode::Production => self.receive_production(),
        }
    }

    fn fire_due_automation(&self) -> Result<(), FeatureHostError> {
        let now = now_millis();
        let due = self
            .state()?
            .automations
            .values()
            .find(|automation| {
                automation.enabled && automation.next_run_at_ms.is_some_and(|next| next <= now)
            })
            .map(|automation| (automation.id.clone(), automation.agent_id.clone()));
        if let Some((id, agent_id)) = due {
            let _ = self.execute_automation(FeatureCommand::AutomationRun {
                request_id: format!("scheduled-{id}-{now}"),
                id,
                agent_id,
            })?;
        }
        Ok(())
    }

    fn persist_automations(
        &self,
        automations: &BTreeMap<String, AutomationSummary>,
    ) -> Result<(), FeatureHostError> {
        let Some(path) = self.automation_path.as_deref() else {
            return Ok(());
        };
        persist_automations(path, automations)
    }

    fn persist_bots(&self, bots: &BTreeMap<String, BotSummary>) -> Result<(), FeatureHostError> {
        let Some(path) = self.bot_state_path.as_deref() else {
            return Ok(());
        };
        persist_bots(path, bots)
    }

    fn persist_groups(
        &self,
        groups: &BTreeMap<String, GroupSummary>,
    ) -> Result<(), FeatureHostError> {
        let Some(path) = self.group_state_path.as_deref() else {
            return Ok(());
        };
        persist_groups(path, groups)
    }

    fn persist_peer_messages(&self, messages: &[AgentPeerMessage]) -> Result<(), FeatureHostError> {
        let Some(path) = self.peer_messages_path.as_deref() else {
            return Ok(());
        };
        persist_peer_messages(path, messages)
    }

    fn persist_remote_device_secrets(
        &self,
        secrets: &BTreeMap<String, String>,
    ) -> Result<(), FeatureHostError> {
        let Some(path) = self.remote_device_state_path.as_deref() else {
            return Ok(());
        };
        persist_remote_computer_device_secrets(path, secrets)
    }

    fn remote_device_secret(
        &self,
        device_id: &str,
        create: bool,
    ) -> Result<String, FeatureHostError> {
        if !is_safe_memory_agent_id(device_id) {
            return Err(FeatureHostError::Contract(format!(
                "unsafe remote computer device id: {device_id}"
            )));
        }
        let mut state = self.state()?;
        if let Some(secret) = state.remote_computer_device_secrets.get(device_id) {
            return Ok(secret.clone());
        }
        if !create {
            return Err(FeatureHostError::Contract(
                "remote computer must be registered by this desktop before use".into(),
            ));
        }
        let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        state
            .remote_computer_device_secrets
            .insert(device_id.to_string(), secret.clone());
        let secrets = state.remote_computer_device_secrets.clone();
        drop(state);
        self.persist_remote_device_secrets(&secrets)?;
        Ok(secret)
    }

    pub fn resolve_approval(&self, resolution: ApprovalResolution) -> Result<(), FeatureHostError> {
        let pending = {
            let mut state = self.state()?;
            ensure_open(&state)?;
            state
                .pending_approvals
                .remove(&resolution.approval_id)
                .ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "unknown approval: {}",
                        resolution.approval_id
                    ))
                })?
        };

        #[cfg(not(feature = "production"))]
        let _ = &pending;

        if self.config.mode == HostMode::Production {
            #[cfg(feature = "production")]
            {
                if let Some(runtime_approval_id) = pending.runtime_approval_id {
                    let decision = match resolution.decision {
                        ApprovalDecision::AllowOnce => RuntimeApprovalDecision::Accept,
                        ApprovalDecision::AllowSession => RuntimeApprovalDecision::AcceptForSession,
                        ApprovalDecision::Deny => RuntimeApprovalDecision::Decline,
                    };
                    self.runtime()?.resolve_approval(
                        ApprovalId(runtime_approval_id),
                        decision,
                        json!({
                            "miniAppId": pending.mini_app_id,
                            "capability": pending.capability,
                        }),
                    )?;
                }
            }
            #[cfg(not(feature = "production"))]
            {
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }

        self.state()?.events.push_back(HostEvent::ApprovalResolved {
            timestamp: timestamp(),
            approval_id: resolution.approval_id,
            decision: resolution.decision,
        });
        Ok(())
    }

    pub fn interrupt(&self, operation_id: &str) -> Result<(), FeatureHostError> {
        {
            let state = self.state()?;
            ensure_open(&state)?;
            if !state.operations.contains(operation_id) {
                return Err(FeatureHostError::Contract(format!(
                    "unknown operation: {operation_id}"
                )));
            }
        }

        if self.config.mode == HostMode::Production {
            #[cfg(feature = "production")]
            if !operation_id.starts_with("host-task-") {
                self.runtime()?
                    .interrupt(OperationId(operation_id.to_string()))?;
            }
            #[cfg(not(feature = "production"))]
            return Err(FeatureHostError::ProductionUnavailable);
        }

        let mut state = self.state()?;
        state.operations.remove(operation_id);
        state.operation_agents.remove(operation_id);
        state.events.push_back(HostEvent::OperationInterrupted {
            timestamp: timestamp(),
            operation_id: operation_id.to_string(),
        });
        Ok(())
    }

    pub fn close(&self) -> Result<(), FeatureHostError> {
        let mut state = self.state()?;
        if state.closed {
            return Ok(());
        }
        state.closed = true;
        state.events.push_back(HostEvent::HostClosed {
            timestamp: timestamp(),
        });
        Ok(())
    }

    #[cfg(feature = "production")]
    fn runtime(&self) -> Result<&MahayanaHost, FeatureHostError> {
        self.runtime
            .as_ref()
            .ok_or(FeatureHostError::ProductionUnavailable)
    }

    #[cfg(not(feature = "production"))]
    fn execute_production(
        &self,
        _command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        Err(FeatureHostError::ProductionUnavailable)
    }

    #[cfg(not(feature = "production"))]
    fn receive_production(&self) -> Result<Option<HostEvent>, FeatureHostError> {
        Err(FeatureHostError::ProductionUnavailable)
    }

    #[cfg(feature = "production")]
    fn start_next_group_turn(&self, group_id: &str) -> Result<Option<String>, FeatureHostError> {
        let prepared = {
            let state = self.state()?;
            let Some(run) = state.group_runs.get(group_id).cloned() else {
                return Ok(None);
            };
            let Some(member_id) = run.speaker_order.get(run.speaker_index).cloned() else {
                return Ok(None);
            };
            let Some(group) = state.groups.get(group_id).cloned() else {
                return Ok(None);
            };
            let Some(member) = state.bots.get(&member_id).cloned() else {
                return Ok(None);
            };
            let Some(conversation_id) = member.conversation_id.clone() else {
                return Err(FeatureHostError::Contract(format!(
                    "group member {} has no conversation id",
                    member.id
                )));
            };
            let peers = group
                .member_ids
                .iter()
                .filter(|id| **id != member.id)
                .filter_map(|id| state.bots.get(id).cloned())
                .collect::<Vec<_>>();
            let new_messages = group_messages_since_member_last_spoke(&group.messages, &member.id);
            let system_prompt = build_group_member_system_prompt(&member, &group, &peers);
            let turn_prompt = build_group_turn_prompt(&member, &group, &peers, new_messages);
            let memory_prompt = self
                .memory_root_path
                .as_deref()
                .map(|root| render_memory_system_prompt(&root.join(&member.id).join("memory")))
                .unwrap_or_default();
            let workflow_catalog = match (
                self.workflow_root_path.as_deref(),
                self.memory_root_path.as_deref(),
            ) {
                (Some(workflow_root), Some(agent_root)) => {
                    render_workflow_catalog(workflow_root, agent_root, &member.id)
                }
                _ => String::new(),
            };
            let mut context_sections = vec![system_prompt];
            if !memory_prompt.is_empty() {
                context_sections.push(format!("[Persistent agent memory]\n{memory_prompt}"));
            }
            if !workflow_catalog.is_empty() {
                context_sections.push(format!("[Available workflows]\n{workflow_catalog}"));
            }
            context_sections.push(turn_prompt);
            let runtime_text = format!(
                "[MAHAYANA_HIDDEN_CONTEXT]\n{}",
                context_sections.join("\n\n")
            );
            (
                GroupOperationContext {
                    run_id: run.run_id,
                    group_id: group.id,
                    member_id: member.id,
                    member_name: member.name,
                },
                conversation_id,
                runtime_text,
            )
        };
        let (context, conversation_id, runtime_text) = prepared;
        let response = self.runtime()?.execute(RuntimeCommand::SendMessage {
            conversation_id: ConversationId(conversation_id),
            text: runtime_text,
            client_message_id: Some(format!(
                "{}:{}:{}",
                context.run_id, context.group_id, context.member_id
            )),
            hidden: true,
        })?;
        let operation_id = match response {
            RuntimeResponse::Accepted { operation_id } => operation_id.to_string(),
            other => return Err(unexpected_response("group.member.turn", other)),
        };
        let mut state = self.state()?;
        state.group_operations.insert(operation_id.clone(), context);
        Ok(Some(operation_id))
    }

    #[cfg(feature = "production")]
    fn advance_group_run_after_turn(
        &self,
        context: &GroupOperationContext,
    ) -> Result<Option<String>, FeatureHostError> {
        let should_continue = {
            let mut state = self.state()?;
            let Some(snapshot) = state.group_runs.get(&context.group_id).cloned() else {
                return Ok(None);
            };
            if snapshot.run_id != context.run_id {
                return Ok(None);
            }
            let mut next = snapshot;
            next.speaker_index += 1;
            let mut done = next.total_messages >= GROUP_MAX_MEMBER_TURNS;
            if !done && next.speaker_index >= next.speaker_order.len() {
                if next.messages_this_round == 0 {
                    done = true;
                } else {
                    next.round += 1;
                    if next.round >= GROUP_MAX_ROUNDS {
                        done = true;
                    } else if let Some(group) = state.groups.get(&context.group_id) {
                        let responders = resolve_group_responders(group, &state.bots);
                        next.speaker_order = order_round_speakers(&responders, next.round);
                        next.speaker_index = 0;
                        next.messages_this_round = 0;
                        if next.speaker_order.is_empty() {
                            done = true;
                        }
                    } else {
                        done = true;
                    }
                }
            }
            if done {
                state.group_runs.remove(&context.group_id);
                false
            } else {
                state.group_runs.insert(context.group_id.clone(), next);
                true
            }
        };
        if should_continue {
            self.start_next_group_turn(&context.group_id)
        } else {
            Ok(None)
        }
    }

    #[cfg(feature = "production")]
    fn translate_runtime_event(
        &self,
        event: RuntimeEvent,
    ) -> Result<Option<HostEvent>, FeatureHostError> {
        let event = match event {
            RuntimeEvent::Ready { .. } => None,
            RuntimeEvent::MessageDelta {
                operation_id,
                delta,
                ..
            } => {
                let operation_id = operation_id.to_string();
                let group_context = self.state()?.group_operations.get(&operation_id).cloned();
                if let Some(context) = group_context {
                    Some(HostEvent::GroupDelta {
                        timestamp: timestamp(),
                        group_id: context.group_id,
                        member_id: context.member_id,
                        member_name: context.member_name,
                        operation_id,
                        delta,
                    })
                } else if let Some(context) = self
                    .state()?
                    .background_operations
                    .get(&operation_id)
                    .cloned()
                {
                    Some(HostEvent::AgentBackgroundDelta {
                        timestamp: timestamp(),
                        agent_id: context.agent_id,
                        agent_name: context.agent_name,
                        operation_id,
                        source: context.source,
                        delta,
                    })
                } else {
                    Some(HostEvent::ChatDelta {
                        timestamp: timestamp(),
                        operation_id,
                        delta,
                    })
                }
            }
            RuntimeEvent::MessageCompleted {
                operation_id,
                message,
                ..
            } => {
                let operation_id = operation_id.to_string();
                let group_context = self.state()?.group_operations.get(&operation_id).cloned();
                if let Some(context) = group_context {
                    if message.role != RuntimeMessageRole::Assistant {
                        None
                    } else {
                        let content = message.text.trim();
                        if is_group_pass_content(content) {
                            None
                        } else {
                            let mut state = self.state()?;
                            let message_id = next_id(&mut state, "group-message");
                            let now = now_millis();
                            let Some(group) = state.groups.get_mut(&context.group_id) else {
                                return Ok(None);
                            };
                            group.messages.push(GroupMessage {
                                id: message_id,
                                speaker: GroupSpeaker::Member {
                                    id: context.member_id.clone(),
                                    name: context.member_name.clone(),
                                },
                                content: content.to_string(),
                                created_at_ms: now,
                            });
                            if group.messages.len() > 500 {
                                let overflow = group.messages.len() - 500;
                                group.messages.drain(0..overflow);
                            }
                            group.updated_at_ms = now;
                            let group = group.clone();
                            if let Some(run) = state.group_runs.get_mut(&context.group_id) {
                                if run.run_id == context.run_id
                                    && run.total_messages < GROUP_MAX_MEMBER_TURNS
                                {
                                    run.total_messages += 1;
                                    run.messages_this_round += 1;
                                }
                            }
                            self.persist_groups(&state.groups)?;
                            Some(HostEvent::GroupChanged {
                                timestamp: timestamp(),
                                action: "message".into(),
                                group,
                            })
                        }
                    }
                } else if let Some(context) = self
                    .state()?
                    .background_operations
                    .get(&operation_id)
                    .cloned()
                {
                    if message.role == RuntimeMessageRole::Assistant {
                        if let Some(artifact) = context.teach_artifact.as_deref() {
                            if !message.text.trim().is_empty() {
                                match self.persist_teach_workflow(
                                    &context.agent_id,
                                    artifact,
                                    &message.text,
                                ) {
                                    Ok(workflow) => {
                                        self.state()?.events.push_back(
                                            HostEvent::WorkflowChanged {
                                                timestamp: timestamp(),
                                                agent_id: context.agent_id.clone(),
                                                action: "learned".into(),
                                                workflow: Some(workflow),
                                                id: None,
                                            },
                                        );
                                    }
                                    Err(error) => {
                                        let mut state = self.state()?;
                                        push_error_tray(
                                            &mut state,
                                            context.agent_id.clone(),
                                            "Teach workflow could not be saved".into(),
                                            Some(error.to_string()),
                                            Some(operation_id.clone()),
                                            Some(format!("teach-workflow:{}", context.agent_id)),
                                        );
                                    }
                                }
                            }
                        }
                        Some(HostEvent::AgentBackgroundMessage {
                            timestamp: timestamp(),
                            agent_id: context.agent_id,
                            agent_name: context.agent_name,
                            operation_id,
                            source: context.source,
                            text: message.text,
                        })
                    } else {
                        None
                    }
                } else {
                    let mut cards = transcript_cards_from_metadata(&message.metadata);
                    let message_id = message.id.to_string();
                    if message.text.trim().is_empty() && !cards.is_empty() {
                        let first = cards.remove(0);
                        let mut state = self.state()?;
                        for (index, card) in cards.into_iter().enumerate() {
                            state.events.push_back(HostEvent::TranscriptCard {
                                timestamp: timestamp(),
                                entry_id: format!("{message_id}-card-{}", index + 1),
                                operation_id: Some(operation_id.clone()),
                                card,
                            });
                        }
                        Some(HostEvent::TranscriptCard {
                            timestamp: timestamp(),
                            entry_id: format!("{message_id}-card-0"),
                            operation_id: Some(operation_id),
                            card: first,
                        })
                    } else {
                        if !cards.is_empty() {
                            let mut state = self.state()?;
                            for (index, card) in cards.into_iter().enumerate() {
                                state.events.push_back(HostEvent::TranscriptCard {
                                    timestamp: timestamp(),
                                    entry_id: format!("{message_id}-card-{index}"),
                                    operation_id: Some(operation_id.clone()),
                                    card,
                                });
                            }
                        }
                        let role = match message.role {
                            RuntimeMessageRole::User => MessageRole::User,
                            RuntimeMessageRole::Assistant
                            | RuntimeMessageRole::Contact
                            | RuntimeMessageRole::MiniApp
                            | RuntimeMessageRole::System => MessageRole::Assistant,
                        };
                        Some(HostEvent::ChatMessage {
                            timestamp: timestamp(),
                            role,
                            text: message.text,
                            operation_id: Some(operation_id),
                        })
                    }
                }
            }
            RuntimeEvent::ApprovalRequested {
                approval_id,
                title,
                details,
                ..
            } => Some(self.translate_runtime_approval(approval_id, title, details)?),
            RuntimeEvent::OperationCompleted { operation_id } => {
                let operation_id = operation_id.to_string();
                let group_context = self.state()?.group_operations.remove(&operation_id);
                if let Some(context) = group_context {
                    let _ = self.advance_group_run_after_turn(&context)?;
                    None
                } else if let Some(context) =
                    self.state()?.background_operations.remove(&operation_id)
                {
                    Some(HostEvent::AgentBackgroundFinished {
                        timestamp: timestamp(),
                        agent_id: context.agent_id,
                        agent_name: context.agent_name,
                        operation_id,
                        source: context.source,
                        error: None,
                    })
                } else {
                    let mut state = self.state()?;
                    state.operations.remove(&operation_id);
                    state.operation_agents.remove(&operation_id);
                    Some(HostEvent::OperationCompleted {
                        timestamp: timestamp(),
                        operation_id,
                    })
                }
            }
            RuntimeEvent::OperationFailed {
                operation_id,
                code,
                message,
            } => {
                let operation_id = operation_id.to_string();
                let group_context = self.state()?.group_operations.remove(&operation_id);
                if let Some(context) = group_context {
                    let group = {
                        let mut state = self.state()?;
                        let group = state.groups.get(&context.group_id).cloned();
                        push_error_tray(
                            &mut state,
                            context.member_id.clone(),
                            format!("{} failed in {}", context.member_name, context.group_id),
                            Some(message.clone()),
                            Some(operation_id.clone()),
                            Some(format!("group:{}:{}", context.group_id, code)),
                        );
                        group
                    };
                    let _ = self.advance_group_run_after_turn(&context)?;
                    group.map(|group| HostEvent::GroupChanged {
                        timestamp: timestamp(),
                        action: format!("turnFailed:{code}:{message}"),
                        group,
                    })
                } else if let Some(context) =
                    self.state()?.background_operations.remove(&operation_id)
                {
                    let mut state = self.state()?;
                    push_error_tray(
                        &mut state,
                        context.agent_id.clone(),
                        format!("{} background task failed", context.agent_name),
                        Some(message.clone()),
                        Some(operation_id.clone()),
                        Some(format!("background:{}:{code}", context.agent_id)),
                    );
                    Some(HostEvent::AgentBackgroundFinished {
                        timestamp: timestamp(),
                        agent_id: context.agent_id,
                        agent_name: context.agent_name,
                        operation_id,
                        source: context.source,
                        error: Some(message),
                    })
                } else {
                    let mut state = self.state()?;
                    state.operations.remove(&operation_id);
                    let agent_id = state
                        .operation_agents
                        .remove(&operation_id)
                        .unwrap_or_else(|| "mahayana-assistant".into());
                    let agent_name = state
                        .bots
                        .get(&agent_id)
                        .map(|bot| bot.name.clone())
                        .unwrap_or_else(|| "Agent".into());
                    push_error_tray(
                        &mut state,
                        agent_id.clone(),
                        format!("{agent_name} task failed"),
                        Some(message.clone()),
                        Some(operation_id.clone()),
                        Some(format!("agent-run:{agent_id}:{code}")),
                    );
                    Some(HostEvent::OperationFailed {
                        timestamp: timestamp(),
                        operation_id,
                        code,
                        message,
                    })
                }
            }
            RuntimeEvent::ModelUsageUpdated {
                operation_id,
                usage,
            } => {
                let tokens = usage.total.unwrap_or_else(|| usage.last.clone());
                Some(HostEvent::UsageUpdated {
                    timestamp: timestamp(),
                    operation_id: operation_id.to_string(),
                    input_tokens: tokens.input_tokens,
                    cached_input_tokens: tokens.cached_input_tokens,
                    output_tokens: tokens.output_tokens,
                    reasoning_tokens: tokens.reasoning_output_tokens,
                    total_tokens: tokens.total_tokens,
                    context_window: usage.model_context_window,
                })
            }
            RuntimeEvent::PluginProgress {
                operation_id,
                plugin_id,
                tool,
                progress,
                total,
                message,
            } => Some(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: Some(operation_id.to_string()),
                step_id: format!("{plugin_id}:{tool}"),
                kind: "tool".into(),
                title: tool,
                detail: Some(message),
                status: if total > 0 && progress >= total {
                    AgentStepStatus::Completed
                } else {
                    AgentStepStatus::Running
                },
                progress: Some(progress),
                total: Some(total),
            }),
            RuntimeEvent::AgentActivity {
                operation_id,
                step_id,
                kind,
                title,
                detail,
                status,
                metadata,
            } => {
                let operation_id = operation_id.to_string();
                let agent_id = {
                    let state = self.state()?;
                    activity_parent_agent_id(&state, &operation_id)
                };
                if kind == "subagent" {
                    let mut state = self.state()?;
                    let changed = update_subagents_from_activity(
                        &mut state,
                        &agent_id,
                        &operation_id,
                        &title,
                        detail.as_deref(),
                        status,
                        metadata.as_ref(),
                    );
                    for subagent in changed {
                        state.events.push_back(HostEvent::SubagentChanged {
                            timestamp: timestamp(),
                            subagent,
                        });
                    }
                    let mut tasks = state
                        .async_tasks
                        .values()
                        .filter(|task| task.parent_agent_id == agent_id)
                        .cloned()
                        .collect::<Vec<_>>();
                    tasks.sort_by_key(|task| task.started_at_ms);
                    state.events.push_back(HostEvent::AsyncTaskChanged {
                        timestamp: timestamp(),
                        agent_id: agent_id.clone(),
                        tasks,
                    });
                }
                if matches!(
                    kind.as_str(),
                    "shell" | "command" | "local-exec" | "exec" | "cloud-agent" | "cloud_agent"
                ) {
                    let task_id = format!("{operation_id}:{step_id}");
                    let task_kind = if matches!(kind.as_str(), "cloud-agent" | "cloud_agent") {
                        AsyncTaskKind::CloudAgent
                    } else {
                        AsyncTaskKind::Shell
                    };
                    let resource_id = if task_kind == AsyncTaskKind::CloudAgent {
                        cloud_task_resource_id(metadata.as_ref())
                    } else {
                        None
                    };
                    let mut state = self.state()?;
                    if status == RuntimeActivityStatus::Running {
                        state.async_tasks.insert(
                            task_id.clone(),
                            AsyncTaskSummary {
                                kind: task_kind,
                                id: task_id.clone(),
                                parent_agent_id: agent_id.clone(),
                                label: title.clone(),
                                status: AsyncTaskStatus::Running,
                                started_at_ms: now_millis(),
                                detail: detail.clone(),
                                subagent_type: None,
                                resource_id: resource_id.clone(),
                            },
                        );
                    } else {
                        state.async_tasks.remove(&task_id);
                    }
                    let mut tasks = state
                        .async_tasks
                        .values()
                        .filter(|task| task.parent_agent_id == agent_id)
                        .cloned()
                        .collect::<Vec<_>>();
                    tasks.sort_by_key(|task| task.started_at_ms);
                    state.events.push_back(HostEvent::AsyncTaskChanged {
                        timestamp: timestamp(),
                        agent_id: agent_id.clone(),
                        tasks,
                    });
                }
                if matches!(kind.as_str(), "shell" | "command" | "local-exec" | "exec")
                    && status != RuntimeActivityStatus::Running
                {
                    let _ = self.append_action_audit(
                        &agent_id,
                        Some(&operation_id),
                        json!({
                            "kind": "shellCommand",
                            "command": detail.clone().unwrap_or_else(|| title.clone()),
                            "shellKind": kind,
                            "target": "runtime",
                            "status": match status {
                                RuntimeActivityStatus::Completed => "completed",
                                RuntimeActivityStatus::Failed => "failed",
                                RuntimeActivityStatus::Running => "running",
                            },
                        }),
                    );
                }
                if kind == "computer" && status != RuntimeActivityStatus::Running {
                    let computer_metadata = metadata
                        .as_ref()
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    let _ = self.append_action_audit(
                        &agent_id,
                        Some(&operation_id),
                        json!({
                            "kind": "computerUse",
                            "origin": computer_metadata
                                .get("origin")
                                .and_then(Value::as_str)
                                .unwrap_or("ai"),
                            "actions": computer_metadata.get("arguments").cloned().unwrap_or(Value::Null),
                            "detail": detail.clone(),
                            "title": title.clone(),
                            "status": match status {
                                RuntimeActivityStatus::Completed => "completed",
                                RuntimeActivityStatus::Failed => "failed",
                                RuntimeActivityStatus::Running => "running",
                            },
                        }),
                    );
                }
                Some(HostEvent::AgentStep {
                    timestamp: timestamp(),
                    operation_id: Some(operation_id),
                    step_id,
                    kind,
                    title,
                    detail,
                    status: match status {
                        RuntimeActivityStatus::Running => AgentStepStatus::Running,
                        RuntimeActivityStatus::Completed => AgentStepStatus::Completed,
                        RuntimeActivityStatus::Failed => AgentStepStatus::Failed,
                    },
                    progress: None,
                    total: None,
                })
            }
            RuntimeEvent::ProviderDegraded { provider, message } => Some(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: None,
                step_id: format!("provider:{provider}"),
                kind: "provider".into(),
                title: format!("{provider} 服务降级"),
                detail: Some(message),
                status: AgentStepStatus::Failed,
                progress: None,
                total: None,
            }),
            RuntimeEvent::Lagged { skipped } => Some(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: None,
                step_id: "runtime:event-lag".into(),
                kind: "runtime".into(),
                title: "事件流正在追赶".into(),
                detail: Some(format!("跳过 {skipped} 个过期事件")),
                status: AgentStepStatus::Failed,
                progress: None,
                total: None,
            }),
        };
        Ok(event)
    }

    #[cfg(feature = "production")]
    fn translate_runtime_approval(
        &self,
        approval_id: ApprovalId,
        title: String,
        details: serde_json::Value,
    ) -> Result<HostEvent, FeatureHostError> {
        let approval_key = approval_id.to_string();
        let mini_app_id = details
            .get("pluginId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("runtime")
            .to_string();
        let capability = details
            .get("capability")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(title.as_str())
            .to_string();
        let reason = details
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| details.to_string());

        let settings = self.state()?.settings.clone();
        let is_local_tool_request = details.get("command").is_some()
            || details
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "command" | "local-tool" | "local_tool"));
        let has_ask_match = settings.auto_review_rules.iter().any(|rule| {
            rule.behavior == AutoReviewBehavior::Ask
                && auto_review_rule_matches(rule, &title, &details)
        });
        let has_allow_match = !has_ask_match
            && settings.auto_review_rules.iter().any(|rule| {
                rule.behavior == AutoReviewBehavior::Allow
                    && auto_review_rule_matches(rule, &title, &details)
            });
        let auto_decision = if is_local_tool_request
            && (!settings.local_execution
                || settings.local_tool_permission == LocalToolPermission::Never)
        {
            Some(ApprovalDecision::Deny)
        } else if is_local_tool_request
            && settings.local_tool_permission == LocalToolPermission::Always
            && !has_ask_match
        {
            Some(ApprovalDecision::AllowSession)
        } else if has_allow_match {
            Some(ApprovalDecision::AllowOnce)
        } else {
            None
        };
        if let Some(decision) = auto_decision {
            let runtime_decision = match decision {
                ApprovalDecision::AllowOnce => RuntimeApprovalDecision::Accept,
                ApprovalDecision::AllowSession => RuntimeApprovalDecision::AcceptForSession,
                ApprovalDecision::Deny => RuntimeApprovalDecision::Decline,
            };
            self.runtime()?.resolve_approval(
                ApprovalId(approval_key.clone()),
                runtime_decision,
                json!({
                    "source": "fabushi-auto-review",
                    "capability": capability,
                    "reason": reason,
                }),
            )?;
            let agent_id = details
                .get("agentId")
                .and_then(Value::as_str)
                .unwrap_or("mahayana-assistant");
            let _ = self.append_action_audit(
                agent_id,
                None,
                json!({
                    "kind": "autoReview",
                    "approvalId": approval_key,
                    "decision": match decision {
                        ApprovalDecision::AllowOnce => "allow-once",
                        ApprovalDecision::AllowSession => "allow-session",
                        ApprovalDecision::Deny => "deny",
                    },
                    "title": title,
                    "capability": capability,
                }),
            );
            self.state()?.events.push_back(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: None,
                step_id: format!("auto-review:{approval_key}"),
                kind: "auto-review".into(),
                title: match decision {
                    ApprovalDecision::AllowOnce => "自动审批：本次允许".into(),
                    ApprovalDecision::AllowSession => "自动审批：按本机权限允许".into(),
                    ApprovalDecision::Deny => "自动审批：已拒绝".into(),
                },
                detail: Some(title),
                status: AgentStepStatus::Completed,
                progress: None,
                total: None,
            });
            return Ok(HostEvent::ApprovalResolved {
                timestamp: timestamp(),
                approval_id: approval_key,
                decision,
            });
        }

        self.state()?.pending_approvals.insert(
            approval_key.clone(),
            PendingApproval {
                mini_app_id: mini_app_id.clone(),
                capability: capability.clone(),
                runtime_approval_id: Some(approval_id.to_string()),
            },
        );
        Ok(HostEvent::ApprovalRequested {
            timestamp: timestamp(),
            approval_id: approval_key,
            mini_app_id,
            capability,
            reason,
            kind: details
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_string),
            subject: details
                .get("subject")
                .or_else(|| details.get("command"))
                .and_then(Value::as_str)
                .map(str::to_string),
            detail: details
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string),
            proposed_rule: details
                .get("proposedRule")
                .and_then(Value::as_str)
                .map(str::to_string),
            location: details
                .get("location")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    #[cfg(feature = "production")]
    fn receive_production(&self) -> Result<Option<HostEvent>, FeatureHostError> {
        for _ in 0..16 {
            let Some(event) = self.runtime()?.receive(Duration::from_millis(1))? else {
                return Ok(None);
            };
            if let Some(event) = self.translate_runtime_event(event)? {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    #[cfg(feature = "production")]
    fn production_long_task(
        &self,
        request_id: String,
        label: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let label = required(label, "operation label")?;
        let mut state = self.state()?;
        let operation_id = next_id(&mut state, "host-task");
        state.operations.insert(operation_id.clone());
        state.events.push_back(HostEvent::OperationStarted {
            timestamp: timestamp(),
            operation_id: operation_id.clone(),
            label,
            interruptible: true,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: Some(operation_id),
        })
    }

    #[cfg(feature = "production")]
    fn production_clear_session(
        &self,
        request_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        self.runtime()?.clear_session()?;
        let mut state = self.state()?;
        state.session_active = false;
        state.events.push_back(HostEvent::SessionCleared {
            timestamp: timestamp(),
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_capability_request(
        &self,
        request_id: String,
        mini_app_id: String,
        capability: String,
        reason: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let mini_app_id = required(mini_app_id, "miniAppId")?;
        let capability = required(capability, "capability")?;
        let reason = required(reason, "reason")?;
        let mut state = self.state()?;
        if !state.installed.contains_key(&mini_app_id) {
            return Err(FeatureHostError::Contract(format!(
                "MiniApp is not installed: {mini_app_id}"
            )));
        }
        let approval_id = next_id(&mut state, "approval");
        state.pending_approvals.insert(
            approval_id.clone(),
            PendingApproval {
                mini_app_id: mini_app_id.clone(),
                capability: capability.clone(),
                runtime_approval_id: None,
            },
        );
        state.events.push_back(HostEvent::ApprovalRequested {
            timestamp: timestamp(),
            approval_id,
            mini_app_id,
            capability,
            reason,
            kind: Some("capability".into()),
            subject: None,
            detail: None,
            proposed_rule: None,
            location: None,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_open(
        &self,
        request_id: String,
        mini_app_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let mini_app_id = required(mini_app_id, "miniAppId")?;
        if !self.state()?.installed.contains_key(&mini_app_id) {
            return Err(FeatureHostError::Contract(format!(
                "MiniApp is not installed: {mini_app_id}"
            )));
        }
        let html = match self.runtime()?.execute(RuntimeCommand::PluginUi {
            plugin_id: mini_app_id.clone(),
        })? {
            RuntimeResponse::PluginUi { html, .. } => html,
            other => return Err(unexpected_response("miniapp.open", other)),
        };
        self.state()?.events.push_back(HostEvent::MiniAppOpened {
            timestamp: timestamp(),
            mini_app_id,
            html: Some(html),
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_install(
        &self,
        request_id: String,
        mini_app_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let mini_app_id = required(mini_app_id, "miniAppId")?;
        let response = self.runtime()?.execute(RuntimeCommand::ListCapabilities {
            query: Some(format!("miniapp.{mini_app_id}")),
        })?;
        let available = match response {
            RuntimeResponse::Capabilities { data } => data.into_iter().any(|capability| {
                capability.plugin_id.as_deref() == Some(mini_app_id.as_str())
                    && capability.is_invokable()
            }),
            other => return Err(unexpected_response("marketplace.install", other)),
        };
        if !available {
            return Err(FeatureHostError::Contract(format!(
                "MiniApp is unavailable in the production Runtime: {mini_app_id}"
            )));
        }
        let version = "bundled".to_string();
        let mut state = self.state()?;
        state.installed.insert(mini_app_id.clone(), version.clone());
        state.events.push_back(HostEvent::MarketplaceInstalled {
            timestamp: timestamp(),
            mini_app_id,
            version,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn mcp_instruction_context(&self) -> Result<Option<String>, FeatureHostError> {
        let instructions = match self
            .runtime()?
            .execute(RuntimeCommand::McpCustomInstructions)?
        {
            RuntimeResponse::McpCustomInstructions { instructions } => instructions,
            other => return Err(unexpected_response("mcp.customInstructions", other)),
        };
        Ok(render_mcp_instruction_context(&instructions))
    }

    #[cfg(feature = "production")]
    fn production_chat(
        &self,
        request_id: String,
        text: String,
        agent_id: Option<String>,
        requested_conversation_id: Option<String>,
        mode: AgentMode,
        mode_statement: Option<String>,
        model: Option<String>,
        attachments: Vec<AttachmentContext>,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let text = required(text, "chat text")?;
        let bot_conversation_id = if let Some(agent_id) = agent_id.as_deref() {
            self.state()?
                .bots
                .get(agent_id)
                .and_then(|bot| bot.conversation_id.clone())
        } else {
            None
        };
        if let Some(mini_app_id) = agent_id
            .as_deref()
            .filter(|id| *id != "mahayana-assistant" && bot_conversation_id.is_none())
        {
            match self
                .runtime()?
                .execute(RuntimeCommand::ApproveLocalPluginTool {
                    plugin_id: mini_app_id.to_string(),
                    tool: "chat".to_string(),
                })? {
                RuntimeResponse::LocalPluginToolApproved { .. } => {}
                other => return Err(unexpected_response("miniapp.chat.approve", other)),
            }
            let response = self
                .runtime()?
                .execute(RuntimeCommand::CallLocalPluginTool {
                    plugin_id: mini_app_id.to_string(),
                    tool: "chat".to_string(),
                    arguments: json!({"message": text}),
                })?;
            let reply = match response {
                RuntimeResponse::LocalPluginToolResult { result, .. } => result
                    .pointer("/content/0/text")
                    .and_then(Value::as_str)
                    .filter(|reply| !reply.is_empty())
                    .unwrap_or("已收到。请选择应用内的快捷操作继续。")
                    .to_string(),
                other => return Err(unexpected_response("miniapp.chat", other)),
            };
            let mut state = self.state()?;
            state.events.push_back(HostEvent::ChatMessage {
                timestamp: timestamp(),
                role: MessageRole::User,
                text,
                operation_id: None,
            });
            state.events.push_back(HostEvent::ChatMessage {
                timestamp: timestamp(),
                role: MessageRole::Assistant,
                text: reply,
                operation_id: None,
            });
            return Ok(CommandAccepted {
                request_id,
                operation_id: None,
            });
        }
        let conversation_id = requested_conversation_id
            .map(ConversationId)
            .or_else(|| bot_conversation_id.map(ConversationId))
            .unwrap_or_else(|| ConversationId(CODEX_ASSISTANT_CONVERSATION_ID.to_string()));
        let (provider, routed_model) = match self.runtime()?.execute(RuntimeCommand::Status)? {
            RuntimeResponse::Status(status) => (
                format!("{:?}", status.model_provider).to_lowercase(),
                status.model,
            ),
            other => return Err(unexpected_response("runtime.status", other)),
        };
        let mut runtime_text =
            compose_agent_input(&text, mode, mode_statement.as_deref(), &attachments);
        if let Some(mcp_context) = self.mcp_instruction_context()? {
            runtime_text = format!(
                "{mcp_context}

[Current turn]
{runtime_text}"
            );
        }
        let memory_agent_id = agent_id.as_deref().unwrap_or("mahayana-assistant");
        if is_safe_memory_agent_id(memory_agent_id) {
            if let Some(root) = self.memory_root_path.as_deref() {
                let memory_dir = root.join(memory_agent_id).join("memory");
                let memory_prompt = render_memory_system_prompt(&memory_dir);
                if !memory_prompt.is_empty() {
                    runtime_text = format!(
                        "[Persistent agent memory]\n{memory_prompt}\n\n[Current turn]\n{runtime_text}"
                    );
                }
            }
            if let (Some(workflow_root), Some(agent_root)) = (
                self.workflow_root_path.as_deref(),
                self.memory_root_path.as_deref(),
            ) {
                let workflow_catalog =
                    render_workflow_catalog(workflow_root, agent_root, memory_agent_id);
                if !workflow_catalog.is_empty() {
                    runtime_text =
                        format!("[Available workflows]\n{workflow_catalog}\n\n{runtime_text}");
                }
            }
        }
        let response = self.runtime()?.execute(RuntimeCommand::SendMessage {
            conversation_id,
            text: runtime_text,
            client_message_id: Some(request_id.clone()),
            hidden: false,
        })?;
        let operation_id = match response {
            RuntimeResponse::Accepted { operation_id } => operation_id.to_string(),
            other => return Err(unexpected_response("chat.send", other)),
        };
        let mut state = self.state()?;
        state.operations.insert(operation_id.clone());
        state.operation_agents.insert(
            operation_id.clone(),
            agent_id
                .clone()
                .unwrap_or_else(|| "mahayana-assistant".into()),
        );
        state.events.push_back(HostEvent::ModelRouted {
            timestamp: timestamp(),
            operation_id: operation_id.clone(),
            provider,
            model: routed_model.clone(),
            mode,
        });
        if let Some(preferred_model) = model.filter(|preferred| preferred != &routed_model) {
            state.events.push_back(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: Some(operation_id.clone()),
                step_id: format!("{operation_id}:model-preference"),
                kind: "model".into(),
                title: format!("使用已配置模型 {routed_model}"),
                detail: Some(format!(
                    "本次偏好 {preferred_model}；当前 Runtime 不支持会话中热切换"
                )),
                status: AgentStepStatus::Completed,
                progress: None,
                total: None,
            });
        }
        state.events.push_back(HostEvent::ChatMessage {
            timestamp: timestamp(),
            role: MessageRole::User,
            text,
            operation_id: None,
        });
        state.events.push_back(HostEvent::OperationStarted {
            timestamp: timestamp(),
            operation_id: operation_id.clone(),
            label: "chat-response".into(),
            interruptible: true,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: Some(operation_id),
        })
    }

    #[cfg(feature = "production")]
    fn production_list_conversations(
        &self,
        request_id: String,
        query: Option<String>,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let conversations = match self.runtime()?.execute(RuntimeCommand::ListConversations)? {
            RuntimeResponse::Conversations { data } => data,
            other => return Err(unexpected_response("conversation.list", other)),
        };
        let query = query.map(|query| query.to_lowercase());
        let conversations = conversations
            .into_iter()
            .filter(|conversation| {
                query.as_ref().is_none_or(|query| {
                    conversation.title.to_lowercase().contains(query)
                        || conversation.id.0.to_lowercase().contains(query)
                })
            })
            .map(|conversation| ConversationSummary {
                id: conversation.id.0,
                title: conversation.title,
                kind: conversation.peer.provider_key().into(),
                pinned: conversation.pinned,
                unread_count: conversation.unread_count,
                updated_at_ms: conversation.updated_at_ms,
            })
            .collect();
        self.state()?
            .events
            .push_back(HostEvent::ConversationListed {
                timestamp: timestamp(),
                conversations,
            });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_open_conversation(
        &self,
        request_id: String,
        conversation_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let conversation_id = required(conversation_id, "conversationId")?;
        let messages = match self
            .runtime()?
            .execute(RuntimeCommand::ConversationHistory {
                conversation_id: ConversationId(conversation_id.clone()),
                limit: Some(200),
            })? {
            RuntimeResponse::History { data } => data,
            other => return Err(unexpected_response("conversation.open", other)),
        };
        let messages = messages
            .into_iter()
            .map(|message| ConversationMessage {
                id: message.id.0,
                role: match message.role {
                    RuntimeMessageRole::User => MessageRole::User,
                    _ => MessageRole::Assistant,
                },
                text: message.text,
                created_at_ms: message.created_at_ms,
            })
            .collect();
        self.state()?
            .events
            .push_back(HostEvent::ConversationOpened {
                timestamp: timestamp(),
                conversation_id,
                messages,
            });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_list_capabilities(
        &self,
        request_id: String,
        query: Option<String>,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let response = self
            .runtime()?
            .execute(RuntimeCommand::ListCapabilities { query })?;
        let data = match response {
            RuntimeResponse::Capabilities { data } => data,
            other => return Err(unexpected_response("capability.list", other)),
        };
        let capabilities = data
            .into_iter()
            .map(|capability| CapabilitySummary {
                id: capability.id,
                title: capability.title,
                kind: match capability.kind {
                    CapabilityKind::Agent => "agent",
                    CapabilityKind::Bot => "bot",
                    CapabilityKind::Plugin => "plugin",
                    CapabilityKind::MiniApp => "miniApp",
                    CapabilityKind::Application => "application",
                    CapabilityKind::Contact => "contact",
                }
                .into(),
                mention: capability.mention,
                conversation_id: capability.conversation_id.to_string(),
                provider: capability.provider,
                plugin_id: capability.plugin_id,
                description: capability.description,
                required_permissions: capability.required_permissions,
                availability: match capability.availability {
                    CapabilityAvailability::Ready => "ready",
                    CapabilityAvailability::PermissionRequired => "permissionRequired",
                    CapabilityAvailability::Unavailable => "unavailable",
                }
                .into(),
                unavailable_reason: capability.unavailable_reason,
            })
            .collect();
        self.state()?.events.push_back(HostEvent::CapabilityListed {
            timestamp: timestamp(),
            capabilities,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn execute_production(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        {
            let state = self.state()?;
            ensure_open(&state)?;
        }
        match command {
            FeatureCommand::ChatSend {
                text,
                agent_id,
                conversation_id,
                mode,
                mode_statement,
                model,
                attachments,
                ..
            } => self.production_chat(
                request_id,
                text,
                agent_id,
                conversation_id,
                mode,
                mode_statement,
                model,
                attachments,
            ),
            FeatureCommand::ConversationList { query, .. } => {
                self.production_list_conversations(request_id, query)
            }
            FeatureCommand::ConversationOpen {
                conversation_id, ..
            } => self.production_open_conversation(request_id, conversation_id),
            FeatureCommand::CapabilityList { query, .. } => {
                self.production_list_capabilities(request_id, query)
            }
            FeatureCommand::MarketplaceInstall { mini_app_id, .. } => {
                self.production_install(request_id, mini_app_id)
            }
            FeatureCommand::MiniAppOpen { mini_app_id, .. } => {
                self.production_open(request_id, mini_app_id)
            }
            FeatureCommand::CapabilityRequest {
                mini_app_id,
                capability,
                reason,
                ..
            } => self.production_capability_request(request_id, mini_app_id, capability, reason),
            FeatureCommand::RuntimeLongTask { label, .. } => {
                self.production_long_task(request_id, label)
            }
            FeatureCommand::SessionClear { .. } => self.production_clear_session(request_id),
            _ => unreachable!(
                "automation and product-surface commands are intercepted before production dispatch"
            ),
        }
    }

    fn execute_test(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let mut state = self.state()?;
        ensure_open(&state)?;
        match command {
            FeatureCommand::ChatSend {
                text,
                agent_id,
                mode,
                model,
                attachments,
                ..
            } => {
                let text = required(text, "chat text")?;
                let operation_id = next_id(&mut state, "chat");
                state.events.push_back(HostEvent::ChatMessage {
                    timestamp: timestamp(),
                    role: MessageRole::User,
                    text: text.clone(),
                    operation_id: None,
                });
                state.events.push_back(HostEvent::OperationStarted {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    label: "chat-response".into(),
                    interruptible: false,
                });
                state.events.push_back(HostEvent::ModelRouted {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    provider: "mahayana-test".into(),
                    model: model.unwrap_or_else(|| "auto".into()),
                    mode,
                });
                state.events.push_back(HostEvent::AgentStep {
                    timestamp: timestamp(),
                    operation_id: Some(operation_id.clone()),
                    step_id: format!("{operation_id}:context"),
                    kind: "context".into(),
                    title: if attachments.is_empty() {
                        "分析请求".into()
                    } else {
                        format!("读取 {} 个附件", attachments.len())
                    },
                    detail: None,
                    status: AgentStepStatus::Completed,
                    progress: Some(1),
                    total: Some(1),
                });
                state.events.push_back(HostEvent::ChatMessage {
                    timestamp: timestamp(),
                    role: MessageRole::Assistant,
                    text: agent_id
                        .filter(|id| id != "mahayana-assistant")
                        .map(|id| format!("{id}机器人收到：{text}"))
                        .unwrap_or_else(|| format!("收到：{text}")),
                    operation_id: Some(operation_id.clone()),
                });
                state.events.push_back(HostEvent::UsageUpdated {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    input_tokens: text.chars().count() as i64,
                    cached_input_tokens: 0,
                    output_tokens: 8,
                    reasoning_tokens: 0,
                    total_tokens: text.chars().count() as i64 + 8,
                    context_window: Some(128_000),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: Some(operation_id),
                })
            }
            FeatureCommand::ConversationList { query, .. } => {
                let mut conversations = vec![ConversationSummary {
                    id: "codex:agent:assistant".into(),
                    title: "大乘助手".into(),
                    kind: "codex".into(),
                    pinned: true,
                    unread_count: 0,
                    updated_at_ms: 0,
                }];
                if let Some(query) = query {
                    conversations.retain(|item| item.title.contains(&query));
                }
                state.events.push_back(HostEvent::ConversationListed {
                    timestamp: timestamp(),
                    conversations,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::ConversationOpen {
                conversation_id, ..
            } => {
                state.events.push_back(HostEvent::ConversationOpened {
                    timestamp: timestamp(),
                    conversation_id,
                    messages: Vec::new(),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::CapabilityList { query, .. } => {
                let mut capabilities = vec![CapabilitySummary {
                    id: "agent.mahayana".into(),
                    title: "大乘助手".into(),
                    kind: "agent".into(),
                    mention: "@agent.mahayana".into(),
                    conversation_id: "codex:agent:assistant".into(),
                    provider: "codex".into(),
                    plugin_id: None,
                    description: "大乘共享智能代理".into(),
                    required_permissions: Vec::new(),
                    availability: "ready".into(),
                    unavailable_reason: None,
                }];
                capabilities.extend(state.installed.keys().map(|plugin_id| CapabilitySummary {
                    id: format!("miniapp.{plugin_id}"),
                    title: plugin_id.clone(),
                    kind: "miniApp".into(),
                    mention: format!("@miniapp.{plugin_id}"),
                    conversation_id: format!("miniapp:{plugin_id}"),
                    provider: "miniapp".into(),
                    plugin_id: Some(plugin_id.clone()),
                    description: "大乘共享插件、小程序、应用或机器人能力".into(),
                    required_permissions: Vec::new(),
                    availability: "ready".into(),
                    unavailable_reason: None,
                }));
                if let Some(query) = query {
                    let query = query.to_lowercase();
                    capabilities.retain(|item| {
                        item.id.to_lowercase().contains(&query)
                            || item.title.to_lowercase().contains(&query)
                            || item.description.to_lowercase().contains(&query)
                    });
                }
                state.events.push_back(HostEvent::CapabilityListed {
                    timestamp: timestamp(),
                    capabilities,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::MarketplaceInstall { mini_app_id, .. } => {
                let mini_app_id = required(mini_app_id, "miniAppId")?;
                let version = "1.0.0".to_string();
                state.installed.insert(mini_app_id.clone(), version.clone());
                state.events.push_back(HostEvent::MarketplaceInstalled {
                    timestamp: timestamp(),
                    mini_app_id,
                    version,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::MiniAppOpen { mini_app_id, .. } => {
                let mini_app_id = required(mini_app_id, "miniAppId")?;
                if !state.installed.contains_key(&mini_app_id) {
                    return Err(FeatureHostError::Contract(format!(
                        "MiniApp is not installed: {mini_app_id}"
                    )));
                }
                state.events.push_back(HostEvent::MiniAppOpened {
                    timestamp: timestamp(),
                    mini_app_id,
                    html: Some(
                        "<!doctype html><html><body><h1>测试 MiniApp</h1></body></html>".into(),
                    ),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::CapabilityRequest {
                mini_app_id,
                capability,
                reason,
                ..
            } => {
                let mini_app_id = required(mini_app_id, "miniAppId")?;
                let capability = required(capability, "capability")?;
                let reason = required(reason, "reason")?;
                let approval_id = next_id(&mut state, "approval");
                state.pending_approvals.insert(
                    approval_id.clone(),
                    PendingApproval {
                        mini_app_id: mini_app_id.clone(),
                        capability: capability.clone(),
                        runtime_approval_id: None,
                    },
                );
                state.events.push_back(HostEvent::ApprovalRequested {
                    timestamp: timestamp(),
                    approval_id,
                    mini_app_id,
                    capability,
                    reason,
                    kind: Some("capability".into()),
                    subject: None,
                    detail: None,
                    proposed_rule: None,
                    location: None,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::RuntimeLongTask { label, .. } => {
                let label = required(label, "operation label")?;
                let operation_id = next_id(&mut state, "operation");
                state.operations.insert(operation_id.clone());
                state.events.push_back(HostEvent::OperationStarted {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    label,
                    interruptible: true,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: Some(operation_id),
                })
            }
            FeatureCommand::SessionClear { .. } => {
                state.session_active = false;
                state.events.push_back(HostEvent::SessionCleared {
                    timestamp: timestamp(),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            _ => unreachable!(
                "automation and product-surface commands are intercepted before test dispatch"
            ),
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, FeatureState>, FeatureHostError> {
        self.state
            .lock()
            .map_err(|_| FeatureHostError::StatePoisoned)
    }
}

fn transcript_cards_from_metadata(metadata: &Value) -> Vec<TranscriptCard> {
    if let Some(cards) = metadata.get("cards").and_then(Value::as_array) {
        return cards.iter().filter_map(decode_transcript_card).collect();
    }
    for field in ["transcriptCard", "card", "artifact"] {
        if let Some(card) = metadata.get(field).and_then(decode_transcript_card) {
            return vec![card];
        }
    }
    decode_transcript_card(metadata).into_iter().collect()
}

fn decode_transcript_card(value: &Value) -> Option<TranscriptCard> {
    let mut value = value.clone();
    let object = value.as_object_mut()?;
    if !object.contains_key("kind") {
        let kind = object.get("type").cloned()?;
        object.insert("kind".into(), kind);
    }
    serde_json::from_value(value).ok()
}

fn is_product_surface_command(command: &FeatureCommand) -> bool {
    matches!(
        command,
        FeatureCommand::ConnectorList { .. }
            | FeatureCommand::ConnectorConnect { .. }
            | FeatureCommand::ConnectorRenameAccount { .. }
            | FeatureCommand::ConnectorRemoveAccount { .. }
            | FeatureCommand::ConnectorSetToolEnabled { .. }
            | FeatureCommand::SkillList { .. }
            | FeatureCommand::SkillUpsert { .. }
            | FeatureCommand::SkillDelete { .. }
            | FeatureCommand::SkillPublish { .. }
            | FeatureCommand::SkillUnpublish { .. }
            | FeatureCommand::SkillSync { .. }
            | FeatureCommand::BotList { .. }
            | FeatureCommand::BotSetHidden { .. }
            | FeatureCommand::DraftResolve { .. }
            | FeatureCommand::SecretProvide { .. }
            | FeatureCommand::ListenerList { .. }
            | FeatureCommand::ListenerConnect { .. }
            | FeatureCommand::ListenerDisconnect { .. }
            | FeatureCommand::UpdateStatus { .. }
            | FeatureCommand::UpdateCheck { .. }
            | FeatureCommand::UpdateInstall { .. }
    )
}

#[cfg(feature = "production")]
#[derive(Debug, Clone, Default)]
struct LiveConnectorProjection {
    server_name: Option<String>,
    connector_id: Option<String>,
    install_url: Option<String>,
    status: Option<ConnectorStatus>,
    accounts: BTreeMap<String, ConnectorAccountSummary>,
    tools: BTreeMap<String, ConnectorToolSummary>,
    tool_schemas: BTreeMap<String, Value>,
}

#[cfg(feature = "production")]
fn connector_key_from_name(value: &str) -> Option<&'static str> {
    let normalized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.contains("gmail") || normalized.contains("googlemail") {
        Some("gmail")
    } else if normalized.contains("github") {
        Some("github")
    } else if normalized.contains("slack") {
        Some("slack")
    } else if normalized.contains("microsoftteams") || normalized == "teams" {
        Some("teams")
    } else if normalized.contains("linear") {
        Some("linear")
    } else if normalized.contains("sentry") {
        Some("sentry")
    } else if normalized.contains("pagerduty") {
        Some("pagerduty")
    } else if normalized == "git" {
        Some("git")
    } else {
        None
    }
}

#[cfg(feature = "production")]
fn connector_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut needs_dash = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if needs_dash && !slug.is_empty() {
                slug.push('-');
            }
            needs_dash = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            needs_dash = true;
        }
    }
    if slug.is_empty() { "app".into() } else { slug }
}

#[cfg(feature = "production")]
fn connector_status_from_auth(auth_status: Option<&str>, has_tools: bool) -> ConnectorStatus {
    match auth_status.unwrap_or_default() {
        "notLoggedIn" => ConnectorStatus::AuthRequired,
        "oAuth" | "bearerToken" => ConnectorStatus::Connected,
        "unsupported" if has_tools => ConnectorStatus::Connected,
        "unknown" if has_tools => ConnectorStatus::Connected,
        _ if has_tools => ConnectorStatus::Connected,
        _ => ConnectorStatus::Disconnected,
    }
}

#[cfg(feature = "production")]
fn live_connector_projections(
    servers: &[Value],
    apps: &[Value],
) -> BTreeMap<String, LiveConnectorProjection> {
    let mut live = BTreeMap::<String, LiveConnectorProjection>::new();
    for app in apps {
        let name = app.get("name").and_then(Value::as_str).unwrap_or_default();
        let id = app.get("id").and_then(Value::as_str).unwrap_or_default();
        let Some(key) = connector_key_from_name(name).or_else(|| connector_key_from_name(id))
        else {
            continue;
        };
        let entry = live.entry(key.to_string()).or_default();
        entry.connector_id = (!id.is_empty()).then(|| id.to_string());
        entry.install_url = app
            .get("installUrl")
            .and_then(Value::as_str)
            .map(str::to_string);
        if app.get("isAccessible").and_then(Value::as_bool) == Some(true) {
            entry.status = Some(ConnectorStatus::Connected);
        }
    }
    for server in servers {
        let server_name = server
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let auth_status = server.get("authStatus").and_then(Value::as_str);
        let tools = server
            .get("tools")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let direct_key = connector_key_from_name(server_name);
        if let Some(key) = direct_key {
            let entry = live.entry(key.to_string()).or_default();
            entry.server_name = Some(server_name.to_string());
            entry.status = Some(connector_status_from_auth(auth_status, !tools.is_empty()));
        }
        for (wire_name, tool) in tools {
            let meta = tool.get("_meta").and_then(Value::as_object);
            let connector_name = meta
                .and_then(|meta| meta.get("connector_name"))
                .and_then(Value::as_str);
            let connector_id = meta
                .and_then(|meta| meta.get("connector_id"))
                .and_then(Value::as_str);
            let Some(key) = connector_name
                .and_then(connector_key_from_name)
                .or_else(|| connector_id.and_then(connector_key_from_name))
                .or(direct_key)
            else {
                continue;
            };
            let entry = live.entry(key.to_string()).or_default();
            entry.server_name = Some(server_name.to_string());
            entry.status = Some(ConnectorStatus::Connected);
            if let Some(connector_id) = connector_id {
                entry.connector_id = Some(connector_id.to_string());
                if entry.install_url.is_none() {
                    let display = connector_name.unwrap_or(key);
                    entry.install_url = Some(format!(
                        "https://chatgpt.com/apps/{}/{}",
                        connector_slug(display),
                        connector_id
                    ));
                }
            }
            let tool_id = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(wire_name.as_str())
                .to_string();
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let read_only = tool
                .pointer("/annotations/readOnlyHint")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(schema) = tool
                .get("inputSchema")
                .or_else(|| tool.get("input_schema"))
                .cloned()
            {
                entry.tool_schemas.insert(tool_id.clone(), schema);
            }
            entry.tools.insert(
                tool_id.clone(),
                ConnectorToolSummary {
                    id: tool_id.clone(),
                    name: tool
                        .get("title")
                        .and_then(Value::as_str)
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or(tool_id.as_str())
                        .to_string(),
                    description,
                    enabled: true,
                    requires_approval: Some(!read_only),
                },
            );
            let link_id = meta
                .and_then(|meta| meta.get("link_id"))
                .and_then(Value::as_str);
            let owner = meta
                .and_then(|meta| meta.get("link_owner_profile"))
                .and_then(Value::as_object);
            if link_id.is_some() || owner.is_some() {
                let account_id = link_id
                    .map(|link_id| format!("mcp:{key}:{link_id}"))
                    .unwrap_or_else(|| format!("mcp:{key}"));
                let email = owner
                    .and_then(|owner| owner.get("email"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let label = owner
                    .and_then(|owner| {
                        owner
                            .get("name")
                            .or_else(|| owner.get("nickname"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_string)
                    .or_else(|| email.clone())
                    .or_else(|| connector_name.map(str::to_string))
                    .unwrap_or_else(|| key.to_string());
                entry.accounts.insert(
                    account_id.clone(),
                    ConnectorAccountSummary {
                        id: account_id,
                        label,
                        status: ConnectorStatus::Connected,
                        email,
                        team_managed: Some(false),
                        error: None,
                    },
                );
            }
        }
    }
    live
}

#[cfg(feature = "production")]
fn projection_send_tool<'a>(
    projection: &'a LiveConnectorProjection,
    connector_id: &str,
) -> Option<(&'a str, Option<&'a Value>)> {
    let preferred: &[&str] = match connector_id {
        "gmail" => &[
            "gmail.send_email",
            "send_email",
            "gmail.send_draft",
            "send_draft",
        ],
        "slack" => &[
            "slack.send_message",
            "slack.post_message",
            "send_message",
            "post_message",
        ],
        _ => &[],
    };
    for candidate in preferred {
        if let Some((tool_id, _)) = projection
            .tools
            .iter()
            .find(|(tool_id, _)| tool_id.as_str() == *candidate || tool_id.ends_with(candidate))
        {
            return Some((tool_id.as_str(), projection.tool_schemas.get(tool_id)));
        }
    }
    None
}

#[cfg(feature = "production")]
fn schema_property_is_array(schema: Option<&Value>, name: &str) -> bool {
    schema
        .and_then(|schema| schema.pointer(&format!("/properties/{name}/type")))
        .and_then(Value::as_str)
        == Some("array")
}

#[cfg(feature = "production")]
fn draft_tool_arguments(
    draft: &MessageDraft,
    schema: Option<&Value>,
) -> Result<Value, FeatureHostError> {
    let properties = schema
        .and_then(|schema| schema.get("properties"))
        .and_then(Value::as_object);
    match draft {
        MessageDraft::Email {
            from,
            to,
            cc,
            subject,
            body,
            ..
        } => {
            if to.is_empty() {
                return Err(FeatureHostError::Contract(
                    "email draft requires at least one recipient".into(),
                ));
            }
            let mut arguments = serde_json::Map::new();
            if schema_property_is_array(schema, "to") {
                arguments.insert("to".into(), json!(to));
            } else {
                arguments.insert("to".into(), Value::String(to.join(", ")));
            }
            arguments.insert("subject".into(), Value::String(subject.clone()));
            if properties.is_some_and(|properties| properties.contains_key("payload")) {
                arguments.insert(
                    "payload".into(),
                    json!({
                        "mime_type": "text/plain",
                        "charset": "UTF-8",
                        "body": {"content": body}
                    }),
                );
            } else if properties.is_some_and(|properties| properties.contains_key("message")) {
                arguments.insert("message".into(), Value::String(body.clone()));
            } else {
                arguments.insert("body".into(), Value::String(body.clone()));
            }
            if let Some(cc) = cc.as_ref().filter(|cc| !cc.is_empty()) {
                if schema_property_is_array(schema, "cc") {
                    arguments.insert("cc".into(), json!(cc));
                } else {
                    arguments.insert("cc".into(), Value::String(cc.join(", ")));
                }
            }
            if let Some(from) = from.as_ref().filter(|from| !from.trim().is_empty()) {
                let key = if properties
                    .is_some_and(|properties| properties.contains_key("from_address"))
                {
                    "from_address"
                } else {
                    "from"
                };
                if properties.is_none_or(|properties| properties.contains_key(key)) {
                    arguments.insert(key.into(), Value::String(from.clone()));
                }
            }
            Ok(Value::Object(arguments))
        }
        MessageDraft::Slack {
            target,
            thread,
            body,
            ..
        } => {
            if target.trim().is_empty() || body.trim().is_empty() {
                return Err(FeatureHostError::Contract(
                    "Slack draft requires a target and message".into(),
                ));
            }
            let properties = properties.ok_or_else(|| {
                FeatureHostError::Contract(
                    "Slack connector did not expose an input schema for its send tool".into(),
                )
            })?;
            let target_key = [
                "channel",
                "channel_id",
                "target",
                "conversation",
                "conversation_id",
            ]
            .into_iter()
            .find(|key| properties.contains_key(*key))
            .ok_or_else(|| {
                FeatureHostError::Contract(
                    "Slack send tool has no supported channel/target parameter".into(),
                )
            })?;
            let body_key = ["text", "message", "body"]
                .into_iter()
                .find(|key| properties.contains_key(*key))
                .ok_or_else(|| {
                    FeatureHostError::Contract(
                        "Slack send tool has no supported message parameter".into(),
                    )
                })?;
            let mut arguments = serde_json::Map::new();
            arguments.insert(target_key.into(), Value::String(target.clone()));
            arguments.insert(body_key.into(), Value::String(body.clone()));
            if let Some(thread) = thread.as_ref().filter(|thread| !thread.trim().is_empty())
                && let Some(thread_key) = ["thread_ts", "thread", "thread_id"]
                    .into_iter()
                    .find(|key| properties.contains_key(*key))
            {
                arguments.insert(thread_key.into(), Value::String(thread.clone()));
            }
            Ok(Value::Object(arguments))
        }
    }
}

#[cfg(feature = "production")]
fn merge_live_connectors(
    mut connectors: Vec<ConnectorSummary>,
    live: &BTreeMap<String, LiveConnectorProjection>,
) -> Vec<ConnectorSummary> {
    for connector in &mut connectors {
        let Some(projection) = live.get(&connector.id) else {
            if connector.id == "git" {
                connector.status = ConnectorStatus::Connected;
                connector.can_add_account = false;
            }
            continue;
        };
        if let Some(status) = projection.status {
            connector.status = status;
        }
        connector.can_add_account = projection.install_url.is_some()
            || projection
                .server_name
                .as_deref()
                .is_some_and(|name| name != "codex_apps");
        if let Some(source) = projection
            .connector_id
            .as_ref()
            .or(projection.server_name.as_ref())
        {
            connector.source = Some(source.clone());
        }
        let enabled_preferences = connector
            .tools
            .iter()
            .map(|tool| (tool.id.clone(), tool.enabled))
            .collect::<BTreeMap<_, _>>();
        if !projection.tools.is_empty() {
            connector.tools = projection
                .tools
                .values()
                .cloned()
                .map(|mut tool| {
                    if let Some(enabled) = enabled_preferences.get(&tool.id) {
                        tool.enabled = *enabled;
                    }
                    tool
                })
                .collect();
        }
        if !projection.accounts.is_empty() {
            let labels = connector
                .accounts
                .iter()
                .map(|account| (account.id.clone(), account.label.clone()))
                .collect::<BTreeMap<_, _>>();
            connector.accounts = projection
                .accounts
                .values()
                .cloned()
                .map(|mut account| {
                    if let Some(label) = labels.get(&account.id) {
                        account.label = label.clone();
                    }
                    account
                })
                .collect();
        } else if connector.status == ConnectorStatus::Connected
            && connector.accounts.is_empty()
            && projection.server_name.as_deref() != Some("codex_apps")
        {
            connector.accounts.push(ConnectorAccountSummary {
                id: format!("mcp:{}", connector.id),
                label: projection
                    .server_name
                    .clone()
                    .unwrap_or_else(|| connector.display_name.clone()),
                status: ConnectorStatus::Connected,
                email: None,
                team_managed: Some(false),
                error: None,
            });
        }
    }
    connectors.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    connectors
}

fn listener_platform_slug(platform: ListenerPlatform) -> &'static str {
    match platform {
        ListenerPlatform::Slack => "slack",
        ListenerPlatform::Github => "github",
        ListenerPlatform::Git => "git",
        ListenerPlatform::Teams => "teams",
        ListenerPlatform::Linear => "linear",
        ListenerPlatform::Sentry => "sentry",
        ListenerPlatform::Pagerduty => "pagerduty",
    }
}

fn listener_platform_display(platform: ListenerPlatform) -> &'static str {
    match platform {
        ListenerPlatform::Slack => "Slack",
        ListenerPlatform::Github => "GitHub",
        ListenerPlatform::Git => "Git",
        ListenerPlatform::Teams => "Microsoft Teams",
        ListenerPlatform::Linear => "Linear",
        ListenerPlatform::Sentry => "Sentry",
        ListenerPlatform::Pagerduty => "PagerDuty",
    }
}

fn automation_next_run(
    trigger: &AutomationTrigger,
    schedule: &str,
    enabled: bool,
    after_ms: i64,
) -> Option<i64> {
    if !enabled || matches!(trigger, AutomationTrigger::Event { .. }) {
        None
    } else {
        next_automation_run(schedule, after_ms)
    }
}

fn connector_tool(id: &str, name: &str, description: &str) -> ConnectorToolSummary {
    ConnectorToolSummary {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        enabled: true,
        requires_approval: Some(true),
    }
}

fn connector_summary(
    id: &str,
    display_name: &str,
    description: &str,
    transport: ConnectorTransport,
    tools: Vec<ConnectorToolSummary>,
) -> ConnectorSummary {
    ConnectorSummary {
        id: id.into(),
        display_name: display_name.into(),
        description: description.into(),
        status: ConnectorStatus::Disconnected,
        is_team: false,
        can_add_account: true,
        transport,
        source: Some("Built in".into()),
        teammate_count: None,
        accounts: Vec::new(),
        tools,
    }
}

fn default_connectors() -> BTreeMap<String, ConnectorSummary> {
    [
        connector_summary(
            "github",
            "GitHub",
            "Repositories, pull requests, issues, comments and CI.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_repository",
                    "Read repository",
                    "Read repository files and metadata.",
                ),
                connector_tool(
                    "create_issue",
                    "Create issue",
                    "Create and update GitHub issues.",
                ),
                connector_tool(
                    "comment_pull_request",
                    "Comment on pull request",
                    "Post review comments on pull requests.",
                ),
            ],
        ),
        connector_summary(
            "slack",
            "Slack",
            "Messages, mentions, reactions and approved drafts.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "search_messages",
                    "Search messages",
                    "Search workspace messages and threads.",
                ),
                connector_tool(
                    "post_message",
                    "Post message",
                    "Send an approved message or thread reply.",
                ),
                connector_tool(
                    "add_reaction",
                    "Add reaction",
                    "Add a reaction to a message.",
                ),
            ],
        ),
        connector_summary(
            "teams",
            "Microsoft Teams",
            "Teams messages, mentions, channels and approved drafts.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "search_messages",
                    "Search messages",
                    "Search Teams channels and chats.",
                ),
                connector_tool(
                    "post_message",
                    "Post message",
                    "Send an approved Teams message.",
                ),
            ],
        ),
        connector_summary(
            "linear",
            "Linear",
            "Issues, comments, status changes and projects.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_issues",
                    "Read issues",
                    "Read Linear issues and projects.",
                ),
                connector_tool(
                    "update_issue",
                    "Update issue",
                    "Update issue state, assignee and fields.",
                ),
            ],
        ),
        connector_summary(
            "sentry",
            "Sentry",
            "Errors, regressions, releases and issue ownership.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_issues",
                    "Read issues",
                    "Read Sentry issues and events.",
                ),
                connector_tool(
                    "resolve_issue",
                    "Resolve issue",
                    "Resolve or assign a Sentry issue.",
                ),
            ],
        ),
        connector_summary(
            "pagerduty",
            "PagerDuty",
            "Incidents, acknowledgements, responders and escalation.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_incidents",
                    "Read incidents",
                    "Read incident details and timelines.",
                ),
                connector_tool(
                    "acknowledge_incident",
                    "Acknowledge incident",
                    "Acknowledge an incident after approval.",
                ),
            ],
        ),
        connector_summary(
            "git",
            "Git",
            "Local commits, branches and repository state.",
            ConnectorTransport::Command,
            vec![
                connector_tool(
                    "read_status",
                    "Read status",
                    "Read local repository status.",
                ),
                connector_tool(
                    "read_history",
                    "Read history",
                    "Read commit and branch history.",
                ),
            ],
        ),
    ]
    .into_iter()
    .map(|connector| (connector.id.clone(), connector))
    .collect()
}

fn default_skill_teams() -> Vec<SkillTeamSummary> {
    vec![SkillTeamSummary {
        id: "team-mahayana".into(),
        name: "Mahayana Team".into(),
    }]
}

fn default_skills() -> BTreeMap<String, SkillSummary> {
    [
        SkillSummary {
            id: "skill-research-brief".into(),
            name: "Research brief".into(),
            description: "Turn verified sources into a concise research brief.".into(),
            use_when: "Use when a task needs sourced research and a decision-ready summary.".into(),
            instructions: "Verify sources, distinguish facts from inference, and end with actionable conclusions.".into(),
            source: SkillSource::Private,
            publish_state: SkillPublishState::Local,
            owner_agent_id: Some("mahayana-assistant".into()),
            team_id: None,
            team_name: None,
            read_only: Some(false),
            updated_at_ms: 0,
        },
        SkillSummary {
            id: "skill-incident-response".into(),
            name: "Incident response".into(),
            description: "Coordinate incident triage across monitoring and communication tools.".into(),
            use_when: "Use when an alert or incident needs coordinated triage.".into(),
            instructions: "Establish severity, collect evidence, propose actions, and request approval before external changes.".into(),
            source: SkillSource::Team,
            publish_state: SkillPublishState::Managed,
            owner_agent_id: None,
            team_id: Some("team-mahayana".into()),
            team_name: Some("Mahayana Team".into()),
            read_only: Some(true),
            updated_at_ms: 0,
        },
    ]
    .into_iter()
    .map(|skill| (skill.id.clone(), skill))
    .collect()
}

fn default_bots() -> BTreeMap<String, BotSummary> {
    [
        BotSummary {
            id: "mahayana-assistant".into(),
            name: "大乘助手".into(),
            description: "General-purpose Mahayana assistant.".into(),
            title: String::new(),
            hidden: false,
            avatar: None,
            avatar_shape: None,
            avatar_color: None,
            notifications_enabled: true,
            notify_on_updates: true,
            unread: false,
            conversation_id: Some("codex:agent:assistant".into()),
        },
        BotSummary {
            id: "research-bot".into(),
            name: "Research Bot".into(),
            description: "Source verification and research synthesis.".into(),
            title: String::new(),
            hidden: false,
            avatar: None,
            avatar_shape: None,
            avatar_color: None,
            notifications_enabled: true,
            notify_on_updates: true,
            unread: false,
            conversation_id: Some("codex:agent:research".into()),
        },
        BotSummary {
            id: "incident-bot".into(),
            name: "Incident Bot".into(),
            description: "Incident triage and operational coordination.".into(),
            title: String::new(),
            hidden: true,
            avatar: None,
            avatar_shape: None,
            avatar_color: None,
            notifications_enabled: true,
            notify_on_updates: true,
            unread: false,
            conversation_id: Some("codex:agent:incident".into()),
        },
    ]
    .into_iter()
    .map(|bot| (bot.id.clone(), bot))
    .collect()
}

fn listener_summary(
    platform: ListenerPlatform,
    display_name: &str,
    blurb: &str,
) -> ListenerIntegrationSummary {
    ListenerIntegrationSummary {
        platform,
        display_name: display_name.into(),
        blurb: blurb.into(),
        is_connected: false,
        account_label: None,
        error: None,
    }
}

fn default_listeners() -> BTreeMap<ListenerPlatform, ListenerIntegrationSummary> {
    [
        listener_summary(
            ListenerPlatform::Github,
            "GitHub",
            "Let automations watch a repo's PRs, comments, issues, and CI.",
        ),
        listener_summary(
            ListenerPlatform::Git,
            "Git",
            "Wake automations on local commits, branches, tags, and repository changes.",
        ),
        listener_summary(
            ListenerPlatform::Slack,
            "Slack",
            "Wake automations on Slack messages, mentions, and reactions.",
        ),
        listener_summary(
            ListenerPlatform::Teams,
            "Microsoft Teams",
            "Wake automations on Teams messages, mentions, and reactions.",
        ),
        listener_summary(
            ListenerPlatform::Linear,
            "Linear",
            "Wake automations on issues, comments, status changes, and assignments.",
        ),
        listener_summary(
            ListenerPlatform::Sentry,
            "Sentry",
            "Wake automations on new, regressed, assigned, and resolved issues.",
        ),
        listener_summary(
            ListenerPlatform::Pagerduty,
            "PagerDuty",
            "Wake automations when incidents are triggered, acknowledged, escalated, or resolved.",
        ),
    ]
    .into_iter()
    .map(|integration| (integration.platform, integration))
    .collect()
}

fn listener_platform_for_connector(connector_id: &str) -> Option<ListenerPlatform> {
    match connector_id {
        "github" => Some(ListenerPlatform::Github),
        "git" => Some(ListenerPlatform::Git),
        "slack" => Some(ListenerPlatform::Slack),
        "teams" => Some(ListenerPlatform::Teams),
        "linear" => Some(ListenerPlatform::Linear),
        "sentry" => Some(ListenerPlatform::Sentry),
        "pagerduty" => Some(ListenerPlatform::Pagerduty),
        _ => None,
    }
}

fn connector_for_listener_platform(platform: ListenerPlatform) -> Option<&'static str> {
    match platform {
        ListenerPlatform::Github => Some("github"),
        ListenerPlatform::Git => Some("git"),
        ListenerPlatform::Slack => Some("slack"),
        ListenerPlatform::Teams => Some("teams"),
        ListenerPlatform::Linear => Some("linear"),
        ListenerPlatform::Sentry => Some("sentry"),
        ListenerPlatform::Pagerduty => Some("pagerduty"),
    }
}

fn validate_draft(draft: &MessageDraft) -> Result<(), FeatureHostError> {
    match draft {
        MessageDraft::Email {
            to, subject, body, ..
        } => {
            if to.is_empty()
                || to
                    .iter()
                    .any(|recipient| !recipient.contains('@') || recipient.trim().is_empty())
            {
                return Err(FeatureHostError::Contract(
                    "email draft requires valid recipients".into(),
                ));
            }
            required(subject.clone(), "email subject")?;
            required(body.clone(), "email body")?;
        }
        MessageDraft::Slack { target, body, .. } => {
            required(target.clone(), "Slack target")?;
            required(body.clone(), "Slack body")?;
        }
    }
    Ok(())
}

#[cfg(feature = "production")]
fn product_surface_method(command: &FeatureCommand) -> &'static str {
    match command {
        FeatureCommand::ConnectorList { .. } => "mahayana.connector.list",
        FeatureCommand::ConnectorConnect { .. } => "mahayana.connector.connect",
        FeatureCommand::ConnectorRenameAccount { .. } => "mahayana.connector.account.rename",
        FeatureCommand::ConnectorRemoveAccount { .. } => "mahayana.connector.account.remove",
        FeatureCommand::ConnectorSetToolEnabled { .. } => "mahayana.connector.tool.setEnabled",
        FeatureCommand::SkillList { .. } => "mahayana.skill.list",
        FeatureCommand::SkillUpsert { .. } => "mahayana.skill.upsert",
        FeatureCommand::SkillDelete { .. } => "mahayana.skill.delete",
        FeatureCommand::SkillPublish { .. } => "mahayana.skill.publish",
        FeatureCommand::SkillUnpublish { .. } => "mahayana.skill.unpublish",
        FeatureCommand::SkillSync { .. } => "mahayana.skill.sync",
        FeatureCommand::BotList { .. } => "mahayana.bot.list",
        FeatureCommand::BotSetHidden { .. } => "mahayana.bot.setHidden",
        FeatureCommand::DraftResolve { .. } => "mahayana.draft.resolve",
        FeatureCommand::SecretProvide { .. } => "mahayana.secret.provide",
        FeatureCommand::ListenerList { .. } => "mahayana.listener.list",
        FeatureCommand::ListenerConnect { .. } => "mahayana.listener.connect",
        FeatureCommand::ListenerDisconnect { .. } => "mahayana.listener.disconnect",
        FeatureCommand::UpdateStatus { .. } => "mahayana.update.status",
        FeatureCommand::UpdateCheck { .. } => "mahayana.update.check",
        FeatureCommand::UpdateInstall { .. } => "mahayana.update.install",
        _ => unreachable!("non-product command has no product method"),
    }
}

#[cfg(feature = "production")]
fn decode_product_value<T: DeserializeOwned>(
    value: Value,
    method: &str,
) -> Result<T, FeatureHostError> {
    serde_json::from_value(value)
        .map_err(|error| FeatureHostError::Contract(format!("decode {method} response: {error}")))
}

#[cfg(feature = "production")]
fn decode_product_field<T: DeserializeOwned>(
    value: Value,
    field: &str,
    method: &str,
) -> Result<T, FeatureHostError> {
    let value = value.get(field).cloned().unwrap_or(value);
    decode_product_value(value, method)
}

fn ensure_open(state: &FeatureState) -> Result<(), FeatureHostError> {
    if state.closed {
        Err(FeatureHostError::Closed)
    } else {
        Ok(())
    }
}

fn next_id(state: &mut FeatureState, prefix: &str) -> String {
    state.sequence += 1;
    format!("{prefix}-{}", state.sequence)
}

const MAX_TRAYS: usize = 20;

fn push_error_tray(
    state: &mut FeatureState,
    agent_id: String,
    title: String,
    detail: Option<String>,
    request_id: Option<String>,
    dedupe_key: Option<String>,
) -> ErrorTray {
    let now = now_millis();
    if let Some(key) = dedupe_key.as_deref() {
        if let Some(index) = state
            .trays
            .iter()
            .position(|tray| tray.kind == "error" && tray.dedupe_key.as_deref() == Some(key))
        {
            let mut updated = state.trays[index].clone();
            updated.agent_id = agent_id;
            updated.title = title;
            updated.detail = detail;
            updated.request_id = request_id;
            updated.count = Some(updated.count.unwrap_or(1).saturating_add(1));
            updated.created_at = now;
            updated.error_kind = None;
            updated.raw_detail = None;
            updated.actions = None;
            state.trays[index] = updated.clone();
            state.events.push_back(HostEvent::TrayChanged {
                timestamp: timestamp(),
                action: "pushed".into(),
                tray: Some(updated.clone()),
                id: None,
            });
            return updated;
        }
    }
    let tray = ErrorTray {
        kind: "error".into(),
        id: next_id(state, "tray"),
        agent_id,
        title,
        detail,
        request_id,
        created_at: now,
        error_kind: None,
        raw_detail: None,
        actions: None,
        dedupe_key,
        count: None,
    };
    let mut tray = tray;
    if tray.dedupe_key.is_some() {
        tray.count = Some(1);
    }
    state.trays.push(tray.clone());
    state.events.push_back(HostEvent::TrayChanged {
        timestamp: timestamp(),
        action: "pushed".into(),
        tray: Some(tray.clone()),
        id: None,
    });
    if state.trays.len() > MAX_TRAYS {
        let overflow = state.trays.len() - MAX_TRAYS;
        let dropped = state.trays.drain(0..overflow).collect::<Vec<_>>();
        for dropped in dropped {
            state.events.push_back(HostEvent::TrayChanged {
                timestamp: timestamp(),
                action: "dismissed".into(),
                tray: None,
                id: Some(dropped.id),
            });
        }
    }
    tray
}

fn validate_config(config: &HostConfig) -> Result<(), FeatureHostError> {
    if config.profile_id.trim().is_empty() {
        Err(FeatureHostError::Contract(
            "profileId must not be empty".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(feature = "production")]
fn unexpected_response(command: &str, response: RuntimeResponse) -> FeatureHostError {
    FeatureHostError::Contract(format!(
        "unexpected Runtime response for {command}: {response:?}"
    ))
}

fn required(value: String, name: &str) -> Result<String, FeatureHostError> {
    let value = value.trim();
    if value.is_empty() {
        Err(FeatureHostError::Contract(format!(
            "{name} must not be empty"
        )))
    } else {
        Ok(value.to_string())
    }
}

// Shared Fabushi text-shaping and profile semantics.
fn clamp_line(raw: &str, max_length: usize) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_length)
        .collect()
}

fn clamp_block(raw: &str, max_length: usize) -> String {
    raw.trim().chars().take(max_length).collect()
}

fn attachment_byte_limit_for_name(name: &str) -> u64 {
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" | "mpg" | "mpeg"
    ) {
        VIDEO_BYTE_LIMIT
    } else {
        ATTACHMENT_BYTE_LIMIT
    }
}

fn resolve_agent_attachment_path(
    agent_root: &Path,
    agent_id: &str,
    raw_path: &str,
) -> Result<PathBuf, FeatureHostError> {
    if !is_safe_memory_agent_id(agent_id) {
        return Err(FeatureHostError::Contract(
            "invalid attachment owner".into(),
        ));
    }
    let base = agent_root.join(agent_id).join("attachments");
    let base = std::fs::canonicalize(&base).map_err(|error| {
        FeatureHostError::Contract(format!("attachment directory unavailable: {error}"))
    })?;
    let candidate = {
        let path = PathBuf::from(raw_path);
        if path.is_absolute() {
            path
        } else {
            base.join(path)
        }
    };
    let candidate = std::fs::canonicalize(&candidate).map_err(|error| {
        FeatureHostError::Contract(format!("attachment path unavailable: {error}"))
    })?;
    if !candidate.starts_with(&base) {
        return Err(FeatureHostError::Contract(
            "attachment path escapes the agent attachment directory".into(),
        ));
    }
    Ok(candidate)
}

fn read_file_prefix(path: &Path, max_bytes: usize) -> Result<Vec<u8>, FeatureHostError> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| FeatureHostError::Contract(format!("open attachment: {error}")))?;
    let mut buffer = vec![0u8; max_bytes];
    let bytes_read = file
        .read(&mut buffer)
        .map_err(|error| FeatureHostError::Contract(format!("read attachment: {error}")))?;
    buffer.truncate(bytes_read);
    Ok(buffer)
}

fn read_file_range(path: &Path, offset: u64, length: usize) -> Result<Vec<u8>, FeatureHostError> {
    if length == 0 {
        return Ok(Vec::new());
    }
    let mut file = std::fs::File::open(path)
        .map_err(|error| FeatureHostError::Contract(format!("open attachment: {error}")))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| FeatureHostError::Contract(format!("seek attachment: {error}")))?;
    let mut buffer = vec![0u8; length];
    let bytes_read = file
        .read(&mut buffer)
        .map_err(|error| FeatureHostError::Contract(format!("read attachment range: {error}")))?;
    buffer.truncate(bytes_read);
    Ok(buffer)
}

fn is_text_previewable_name(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "mdc"
            | "csv"
            | "tsv"
            | "json"
            | "jsonl"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "htm"
            | "css"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "py"
            | "rs"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "log"
            | "sql"
            | "ini"
            | "conf"
    )
}

fn looks_like_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }
    let control = bytes
        .iter()
        .filter(|byte| **byte < 0x09 || (**byte > 0x0d && **byte < 0x20))
        .count();
    control * 100 / bytes.len() > 5
}

fn image_dimensions(bytes: &[u8], mime: &str) -> (Option<u32>, Option<u32>) {
    if mime == "image/png" && bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return (Some(width), Some(height));
    }
    if mime == "image/gif"
        && bytes.len() >= 10
        && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a")
    {
        let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return (Some(width), Some(height));
    }
    if mime == "image/jpeg" && bytes.len() > 4 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        let mut index = 2usize;
        while index + 8 < bytes.len() {
            if bytes[index] != 0xff {
                index += 1;
                continue;
            }
            let marker = bytes[index + 1];
            index += 2;
            if marker == 0xd8 || marker == 0xd9 || marker == 0x01 || (0xd0..=0xd7).contains(&marker)
            {
                continue;
            }
            if index + 2 > bytes.len() {
                break;
            }
            let segment_length = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
            if segment_length < 2 || index + segment_length > bytes.len() {
                break;
            }
            if matches!(
                marker,
                0xc0 | 0xc1
                    | 0xc2
                    | 0xc3
                    | 0xc5
                    | 0xc6
                    | 0xc7
                    | 0xc9
                    | 0xca
                    | 0xcb
                    | 0xcd
                    | 0xce
                    | 0xcf
            ) && segment_length >= 7
            {
                let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]) as u32;
                let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
                return (Some(width), Some(height));
            }
            index += segment_length;
        }
    }
    (None, None)
}

fn build_content_snippet(text: &str, normalized_query: &str) -> Option<String> {
    if normalized_query.is_empty() {
        return None;
    }
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = flat.to_ascii_lowercase();
    let query = normalized_query.to_ascii_lowercase();
    let byte_index = lower.find(&query)?;
    let match_start = flat[..byte_index].chars().count();
    let match_len = flat[byte_index..byte_index + query.len()].chars().count();
    let chars = flat.chars().collect::<Vec<_>>();
    let start = match_start.saturating_sub(SEARCH_SNIPPET_LEAD);
    let end = (match_start + match_len + SEARCH_SNIPPET_TRAIL).min(chars.len());
    let core = chars[start..end].iter().collect::<String>();
    Some(format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        core,
        if end < chars.len() { "…" } else { "" }
    ))
}

fn collect_agent_media_matches(
    root: &Path,
    agent_id: &str,
    agent_name: &str,
    normalized_query: &str,
    out: &mut Vec<SearchMediaMatch>,
) {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 3 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !normalized_query.is_empty() && !name.to_ascii_lowercase().contains(normalized_query)
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let timestamp_ms = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(0);
            out.push(SearchMediaMatch {
                agent_id: agent_id.to_string(),
                agent_name: agent_name.to_string(),
                path: path.to_string_lossy().to_string(),
                mime_type: media_mime_type(&name).map(str::to_string),
                name,
                size_bytes: metadata.len(),
                timestamp_ms,
            });
        }
    }
}

fn media_mime_type(name: &str) -> Option<&'static str> {
    let extension = Path::new(name)
        .extension()?
        .to_string_lossy()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        "txt" | "md" | "markdown" | "log" => Some("text/plain"),
        "json" => Some("application/json"),
        "csv" => Some("text/csv"),
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "mp4" => Some("video/mp4"),
        "mov" => Some("video/quicktime"),
        _ => None,
    }
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

fn sanitize_avatar_data_url(value: Option<String>) -> Result<Option<String>, FeatureHostError> {
    const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let (header, payload) = value.split_once(',').ok_or_else(|| {
        FeatureHostError::Contract("avatar must be a base64 image data URL".into())
    })?;
    if !matches!(
        header,
        "data:image/png;base64"
            | "data:image/jpeg;base64"
            | "data:image/webp;base64"
            | "data:image/gif;base64"
    ) {
        return Err(FeatureHostError::Contract(
            "avatar format must be PNG, JPEG, WebP, or GIF".into(),
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|_| FeatureHostError::Contract("avatar base64 payload is invalid".into()))?;
    if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
        return Err(FeatureHostError::Contract(format!(
            "avatar must be between 1 byte and {MAX_AVATAR_BYTES} bytes"
        )));
    }
    Ok(Some(value.to_string()))
}

fn clone_agent_display_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "copy".into()
    } else {
        format!("{trimmed} copy")
    }
}

fn compose_agent_input(
    text: &str,
    mode: AgentMode,
    mode_statement: Option<&str>,
    attachments: &[AttachmentContext],
) -> String {
    if mode == AgentMode::Agent && attachments.is_empty() {
        return text.to_string();
    }
    let mode_instruction = match mode {
        AgentMode::Agent => "请自主使用可用工具完成任务，并明确报告结果。",
        AgentMode::Ask => {
            "请只分析并回答问题；未经用户明确要求，不要修改文件或执行有副作用的操作。"
        }
        AgentMode::Plan => "请先形成可执行计划，列出依赖、风险和验证方式；暂不执行有副作用的操作。",
    };
    let mut input = format!(
        "[Agent 模式]\n{}\n{mode_instruction}\n\n[用户请求]\n{text}",
        mode_statement.unwrap_or("")
    );
    for attachment in attachments {
        input.push_str("\n\n[附件: ");
        input.push_str(&attachment.name);
        input.push_str("]\n");
        if let Some(path) = attachment.path.as_deref() {
            input.push_str("持久文件路径：");
            input.push_str(path);
            if let Some(size_bytes) = attachment.size_bytes {
                input.push_str(&format!("\n文件大小：{size_bytes} bytes"));
            }
            if let Some(mime_type) = attachment.mime_type.as_deref() {
                input.push_str("\nMIME：");
                input.push_str(mime_type);
            }
            input.push_str("\n需要完整内容时，请直接读取上述本地文件路径。\n");
        }
        if let Some(text) = attachment.text.as_deref() {
            input.push_str("文本预览（最多 64 KiB）：\n");
            input.push_str(text);
        } else if attachment.path.is_none() {
            input.push_str("（仅提供文件元数据）");
        }
    }
    input
}

fn is_safe_automation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn render_mcp_instruction_context(
    instructions: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let mut entries = instructions
        .iter()
        .filter_map(|(server, instruction)| {
            let server = clamp_line(server, 200);
            let instruction = clamp_block(instruction, 4_000);
            (!server.is_empty() && !instruction.is_empty()).then_some((server, instruction))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries.truncate(16);
    if entries.is_empty() {
        return None;
    }
    let mut context = String::from(
        "[MCP connector operating instructions]
These are user-configured rules for the named connector. Apply them whenever using tools from that connector. They are runtime context, not user message text.
",
    );
    for (server, instruction) in entries {
        let remaining = 14_000usize.saturating_sub(context.chars().count());
        if remaining == 0 {
            break;
        }
        let block = format!(
            "
Connector: {server}
{instruction}
"
        );
        context.push_str(&clamp_block(&block, remaining));
    }
    Some(context)
}

fn cloud_task_resource_id(metadata: Option<&Value>) -> Option<String> {
    let object = metadata?.as_object()?;
    for key in ["bcId", "runId", "run_id", "cloudRunId", "cloud_run_id"] {
        if let Some(value) = object.get(key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() && value.len() <= 240 {
                return Some(value.to_string());
            }
        }
    }
    for key in ["run", "cloud", "metadata"] {
        if let Some(nested) = object.get(key) {
            if let Some(value) = cloud_task_resource_id(Some(nested)) {
                return Some(value);
            }
        }
    }
    None
}

fn ensure_automation_agent_scope(
    automation: &AutomationSummary,
    agent_id: Option<&str>,
) -> Result<(), FeatureHostError> {
    let Some(agent_id) = agent_id else {
        return Ok(());
    };
    if automation.agent_id.as_deref() == Some(agent_id) {
        return Ok(());
    }
    Err(FeatureHostError::Contract(format!(
        "automation {} does not belong to agent {agent_id}",
        automation.id
    )))
}

const MEMORY_PROFILE_HEADER: &str = "# About the user\n\n<!-- Enduring facts: who the user is, how to address them, lasting preferences.\n     Kept in mind every turn. Safe to read, grep, and edit.\n     One fact per line, as \"- (YYYY-MM-DD) <fact>\". -->\n";
const MEMORY_LOG_HEADER: &str = "# Memory log\n\n<!-- Dated facts, one per line as \"- (YYYY-MM-DD) <fact>\". Safe to read, grep, and edit. -->\n";
const MEMORY_MAX_CONTENT_LENGTH: usize = 500;
const MEMORY_PROFILE_PROMPT_LIMIT: usize = 100;
const MEMORY_RECENT_PROMPT_LIMIT: usize = 30;
const MEMORY_RECENT_PROMPT_CHAR_BUDGET: usize = 4000;
const MEMORY_DECAY_HALF_LIFE_DAYS: f64 = 30.0;

#[derive(Clone)]
struct ParsedMemoryFact {
    record: MemoryRecord,
    path: PathBuf,
    line_index: usize,
    order: usize,
}

fn is_safe_memory_agent_id(agent_id: &str) -> bool {
    !agent_id.is_empty()
        && agent_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xefcdab89;
    let mut h2: u32 = 0x98badcfe;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xc3d2e1f0;
    let bit_len = (input.len() as u64) * 8;
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }
    let mut output = [0u8; 20];
    for (index, word) in [h0, h1, h2, h3, h4].into_iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

fn normalize_memory_content(raw: &str) -> String {
    clamp_line(raw, MEMORY_MAX_CONTENT_LENGTH)
}

fn memory_dedupe_key(content: &str) -> String {
    normalize_memory_content(content).to_lowercase()
}

fn memory_id_for(content: &str) -> String {
    sha1_digest(memory_dedupe_key(content).as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn format_memory_date(created_at_ms: i64) -> String {
    if created_at_ms <= 0 {
        return "unknown date".into();
    }
    Utc.timestamp_millis_opt(created_at_ms)
        .single()
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown date".into())
}

fn memory_log_files(memory_dir: &Path) -> Vec<PathBuf> {
    let log_dir = memory_dir.join("log");
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn read_memory_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn write_memory_atomic(path: &Path, content: &str) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create memory directory: {error}"))
        })?;
    }
    let temp = path.with_extension("md.tmp");
    std::fs::write(&temp, content)
        .map_err(|error| FeatureHostError::Contract(format!("write memory: {error}")))?;
    std::fs::rename(&temp, path)
        .map_err(|error| FeatureHostError::Contract(format!("commit memory: {error}")))?;
    Ok(())
}

fn parse_memory_facts(
    raw: &str,
    kind: MemoryKind,
    base: usize,
    path: &Path,
) -> Vec<ParsedMemoryFact> {
    let mut facts = Vec::new();
    let mut order = base;
    for (line_index, line) in raw.lines().enumerate() {
        let line = line.trim_end();
        if !line.starts_with("- (") {
            continue;
        }
        let Some(close) = line[3..].find(')') else {
            continue;
        };
        let close = close + 3;
        let date = &line[3..close];
        if date.len() != 10 || !line[close + 1..].starts_with(' ') {
            continue;
        }
        let content = normalize_memory_content(line[close + 2..].trim());
        if content.is_empty() {
            continue;
        }
        let created_at = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .map(|date| date.and_utc().timestamp_millis())
            .unwrap_or(0);
        facts.push(ParsedMemoryFact {
            record: MemoryRecord {
                id: memory_id_for(&content),
                content,
                created_at,
                kind,
            },
            path: path.to_path_buf(),
            line_index,
            order,
        });
        order += 1;
    }
    facts
}

fn all_memory_facts(memory_dir: &Path) -> Vec<ParsedMemoryFact> {
    let profile = memory_dir.join("profile.md");
    let mut facts = parse_memory_facts(
        &read_memory_text(&profile),
        MemoryKind::Profile,
        0,
        &profile,
    );
    for path in memory_log_files(memory_dir) {
        let base = facts.len();
        facts.extend(parse_memory_facts(
            &read_memory_text(&path),
            MemoryKind::Log,
            base,
            &path,
        ));
    }
    facts
}

fn sort_memories_most_recent(facts: &mut [ParsedMemoryFact]) {
    facts.sort_by(|a, b| {
        b.record
            .created_at
            .cmp(&a.record.created_at)
            .then_with(|| b.order.cmp(&a.order))
    });
}

fn list_memories(memory_dir: &Path, limit: usize) -> Result<Vec<MemoryRecord>, FeatureHostError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut profile = all_memory_facts(memory_dir)
        .into_iter()
        .filter(|fact| fact.record.kind == MemoryKind::Profile)
        .collect::<Vec<_>>();
    let mut logs = all_memory_facts(memory_dir)
        .into_iter()
        .filter(|fact| fact.record.kind == MemoryKind::Log)
        .collect::<Vec<_>>();
    sort_memories_most_recent(&mut profile);
    sort_memories_most_recent(&mut logs);
    Ok(profile
        .into_iter()
        .chain(logs)
        .take(limit)
        .map(|fact| fact.record)
        .collect())
}

fn count_memories(memory_dir: &Path) -> Result<usize, FeatureHostError> {
    Ok(all_memory_facts(memory_dir).len())
}

fn add_memory(
    memory_dir: &Path,
    content: &str,
    created_at: i64,
    kind: MemoryKind,
) -> Result<Option<MemoryRecord>, FeatureHostError> {
    let content = normalize_memory_content(content);
    if content.is_empty() {
        return Ok(None);
    }
    let key = memory_dedupe_key(&content);
    if all_memory_facts(memory_dir)
        .iter()
        .any(|fact| memory_dedupe_key(&fact.record.content) == key)
    {
        return Ok(None);
    }
    let path = match kind {
        MemoryKind::Profile => memory_dir.join("profile.md"),
        MemoryKind::Log => {
            let bucket = format_memory_date(created_at)
                .chars()
                .take(7)
                .collect::<String>();
            memory_dir.join("log").join(format!("{bucket}.md"))
        }
    };
    let header = match kind {
        MemoryKind::Profile => MEMORY_PROFILE_HEADER,
        MemoryKind::Log => MEMORY_LOG_HEADER,
    };
    let raw = read_memory_text(&path);
    let base = if raw.is_empty() {
        header.to_string()
    } else {
        raw
    };
    let separator = if base.ends_with('\n') || base.is_empty() {
        ""
    } else {
        "\n"
    };
    let line = format!("- ({}) {}", format_memory_date(created_at), content);
    write_memory_atomic(&path, &format!("{base}{separator}{line}\n"))?;
    Ok(Some(MemoryRecord {
        id: memory_id_for(&content),
        content,
        created_at,
        kind,
    }))
}

fn remove_memory(memory_dir: &Path, id: &str) -> Result<bool, FeatureHostError> {
    let mut paths = vec![memory_dir.join("profile.md")];
    paths.extend(memory_log_files(memory_dir));
    for path in paths {
        let raw = read_memory_text(&path);
        if raw.is_empty() {
            continue;
        }
        let kind = if path.file_name().is_some_and(|name| name == "profile.md") {
            MemoryKind::Profile
        } else {
            MemoryKind::Log
        };
        let Some(fact) = parse_memory_facts(&raw, kind, 0, &path)
            .into_iter()
            .find(|fact| fact.record.id == id)
        else {
            continue;
        };
        let mut lines = raw.split('\n').map(str::to_string).collect::<Vec<_>>();
        if fact.line_index < lines.len() {
            lines.remove(fact.line_index);
            write_memory_atomic(&fact.path, &lines.join("\n"))?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn memory_importance(content: &str) -> f64 {
    if content.starts_with("[episode] ") {
        1.5
    } else if content.starts_with("[note] ") {
        0.5
    } else {
        1.0
    }
}

fn memory_recall_rank(memory: &MemoryRecord) -> f64 {
    memory_importance(&memory.content).log2()
        + memory.created_at as f64 / (MEMORY_DECAY_HALF_LIFE_DAYS * 86_400_000.0)
}

fn render_memory_system_prompt(memory_dir: &Path) -> String {
    let facts = all_memory_facts(memory_dir);
    let mut profile = facts
        .iter()
        .filter(|fact| fact.record.kind == MemoryKind::Profile)
        .cloned()
        .collect::<Vec<_>>();
    sort_memories_most_recent(&mut profile);
    profile.truncate(MEMORY_PROFILE_PROMPT_LIMIT);
    let mut recent = facts
        .into_iter()
        .filter(|fact| fact.record.kind == MemoryKind::Log)
        .collect::<Vec<_>>();
    recent.sort_by(|a, b| {
        memory_recall_rank(&b.record)
            .partial_cmp(&memory_recall_rank(&a.record))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.record.created_at.cmp(&a.record.created_at))
            .then_with(|| b.order.cmp(&a.order))
    });
    recent.truncate(MEMORY_RECENT_PROMPT_LIMIT);
    if profile.is_empty() && recent.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "Memory: durable facts you have learned about the user and their world.".to_string(),
        "These persist across every conversation with this agent, even after the chat is cleared. Rely on them so you stay consistent and avoid re-asking what you already know.".to_string(),
        format!(
            "Your memory lives in a folder at {}: profile.md holds who the user is and log/ holds dated history.",
            memory_dir.to_string_lossy()
        ),
    ];
    if !profile.is_empty() {
        lines.push("About the user:".into());
        for fact in profile {
            lines.push(format!(
                "- (learned {}) {}",
                format_memory_date(fact.record.created_at),
                fact.record.content
            ));
        }
    }
    if !recent.is_empty() {
        lines.push("Recently:".into());
        let mut budget = MEMORY_RECENT_PROMPT_CHAR_BUDGET;
        for fact in recent {
            let line = format!(
                "- (learned {}) {}",
                format_memory_date(fact.record.created_at),
                fact.record.content
            );
            if line.len() > budget {
                break;
            }
            budget -= line.len();
            lines.push(line);
        }
    }
    lines.join("\n")
}

const WORKFLOW_FILENAME: &str = "SKILL.md";
const LEGACY_WORKFLOW_FILENAME: &str = "workflow.md";
const WORKFLOW_MAX_NAME_LENGTH: usize = 80;
const WORKFLOW_MAX_DESCRIPTION_LENGTH: usize = 1536;
const WORKFLOW_MAX_BODY_LENGTH: usize = 100_000;
const WORKFLOW_UI_LIMIT: usize = 100;
const WORKFLOW_INJECTED_BODY_LIMIT: usize = 8_000;
const ATTACHMENT_BYTE_LIMIT: u64 = 25 * 1024 * 1024;
const VIDEO_BYTE_LIMIT: u64 = 200 * 1024 * 1024;
const ATTACHMENT_CHUNK_MAX_BYTES: usize = 8 * 1024 * 1024;
const ATTACHMENT_TEXT_PREVIEW_BYTE_CAP: usize = 64 * 1024;
const AGENT_CONTENT_SEARCH_MAX_MATCHES_PER_AGENT: usize = 5;
const AGENT_CONTENT_SEARCH_MAX_RESULTS: usize = 50;
const SEARCH_SNIPPET_LEAD: usize = 30;
const SEARCH_SNIPPET_TRAIL: usize = 60;
const WORKFLOW_MAX_PER_AGENT: usize = 100;
const WORKFLOW_ENABLEMENT_FILENAME: &str = "enabled-workflows.json";

#[derive(Clone)]
struct ParsedWorkflowFile {
    name: String,
    description: String,
    trigger: Option<WorkflowTrigger>,
    body: String,
    source_ref: Option<String>,
    data: serde_yaml::Mapping,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct WorkflowEnablementFile {
    #[serde(default)]
    disabled: Vec<String>,
    #[serde(default)]
    enabled: Vec<String>,
}

fn clamp_workflow_name(name: &str) -> String {
    clamp_line(name, WORKFLOW_MAX_NAME_LENGTH)
}

fn clamp_workflow_description(description: &str) -> String {
    clamp_line(description, WORKFLOW_MAX_DESCRIPTION_LENGTH)
}

fn clamp_workflow_body(body: &str) -> String {
    clamp_block(body, WORKFLOW_MAX_BODY_LENGTH)
}

fn slugify_workflow_name(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for character in name.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            if out.len() < 48 {
                out.push(character);
            }
        } else {
            pending_dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        format!("workflow-{}", now_millis())
    } else {
        out
    }
}

fn yaml_key(name: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(name.to_string())
}

fn yaml_string(data: &serde_yaml::Mapping, name: &str) -> Option<String> {
    data.get(yaml_key(name))
        .and_then(serde_yaml::Value::as_str)
        .map(str::to_string)
}

fn read_workflow_trigger(data: &serde_yaml::Mapping) -> Option<WorkflowTrigger> {
    let trigger = data.get(yaml_key("trigger"))?.as_mapping()?;
    let raw_schedule = trigger.get(yaml_key("schedule"))?.as_str()?;
    let schedule = normalize_automation_schedule(raw_schedule).ok()?;
    if schedule.is_empty() {
        return None;
    }
    let is_enabled = trigger
        .get(yaml_key("enabled"))
        .and_then(serde_yaml::Value::as_bool)
        .unwrap_or(true);
    Some(WorkflowTrigger {
        schedule,
        is_enabled,
    })
}

fn read_workflow_source_ref(data: &serde_yaml::Mapping) -> Option<String> {
    let nested = data
        .get(yaml_key("metadata"))
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|metadata| metadata.get(yaml_key("source")))
        .and_then(serde_yaml::Value::as_str);
    let raw = nested.or_else(|| {
        data.get(yaml_key("source"))
            .and_then(serde_yaml::Value::as_str)
    })?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn split_workflow_frontmatter(raw: &str) -> (serde_yaml::Mapping, String) {
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let frontmatter = &rest[..end];
            let after = &rest[end + 4..];
            let content = after.strip_prefix('\n').unwrap_or(after).to_string();
            if let Ok(serde_yaml::Value::Mapping(mapping)) =
                serde_yaml::from_str::<serde_yaml::Value>(frontmatter)
            {
                return (mapping, content);
            }
        }
    }
    (serde_yaml::Mapping::new(), raw.to_string())
}

fn parse_workflow_file(raw: &str) -> Option<ParsedWorkflowFile> {
    let (data, content) = split_workflow_frontmatter(raw);
    let body = clamp_workflow_body(&content);
    if body.is_empty() && data.is_empty() {
        return None;
    }
    Some(ParsedWorkflowFile {
        name: clamp_workflow_name(&yaml_string(&data, "name").unwrap_or_default()),
        description: clamp_workflow_description(
            &yaml_string(&data, "description").unwrap_or_default(),
        ),
        trigger: read_workflow_trigger(&data),
        body,
        source_ref: read_workflow_source_ref(&data),
        data,
    })
}

fn serialize_workflow_file(
    name: &str,
    description: &str,
    body: &str,
    trigger: Option<&WorkflowTrigger>,
    source_ref: Option<&str>,
    existing_data: Option<&serde_yaml::Mapping>,
) -> Result<String, FeatureHostError> {
    let mut data = existing_data.cloned().unwrap_or_default();
    data.insert(
        yaml_key("name"),
        serde_yaml::Value::String(name.to_string()),
    );
    if description.is_empty() {
        data.remove(yaml_key("description"));
    } else {
        data.insert(
            yaml_key("description"),
            serde_yaml::Value::String(description.to_string()),
        );
    }
    let legacy_source = data.remove(yaml_key("source"));
    let mut metadata = data
        .get(yaml_key("metadata"))
        .and_then(serde_yaml::Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    let next_source = source_ref
        .map(str::to_string)
        .or_else(|| {
            metadata
                .get(yaml_key("source"))
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| legacy_source.and_then(|value| value.as_str().map(str::to_string)));
    if let Some(source) = next_source.filter(|source| !source.is_empty()) {
        metadata.insert(yaml_key("source"), serde_yaml::Value::String(source));
    } else {
        metadata.remove(yaml_key("source"));
    }
    if metadata.is_empty() {
        data.remove(yaml_key("metadata"));
    } else {
        data.insert(yaml_key("metadata"), serde_yaml::Value::Mapping(metadata));
    }
    if let Some(trigger) = trigger {
        let mut trigger_data = serde_yaml::Mapping::new();
        trigger_data.insert(
            yaml_key("schedule"),
            serde_yaml::Value::String(trigger.schedule.clone()),
        );
        trigger_data.insert(
            yaml_key("enabled"),
            serde_yaml::Value::Bool(trigger.is_enabled),
        );
        data.insert(
            yaml_key("trigger"),
            serde_yaml::Value::Mapping(trigger_data),
        );
    }
    let mut yaml = serde_yaml::to_string(&data).map_err(|error| {
        FeatureHostError::Contract(format!("serialize workflow frontmatter: {error}"))
    })?;
    if let Some(stripped) = yaml.strip_prefix("---\n") {
        yaml = stripped.to_string();
    }
    Ok(format!("---\n{}---\n{}\n", yaml.trim_end(), body.trim()))
}

fn derive_workflow_name_from_markdown(body: &str) -> Option<String> {
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let text = if let Some(index) = line.find(char::is_whitespace) {
            if line[..index].chars().all(|character| character == '#') {
                &line[index..]
            } else {
                line
            }
        } else {
            line
        };
        let cleaned = text
            .chars()
            .filter(|character| !matches!(character, '*' | '_' | '`' | '#' | '>'))
            .collect::<String>();
        let cleaned = cleaned.trim();
        if !cleaned.is_empty() {
            return Some(clamp_workflow_name(cleaned));
        }
    }
    None
}

fn derive_workflow_name_from_source(source: &str) -> String {
    let raw = source
        .split('/')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or("Imported skill");
    let raw = [".markdown", ".mdc", ".md", ".txt"]
        .iter()
        .find_map(|suffix| raw.strip_suffix(suffix))
        .unwrap_or(raw);
    let name = raw.replace(['-', '_'], " ");
    let name = clamp_workflow_name(name.trim());
    if name.is_empty() {
        "Imported skill".into()
    } else {
        name
    }
}

fn build_live_source_pointer_body(source: &str) -> String {
    format!(
        "This workflow is a live reference to the skill at `{source}`.\nRead that source now with your file or fetch tools and follow it as written. Do not assume its contents from this note; the source is the source of truth and may have changed since this workflow was created."
    )
}

fn build_live_source_description(name: &str, source: &str) -> String {
    clamp_workflow_description(&format!(
        "Use when the \"{name}\" skill applies; it is a live reference to {source}."
    ))
}

fn workflow_enablement_path(agent_root: &Path, agent_id: &str) -> PathBuf {
    agent_root.join(agent_id).join(WORKFLOW_ENABLEMENT_FILENAME)
}

fn read_workflow_enablement(agent_root: &Path, agent_id: &str) -> WorkflowEnablementFile {
    let path = workflow_enablement_path(agent_root, agent_id);
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<WorkflowEnablementFile>(&bytes).ok())
        .unwrap_or_default()
}

fn is_workflow_enabled(agent_root: &Path, agent_id: &str, id: &str) -> bool {
    !read_workflow_enablement(agent_root, agent_id)
        .disabled
        .iter()
        .any(|disabled| disabled == id)
}

fn write_workflow_enablement(
    agent_root: &Path,
    agent_id: &str,
    mut file: WorkflowEnablementFile,
) -> Result<(), FeatureHostError> {
    file.disabled.sort();
    file.disabled.dedup();
    file.enabled.sort();
    file.enabled.dedup();
    let path = workflow_enablement_path(agent_root, agent_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create workflow enablement directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(&file).map_err(|error| {
        FeatureHostError::Contract(format!("serialize workflow enablement: {error}"))
    })?;
    std::fs::write(&temp, [body.as_slice(), b"\n"].concat()).map_err(|error| {
        FeatureHostError::Contract(format!("write workflow enablement: {error}"))
    })?;
    std::fs::rename(&temp, &path).map_err(|error| {
        FeatureHostError::Contract(format!("commit workflow enablement: {error}"))
    })?;
    Ok(())
}

fn set_workflow_enabled(
    agent_root: &Path,
    agent_id: &str,
    id: &str,
    enabled: bool,
) -> Result<(), FeatureHostError> {
    let mut file = read_workflow_enablement(agent_root, agent_id);
    let had = file.disabled.iter().any(|item| item == id);
    if enabled {
        if !had {
            return Ok(());
        }
        file.disabled.retain(|item| item != id);
    } else {
        if had {
            return Ok(());
        }
        file.disabled.push(id.to_string());
    }
    write_workflow_enablement(agent_root, agent_id, file)
}

fn forget_workflow_enablement(
    agent_root: &Path,
    agent_id: &str,
    id: &str,
) -> Result<(), FeatureHostError> {
    let mut file = read_workflow_enablement(agent_root, agent_id);
    let before = file.disabled.len();
    file.disabled.retain(|item| item != id);
    if before == file.disabled.len() {
        return Ok(());
    }
    write_workflow_enablement(agent_root, agent_id, file)
}

fn workflow_helper_scripts(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, current: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.is_file() {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                let name = relative.to_string_lossy();
                if name != WORKFLOW_FILENAME
                    && name != LEGACY_WORKFLOW_FILENAME
                    && name != "runs.json"
                {
                    out.push(path.to_string_lossy().into_owned());
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn workflow_created_at(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.created().or_else(|_| metadata.modified()).ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_else(now_millis)
}

fn load_workflow_summary(
    workflow_root: &Path,
    agent_root: &Path,
    agent_id: &str,
    id: &str,
) -> Option<WorkflowSummary> {
    let dir = workflow_root.join(id);
    let file_path = dir.join(WORKFLOW_FILENAME);
    let legacy_path = dir.join(LEGACY_WORKFLOW_FILENAME);
    if !file_path.exists() && legacy_path.exists() {
        let _ = std::fs::rename(&legacy_path, &file_path);
    }
    let raw = std::fs::read_to_string(&file_path).ok()?;
    let parsed = parse_workflow_file(&raw)?;
    let name = if parsed.name.is_empty() {
        clamp_workflow_name(id)
    } else {
        parsed.name
    };
    let disable_model_invocation = parsed
        .data
        .get(yaml_key("disable-model-invocation"))
        .and_then(serde_yaml::Value::as_bool);
    let next_run_at = parsed
        .trigger
        .as_ref()
        .filter(|trigger| trigger.is_enabled)
        .and_then(|trigger| next_automation_run(&trigger.schedule, now_millis()));
    Some(WorkflowSummary {
        id: id.to_string(),
        name,
        description: parsed.description,
        body: parsed.body,
        trigger: parsed.trigger.clone(),
        source_ref: parsed.source_ref,
        source: WorkflowSource::Workflow,
        plugin_id: None,
        published_by_current_user: false,
        is_enabled_for_agent: is_workflow_enabled(agent_root, agent_id, id),
        disable_model_invocation,
        schedule_description: parsed
            .trigger
            .as_ref()
            .map(|trigger| trigger.schedule.clone()),
        created_at: workflow_created_at(&file_path),
        last_run_at: None,
        next_run_at,
        helper_scripts: workflow_helper_scripts(&dir),
        file_path: file_path.to_string_lossy().into_owned(),
    })
}

fn list_workflow_summaries(
    workflow_root: &Path,
    agent_root: &Path,
    agent_id: &str,
) -> Vec<WorkflowSummary> {
    let Ok(entries) = std::fs::read_dir(workflow_root) else {
        return Vec::new();
    };
    let mut workflows = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            load_workflow_summary(workflow_root, agent_root, agent_id, &id)
        })
        .collect::<Vec<_>>();
    workflows.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    workflows.truncate(WORKFLOW_UI_LIMIT);
    workflows
}

fn write_workflow(
    workflow_root: &Path,
    agent_root: &Path,
    agent_id: &str,
    id: Option<&str>,
    name: &str,
    description: &str,
    body: &str,
    trigger: Option<&WorkflowTrigger>,
    source_ref: Option<&str>,
) -> Result<WorkflowSummary, FeatureHostError> {
    let name = clamp_workflow_name(name);
    let description = clamp_workflow_description(description);
    let body = clamp_workflow_body(body);
    if name.is_empty() || body.is_empty() {
        return Err(FeatureHostError::Contract(
            "workflow name and body must not be empty".into(),
        ));
    }
    std::fs::create_dir_all(workflow_root)
        .map_err(|error| FeatureHostError::Contract(format!("create workflow root: {error}")))?;
    let id = id
        .map(str::to_string)
        .unwrap_or_else(|| slugify_workflow_name(&name));
    if !is_safe_memory_agent_id(&id) {
        return Err(FeatureHostError::Contract(format!(
            "unsafe workflow id: {id}"
        )));
    }
    let existing_count = std::fs::read_dir(workflow_root)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .count()
        })
        .unwrap_or(0);
    if !workflow_root.join(&id).exists() && existing_count >= WORKFLOW_MAX_PER_AGENT {
        return Err(FeatureHostError::Contract(format!(
            "workflow library is limited to {WORKFLOW_MAX_PER_AGENT} user workflows"
        )));
    }
    let dir = workflow_root.join(&id);
    let path = dir.join(WORKFLOW_FILENAME);
    let existing_data = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| parse_workflow_file(&raw))
        .map(|parsed| parsed.data);
    let raw = serialize_workflow_file(
        &name,
        &description,
        &body,
        trigger,
        source_ref,
        existing_data.as_ref(),
    )?;
    std::fs::create_dir_all(&dir).map_err(|error| {
        FeatureHostError::Contract(format!("create workflow directory: {error}"))
    })?;
    let temp = path.with_extension("md.tmp");
    std::fs::write(&temp, raw)
        .map_err(|error| FeatureHostError::Contract(format!("write workflow: {error}")))?;
    std::fs::rename(&temp, &path)
        .map_err(|error| FeatureHostError::Contract(format!("commit workflow: {error}")))?;
    load_workflow_summary(workflow_root, agent_root, agent_id, &id)
        .ok_or_else(|| FeatureHostError::Contract("workflow could not be reloaded".into()))
}

fn workflow_from_automation(automation: &AutomationSummary) -> WorkflowSummary {
    WorkflowSummary {
        id: automation.id.clone(),
        name: automation.name.clone(),
        description: String::new(),
        body: automation.prompt.clone(),
        trigger: Some(WorkflowTrigger {
            schedule: automation.schedule.clone(),
            is_enabled: automation.enabled,
        }),
        source_ref: None,
        source: WorkflowSource::Automation,
        plugin_id: None,
        published_by_current_user: false,
        is_enabled_for_agent: true,
        disable_model_invocation: None,
        schedule_description: Some(automation.schedule.clone()),
        created_at: automation.created_at_ms,
        last_run_at: automation.last_run_at_ms,
        next_run_at: automation.next_run_at_ms,
        helper_scripts: Vec::new(),
        file_path: String::new(),
    }
}

fn render_workflow_catalog(workflow_root: &Path, agent_root: &Path, agent_id: &str) -> String {
    let workflows = list_workflow_summaries(workflow_root, agent_root, agent_id)
        .into_iter()
        .filter(|workflow| workflow.trigger.is_none())
        .filter(|workflow| workflow.is_enabled_for_agent)
        .filter(|workflow| workflow.disable_model_invocation != Some(true))
        .collect::<Vec<_>>();
    if workflows.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "Workflows: reusable recipes available to this agent. Read the referenced SKILL.md when one applies, then follow it as written.".to_string(),
    ];
    for workflow in workflows {
        lines.push(format!(
            "- {} — {} (file: {})",
            workflow.name,
            if workflow.description.is_empty() {
                "No description"
            } else {
                workflow.description.as_str()
            },
            workflow.file_path
        ));
    }
    lines.join("\n")
}

fn group_member_handles(name: &str) -> Vec<String> {
    let lower = name.trim().to_lowercase();
    if lower.is_empty() {
        return Vec::new();
    }
    let mut handles = Vec::new();
    handles.push(lower.clone());
    let compact = lower.split_whitespace().collect::<String>();
    if !compact.is_empty() && compact != lower {
        handles.push(compact);
    }
    if let Some(first) = lower.split_whitespace().next() {
        if !first.is_empty() && !handles.iter().any(|handle| handle == first) {
            handles.push(first.to_string());
        }
    }
    handles
}

fn is_group_word_char(character: Option<char>) -> bool {
    character.is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}

fn has_group_mention_at(lower: &str, handle: &str) -> bool {
    let needle = format!("@{handle}");
    let mut search_from = 0usize;
    while let Some(relative) = lower[search_from..].find(&needle) {
        let index = search_from + relative;
        let before = lower[..index].chars().next_back();
        let after_index = index + needle.len();
        let after = lower[after_index..].chars().next();
        if !is_group_word_char(before) && !is_group_word_char(after) {
            return true;
        }
        search_from = index + 1;
        if search_from >= lower.len() {
            break;
        }
    }
    false
}

fn has_everyone_group_mention(lower: &str) -> bool {
    has_group_mention_at(lower, "everyone") || has_group_mention_at(lower, "all")
}

fn resolve_group_responders(
    group: &GroupSummary,
    bots: &BTreeMap<String, BotSummary>,
) -> Vec<String> {
    let members = group
        .member_ids
        .iter()
        .filter_map(|id| bots.get(id).map(|bot| (id.clone(), bot.name.clone())))
        .collect::<Vec<_>>();
    if members.is_empty() {
        return Vec::new();
    }
    let start = group
        .messages
        .iter()
        .rposition(|message| matches!(message.speaker, GroupSpeaker::User { .. }))
        .unwrap_or(0);
    let mut is_everyone = false;
    let mut mentioned = BTreeSet::new();
    for message in group.messages.iter().skip(start) {
        let lower = message.content.to_lowercase();
        if has_everyone_group_mention(&lower) {
            is_everyone = true;
        }
        for (id, name) in &members {
            if mentioned.contains(id) {
                continue;
            }
            if group_member_handles(name)
                .iter()
                .any(|handle| has_group_mention_at(&lower, handle))
            {
                mentioned.insert(id.clone());
            }
        }
    }
    if is_everyone || mentioned.is_empty() {
        return members.into_iter().map(|(id, _)| id).collect();
    }
    members
        .into_iter()
        .filter_map(|(id, _)| mentioned.contains(&id).then_some(id))
        .collect()
}

fn order_round_speakers(member_ids: &[String], round: usize) -> Vec<String> {
    if member_ids.is_empty() {
        return Vec::new();
    }
    let offset = round % member_ids.len();
    member_ids[offset..]
        .iter()
        .chain(member_ids[..offset].iter())
        .cloned()
        .collect()
}

fn is_group_pass_content(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    let mut normalized = trimmed.to_ascii_lowercase();
    if normalized.ends_with('.') {
        normalized.pop();
    }
    let normalized = normalized.trim();
    let normalized = normalized
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(normalized)
        .trim();
    normalized.eq_ignore_ascii_case("pass")
}

fn group_messages_since_member_last_spoke<'a>(
    history: &'a [GroupMessage],
    member_id: &str,
) -> &'a [GroupMessage] {
    if let Some(index) = history.iter().rposition(
        |message| matches!(&message.speaker, GroupSpeaker::Member { id, .. } if id == member_id),
    ) {
        &history[index + 1..]
    } else {
        history
    }
}

fn format_group_message_line(message: &GroupMessage, viewer_id: &str) -> String {
    match &message.speaker {
        GroupSpeaker::User { name } => name
            .as_ref()
            .filter(|name| !name.is_empty())
            .map(|name| format!("{name} (user): {}", message.content))
            .unwrap_or_else(|| format!("User: {}", message.content)),
        GroupSpeaker::Member { id, name } => {
            let suffix = if id == viewer_id { " (you)" } else { "" };
            format!("{name}{suffix}: {}", message.content)
        }
    }
}

fn format_group_history(history: &[GroupMessage], viewer_id: &str) -> String {
    let start = history.len().saturating_sub(GROUP_PROMPT_HISTORY_LIMIT);
    let recent = &history[start..];
    if recent.is_empty() {
        return "(no messages yet)".into();
    }
    recent
        .iter()
        .map(|message| format_group_message_line(message, viewer_id))
        .collect::<Vec<_>>()
        .join("\n")
}

fn group_display_name(group: &GroupSummary) -> &str {
    let name = group.name.trim();
    if name.is_empty() { "the group" } else { name }
}

fn build_group_member_system_prompt(
    member: &BotSummary,
    group: &GroupSummary,
    peers: &[BotSummary],
) -> String {
    let description = group.description.trim();
    let group_label = if description.is_empty() {
        format!("\"{}\"", group_display_name(group))
    } else {
        format!("\"{}\" — {description}", group_display_name(group))
    };
    let mut lines = vec![format!(
        "You are {}, one participant in a group chat ({}).",
        member.name, group_label
    )];
    if !member.description.trim().is_empty() {
        lines.push(format!("Your persona: {}", member.description.trim()));
    }
    if !peers.is_empty() {
        lines.push(String::new());
        lines.push("Other participants in the room:".into());
        for peer in peers {
            let peer_description = if peer.description.trim().is_empty() {
                String::new()
            } else {
                format!(" ({})", peer.description.trim())
            };
            lines.push(format!("- {}{peer_description}", peer.name));
        }
    }
    lines.push(String::new());
    lines.push(if peers.is_empty() {
        "Right now you are speaking in this group chat.".into()
    } else {
        format!(
            "Right now you are speaking in this group chat, with {}.",
            peers
                .iter()
                .map(|peer| peer.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    lines.extend([
        String::new(),
        "Several distinct participants share this room. Stay fully in character as yourself. Never speak or write as another participant or as the user, and never narrate the conversation from the outside.".into(),
        String::new(),
        "How you talk in the room:".into(),
        "- Keep each message short and conversational — usually one to three sentences, the way people actually chat. Do not monologue or summarize the whole thread.".into(),
        "- React to what was just said: build on it, agree, disagree, or ask a pointed question. Address others by name when it helps.".into(),
        "- Mentions: write @Name to direct your message at a specific teammate, or @everyone for the whole room. If you are @-mentioned you are being asked to weigh in, so respond; to pull a specific teammate into the conversation, @-mention them.".into(),
        "- Do not repeat points already made, and do not restate other people's messages back to them.".into(),
        "- If you have nothing new worth adding right now, send exactly \"(pass)\". Staying quiet is good — it lets the conversation settle instead of spinning forever.".into(),
        "- Say your piece in one turn, then stop. Never role-play other participants' replies.".into(),
        String::new(),
        "Conversations are private to the people in them: what you and the user discuss in your one-on-one chat stays there. Never quote, summarize, or reveal it in this room.".into(),
    ]);
    lines.join("\n")
}

fn build_group_turn_prompt(
    member: &BotSummary,
    group: &GroupSummary,
    peers: &[BotSummary],
    new_messages: &[GroupMessage],
) -> String {
    let with_clause = if peers.is_empty() {
        String::new()
    } else {
        format!(
            " - with {}",
            peers
                .iter()
                .map(|peer| peer.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut lines = vec![format!(
        "[Group chat: \"{}\"{with_clause}]",
        group_display_name(group)
    )];
    if new_messages.is_empty() {
        lines.push("No new messages in the room since your last turn.".into());
    } else {
        lines.push("New messages in the room (oldest first):".into());
        lines.push(format_group_history(new_messages, &member.id));
    }
    lines.extend([
        String::new(),
        format!(
            "It's your turn, {}. Reply in character with one short room message if you have something worth adding, or reply exactly \"(pass)\" if you don't.",
            member.name
        ),
    ]);
    lines.join("\n")
}

fn validate_group_members(
    state: &FeatureState,
    member_ids: Vec<String>,
) -> Result<Vec<String>, FeatureHostError> {
    let mut seen = BTreeSet::new();
    let mut members = Vec::new();
    for id in member_ids {
        let id = id.trim().to_string();
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        if state.groups.contains_key(&id) {
            return Err(FeatureHostError::Contract(
                "a group chat can only contain individual agents, not other group chats".into(),
            ));
        }
        if !state.bots.contains_key(&id) {
            return Err(FeatureHostError::Contract(format!(
                "unknown group member: {id}"
            )));
        }
        members.push(id);
    }
    if members.is_empty() {
        return Err(FeatureHostError::Contract(
            "group chat must contain at least one agent".into(),
        ));
    }
    Ok(members)
}

fn build_agent_inbound_wake_prompt(sender: &BotSummary, text: &str, priority: bool) -> String {
    let priority_line = if priority {
        "This is a priority instruction from another assistant. It may supersede non-user background work."
    } else {
        "This is another assistant reaching out asynchronously, not the user typing in this chat."
    };
    format!(
        "[agent] A message arrived from {} (id: {}).\n{}\n\n{}: {}\n\nHandle any useful request or action. If a reply is needed, send it back asynchronously through the agent messaging capability; do not create acknowledgement loops.",
        sender.name,
        sender.id,
        priority_line,
        sender.name,
        clamp_block(text, 8000)
    )
}

fn build_admin_broadcast_wake_prompt(message: &str) -> String {
    format!(
        "[broadcast] A direct message from the user who owns and runs this agent was broadcast to their agents.\nTreat it as a user directive, not as another agent or a scheduled routine.\n\nThe user says: {}\n\nAct on it as appropriate. Do not rebroadcast it to other agents; the user already reached them separately.",
        clamp_block(message, 8000)
    )
}

#[cfg(feature = "production")]
fn activity_parent_agent_id(state: &FeatureState, operation_id: &str) -> String {
    state
        .group_operations
        .get(operation_id)
        .map(|context| context.member_id.clone())
        .or_else(|| {
            state
                .background_operations
                .get(operation_id)
                .map(|context| context.agent_id.clone())
        })
        .or_else(|| state.operation_agents.get(operation_id).cloned())
        .unwrap_or_else(|| "mahayana-assistant".into())
}

#[cfg(feature = "production")]
fn subagent_title(prompt: Option<&str>, fallback: &str) -> String {
    prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .and_then(|prompt| prompt.lines().find(|line| !line.trim().is_empty()))
        .map(|line| clamp_line(line, 120))
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| clamp_line(fallback, 120))
}

#[cfg(feature = "production")]
fn subagent_status_from_agent_state(value: Option<&Value>) -> Option<SubagentStatus> {
    let status = value?.get("status").and_then(Value::as_str).unwrap_or("");
    match status {
        "pendingInit" | "running" => Some(SubagentStatus::Running),
        "completed" | "shutdown" => Some(SubagentStatus::Done),
        "errored" | "notFound" => Some(SubagentStatus::Error),
        "interrupted" => Some(SubagentStatus::Aborted),
        _ => None,
    }
}

#[cfg(feature = "production")]
fn update_subagents_from_activity(
    state: &mut FeatureState,
    parent_agent_id: &str,
    operation_id: &str,
    title: &str,
    detail: Option<&str>,
    runtime_status: RuntimeActivityStatus,
    metadata: Option<&Value>,
) -> Vec<SubagentSummary> {
    let Some(metadata) = metadata else {
        return Vec::new();
    };
    let event_type = metadata.get("type").and_then(Value::as_str).unwrap_or("");
    let now = now_millis();
    let mut changed = Vec::new();

    if event_type == "collabAgentToolCall" {
        let tool = metadata.get("tool").and_then(Value::as_str).unwrap_or("");
        let prompt = metadata.get("prompt").and_then(Value::as_str);
        let model = metadata
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| !model.trim().is_empty());
        let receivers = metadata
            .get("receiverThreadIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .collect::<Vec<_>>();
        let agent_states = metadata.get("agentsStates").and_then(Value::as_object);
        for receiver in receivers {
            let existing = state.subagents.get(receiver).cloned();
            let state_value = agent_states.and_then(|states| states.get(receiver));
            let inferred_status =
                subagent_status_from_agent_state(state_value).unwrap_or_else(|| {
                    if runtime_status == RuntimeActivityStatus::Failed {
                        SubagentStatus::Error
                    } else if tool == "closeAgent"
                        && runtime_status == RuntimeActivityStatus::Completed
                    {
                        SubagentStatus::Done
                    } else {
                        existing
                            .as_ref()
                            .map(|subagent| subagent.status)
                            .unwrap_or(SubagentStatus::Running)
                    }
                });
            let state_message = state_value
                .and_then(|state| state.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let subagent = SubagentSummary {
                id: receiver.to_string(),
                parent_agent_id: parent_agent_id.to_string(),
                subagent_type: model
                    .map(str::to_string)
                    .or_else(|| {
                        existing
                            .as_ref()
                            .map(|subagent| subagent.subagent_type.clone())
                    })
                    .unwrap_or_else(|| "codex".into()),
                title: subagent_title(
                    prompt,
                    existing
                        .as_ref()
                        .map(|subagent| subagent.title.as_str())
                        .unwrap_or(title),
                ),
                status: inferred_status,
                started_at_ms: existing
                    .as_ref()
                    .map(|subagent| subagent.started_at_ms)
                    .unwrap_or(now),
                updated_at_ms: now,
                detail: state_message
                    .or_else(|| detail.map(str::to_string))
                    .or_else(|| prompt.map(|prompt| clamp_block(prompt, 1000))),
            };
            state
                .subagents
                .insert(receiver.to_string(), subagent.clone());
            changed.push(subagent);
        }
    } else if event_type == "subAgentActivity" {
        let Some(receiver) = metadata
            .get("agentThreadId")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
        else {
            return Vec::new();
        };
        let activity_kind = metadata.get("kind").and_then(Value::as_str).unwrap_or("");
        let path = metadata
            .get("agentPath")
            .and_then(Value::as_str)
            .unwrap_or("");
        let existing = state.subagents.get(receiver).cloned();
        let status = match activity_kind {
            "interrupted" => SubagentStatus::Aborted,
            "started" | "interacted" => SubagentStatus::Running,
            _ if runtime_status == RuntimeActivityStatus::Failed => SubagentStatus::Error,
            _ => existing
                .as_ref()
                .map(|subagent| subagent.status)
                .unwrap_or(SubagentStatus::Running),
        };
        let fallback_title = path
            .rsplit('/')
            .find(|part| !part.trim().is_empty())
            .unwrap_or(title);
        let subagent = SubagentSummary {
            id: receiver.to_string(),
            parent_agent_id: parent_agent_id.to_string(),
            subagent_type: existing
                .as_ref()
                .map(|subagent| subagent.subagent_type.clone())
                .unwrap_or_else(|| "codex".into()),
            title: existing
                .as_ref()
                .map(|subagent| subagent.title.clone())
                .unwrap_or_else(|| subagent_title(None, fallback_title)),
            status,
            started_at_ms: existing
                .as_ref()
                .map(|subagent| subagent.started_at_ms)
                .unwrap_or(now),
            updated_at_ms: now,
            detail: detail
                .map(str::to_string)
                .or_else(|| (!path.is_empty()).then(|| path.to_string())),
        };
        state
            .subagents
            .insert(receiver.to_string(), subagent.clone());
        changed.push(subagent);
    }

    for subagent in &changed {
        if subagent.status == SubagentStatus::Running {
            state.async_tasks.insert(
                subagent.id.clone(),
                AsyncTaskSummary {
                    kind: AsyncTaskKind::Subagent,
                    id: subagent.id.clone(),
                    parent_agent_id: subagent.parent_agent_id.clone(),
                    label: subagent.title.clone(),
                    status: AsyncTaskStatus::Running,
                    started_at_ms: subagent.started_at_ms,
                    detail: subagent.detail.clone(),
                    subagent_type: Some(subagent.subagent_type.clone()),
                    resource_id: None,
                },
            );
        } else {
            state.async_tasks.remove(&subagent.id);
        }
    }

    // A generic provider may report a subagent activity without a receiver id.
    // Do not fabricate a durable identity from the operation id; the agent roster
    // only contains actual subagent ids.
    let _ = operation_id;
    changed
}

fn derive_teach_workflow_name(markdown: &str) -> String {
    for raw in markdown.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let line = line.trim_start_matches('#').trim();
        let cleaned = line
            .chars()
            .filter(|character| !matches!(character, '*' | '_' | '`' | '>' | '[' | ']'))
            .collect::<String>();
        let name = clamp_line(&cleaned, 80);
        if !name.is_empty() {
            return name;
        }
    }
    "Taught workflow".into()
}

fn slugify_teach_workflow_name(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for character in name.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(character);
        } else if !slug.is_empty() {
            pending_dash = true;
        }
        if slug.len() >= 60 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("taught-workflow-{}", now_millis())
    } else {
        slug.to_string()
    }
}

fn teach_recording_status(active: Option<&TeachCaptureProcess>) -> TeachRecordingStatus {
    match active {
        Some(active) => TeachRecordingStatus {
            state: "recording".into(),
            agent_id: Some(active.agent_id.clone()),
            started_at_ms: Some(active.started_at_ms),
            max_duration_ms: TEACH_MAX_DURATION_MS,
        },
        None => TeachRecordingStatus::default(),
    }
}

fn find_ffmpeg_binary() -> Result<PathBuf, FeatureHostError> {
    for candidate in [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "ffmpeg",
    ] {
        let ok = std::process::Command::new(candidate)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if ok {
            return Ok(PathBuf::from(candidate));
        }
    }
    Err(FeatureHostError::Contract(
        "Teach Recording requires ffmpeg on this computer".into(),
    ))
}

#[cfg(target_os = "macos")]
fn avfoundation_screen_index(ffmpeg: &Path) -> Result<String, FeatureHostError> {
    let output = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ])
        .output()
        .map_err(|error| {
            FeatureHostError::Contract(format!("list screen capture devices: {error}"))
        })?;
    let text = String::from_utf8_lossy(&output.stderr);
    for line in text.lines() {
        if !line.contains("Capture screen") {
            continue;
        }
        let Some(open) = line.rfind('[') else {
            continue;
        };
        let Some(close_rel) = line[open + 1..].find(']') else {
            continue;
        };
        let index = &line[open + 1..open + 1 + close_rel];
        if !index.is_empty() && index.chars().all(|character| character.is_ascii_digit()) {
            return Ok(index.to_string());
        }
    }
    Err(FeatureHostError::Contract(
        "ffmpeg could not find a macOS screen capture device; grant Screen Recording permission and retry".into(),
    ))
}

fn spawn_teach_capture(video_path: &Path) -> Result<std::process::Child, FeatureHostError> {
    let ffmpeg = find_ffmpeg_binary()?;
    let mut command = std::process::Command::new(&ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(target_os = "macos")]
    {
        let screen = avfoundation_screen_index(&ffmpeg)?;
        command.args([
            "-f",
            "avfoundation",
            "-framerate",
            "15",
            "-capture_cursor",
            "1",
            "-i",
            &format!("{screen}:none"),
        ]);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0.0".into());
        command.args(["-f", "x11grab", "-framerate", "15", "-i", &display]);
    }
    #[cfg(target_os = "windows")]
    {
        command.args(["-f", "gdigrab", "-framerate", "15", "-i", "desktop"]);
    }

    command.args([
        "-t",
        "600",
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        video_path.to_string_lossy().as_ref(),
    ]);
    command
        .spawn()
        .map_err(|error| FeatureHostError::Contract(format!("start teach recording: {error}")))
}

fn stop_teach_capture(child: &mut std::process::Child) -> Result<(), FeatureHostError> {
    if child
        .try_wait()
        .map_err(|error| FeatureHostError::Contract(format!("poll teach recorder: {error}")))?
        .is_some()
    {
        return Ok(());
    }
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(b"q\n");
        let _ = stdin.flush();
    }
    for _ in 0..40 {
        if child
            .try_wait()
            .map_err(|error| {
                FeatureHostError::Contract(format!("wait for teach recorder: {error}"))
            })?
            .is_some()
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    child
        .kill()
        .map_err(|error| FeatureHostError::Contract(format!("stop teach recorder: {error}")))?;
    let _ = child.wait();
    Ok(())
}

fn extract_teach_frames(video_path: &Path, frames_dir: &Path) -> Result<(), FeatureHostError> {
    let ffmpeg = find_ffmpeg_binary()?;
    std::fs::create_dir_all(frames_dir).map_err(|error| {
        FeatureHostError::Contract(format!("create teach frames directory: {error}"))
    })?;
    let pattern = frames_dir.join("frame-%03d.jpg");
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            video_path.to_string_lossy().as_ref(),
            "-vf",
            "fps=0.5,scale=1280:-2:force_original_aspect_ratio=decrease",
            "-frames:v",
            "120",
            pattern.to_string_lossy().as_ref(),
        ])
        .status()
        .map_err(|error| FeatureHostError::Contract(format!("extract teach frames: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(FeatureHostError::Contract(
            "ffmpeg failed to extract teach frames".into(),
        ))
    }
}

fn sanitize_auto_review_rules(rules: Vec<AutoReviewRule>) -> Vec<AutoReviewRule> {
    let mut seen = BTreeSet::new();
    let mut sanitized = Vec::new();
    for mut rule in rules.into_iter().take(200) {
        rule.id = clamp_line(&rule.id, 96);
        rule.text = clamp_line(&rule.text, 2000);
        if rule.text.is_empty() {
            continue;
        }
        if rule.id.is_empty() {
            rule.id = format!("rule-{}", sanitized.len() + 1);
        }
        if seen.insert(rule.id.clone()) {
            sanitized.push(rule);
        }
    }
    sanitized
}

fn load_product_host_settings(path: &Path) -> ProductHostSettings {
    let Ok(bytes) = std::fs::read(path) else {
        return ProductHostSettings::default();
    };
    let Ok(mut settings) = serde_json::from_slice::<ProductHostSettings>(&bytes) else {
        return ProductHostSettings::default();
    };
    settings.auto_review_rules = sanitize_auto_review_rules(settings.auto_review_rules);
    settings
}

fn persist_product_host_settings(
    path: &Path,
    settings: &ProductHostSettings,
) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create settings directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| FeatureHostError::Contract(format!("serialize host settings: {error}")))?;
    std::fs::write(&temp, bytes)
        .map_err(|error| FeatureHostError::Contract(format!("write host settings: {error}")))?;
    std::fs::rename(&temp, path)
        .map_err(|error| FeatureHostError::Contract(format!("commit host settings: {error}")))
}

fn load_remote_computer_device_secrets(path: &Path) -> BTreeMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, String>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|(device_id, secret)| {
            is_safe_memory_agent_id(device_id)
                && secret.len() >= 48
                && secret.len() <= 256
                && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .collect()
}

fn persist_remote_computer_device_secrets(
    path: &Path,
    secrets: &BTreeMap<String, String>,
) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create remote device directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(secrets).map_err(|error| {
        FeatureHostError::Contract(format!("serialize remote device secret: {error}"))
    })?;
    std::fs::write(&temp, bytes).map_err(|error| {
        FeatureHostError::Contract(format!("write remote device secret: {error}"))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| FeatureHostError::Contract(format!("protect remote device secret: {error}")),
        )?;
    }
    std::fs::rename(&temp, path).map_err(|error| {
        FeatureHostError::Contract(format!("commit remote device secret: {error}"))
    })
}

#[cfg(test)]
mod remote_device_secret_tests {
    use super::*;

    #[test]
    fn remote_device_secret_round_trip_is_private_and_never_contains_control_data() {
        let path = std::env::temp_dir().join(format!(
            "fabushi-remote-device-secret-test-{}-{}.json",
            std::process::id(),
            now_millis()
        ));
        let device_id = "fabushi-mac-test".to_string();
        let secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
        let secrets = BTreeMap::from([(device_id.clone(), secret.clone())]);
        persist_remote_computer_device_secrets(&path, &secrets).expect("persist device secret");
        let restored = load_remote_computer_device_secrets(&path);
        assert_eq!(restored.get(&device_id), Some(&secret));
        let raw = std::fs::read_to_string(&path).expect("read device secret file");
        assert!(!raw.contains("screenshot"));
        assert!(!raw.contains("computer.action"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("device secret metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_file(path);
    }
}

fn read_action_audit(path: &Path, limit: usize) -> Vec<Value> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn auto_review_rule_matches(rule: &AutoReviewRule, title: &str, details: &Value) -> bool {
    let needle = rule.text.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let proposed_rule = details
        .get("proposedRule")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if !proposed_rule.is_empty() && (proposed_rule == needle || proposed_rule.contains(&needle)) {
        return true;
    }
    let subject = details
        .get("subject")
        .or_else(|| details.get("command"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if !subject.is_empty() && (subject == needle || subject.contains(&needle)) {
        return true;
    }
    let capability = details
        .get("capability")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if !capability.is_empty() && (capability == needle || capability.contains(&needle)) {
        return true;
    }
    title.trim().to_lowercase().contains(&needle)
}

fn load_peer_messages(path: &Path) -> Vec<AgentPeerMessage> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(mut messages) = serde_json::from_slice::<Vec<AgentPeerMessage>>(&bytes) else {
        return Vec::new();
    };
    messages.retain(|message| {
        !message.id.trim().is_empty()
            && !message.from_agent_id.trim().is_empty()
            && !message.target_id.trim().is_empty()
            && !clamp_block(&message.text, 8000).is_empty()
    });
    for message in &mut messages {
        message.text = clamp_block(&message.text, 8000);
        message.from_agent_name = clamp_line(&message.from_agent_name, 72);
        message.target_name = clamp_line(&message.target_name, 72);
    }
    if messages.len() > 5000 {
        messages.drain(0..messages.len() - 5000);
    }
    messages
}

fn persist_peer_messages(
    path: &Path,
    messages: &[AgentPeerMessage],
) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create peer message directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(messages)
        .map_err(|error| FeatureHostError::Contract(format!("serialize peer messages: {error}")))?;
    std::fs::write(&temp, data).map_err(|error| {
        FeatureHostError::Contract(format!("write peer message store: {error}"))
    })?;
    std::fs::rename(&temp, path).map_err(|error| {
        FeatureHostError::Contract(format!("commit peer message store: {error}"))
    })?;
    Ok(())
}

fn load_groups(path: &Path) -> BTreeMap<String, GroupSummary> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(items) = serde_json::from_slice::<Vec<GroupSummary>>(&bytes) else {
        return BTreeMap::new();
    };
    items
        .into_iter()
        .filter_map(|mut group| {
            group.name = clamp_line(&group.name, 72);
            group.description = clamp_block(&group.description, 2000);
            let mut seen = BTreeSet::new();
            group
                .member_ids
                .retain(|id| !id.trim().is_empty() && seen.insert(id.clone()));
            if group.id.trim().is_empty() || group.name.is_empty() || group.member_ids.is_empty() {
                return None;
            }
            Some((group.id.clone(), group))
        })
        .collect()
}

fn persist_groups(
    path: &Path,
    groups: &BTreeMap<String, GroupSummary>,
) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create group directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(&groups.values().cloned().collect::<Vec<_>>())
        .map_err(|error| FeatureHostError::Contract(format!("serialize groups: {error}")))?;
    std::fs::write(&temp, data)
        .map_err(|error| FeatureHostError::Contract(format!("write group store: {error}")))?;
    std::fs::rename(&temp, path)
        .map_err(|error| FeatureHostError::Contract(format!("commit group store: {error}")))?;
    Ok(())
}

fn load_bots(path: &Path) -> BTreeMap<String, BotSummary> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(items) = serde_json::from_slice::<Vec<BotSummary>>(&bytes) else {
        return BTreeMap::new();
    };
    items
        .into_iter()
        .filter_map(|mut bot| {
            bot.name = clamp_line(&bot.name, 72);
            bot.description = clamp_block(&bot.description, 2000);
            bot.title = bot.title.trim().to_string();
            bot.avatar_shape = clean_optional_string(bot.avatar_shape);
            bot.avatar_color = clean_optional_string(bot.avatar_color);
            if bot.id.trim().is_empty() || bot.name.is_empty() {
                return None;
            }
            Some((bot.id.clone(), bot))
        })
        .collect()
}

fn persist_bots(path: &Path, bots: &BTreeMap<String, BotSummary>) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create bot directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(&bots.values().cloned().collect::<Vec<_>>())
        .map_err(|error| FeatureHostError::Contract(format!("serialize bots: {error}")))?;
    std::fs::write(&temp, data)
        .map_err(|error| FeatureHostError::Contract(format!("write bot store: {error}")))?;
    std::fs::rename(&temp, path)
        .map_err(|error| FeatureHostError::Contract(format!("commit bot store: {error}")))?;
    Ok(())
}

fn load_automations(path: &Path) -> BTreeMap<String, AutomationSummary> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(items) = serde_json::from_slice::<Vec<AutomationSummary>>(&bytes) else {
        return BTreeMap::new();
    };
    let now = now_millis();
    items
        .into_iter()
        .filter_map(|mut item| {
            if !is_safe_automation_id(&item.id)
                || item.name.trim().is_empty()
                || item.prompt.trim().is_empty()
                || normalize_automation_schedule(&item.schedule).is_err()
            {
                return None;
            }
            item.next_run_at_ms = item
                .enabled
                .then(|| {
                    item.next_run_at_ms
                        .filter(|next| *next > now)
                        .or_else(|| next_automation_run(&item.schedule, now))
                })
                .flatten();
            Some((item.id.clone(), item))
        })
        .collect()
}

fn persist_automations(
    path: &Path,
    automations: &BTreeMap<String, AutomationSummary>,
) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create automation directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(&automations.values().cloned().collect::<Vec<_>>())
        .map_err(|error| FeatureHostError::Contract(format!("serialize automations: {error}")))?;
    std::fs::write(&temp, data)
        .map_err(|error| FeatureHostError::Contract(format!("write automation store: {error}")))?;
    std::fs::rename(&temp, path)
        .map_err(|error| FeatureHostError::Contract(format!("commit automation store: {error}")))?;
    Ok(())
}

fn normalize_automation_schedule(raw: &str) -> Result<String, FeatureHostError> {
    let schedule = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if schedule.is_empty() || next_automation_run(&schedule, now_millis()).is_none() {
        return Err(FeatureHostError::Contract(
            "invalid automation schedule; use a 5-field cron, @hourly/@daily/@weekly/@monthly/@yearly, or @every <n><s|m|h|d>"
                .into(),
        ));
    }
    Ok(schedule)
}

fn parse_every_interval_ms(schedule: &str) -> Option<i64> {
    let rest = schedule.strip_prefix("@every ")?.trim();
    let split = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    let amount = rest[..split].parse::<i64>().ok()?;
    let unit = rest[split..].trim().to_ascii_lowercase();
    let unit_ms = match unit.as_str() {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    amount.checked_mul(unit_ms).filter(|value| *value > 0)
}

fn expand_cron_alias(schedule: &str) -> &str {
    match schedule.to_ascii_lowercase().as_str() {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        _ => schedule,
    }
}

fn parse_cron_field(field: &str, min: u32, max: u32) -> Option<BTreeSet<u32>> {
    let mut values = BTreeSet::new();
    for part in field.split(',') {
        let mut step_parts = part.split('/');
        let range = step_parts.next()?;
        let step: u32 = step_parts
            .next()
            .map_or(Some(1), |value| value.parse().ok())?;
        if step_parts.next().is_some() || step == 0 {
            return None;
        }
        let (start, end) = if range == "*" || range.is_empty() {
            (min, max)
        } else if let Some((start, end)) = range.split_once('-') {
            (start.parse().ok()?, end.parse().ok()?)
        } else {
            let start = range.parse().ok()?;
            (start, if part.contains('/') { max } else { start })
        };
        if start < min || end > max || start > end {
            return None;
        }
        for value in (start..=end).step_by(step as usize) {
            values.insert(if max == 7 && value == 7 { 0 } else { value });
        }
    }
    (!values.is_empty()).then_some(values)
}

fn next_automation_run(schedule: &str, after_ms: i64) -> Option<i64> {
    if let Some(interval) = parse_every_interval_ms(schedule) {
        return after_ms.checked_add(interval);
    }
    let expression = expand_cron_alias(schedule);
    let fields = expression.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 5 {
        return None;
    }
    let minute = parse_cron_field(fields[0], 0, 59)?;
    let hour = parse_cron_field(fields[1], 0, 23)?;
    let day_of_month = parse_cron_field(fields[2], 1, 31)?;
    let month = parse_cron_field(fields[3], 1, 12)?;
    let day_of_week = parse_cron_field(fields[4], 0, 7)?;
    let dom_restricted = fields[2] != "*";
    let dow_restricted = fields[4] != "*";
    let mut candidate = after_ms.div_euclid(60_000) * 60_000 + 60_000;
    for _ in 0..(366 * 24 * 60) {
        let date = chrono::DateTime::<Utc>::from_timestamp_millis(candidate)?;
        let dom_ok = day_of_month.contains(&date.day());
        let dow_ok = day_of_week.contains(&date.weekday().num_days_from_sunday());
        let day_ok = if dom_restricted && dow_restricted {
            dom_ok || dow_ok
        } else {
            (!dom_restricted || dom_ok) && (!dow_restricted || dow_ok)
        };
        if minute.contains(&date.minute())
            && hour.contains(&date.hour())
            && month.contains(&date.month())
            && day_ok
        {
            return Some(candidate);
        }
        candidate = candidate.checked_add(60_000)?;
    }
    None
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn timestamp() -> String {
    now_millis().to_string()
}

fn computer_origin_label(origin: ComputerControlOrigin) -> &'static str {
    match origin {
        ComputerControlOrigin::LocalUi => "local-ui",
        ComputerControlOrigin::RemoteMobile => "remote-mobile",
        ComputerControlOrigin::Ai => "ai",
    }
}

fn sync_computer_control_policy(settings: &ProductHostSettings) {
    mahayana_computer::set_control_policy(mahayana_computer::ComputerControlPolicy {
        local_execution_enabled: settings.local_execution,
        remote_control_enabled: settings.remote_control_enabled,
        ai_control_enabled: settings.ai_computer_control_enabled,
        local_tool_permission: settings.local_tool_permission,
    });
}

fn test_computer_snapshot() -> ComputerSnapshot {
    ComputerSnapshot {
        captured_at_ms: now_millis(),
        // Deterministic 1x1 transparent PNG. Test mode must never capture or
        // mutate the developer's real desktop.
        data_url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
        width: Some(1),
        height: Some(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mahayana_host_protocol::ApprovalDecision;

    #[cfg(feature = "production")]
    fn isolated_host_config(profile: &str) -> HostCreateConfig {
        let root = std::env::temp_dir().join(format!(
            "fabushi-feature-host-{profile}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create isolated Host root");
        HostCreateConfig {
            runtime: mahayana_core::RuntimeConfig {
                data_dir: Some(root.join("runtime")),
                ..Default::default()
            },
            product_session_path: Some(root.join("product-session.json")),
            inherit_installed_plugins: Some(false),
            ..Default::default()
        }
    }

    fn controller() -> FeatureHostController {
        FeatureHostController::create(
            HostConfig {
                profile_id: "fast-e2e".into(),
                mode: HostMode::Test,
            },
            SurfacePlatform::Tauri,
        )
        .expect("create feature Host")
    }

    fn drain(controller: &FeatureHostController) -> Vec<HostEvent> {
        let mut events = Vec::new();
        while let Some(event) = controller.receive().expect("receive event") {
            events.push(event);
        }
        events
    }

    #[test]
    fn group_chat_handles_mentions_round_order_and_pass_rules() {
        let bots = BTreeMap::from([
            (
                "research-bot".into(),
                BotSummary {
                    id: "research-bot".into(),
                    name: "Research Bot".into(),
                    description: String::new(),
                    title: String::new(),
                    hidden: false,
                    avatar: None,
                    avatar_shape: None,
                    avatar_color: None,
                    notifications_enabled: true,
                    notify_on_updates: true,
                    unread: false,
                    conversation_id: Some("codex:agent:research".into()),
                },
            ),
            (
                "incident-bot".into(),
                BotSummary {
                    id: "incident-bot".into(),
                    name: "Incident Bot".into(),
                    description: String::new(),
                    title: String::new(),
                    hidden: false,
                    avatar: None,
                    avatar_shape: None,
                    avatar_color: None,
                    notifications_enabled: true,
                    notify_on_updates: true,
                    unread: false,
                    conversation_id: Some("codex:agent:incident".into()),
                },
            ),
        ]);
        let mut group = GroupSummary {
            id: "group-1".into(),
            name: "Ops Room".into(),
            description: String::new(),
            member_ids: vec!["research-bot".into(), "incident-bot".into()],
            messages: vec![GroupMessage {
                id: "message-1".into(),
                speaker: GroupSpeaker::User { name: None },
                content: "@Research please verify the source".into(),
                created_at_ms: 1,
            }],
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        assert_eq!(
            resolve_group_responders(&group, &bots),
            vec!["research-bot"]
        );
        group.messages.push(GroupMessage {
            id: "message-2".into(),
            speaker: GroupSpeaker::User { name: None },
            content: "@everyone weigh in".into(),
            created_at_ms: 2,
        });
        assert_eq!(
            resolve_group_responders(&group, &bots),
            vec!["research-bot", "incident-bot"]
        );
        assert_eq!(
            order_round_speakers(&["a".into(), "b".into(), "c".into()], 1),
            vec!["b", "c", "a"]
        );
        assert!(is_group_pass_content("(pass)."));
        assert!(is_group_pass_content(" PASS "));
        assert!(!is_group_pass_content("pass this to the next agent"));
    }

    #[test]
    fn group_crud_uses_real_host_state_and_rejects_nested_groups() {
        let controller = controller();
        drain(&controller);
        controller
            .execute(FeatureCommand::GroupCreate {
                request_id: "group-create".into(),
                name: "Research room".into(),
                description: "Cross-check sources".into(),
                member_ids: vec!["mahayana-assistant".into(), "research-bot".into()],
            })
            .expect("create group");
        let group = drain(&controller)
            .into_iter()
            .find_map(|event| match event {
                HostEvent::GroupChanged { action, group, .. } if action == "created" => Some(group),
                _ => None,
            })
            .expect("created group event");
        controller
            .execute(FeatureCommand::GroupSend {
                request_id: "group-send".into(),
                id: group.id.clone(),
                text: "@Research check this".into(),
            })
            .expect("send group message");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::GroupChanged { action, group: changed, .. }
                if action == "message" && changed.messages.len() == 1
        )));
        let nested = controller.execute(FeatureCommand::GroupCreate {
            request_id: "nested-group".into(),
            name: "Nested".into(),
            description: String::new(),
            member_ids: vec![group.id.clone()],
        });
        assert!(nested.is_err());
        controller
            .execute(FeatureCommand::GroupDelete {
                request_id: "group-delete".into(),
                id: group.id,
            })
            .expect("delete group");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::GroupChanged { action, .. } if action == "deleted"
        )));
    }

    #[test]
    fn trays_preserve_dedupe_count_and_twenty_item_cap() {
        let controller = controller();
        drain(&controller);
        {
            let mut state = controller.state().expect("state");
            push_error_tray(
                &mut state,
                "research-bot".into(),
                "Provider busy".into(),
                Some("retry later".into()),
                None,
                Some("provider:busy".into()),
            );
            push_error_tray(
                &mut state,
                "research-bot".into(),
                "Provider still busy".into(),
                Some("retry later".into()),
                None,
                Some("provider:busy".into()),
            );
            assert_eq!(state.trays.len(), 1);
            assert_eq!(state.trays[0].count, Some(2));
            assert_eq!(state.trays[0].title, "Provider still busy");
            for index in 0..25 {
                push_error_tray(
                    &mut state,
                    "research-bot".into(),
                    format!("Error {index}"),
                    None,
                    None,
                    Some(format!("error:{index}")),
                );
            }
            assert_eq!(state.trays.len(), MAX_TRAYS);
            assert!(
                state
                    .trays
                    .iter()
                    .all(|tray| tray.id != state.trays[0].dedupe_key.clone().unwrap_or_default())
            );
        }
        controller
            .execute(FeatureCommand::TrayList {
                request_id: "tray-list".into(),
            })
            .expect("list trays");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::TrayListed { ref trays, .. } if trays.len() == MAX_TRAYS
        )));
        controller
            .execute(FeatureCommand::TrayClear {
                request_id: "tray-clear".into(),
            })
            .expect("clear trays");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::TrayChanged { action, .. } if action == "cleared"
        )));
    }

    #[test]
    fn memory_store_preserves_id_dedupe_and_markdown_layout() {
        assert_eq!(memory_id_for("hello"), "aaf4c61ddcc5e8a2");
        let controller = controller();
        drain(&controller);
        controller
            .execute(FeatureCommand::MemoryClear {
                request_id: "memory-clear-initial".into(),
                agent_id: "mahayana-assistant".into(),
            })
            .expect("clear initial memory");
        drain(&controller);
        controller
            .execute(FeatureCommand::MemoryAdd {
                request_id: "memory-add".into(),
                agent_id: "mahayana-assistant".into(),
                content: "  Likes    tea  ".into(),
                kind: MemoryKind::Profile,
            })
            .expect("add profile memory");
        let added = drain(&controller)
            .into_iter()
            .find_map(|event| match event {
                HostEvent::MemoryChanged { action, memory, .. } if action == "added" => memory,
                _ => None,
            })
            .expect("added memory event");
        assert_eq!(added.content, "Likes tea");
        assert_eq!(added.id, memory_id_for("likes tea"));
        controller
            .execute(FeatureCommand::MemoryAdd {
                request_id: "memory-duplicate".into(),
                agent_id: "mahayana-assistant".into(),
                content: "likes tea".into(),
                kind: MemoryKind::Log,
            })
            .expect("dedupe memory");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::MemoryChanged { action, memory: None, .. } if action == "duplicate"
        )));
        controller
            .execute(FeatureCommand::MemoryList {
                request_id: "memory-list".into(),
                agent_id: "mahayana-assistant".into(),
                limit: 1000,
            })
            .expect("list memory");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::MemoryListed { count: 1, ref memories, .. }
                if memories.len() == 1 && memories[0].content == "Likes tea"
        )));
        let profile = controller
            .memory_root_path
            .as_ref()
            .expect("memory root")
            .join("mahayana-assistant/memory/profile.md");
        let raw = std::fs::read_to_string(profile).expect("profile markdown");
        assert!(raw.starts_with(MEMORY_PROFILE_HEADER));
        assert!(raw.contains("Likes tea"));
        controller
            .execute(FeatureCommand::MemoryRemove {
                request_id: "memory-remove".into(),
                agent_id: "mahayana-assistant".into(),
                id: added.id,
            })
            .expect("remove memory");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::MemoryChanged { action, .. } if action == "removed"
        )));
    }

    #[test]
    fn automation_schedule_supports_supported_schedule_grammar() {
        let base = 1_750_000_000_000_i64;
        assert_eq!(parse_every_interval_ms("@every 5m"), Some(300_000));
        assert_eq!(next_automation_run("@every 5m", base), Some(base + 300_000));
        assert!(next_automation_run("@daily", base).is_some());
        assert!(next_automation_run("*/15 9-17 * * 1-5", base).is_some());
        assert!(normalize_automation_schedule("not a schedule").is_err());
    }

    #[test]
    fn automation_crud_and_manual_run_use_the_host_event_contract() {
        let controller = controller();
        drain(&controller);
        controller
            .execute(FeatureCommand::AutomationUpsert {
                request_id: "automation-create".into(),
                id: Some("morning-review".into()),
                agent_id: None,
                name: "晨间复盘".into(),
                prompt: "总结昨天的进展并给出今天的三个行动。".into(),
                schedule: "@daily".into(),
                trigger: Some(AutomationTrigger::Schedule {
                    schedule: "@daily".into(),
                }),
                enabled: true,
            })
            .expect("create automation");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, automation, .. }
                if action == "created" && automation.id == "morning-review"
        )));

        controller
            .execute(FeatureCommand::AutomationList {
                request_id: "automation-list".into(),
                agent_id: None,
            })
            .expect("list automations");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationListed { automations, .. }
                if automations.len() == 1 && automations[0].name == "晨间复盘"
        )));

        controller
            .execute(FeatureCommand::AutomationSetEnabled {
                request_id: "automation-pause".into(),
                id: "morning-review".into(),
                agent_id: None,
                enabled: false,
            })
            .expect("pause automation");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, automation, .. }
                if action == "paused" && !automation.enabled
        )));

        controller
            .execute(FeatureCommand::AutomationRun {
                request_id: "automation-run".into(),
                id: "morning-review".into(),
                agent_id: None,
            })
            .expect("run automation");
        let kinds = drain(&controller)
            .into_iter()
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"automation.changed"));
        assert!(kinds.contains(&"chat.message"));

        controller
            .execute(FeatureCommand::AutomationDelete {
                request_id: "automation-delete".into(),
                id: "morning-review".into(),
                agent_id: None,
            })
            .expect("delete automation");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, .. } if action == "deleted"
        )));
    }

    #[test]
    fn automation_agent_scope_filters_routes_and_blocks_cross_agent_mutation() {
        let controller = controller();
        drain(&controller);

        for (id, agent_id, name) in [
            ("research-digest", "research-bot", "Research digest"),
            ("incident-digest", "incident-bot", "Incident digest"),
        ] {
            controller
                .execute(FeatureCommand::AutomationUpsert {
                    request_id: format!("create-{id}"),
                    id: Some(id.into()),
                    agent_id: Some(agent_id.into()),
                    name: name.into(),
                    prompt: format!("Run the {name} task."),
                    schedule: "@daily".into(),
                    trigger: Some(AutomationTrigger::Schedule {
                        schedule: "@daily".into(),
                    }),
                    enabled: true,
                })
                .expect("create agent automation");
            drain(&controller);
        }

        controller
            .execute(FeatureCommand::AutomationList {
                request_id: "research-list".into(),
                agent_id: Some("research-bot".into()),
            })
            .expect("list research automations");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationListed { automations, .. }
                if automations.len() == 1
                    && automations[0].id == "research-digest"
                    && automations[0].agent_id.as_deref() == Some("research-bot")
        )));

        let error = controller
            .execute(FeatureCommand::AutomationSetEnabled {
                request_id: "wrong-owner-pause".into(),
                id: "research-digest".into(),
                agent_id: Some("incident-bot".into()),
                enabled: false,
            })
            .expect_err("cross-agent automation mutation must be rejected");
        assert!(
            error
                .to_string()
                .contains("does not belong to agent incident-bot")
        );

        controller
            .execute(FeatureCommand::AutomationRun {
                request_id: "research-run".into(),
                id: "research-digest".into(),
                agent_id: Some("research-bot".into()),
            })
            .expect("run research automation");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::ChatMessage { role: MessageRole::Assistant, text, .. }
                if text.starts_with("research-bot机器人收到：")
        )));

        let error = controller
            .execute(FeatureCommand::AutomationDelete {
                request_id: "wrong-owner-delete".into(),
                id: "research-digest".into(),
                agent_id: Some("incident-bot".into()),
            })
            .expect_err("cross-agent automation deletion must be rejected");
        assert!(
            error
                .to_string()
                .contains("does not belong to agent incident-bot")
        );
    }

    #[test]
    fn cloud_task_resource_id_only_accepts_structured_run_keys() {
        assert_eq!(
            cloud_task_resource_id(Some(&json!({"bcId": "cloud-run-1"}))).as_deref(),
            Some("cloud-run-1")
        );
        assert_eq!(
            cloud_task_resource_id(Some(&json!({"metadata": {"runId": "cloud-run-2"}}))).as_deref(),
            Some("cloud-run-2")
        );
        assert_eq!(
            cloud_task_resource_id(Some(&json!({"id": "generic-step-id"}))),
            None
        );
        assert_eq!(cloud_task_resource_id(Some(&json!({"runId": "   "}))), None);
    }

    #[test]
    fn mcp_settings_use_refresh_event_contract_in_test_mode() {
        let controller = controller();
        drain(&controller);
        controller
            .execute(FeatureCommand::McpSetCustomInstructions {
                request_id: "mcp-instructions-test".into(),
                server: "docs".into(),
                instructions: "Prefer source links.".into(),
            })
            .expect("set MCP custom instructions in test mode");
        controller
            .execute(FeatureCommand::McpSetToolDisabled {
                request_id: "mcp-tool-disable-test".into(),
                server: "docs".into(),
                tool: "delete_page".into(),
                disabled: true,
            })
            .expect("disable MCP tool in test mode");
        let refreshed = drain(&controller)
            .into_iter()
            .filter(|event| matches!(event, HostEvent::McpRefreshed { .. }))
            .count();
        assert_eq!(refreshed, 2);
    }

    #[test]
    fn mcp_instruction_context_is_sorted_bounded_and_hidden() {
        let instructions = std::collections::HashMap::from([
            ("zeta".into(), "Use read-only operations.".into()),
            ("alpha".into(), "Always include source links.".into()),
            ("empty".into(), "   ".into()),
        ]);
        let context = render_mcp_instruction_context(&instructions).expect("MCP context");
        assert!(context.starts_with("[MCP connector operating instructions]"));
        assert!(
            context.find("Connector: alpha").unwrap() < context.find("Connector: zeta").unwrap()
        );
        assert!(!context.contains("Connector: empty"));
        assert!(context.len() < 16_000);
    }

    #[test]
    fn mcp_remove_uses_refresh_event_contract_in_test_mode() {
        let controller = controller();
        drain(&controller);
        controller
            .execute(FeatureCommand::McpRemove {
                request_id: "mcp-remove-test".into(),
                server: "docs".into(),
            })
            .expect("remove MCP server in test mode");
        assert!(
            drain(&controller)
                .into_iter()
                .any(|event| matches!(event, HostEvent::McpRefreshed { .. }))
        );
    }

    #[test]
    fn product_surfaces_emit_stateful_events_without_leaking_secrets() {
        let controller = controller();
        drain(&controller);

        controller
            .execute(FeatureCommand::ConnectorList {
                request_id: "connector-list".into(),
            })
            .expect("list connectors");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::ConnectorListed { connectors, .. }
                if connectors.iter().any(|connector| connector.id == "github")
        )));

        controller
            .execute(FeatureCommand::ConnectorConnect {
                request_id: "connector-connect".into(),
                connector_id: "github".into(),
                account_label: Some("Work".into()),
            })
            .expect("connect GitHub");
        let events = drain(&controller);
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::ConnectorChanged { connector, .. }
                if connector.id == "github"
                    && connector.status == ConnectorStatus::Connected
                    && connector.accounts.iter().any(|account| account.label == "Work")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::ListenerChanged { integration, .. }
                if integration.platform == ListenerPlatform::Github
                    && integration.is_connected
        )));

        controller
            .execute(FeatureCommand::ConnectorSetToolEnabled {
                request_id: "connector-tool".into(),
                connector_id: "github".into(),
                tool_id: "create_issue".into(),
                enabled: false,
            })
            .expect("disable connector tool");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::ConnectorChanged { action, connector, .. }
                if action == "toolChanged"
                    && connector.tools.iter().any(|tool| tool.id == "create_issue" && !tool.enabled)
        )));

        controller
            .execute(FeatureCommand::SkillUpsert {
                request_id: "skill-create".into(),
                id: Some("skill-release-check".into()),
                name: "Release check".into(),
                description: "Verify a release candidate before publishing.".into(),
                use_when: "Use before a production release.".into(),
                instructions: "Run tests, inspect diffs, and summarize risks.".into(),
                owner_agent_id: Some("mahayana-assistant".into()),
            })
            .expect("create skill");
        drain(&controller);
        controller
            .execute(FeatureCommand::SkillPublish {
                request_id: "skill-publish".into(),
                id: "skill-release-check".into(),
                team_id: "team-mahayana".into(),
            })
            .expect("publish skill");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::SkillChanged { action, skill, .. }
                if action == "published"
                    && skill.publish_state == SkillPublishState::Published
                    && skill.team_id.as_deref() == Some("team-mahayana")
        )));

        controller
            .execute(FeatureCommand::BotSetHidden {
                request_id: "bot-hide".into(),
                id: "research-bot".into(),
                hidden: true,
            })
            .expect("hide bot");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::BotChanged { bot, .. }
                if bot.id == "research-bot" && bot.hidden
        )));

        controller
            .execute(FeatureCommand::DraftResolve {
                request_id: "draft-send".into(),
                draft: MessageDraft::Email {
                    id: "draft-1".into(),
                    from: None,
                    to: vec!["person@example.com".into()],
                    cc: None,
                    subject: "Release ready".into(),
                    body: "The release candidate passed validation.".into(),
                    status: DraftSendState::Editable,
                    error: None,
                },
                action: DraftAction::Send,
            })
            .expect("send draft");
        let events = drain(&controller);
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::DraftChanged {
                status: DraftSendState::Sending,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::DraftChanged {
                status: DraftSendState::Sent,
                ..
            }
        )));

        controller
            .execute(FeatureCommand::SecretProvide {
                request_id: "secret-provide".into(),
                secret_request_id: "deployment-token".into(),
                value: "super-secret-value".into(),
            })
            .expect("provide secret");
        let secret_events = drain(&controller);
        let serialized = serde_json::to_string(&secret_events).expect("serialize events");
        assert!(!serialized.contains("super-secret-value"));
        assert!(secret_events.into_iter().any(|event| matches!(
            event,
            HostEvent::SecretProvided { secret_request_id, .. }
                if secret_request_id == "deployment-token"
        )));

        controller
            .execute(FeatureCommand::AutomationUpsert {
                request_id: "event-routine-create".into(),
                id: Some("regression-triage".into()),
                agent_id: None,
                name: "Regression triage".into(),
                prompt: "Inspect the regression and summarize impact.".into(),
                schedule: "event:sentry:issue.regressed".into(),
                trigger: Some(AutomationTrigger::Event {
                    source: ListenerPlatform::Sentry,
                    event: "issue.regressed".into(),
                    filter: Some("web".into()),
                }),
                enabled: true,
            })
            .expect("create event routine");
        drain(&controller);
        assert_eq!(
            controller
                .ingest_listener_event(EventCard {
                    source: ListenerPlatform::Sentry,
                    event: "issue.regressed".into(),
                    title: "Checkout regression".into(),
                    summary: "A production regression was detected.".into(),
                    url: Some("https://sentry.example.invalid/issues/42".into()),
                    actor: Some("sentry".into()),
                    fields: Some(vec![EventField {
                        label: "Project".into(),
                        value: "web".into(),
                    }]),
                    occurred_at_ms: Some(now_millis()),
                })
                .expect("ingest listener event"),
            1
        );
        let event_events = drain(&controller);
        assert!(event_events.iter().any(|event| matches!(
            event,
            HostEvent::TranscriptCard {
                card: TranscriptCard::Event { event },
                ..
            } if event.source == ListenerPlatform::Sentry
                && event.event == "issue.regressed"
                && event.title == "Checkout regression"
        )));
        assert!(event_events.iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, automation, .. }
                if action == "triggered" && automation.id == "regression-triage"
        )));

        controller
            .execute(FeatureCommand::UpdateCheck {
                request_id: "update-check".into(),
            })
            .expect("check updates");
        let events = drain(&controller);
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::UpdateChanged {
                state: UpdateState::Checking,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::UpdateChanged {
                state: UpdateState::UpToDate { .. },
                ..
            }
        )));
    }

    #[test]
    fn automation_store_round_trips_atomically() {
        let root = std::env::temp_dir().join(format!(
            "fabushi-automation-store-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let path = root.join("automations.json");
        let mut items = BTreeMap::new();
        items.insert(
            "weekly-review".into(),
            AutomationSummary {
                id: "weekly-review".into(),
                agent_id: None,
                name: "每周复盘".into(),
                prompt: "整理本周工作。".into(),
                schedule: "@weekly".into(),
                trigger: Some(AutomationTrigger::Schedule {
                    schedule: "@weekly".into(),
                }),
                enabled: false,
                created_at_ms: 1,
                last_run_at_ms: None,
                next_run_at_ms: None,
            },
        );
        persist_automations(&path, &items).expect("persist automation store");
        let loaded = load_automations(&path);
        assert_eq!(loaded.get("weekly-review"), items.get("weekly-review"));
        std::fs::remove_dir_all(&root).expect("remove isolated automation store");
    }

    #[test]
    fn deterministic_rust_backend_executes_every_declared_feature_journey() {
        let controller = controller();
        assert_eq!(drain(&controller)[0].kind(), "host.ready");

        controller
            .execute(FeatureCommand::ChatSend {
                request_id: "chat-1".into(),
                text: "验证极速自动化测试".into(),
                agent_id: None,
                conversation_id: None,
                mode: AgentMode::Agent,
                mode_statement: None,
                model: None,
                attachments: Vec::new(),
            })
            .expect("chat");
        controller
            .execute(FeatureCommand::MarketplaceInstall {
                request_id: "install-1".into(),
                mini_app_id: "global-dharma".into(),
            })
            .expect("install");
        controller
            .execute(FeatureCommand::MiniAppOpen {
                request_id: "open-1".into(),
                mini_app_id: "global-dharma".into(),
            })
            .expect("open");
        controller
            .execute(FeatureCommand::CapabilityRequest {
                request_id: "capability-1".into(),
                mini_app_id: "global-dharma".into(),
                capability: "camera".into(),
                reason: "scan scripture".into(),
            })
            .expect("capability");
        let approval_id = drain(&controller)
            .into_iter()
            .find_map(|event| match event {
                HostEvent::ApprovalRequested { approval_id, .. } => Some(approval_id),
                _ => None,
            })
            .expect("approval id");
        controller
            .resolve_approval(ApprovalResolution {
                approval_id,
                decision: ApprovalDecision::AllowOnce,
            })
            .expect("resolve approval");
        let operation = controller
            .execute(FeatureCommand::RuntimeLongTask {
                request_id: "operation-1".into(),
                label: "index scriptures".into(),
            })
            .expect("long task")
            .operation_id
            .expect("operation id");
        controller.interrupt(&operation).expect("interrupt");
        controller
            .execute(FeatureCommand::SessionClear {
                request_id: "session-1".into(),
            })
            .expect("clear session");
        controller.close().expect("close");

        let kinds = drain(&controller)
            .into_iter()
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"approval.resolved"));
        assert!(kinds.contains(&"operation.started"));
        assert!(kinds.contains(&"operation.interrupted"));
        assert!(kinds.contains(&"session.cleared"));
        assert!(kinds.contains(&"host.closed"));
    }

    #[test]
    fn deterministic_browser_login_keeps_credentials_out_of_the_presentation_boundary() {
        let login_controller = controller();
        assert_eq!(login_controller.auth_status().unwrap()["loggedIn"], false);

        let attempt = login_controller
            .browser_login_start()
            .expect("start browser login");
        assert_eq!(attempt["attemptId"], "test-browser-login");
        assert_eq!(
            attempt["loginUrl"],
            "about:blank#fabushi-test-browser-login"
        );
        assert!(attempt.get("accessToken").is_none());
        assert!(attempt.get("refreshToken").is_none());
        assert!(attempt.get("password").is_none());

        let completed = login_controller
            .browser_login_poll(
                attempt["attemptId"]
                    .as_str()
                    .expect("attempt id")
                    .to_string(),
            )
            .expect("complete browser login");
        assert_eq!(completed["status"], "completed");
        assert_eq!(completed["auth"]["loggedIn"], true);
        assert!(completed["auth"].get("accessToken").is_none());
        assert!(completed["auth"].get("refreshToken").is_none());
        assert_eq!(login_controller.auth_status().unwrap()["loggedIn"], true);

        let reopened_controller = controller();
        let reopened_attempt = reopened_controller
            .browser_login_start()
            .expect("start reopenable browser login");
        let reopened = reopened_controller
            .browser_login_reopen(
                reopened_attempt["attemptId"]
                    .as_str()
                    .expect("reopen attempt id")
                    .to_string(),
            )
            .expect("reopen browser login");
        assert_eq!(reopened["status"], "pending");
        assert_eq!(reopened["attemptId"], reopened_attempt["attemptId"]);
        assert_eq!(
            reopened["loginUrl"],
            "about:blank#fabushi-test-browser-login"
        );
        assert!(reopened.get("pollSecret").is_none());

        let cancelled_controller = controller();
        let cancelled_attempt = cancelled_controller
            .browser_login_start()
            .expect("start cancellable browser login");
        let cancelled = cancelled_controller
            .browser_login_cancel(
                cancelled_attempt["attemptId"]
                    .as_str()
                    .expect("cancel attempt id")
                    .to_string(),
            )
            .expect("cancel browser login");
        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(
            cancelled_controller.auth_status().unwrap()["loggedIn"],
            false
        );
    }

    #[test]
    fn deterministic_oauth_journey_matches_the_cross_platform_ui_contract() {
        let controller = controller();
        let providers = controller.auth_providers().expect("OAuth providers");
        assert_eq!(providers.as_array().map(Vec::len), Some(4));
        assert_eq!(providers[0]["id"], "google");

        let attempt = controller
            .oauth_start("google".into())
            .expect("start OAuth");
        assert_eq!(attempt["provider"], "google");
        let completed = controller
            .oauth_poll(
                attempt["attemptId"]
                    .as_str()
                    .expect("attempt id")
                    .to_string(),
            )
            .expect("complete OAuth");
        assert_eq!(completed["status"], "completed");
        assert_eq!(completed["auth"]["loggedIn"], true);
        assert_eq!(controller.auth_status().unwrap()["loggedIn"], true);
    }

    #[test]
    fn attachment_store_enforces_limits_content_addressing_and_scoped_reads() {
        let controller = controller();
        drain(&controller);
        let content = b"hello attachment\nsecond line\n";
        controller
            .execute(FeatureCommand::AttachmentUpload {
                request_id: "attachment-upload".into(),
                agent_id: "mahayana-assistant".into(),
                filename: "notes.txt".into(),
                mime_type: Some("text/plain".into()),
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(content),
            })
            .expect("upload attachment");
        let attachment = drain(&controller)
            .into_iter()
            .find_map(|event| match event {
                HostEvent::AttachmentStored { attachment, .. } => Some(attachment),
                _ => None,
            })
            .expect("stored attachment event");
        assert_eq!(attachment.size_bytes, content.len() as u64);
        assert_eq!(attachment.hash.len(), 64);
        assert!(attachment.path.ends_with(".txt"));
        assert!(Path::new(&attachment.path).is_file());

        controller
            .execute(FeatureCommand::AttachmentReadText {
                request_id: "attachment-text".into(),
                agent_id: "mahayana-assistant".into(),
                path: attachment.path.clone(),
            })
            .expect("read attachment text");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AttachmentTextRead { result, .. }
                if result.kind == "text"
                    && result.text.as_deref() == Some("hello attachment\nsecond line\n")
                    && !result.truncated
        )));

        controller
            .execute(FeatureCommand::AttachmentReadChunk {
                request_id: "attachment-chunk".into(),
                agent_id: "mahayana-assistant".into(),
                path: attachment.path.clone(),
                offset: 6,
                length: 10,
            })
            .expect("read attachment chunk");
        assert!(drain(&controller).into_iter().any(|event| match event {
            HostEvent::AttachmentChunkRead { result, .. } => {
                base64::engine::general_purpose::STANDARD
                    .decode(result.bytes_base64)
                    .ok()
                    .as_deref()
                    == Some(b"attachment".as_slice())
            }
            _ => false,
        }));

        assert_eq!(attachment_byte_limit_for_name("clip.mp4"), VIDEO_BYTE_LIMIT);
        assert_eq!(
            attachment_byte_limit_for_name("document.pdf"),
            ATTACHMENT_BYTE_LIMIT
        );

        let outside =
            std::env::temp_dir().join(format!("fabushi-attachment-outside-{}", std::process::id()));
        std::fs::write(&outside, b"outside").expect("write outside fixture");
        let escaped = controller.execute(FeatureCommand::AttachmentReadText {
            request_id: "attachment-escape".into(),
            agent_id: "mahayana-assistant".into(),
            path: outside.to_string_lossy().to_string(),
        });
        assert!(
            matches!(escaped, Err(FeatureHostError::Contract(message)) if message.contains("escapes"))
        );
        let _ = std::fs::remove_file(outside);
    }

    #[test]
    fn agent_messaging_is_async_persistent_and_broadcasts_without_chat_pollution() {
        let controller = controller();
        drain(&controller);
        controller
            .execute(FeatureCommand::BotCreate {
                request_id: "bot-peer".into(),
                name: "Research".into(),
                description: "Research teammate".into(),
                title: "Researcher".into(),
                avatar: None,
                avatar_shape: None,
                avatar_color: None,
            })
            .expect("create peer bot");
        let peer = drain(&controller)
            .into_iter()
            .find_map(|event| match event {
                HostEvent::BotChanged { bot, .. } if bot.name == "Research" => Some(bot),
                _ => None,
            })
            .expect("created peer");

        controller
            .execute(FeatureCommand::AgentSend {
                request_id: "peer-send".into(),
                from_agent_id: "mahayana-assistant".into(),
                target_id: peer.id.clone(),
                text: "Summarize the evidence.".into(),
                priority: true,
            })
            .expect("send peer message");
        let events = drain(&controller);
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::AgentPeerMessageChanged { message, .. }
                if message.from_agent_id == "mahayana-assistant"
                    && message.target_id == peer.id
                    && message.priority
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::AgentBackgroundMessage { agent_id, source, .. }
                if agent_id == &peer.id && source == "agent-priority"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            HostEvent::ChatMessage {
                role: MessageRole::User,
                ..
            }
        )));

        controller
            .execute(FeatureCommand::AgentPeerHistory {
                request_id: "peer-history".into(),
                agent_id: peer.id.clone(),
                limit: 20,
            })
            .expect("load peer history");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AgentPeerHistoryListed { messages, .. }
                if messages.len() == 1 && messages[0].text == "Summarize the evidence."
        )));

        controller
            .execute(FeatureCommand::AgentBroadcast {
                request_id: "broadcast".into(),
                target_ids: None,
                message: "Owner announcement".into(),
            })
            .expect("broadcast");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AgentBroadcasted { result, .. }
                if result.total >= 2 && result.scheduled == result.total
        )));
    }

    #[cfg(not(feature = "production"))]
    #[test]
    fn production_mode_requires_the_explicit_runtime_feature() {
        let error = FeatureHostController::create(
            HostConfig {
                profile_id: "production".into(),
                mode: HostMode::Production,
            },
            SurfacePlatform::Tauri,
        )
        .err()
        .expect("production must not fall back to the test backend");
        assert!(matches!(error, FeatureHostError::ProductionUnavailable));
    }

    #[cfg(feature = "production")]
    #[test]
    fn production_uses_the_real_runtime_and_rust_owned_session_store() {
        let controller = FeatureHostController::create_with_host_config(
            HostConfig {
                profile_id: "production".into(),
                mode: HostMode::Production,
            },
            SurfacePlatform::Tauri,
            isolated_host_config("production"),
        )
        .expect("create feature Host");
        let ready = drain(&controller);
        assert_eq!(ready[0].kind(), "host.ready");
        assert!(
            controller
                .info()
                .runtime_version
                .starts_with("mahayana-abi-")
        );

        controller
            .execute(FeatureCommand::MarketplaceInstall {
                request_id: "install-1".into(),
                mini_app_id: "global-dharma".into(),
            })
            .expect("verify bundled production MiniApp");
        controller
            .execute(FeatureCommand::SessionClear {
                request_id: "session-1".into(),
            })
            .expect("clear isolated Rust session");

        let kinds = drain(&controller)
            .into_iter()
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"marketplace.installed"));
        assert!(kinds.contains(&"session.cleared"));
    }

    #[cfg(feature = "production")]
    #[test]
    fn draft_tool_arguments_match_live_gmail_and_slack_schemas() {
        let email = MessageDraft::Email {
            id: "email-1".into(),
            from: Some("sender@example.com".into()),
            to: vec!["one@example.com".into(), "two@example.com".into()],
            cc: Some(vec!["copy@example.com".into()]),
            subject: "Subject".into(),
            body: "Plain text body".into(),
            status: DraftSendState::Editable,
            error: None,
        };
        let gmail_schema = json!({
            "type": "object",
            "properties": {
                "to": {"type": "string"},
                "cc": {"type": "string"},
                "subject": {"type": "string"},
                "from_address": {"type": ["string", "null"]},
                "payload": {"type": "object"}
            }
        });
        let email_args = draft_tool_arguments(&email, Some(&gmail_schema)).expect("email args");
        assert_eq!(email_args["to"], "one@example.com, two@example.com");
        assert_eq!(email_args["cc"], "copy@example.com");
        assert_eq!(email_args["from_address"], "sender@example.com");
        assert_eq!(email_args["payload"]["mime_type"], "text/plain");
        assert_eq!(email_args["payload"]["body"]["content"], "Plain text body");

        let slack = MessageDraft::Slack {
            id: "slack-1".into(),
            workspace: None,
            target: "C012345".into(),
            thread: Some("1234.56".into()),
            body: "hello".into(),
            status: DraftSendState::Editable,
            error: None,
        };
        let slack_schema = json!({
            "type": "object",
            "properties": {
                "channel_id": {"type": "string"},
                "text": {"type": "string"},
                "thread_ts": {"type": "string"}
            }
        });
        let slack_args = draft_tool_arguments(&slack, Some(&slack_schema)).expect("Slack args");
        assert_eq!(slack_args["channel_id"], "C012345");
        assert_eq!(slack_args["text"], "hello");
        assert_eq!(slack_args["thread_ts"], "1234.56");
    }

    #[cfg(feature = "production")]
    #[test]
    fn production_runtime_events_preserve_streaming_and_terminal_states() {
        let controller = FeatureHostController::create_with_host_config(
            HostConfig {
                profile_id: "production-events".into(),
                mode: HostMode::Production,
            },
            SurfacePlatform::Tauri,
            isolated_host_config("production-events"),
        )
        .expect("create production event Host");
        let operation_id = OperationId("operation-1".into());
        let conversation_id = ConversationId(CODEX_ASSISTANT_CONVERSATION_ID.into());

        let delta = controller
            .translate_runtime_event(RuntimeEvent::MessageDelta {
                operation_id: operation_id.clone(),
                conversation_id: conversation_id.clone(),
                delta: "般若".into(),
            })
            .expect("translate delta")
            .expect("delta event");
        assert!(matches!(
            delta,
            HostEvent::ChatDelta {
                operation_id: ref current,
                ref delta,
                ..
            } if current == "operation-1" && delta == "般若"
        ));

        let completed = controller
            .translate_runtime_event(RuntimeEvent::MessageCompleted {
                operation_id: operation_id.clone(),
                message: mahayana_core::Message {
                    id: mahayana_core::MessageId("message-1".into()),
                    conversation_id,
                    role: RuntimeMessageRole::Assistant,
                    text: "般若波罗蜜多".into(),
                    created_at_ms: 1,
                    metadata: serde_json::json!({}),
                },
            })
            .expect("translate completed message")
            .expect("completed message event");
        assert!(matches!(
            completed,
            HostEvent::ChatMessage {
                operation_id: Some(ref current),
                role: MessageRole::Assistant,
                ref text,
                ..
            } if current == "operation-1" && text == "般若波罗蜜多"
        ));

        let terminal = controller
            .translate_runtime_event(RuntimeEvent::OperationCompleted {
                operation_id: operation_id.clone(),
            })
            .expect("translate completion")
            .expect("completion event");
        assert!(matches!(
            terminal,
            HostEvent::OperationCompleted {
                operation_id: ref current,
                ..
            } if current == "operation-1"
        ));

        let failed = controller
            .translate_runtime_event(RuntimeEvent::OperationFailed {
                operation_id,
                code: "provider_error".into(),
                message: "provider unavailable".into(),
            })
            .expect("translate failure")
            .expect("failure event");
        assert!(matches!(
            failed,
            HostEvent::OperationFailed {
                operation_id: ref current,
                ref code,
                ref message,
                ..
            } if current == "operation-1"
                && code == "provider_error"
                && message == "provider unavailable"
        ));
    }
}
