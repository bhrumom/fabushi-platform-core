//! Versioned product-level commands and events shared by React/Electron, native
//! native mobile shells, deterministic tests, and WebAssembly hosts.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const HOST_PROTOCOL_VERSION: &str = "1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostMode {
    Test,
    Production,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostConfig {
    pub profile_id: String,
    pub mode: HostMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfacePlatform {
    Mock,
    Electron,
    Ios,
    Android,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub runtime_version: String,
    pub protocol_version: String,
    pub platform: SurfacePlatform,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    #[default]
    Agent,
    Ask,
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentContext {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentStored {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub size_bytes: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentTextResult {
    pub path: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub truncated: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentChunkResult {
    pub path: String,
    pub bytes_base64: String,
    pub total_size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentImageResult {
    pub path: String,
    pub data_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListenerPlatform {
    Slack,
    Github,
    Git,
    Teams,
    Linear,
    Sentry,
    Pagerduty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AutomationTrigger {
    Schedule {
        schedule: String,
    },
    Event {
        source: ListenerPlatform,
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectorStatus {
    Connected,
    Disconnected,
    Connecting,
    AuthRequired,
    Error,
    DisabledByTeamAdminPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorTransport {
    Http,
    Sse,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorToolSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAccountSummary {
    pub id: String,
    pub label: String,
    pub status: ConnectorStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_managed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSummary {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub status: ConnectorStatus,
    pub is_team: bool,
    pub can_add_account: bool,
    pub transport: ConnectorTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teammate_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<ConnectorAccountSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ConnectorToolSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillSource {
    Private,
    Team,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillPublishState {
    Local,
    Published,
    Synced,
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub use_when: String,
    pub instructions: String,
    pub source: SkillSource,
    pub publish_state: SkillPublishState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTeamSummary {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub title: String,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<String>,
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    #[serde(default = "default_true")]
    pub notify_on_updates: bool,
    #[serde(default)]
    pub unread: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GroupSpeaker {
    User {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Member {
        id: String,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMessage {
    pub id: String,
    pub speaker: GroupSpeaker,
    pub content: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub member_ids: Vec<String>,
    #[serde(default)]
    pub messages: Vec<GroupMessage>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPeerMessage {
    pub id: String,
    pub from_agent_id: String,
    pub from_agent_name: String,
    pub target_id: String,
    pub target_name: String,
    pub text: String,
    pub priority: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBroadcastResult {
    pub total: usize,
    pub scheduled: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentStatus {
    Running,
    Done,
    Error,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSummary {
    pub id: String,
    pub parent_agent_id: String,
    pub subagent_type: String,
    pub title: String,
    pub status: SubagentStatus,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AsyncTaskKind {
    Subagent,
    Shell,
    CloudAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsyncTaskStatus {
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncTaskSummary {
    pub kind: AsyncTaskKind,
    pub id: String,
    pub parent_agent_id: String,
    pub label: String,
    pub status: AsyncTaskStatus,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
}

pub const TEACH_MAX_DURATION_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeachEntryPoint {
    ScreenHover,
    ComposerMenu,
    FullscreenTitleBar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachRecordingStatus {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    pub max_duration_ms: i64,
}

impl Default for TeachRecordingStatus {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            agent_id: None,
            started_at_ms: None,
            max_duration_ms: TEACH_MAX_DURATION_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachRecordingResult {
    pub agent_id: String,
    pub video_path: String,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub duration_ms: i64,
    pub saved: bool,
}

pub const COMPUTER_MAX_WAIT_MS: u64 = 30_000;
pub const COMPUTER_MAX_ACTIONS_PER_CALL: usize = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerControlOrigin {
    #[default]
    LocalUi,
    RemoteMobile,
    Ai,
}

pub const COMPUTER_CONTROL_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerTargetKind {
    #[default]
    Desktop,
    Window,
    BrowserTab,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerControlTarget {
    #[serde(default = "computer_control_protocol_version")]
    pub protocol_version: u16,
    #[serde(default)]
    pub kind: ComputerTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub generation: u64,
}

const fn computer_control_protocol_version() -> u16 {
    COMPUTER_CONTROL_PROTOCOL_VERSION
}

impl Default for ComputerControlTarget {
    fn default() -> Self {
        Self {
            protocol_version: COMPUTER_CONTROL_PROTOCOL_VERSION,
            kind: ComputerTargetKind::Desktop,
            device_id: None,
            window_id: None,
            browser_session_id: None,
            browser_target_id: None,
            tab_id: None,
            generation: 0,
        }
    }
}

impl ComputerControlTarget {
    pub fn remote_desktop(device_id: impl Into<String>, generation: u64) -> Self {
        Self {
            device_id: Some(device_id.into()),
            generation,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerActionKind {
    Screenshot,
    Click,
    Move,
    Drag,
    Type,
    Key,
    Scroll,
    Wait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerMouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerPoint {
    pub x: i32,
    pub y: i32,
}

/// Stable computer-action envelope. Fields intentionally stay flat so
/// the exact same payload can be produced by the model tool, desktop UI, and a
/// paired phone without translation-specific semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAction {
    pub action: ComputerActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x2: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y2: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<ComputerPoint>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<ComputerMouseButton>,
    #[serde(
        rename = "count",
        alias = "clickCount",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub click_count: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<ComputerScrollDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i32>,
    #[serde(
        rename = "durationMs",
        alias = "waitMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub wait_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerStatus {
    pub platform: String,
    pub available: bool,
    pub capture_supported: bool,
    pub input_supported: bool,
    pub accessibility_granted: bool,
    pub screen_recording_granted: bool,
    pub local_execution_enabled: bool,
    pub route_egress_locally: bool,
    pub remote_control_enabled: bool,
    pub ai_control_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSnapshot {
    pub captured_at_ms: i64,
    pub data_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerActionResult {
    pub origin: ComputerControlOrigin,
    pub actions_executed: usize,
    pub snapshot: ComputerSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Profile,
    Log,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub id: String,
    pub content: String,
    pub created_at: i64,
    pub kind: MemoryKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TrayAction {
    OpenUrl {
        label: String,
        url: String,
    },
    SwitchModel,
    DashboardAction {
        label: String,
        action: String,
        args: BTreeMap<String, String>,
        #[serde(
            rename = "successMessage",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        success_message: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrayErrorKind {
    ProviderOverloaded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorTray {
    pub kind: String,
    pub id: String,
    pub agent_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<TrayErrorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<TrayAction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoReviewBehavior {
    Allow,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoReviewRule {
    pub id: String,
    pub behavior: AutoReviewBehavior,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalToolPermission {
    Never,
    Ask,
    Always,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InferenceProvider {
    #[default]
    Fabushi,
    Codex,
    ClaudeCode,
    #[serde(rename = "openrouter")]
    OpenRouter,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxRuntime {
    #[default]
    Host,
    LocalDocker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductHostSettings {
    pub notifications: bool,
    pub auto_update_when_idle: bool,
    pub local_execution: bool,
    pub route_egress_locally: bool,
    pub security_keys: bool,
    pub webauthn_proxy_enabled: bool,
    pub local_tool_permission: LocalToolPermission,
    /// Remote control is opt-in because it exposes the user's real desktop to
    /// paired clients. Pairing still requires an authenticated device session.
    #[serde(default)]
    pub remote_control_enabled: bool,
    /// The local Agent may use the same Computer executor as a paired phone.
    #[serde(default = "default_true")]
    pub ai_computer_control_enabled: bool,
    #[serde(default)]
    pub auto_review_rules: Vec<AutoReviewRule>,
    /// Provider preference for newly-created Agent conversations. Existing
    /// conversations keep their recorded provider identity.
    #[serde(default)]
    pub inference_provider: InferenceProvider,
    /// Execution environment preference. Adapters must still pass the normal
    /// Mahayana capability and ownership checks before use.
    #[serde(default)]
    pub sandbox_runtime: SandboxRuntime,
}

impl Default for ProductHostSettings {
    fn default() -> Self {
        Self {
            notifications: true,
            auto_update_when_idle: true,
            local_execution: true,
            route_egress_locally: false,
            security_keys: false,
            webauthn_proxy_enabled: false,
            local_tool_permission: LocalToolPermission::Ask,
            remote_control_enabled: false,
            ai_computer_control_enabled: true,
            auto_review_rules: Vec::new(),
            inference_provider: InferenceProvider::default(),
            sandbox_runtime: SandboxRuntime::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSource {
    Workflow,
    Managed,
    Plugin,
    Automation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTrigger {
    pub schedule: String,
    pub is_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub trigger: Option<WorkflowTrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    pub source: WorkflowSource,
    pub plugin_id: Option<String>,
    pub published_by_current_user: bool,
    pub is_enabled_for_agent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_model_invocation: Option<bool>,
    pub schedule_description: Option<String>,
    pub created_at: i64,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    #[serde(default)]
    pub helper_scripts: Vec<String>,
    pub file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMessageMatch {
    pub agent_id: String,
    pub agent_name: String,
    pub conversation_id: String,
    pub entry_id: String,
    pub role: MessageRole,
    pub timestamp_ms: i64,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMediaMatch {
    pub agent_id: String,
    pub agent_name: String,
    pub path: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub size_bytes: u64,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerIntegrationSummary {
    pub platform: ListenerPlatform,
    pub display_name: String,
    pub blurb: String,
    pub is_connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DraftSendState {
    Editable,
    Sending,
    Sent,
    Discarded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MessageDraft {
    Email {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        to: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cc: Option<Vec<String>>,
        subject: String,
        body: String,
        status: DraftSendState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Slack {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        body: String,
        status: DraftSendState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

impl MessageDraft {
    pub fn id(&self) -> &str {
        match self {
            Self::Email { id, .. } | Self::Slack { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DraftAction {
    Send,
    Discard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetSheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCard {
    pub source: ListenerPlatform,
    pub event: String,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<EventField>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TranscriptCard {
    #[serde(rename = "emailDraft")]
    EmailDraft { draft: MessageDraft },
    #[serde(rename = "slackDraft")]
    SlackDraft { draft: MessageDraft },
    #[serde(rename = "secretRequest")]
    SecretRequest {
        #[serde(rename = "requestId")]
        request_id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        provided: bool,
    },
    #[serde(rename = "listenerConnect")]
    ListenerConnect {
        platform: ListenerPlatform,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        connected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pending: Option<bool>,
    },
    #[serde(rename = "event")]
    Event { event: EventCard },
    #[serde(rename = "pdf")]
    Pdf {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(
            rename = "dataBase64",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        data_base64: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_count: Option<u32>,
    },
    #[serde(rename = "spreadsheet")]
    Spreadsheet {
        name: String,
        sheets: Vec<SpreadsheetSheet>,
    },
    #[serde(rename = "miniApp")]
    MiniApp {
        #[serde(rename = "miniAppId")]
        mini_app_id: String,
        name: String,
        html: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateDisabledReason {
    NotPackaged,
    LabBuild,
    UnsupportedPlatform,
    DisabledByEnv,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UpdateState {
    Loading,
    Disabled {
        reason: UpdateDisabledReason,
    },
    Checking,
    Available {
        version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notes: Option<String>,
    },
    Downloading {
        version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<u8>,
    },
    Staging {
        version: String,
    },
    Ready {
        version: String,
    },
    UpToDate {
        version: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FeatureCommand {
    #[serde(rename = "chat.send")]
    ChatSend {
        #[serde(rename = "requestId")]
        request_id: String,
        text: String,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(
            rename = "conversationId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        conversation_id: Option<String>,
        #[serde(default)]
        mode: AgentMode,
        #[serde(
            rename = "modeStatement",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        mode_statement: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<AttachmentContext>,
    },
    #[serde(rename = "conversation.list")]
    ConversationList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
    },
    #[serde(rename = "conversation.open")]
    ConversationOpen {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "conversationId")]
        conversation_id: String,
    },
    #[serde(rename = "capability.list")]
    CapabilityList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
    },
    #[serde(rename = "automation.list")]
    AutomationList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    #[serde(rename = "automation.upsert")]
    AutomationUpsert {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        name: String,
        prompt: String,
        schedule: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<AutomationTrigger>,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    #[serde(rename = "automation.setEnabled")]
    AutomationSetEnabled {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        enabled: bool,
    },
    #[serde(rename = "automation.delete")]
    AutomationDelete {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    #[serde(rename = "automation.run")]
    AutomationRun {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    #[serde(rename = "marketplace.install")]
    MarketplaceInstall {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "miniAppId")]
        mini_app_id: String,
    },
    #[serde(rename = "miniapp.open")]
    MiniAppOpen {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "miniAppId")]
        mini_app_id: String,
    },
    #[serde(rename = "messaging.execute")]
    MessagingExecute {
        #[serde(rename = "requestId")]
        request_id: String,
        envelope: Value,
    },
    #[serde(rename = "capability.request")]
    CapabilityRequest {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "miniAppId")]
        mini_app_id: String,
        capability: String,
        reason: String,
    },
    #[serde(rename = "connector.list")]
    ConnectorList {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "connector.connect")]
    ConnectorConnect {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "connectorId")]
        connector_id: String,
        #[serde(
            rename = "accountLabel",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        account_label: Option<String>,
    },
    #[serde(rename = "connector.renameAccount")]
    ConnectorRenameAccount {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "connectorId")]
        connector_id: String,
        #[serde(rename = "accountId")]
        account_id: String,
        label: String,
    },
    #[serde(rename = "connector.removeAccount")]
    ConnectorRemoveAccount {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "connectorId")]
        connector_id: String,
        #[serde(rename = "accountId")]
        account_id: String,
    },
    #[serde(rename = "connector.setToolEnabled")]
    ConnectorSetToolEnabled {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "connectorId")]
        connector_id: String,
        #[serde(rename = "toolId")]
        tool_id: String,
        enabled: bool,
    },
    #[serde(rename = "skill.list")]
    SkillList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    #[serde(rename = "skill.upsert")]
    SkillUpsert {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        name: String,
        description: String,
        #[serde(rename = "useWhen")]
        use_when: String,
        instructions: String,
        #[serde(
            rename = "ownerAgentId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        owner_agent_id: Option<String>,
    },
    #[serde(rename = "skill.delete")]
    SkillDelete {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "skill.publish")]
    SkillPublish {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        #[serde(rename = "teamId")]
        team_id: String,
    },
    #[serde(rename = "skill.unpublish")]
    SkillUnpublish {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "skill.sync")]
    SkillSync {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "bot.list")]
    BotList {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "bot.create")]
    BotCreate {
        #[serde(rename = "requestId")]
        request_id: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        avatar: Option<String>,
        #[serde(
            rename = "avatarShape",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        avatar_shape: Option<String>,
        #[serde(
            rename = "avatarColor",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        avatar_color: Option<String>,
    },
    #[serde(rename = "bot.update")]
    BotUpdate {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        avatar: Option<String>,
        #[serde(
            rename = "avatarShape",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        avatar_shape: Option<String>,
        #[serde(
            rename = "avatarColor",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        avatar_color: Option<String>,
        #[serde(
            rename = "notificationsEnabled",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        notifications_enabled: Option<bool>,
        #[serde(
            rename = "notifyOnUpdates",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        notify_on_updates: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unread: Option<bool>,
    },
    #[serde(rename = "bot.clone")]
    BotClone {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "bot.delete")]
    BotDelete {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "bot.setHidden")]
    BotSetHidden {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        hidden: bool,
    },
    #[serde(rename = "group.list")]
    GroupList {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "group.create")]
    GroupCreate {
        #[serde(rename = "requestId")]
        request_id: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(rename = "memberIds")]
        member_ids: Vec<String>,
    },
    #[serde(rename = "group.update")]
    GroupUpdate {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(rename = "memberIds", default, skip_serializing_if = "Option::is_none")]
        member_ids: Option<Vec<String>>,
    },
    #[serde(rename = "group.delete")]
    GroupDelete {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "group.send")]
    GroupSend {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        text: String,
    },
    #[serde(rename = "agent.send")]
    AgentSend {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "fromAgentId")]
        from_agent_id: String,
        #[serde(rename = "targetId")]
        target_id: String,
        text: String,
        #[serde(default)]
        priority: bool,
    },
    #[serde(rename = "agent.broadcast")]
    AgentBroadcast {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "targetIds", default, skip_serializing_if = "Option::is_none")]
        target_ids: Option<Vec<String>>,
        message: String,
    },
    #[serde(rename = "agent.peerHistory")]
    AgentPeerHistory {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(default = "default_peer_history_limit")]
        limit: usize,
    },
    #[serde(rename = "subagent.list")]
    SubagentList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    #[serde(rename = "asyncTask.list")]
    AsyncTaskList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    #[serde(rename = "teach.status")]
    TeachStatus {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "teach.start")]
    TeachStart {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "entryPoint")]
        entry_point: TeachEntryPoint,
    },
    #[serde(rename = "teach.stop")]
    TeachStop {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        save: bool,
    },
    #[serde(rename = "computer.status")]
    ComputerStatus {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "computer.screenshot")]
    ComputerScreenshot {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default)]
        origin: ComputerControlOrigin,
        #[serde(rename = "sessionId", default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default)]
        target: ComputerControlTarget,
    },
    #[serde(rename = "computer.action")]
    ComputerAction {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default)]
        origin: ComputerControlOrigin,
        #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(rename = "sessionId", default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default)]
        target: ComputerControlTarget,
        action: ComputerAction,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        then: Vec<ComputerAction>,
    },
    #[serde(rename = "remoteComputer.register")]
    RemoteComputerRegister {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        platform: Option<String>,
        #[serde(
            rename = "appVersion",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        app_version: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
    },
    #[serde(rename = "remoteComputer.heartbeat")]
    RemoteComputerHeartbeat {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
    },
    #[serde(rename = "remoteComputer.clients")]
    RemoteComputerClients {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
    },
    #[serde(rename = "remoteComputer.clientRevoke")]
    RemoteComputerClientRevoke {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(rename = "clientId")]
        client_id: String,
    },
    #[serde(rename = "remoteComputer.sessions")]
    RemoteComputerSessions {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
    },
    #[serde(rename = "remoteComputer.sessionActivate")]
    RemoteComputerSessionActivate {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "remoteComputer.sessionClose")]
    RemoteComputerSessionClose {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "remoteComputer.signal")]
    RemoteComputerSignal {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        kind: String,
        payload: Value,
    },
    #[serde(rename = "remoteComputer.signalDrain")]
    RemoteComputerSignalDrain {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "afterSignalId", default)]
        after_signal_id: i64,
    },
    #[serde(rename = "memory.list")]
    MemoryList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(default = "default_memory_limit")]
        limit: usize,
    },
    #[serde(rename = "memory.add")]
    MemoryAdd {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        content: String,
        kind: MemoryKind,
    },
    #[serde(rename = "memory.remove")]
    MemoryRemove {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        id: String,
    },
    #[serde(rename = "memory.clear")]
    MemoryClear {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    #[serde(rename = "tray.list")]
    TrayList {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "tray.dismiss")]
    TrayDismiss {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "tray.clear")]
    TrayClear {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "tray.clearForAgent")]
    TrayClearForAgent {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    #[serde(rename = "workflow.list")]
    WorkflowList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    #[serde(rename = "workflow.upsert")]
    WorkflowUpsert {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        name: String,
        #[serde(default)]
        description: String,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<WorkflowTrigger>,
        #[serde(rename = "sourceRef", default, skip_serializing_if = "Option::is_none")]
        source_ref: Option<String>,
    },
    #[serde(rename = "workflow.setEnabled")]
    WorkflowSetEnabled {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        id: String,
        enabled: bool,
    },
    #[serde(rename = "workflow.delete")]
    WorkflowDelete {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        id: String,
    },
    #[serde(rename = "workflow.run")]
    WorkflowRun {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        id: String,
    },
    #[serde(rename = "workflow.importMarkdown")]
    WorkflowImportMarkdown {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        markdown: String,
        #[serde(
            rename = "fallbackName",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        fallback_name: Option<String>,
    },
    #[serde(rename = "workflow.importLiveSource")]
    WorkflowImportLiveSource {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        source: String,
        #[serde(
            rename = "fallbackName",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        fallback_name: Option<String>,
    },
    #[serde(rename = "attachment.upload")]
    AttachmentUpload {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        filename: String,
        #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(rename = "bytesBase64")]
        bytes_base64: String,
    },
    #[serde(rename = "attachment.readText")]
    AttachmentReadText {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        path: String,
    },
    #[serde(rename = "attachment.readChunk")]
    AttachmentReadChunk {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        path: String,
        offset: u64,
        length: u64,
    },
    #[serde(rename = "attachment.readImage")]
    AttachmentReadImage {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        path: String,
    },
    #[serde(rename = "search.messages")]
    SearchMessages {
        #[serde(rename = "requestId")]
        request_id: String,
        query: String,
        #[serde(default = "default_search_limit")]
        limit: usize,
    },
    #[serde(rename = "search.media")]
    SearchMedia {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default)]
        query: String,
        #[serde(default = "default_search_limit")]
        limit: usize,
    },
    #[serde(rename = "mcp.list")]
    McpList {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "mcp.apps")]
    McpApps {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "mcp.oauthLogin")]
    McpOauthLogin {
        #[serde(rename = "requestId")]
        request_id: String,
        server: String,
    },
    #[serde(rename = "mcp.oauthLogout")]
    McpOauthLogout {
        #[serde(rename = "requestId")]
        request_id: String,
        server: String,
    },
    #[serde(rename = "mcp.remove")]
    McpRemove {
        #[serde(rename = "requestId")]
        request_id: String,
        server: String,
    },
    #[serde(rename = "mcp.setCustomInstructions")]
    McpSetCustomInstructions {
        #[serde(rename = "requestId")]
        request_id: String,
        server: String,
        instructions: String,
    },
    #[serde(rename = "mcp.setToolDisabled")]
    McpSetToolDisabled {
        #[serde(rename = "requestId")]
        request_id: String,
        server: String,
        tool: String,
        disabled: bool,
    },
    #[serde(rename = "mcp.refresh")]
    McpRefresh {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "mcp.toolCall")]
    McpToolCall {
        #[serde(rename = "requestId")]
        request_id: String,
        server: String,
        tool: String,
        #[serde(default)]
        arguments: Value,
    },
    #[serde(rename = "settings.get")]
    SettingsGet {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "settings.update")]
    SettingsUpdate {
        #[serde(rename = "requestId")]
        request_id: String,
        settings: ProductHostSettings,
    },
    #[serde(rename = "audit.list")]
    AuditList {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(default = "default_audit_limit")]
        limit: usize,
    },
    #[serde(rename = "draft.resolve")]
    DraftResolve {
        #[serde(rename = "requestId")]
        request_id: String,
        draft: MessageDraft,
        action: DraftAction,
    },
    #[serde(rename = "secret.provide")]
    SecretProvide {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "secretRequestId")]
        secret_request_id: String,
        value: String,
    },
    #[serde(rename = "listener.list")]
    ListenerList {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "listener.connect")]
    ListenerConnect {
        #[serde(rename = "requestId")]
        request_id: String,
        platform: ListenerPlatform,
    },
    #[serde(rename = "listener.disconnect")]
    ListenerDisconnect {
        #[serde(rename = "requestId")]
        request_id: String,
        platform: ListenerPlatform,
    },
    #[serde(rename = "update.status")]
    UpdateStatus {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "update.check")]
    UpdateCheck {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "update.install")]
    UpdateInstall {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "runtime.longTask")]
    RuntimeLongTask {
        #[serde(rename = "requestId")]
        request_id: String,
        label: String,
    },
    #[serde(rename = "session.clear")]
    SessionClear {
        #[serde(rename = "requestId")]
        request_id: String,
    },
}

impl FeatureCommand {
    pub fn request_id(&self) -> &str {
        match self {
            Self::ChatSend { request_id, .. }
            | Self::ConversationList { request_id, .. }
            | Self::ConversationOpen { request_id, .. }
            | Self::CapabilityList { request_id, .. }
            | Self::AutomationList { request_id, .. }
            | Self::AutomationUpsert { request_id, .. }
            | Self::AutomationSetEnabled { request_id, .. }
            | Self::AutomationDelete { request_id, .. }
            | Self::AutomationRun { request_id, .. }
            | Self::MarketplaceInstall { request_id, .. }
            | Self::MiniAppOpen { request_id, .. }
            | Self::MessagingExecute { request_id, .. }
            | Self::CapabilityRequest { request_id, .. }
            | Self::ConnectorList { request_id }
            | Self::ConnectorConnect { request_id, .. }
            | Self::ConnectorRenameAccount { request_id, .. }
            | Self::ConnectorRemoveAccount { request_id, .. }
            | Self::ConnectorSetToolEnabled { request_id, .. }
            | Self::SkillList { request_id, .. }
            | Self::SkillUpsert { request_id, .. }
            | Self::SkillDelete { request_id, .. }
            | Self::SkillPublish { request_id, .. }
            | Self::SkillUnpublish { request_id, .. }
            | Self::SkillSync { request_id, .. }
            | Self::BotList { request_id }
            | Self::BotCreate { request_id, .. }
            | Self::BotUpdate { request_id, .. }
            | Self::BotClone { request_id, .. }
            | Self::BotDelete { request_id, .. }
            | Self::BotSetHidden { request_id, .. }
            | Self::GroupList { request_id }
            | Self::GroupCreate { request_id, .. }
            | Self::GroupUpdate { request_id, .. }
            | Self::GroupDelete { request_id, .. }
            | Self::GroupSend { request_id, .. }
            | Self::AgentSend { request_id, .. }
            | Self::AgentBroadcast { request_id, .. }
            | Self::AgentPeerHistory { request_id, .. }
            | Self::SubagentList { request_id, .. }
            | Self::AsyncTaskList { request_id, .. }
            | Self::TeachStatus { request_id }
            | Self::TeachStart { request_id, .. }
            | Self::TeachStop { request_id, .. }
            | Self::ComputerStatus { request_id }
            | Self::ComputerScreenshot { request_id, .. }
            | Self::ComputerAction { request_id, .. }
            | Self::RemoteComputerRegister { request_id, .. }
            | Self::RemoteComputerHeartbeat { request_id, .. }
            | Self::RemoteComputerClients { request_id, .. }
            | Self::RemoteComputerClientRevoke { request_id, .. }
            | Self::RemoteComputerSessions { request_id, .. }
            | Self::RemoteComputerSessionActivate { request_id, .. }
            | Self::RemoteComputerSessionClose { request_id, .. }
            | Self::RemoteComputerSignal { request_id, .. }
            | Self::RemoteComputerSignalDrain { request_id, .. }
            | Self::MemoryList { request_id, .. }
            | Self::MemoryAdd { request_id, .. }
            | Self::MemoryRemove { request_id, .. }
            | Self::MemoryClear { request_id, .. }
            | Self::TrayList { request_id }
            | Self::TrayDismiss { request_id, .. }
            | Self::TrayClear { request_id }
            | Self::TrayClearForAgent { request_id, .. }
            | Self::WorkflowList { request_id, .. }
            | Self::WorkflowUpsert { request_id, .. }
            | Self::WorkflowSetEnabled { request_id, .. }
            | Self::WorkflowDelete { request_id, .. }
            | Self::WorkflowRun { request_id, .. }
            | Self::WorkflowImportMarkdown { request_id, .. }
            | Self::WorkflowImportLiveSource { request_id, .. }
            | Self::AttachmentUpload { request_id, .. }
            | Self::AttachmentReadText { request_id, .. }
            | Self::AttachmentReadChunk { request_id, .. }
            | Self::AttachmentReadImage { request_id, .. }
            | Self::SearchMessages { request_id, .. }
            | Self::SearchMedia { request_id, .. }
            | Self::McpList { request_id }
            | Self::McpApps { request_id }
            | Self::McpOauthLogin { request_id, .. }
            | Self::McpOauthLogout { request_id, .. }
            | Self::McpRemove { request_id, .. }
            | Self::McpSetCustomInstructions { request_id, .. }
            | Self::McpSetToolDisabled { request_id, .. }
            | Self::McpRefresh { request_id }
            | Self::McpToolCall { request_id, .. }
            | Self::SettingsGet { request_id }
            | Self::SettingsUpdate { request_id, .. }
            | Self::AuditList { request_id, .. }
            | Self::DraftResolve { request_id, .. }
            | Self::SecretProvide { request_id, .. }
            | Self::ListenerList { request_id }
            | Self::ListenerConnect { request_id, .. }
            | Self::ListenerDisconnect { request_id, .. }
            | Self::UpdateStatus { request_id }
            | Self::UpdateCheck { request_id }
            | Self::UpdateInstall { request_id }
            | Self::RuntimeLongTask { request_id, .. }
            | Self::SessionClear { request_id } => request_id,
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn default_memory_limit() -> usize {
    1000
}

const fn default_search_limit() -> usize {
    50
}

const fn default_peer_history_limit() -> usize {
    200
}

const fn default_audit_limit() -> usize {
    200
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandAccepted {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub pinned: bool,
    pub unread_count: u32,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub id: String,
    pub role: MessageRole,
    pub text: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySummary {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub mention: String,
    pub conversation_id: String,
    pub provider: String,
    pub plugin_id: Option<String>,
    pub description: String,
    pub required_permissions: Vec<String>,
    pub availability: String,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSummary {
    pub id: String,
    #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub schedule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<AutomationTrigger>,
    pub enabled: bool,
    pub created_at_ms: i64,
    pub last_run_at_ms: Option<i64>,
    pub next_run_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowSession,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolution {
    pub approval_id: String,
    pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStepStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HostEvent {
    #[serde(rename = "host.ready")]
    HostReady { timestamp: String, info: HostInfo },
    #[serde(rename = "chat.message")]
    ChatMessage {
        timestamp: String,
        role: MessageRole,
        text: String,
        #[serde(
            rename = "operationId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        operation_id: Option<String>,
    },
    #[serde(rename = "chat.delta")]
    ChatDelta {
        timestamp: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        delta: String,
    },
    #[serde(rename = "transcript.card")]
    TranscriptCard {
        timestamp: String,
        #[serde(rename = "entryId")]
        entry_id: String,
        #[serde(
            rename = "operationId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        operation_id: Option<String>,
        card: TranscriptCard,
    },
    #[serde(rename = "draft.changed")]
    DraftChanged {
        timestamp: String,
        #[serde(rename = "draftId")]
        draft_id: String,
        status: DraftSendState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    #[serde(rename = "secret.provided")]
    SecretProvided {
        timestamp: String,
        #[serde(rename = "secretRequestId")]
        secret_request_id: String,
    },
    #[serde(rename = "conversation.listed")]
    ConversationListed {
        timestamp: String,
        conversations: Vec<ConversationSummary>,
    },
    #[serde(rename = "conversation.opened")]
    ConversationOpened {
        timestamp: String,
        #[serde(rename = "conversationId")]
        conversation_id: String,
        messages: Vec<ConversationMessage>,
    },
    #[serde(rename = "messaging.event")]
    MessagingEvent {
        timestamp: String,
        #[serde(rename = "requestId")]
        request_id: String,
        envelope: Value,
    },
    #[serde(rename = "capability.listed")]
    CapabilityListed {
        timestamp: String,
        capabilities: Vec<CapabilitySummary>,
    },
    #[serde(rename = "automation.listed")]
    AutomationListed {
        timestamp: String,
        automations: Vec<AutomationSummary>,
    },
    #[serde(rename = "automation.changed")]
    AutomationChanged {
        timestamp: String,
        action: String,
        automation: AutomationSummary,
    },
    #[serde(rename = "connector.listed")]
    ConnectorListed {
        timestamp: String,
        connectors: Vec<ConnectorSummary>,
    },
    #[serde(rename = "connector.changed")]
    ConnectorChanged {
        timestamp: String,
        action: String,
        connector: ConnectorSummary,
    },
    #[serde(rename = "connector.oauthRequested")]
    ConnectorOauthRequested {
        timestamp: String,
        #[serde(rename = "connectorId")]
        connector_id: String,
        #[serde(rename = "authorizationUrl")]
        authorization_url: String,
    },
    #[serde(rename = "skill.listed")]
    SkillListed {
        timestamp: String,
        skills: Vec<SkillSummary>,
        teams: Vec<SkillTeamSummary>,
    },
    #[serde(rename = "skill.changed")]
    SkillChanged {
        timestamp: String,
        action: String,
        skill: SkillSummary,
    },
    #[serde(rename = "bot.listed")]
    BotListed {
        timestamp: String,
        bots: Vec<BotSummary>,
    },
    #[serde(rename = "bot.changed")]
    BotChanged {
        timestamp: String,
        action: String,
        bot: BotSummary,
    },
    #[serde(rename = "group.listed")]
    GroupListed {
        timestamp: String,
        groups: Vec<GroupSummary>,
    },
    #[serde(rename = "group.changed")]
    GroupChanged {
        timestamp: String,
        action: String,
        group: GroupSummary,
    },
    #[serde(rename = "group.delta")]
    GroupDelta {
        timestamp: String,
        #[serde(rename = "groupId")]
        group_id: String,
        #[serde(rename = "memberId")]
        member_id: String,
        #[serde(rename = "memberName")]
        member_name: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        delta: String,
    },
    #[serde(rename = "agent.peerMessage")]
    AgentPeerMessageChanged {
        timestamp: String,
        message: AgentPeerMessage,
    },
    #[serde(rename = "agent.peerHistory")]
    AgentPeerHistoryListed {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        messages: Vec<AgentPeerMessage>,
    },
    #[serde(rename = "agent.broadcasted")]
    AgentBroadcasted {
        timestamp: String,
        result: AgentBroadcastResult,
    },
    #[serde(rename = "agent.backgroundStarted")]
    AgentBackgroundStarted {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "agentName")]
        agent_name: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        source: String,
    },
    #[serde(rename = "agent.backgroundDelta")]
    AgentBackgroundDelta {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "agentName")]
        agent_name: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        source: String,
        delta: String,
    },
    #[serde(rename = "agent.backgroundMessage")]
    AgentBackgroundMessage {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "agentName")]
        agent_name: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        source: String,
        text: String,
    },
    #[serde(rename = "agent.backgroundFinished")]
    AgentBackgroundFinished {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "agentName")]
        agent_name: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    #[serde(rename = "subagent.listed")]
    SubagentListed {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        subagents: Vec<SubagentSummary>,
    },
    #[serde(rename = "subagent.changed")]
    SubagentChanged {
        timestamp: String,
        subagent: SubagentSummary,
    },
    #[serde(rename = "asyncTask.listed")]
    AsyncTaskListed {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        tasks: Vec<AsyncTaskSummary>,
    },
    #[serde(rename = "asyncTask.changed")]
    AsyncTaskChanged {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        tasks: Vec<AsyncTaskSummary>,
    },
    #[serde(rename = "teach.changed")]
    TeachChanged {
        timestamp: String,
        status: TeachRecordingStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<TeachRecordingResult>,
    },
    #[serde(rename = "computer.status")]
    ComputerStatusChanged {
        timestamp: String,
        #[serde(rename = "requestId")]
        request_id: String,
        status: ComputerStatus,
    },
    #[serde(rename = "computer.snapshot")]
    ComputerSnapshotCaptured {
        timestamp: String,
        #[serde(rename = "requestId")]
        request_id: String,
        origin: ComputerControlOrigin,
        snapshot: ComputerSnapshot,
    },
    #[serde(rename = "computer.result")]
    ComputerActionCompleted {
        timestamp: String,
        #[serde(rename = "requestId")]
        request_id: String,
        result: ComputerActionResult,
    },
    #[serde(rename = "remoteComputer.changed")]
    RemoteComputerChanged {
        timestamp: String,
        #[serde(rename = "requestId")]
        request_id: String,
        action: String,
        data: Value,
    },
    #[serde(rename = "memory.listed")]
    MemoryListed {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        memories: Vec<MemoryRecord>,
        count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<String>,
    },
    #[serde(rename = "memory.changed")]
    MemoryChanged {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<MemoryRecord>,
    },
    #[serde(rename = "tray.listed")]
    TrayListed {
        timestamp: String,
        trays: Vec<ErrorTray>,
    },
    #[serde(rename = "tray.changed")]
    TrayChanged {
        timestamp: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tray: Option<ErrorTray>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "workflow.listed")]
    WorkflowListed {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        workflows: Vec<WorkflowSummary>,
    },
    #[serde(rename = "workflow.changed")]
    WorkflowChanged {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow: Option<WorkflowSummary>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "attachment.stored")]
    AttachmentStored {
        timestamp: String,
        attachment: AttachmentStored,
    },
    #[serde(rename = "attachment.text")]
    AttachmentTextRead {
        timestamp: String,
        result: AttachmentTextResult,
    },
    #[serde(rename = "attachment.chunk")]
    AttachmentChunkRead {
        timestamp: String,
        result: AttachmentChunkResult,
    },
    #[serde(rename = "attachment.image")]
    AttachmentImageRead {
        timestamp: String,
        result: AttachmentImageResult,
    },
    #[serde(rename = "search.messages")]
    SearchMessagesListed {
        timestamp: String,
        query: String,
        matches: Vec<SearchMessageMatch>,
    },
    #[serde(rename = "search.media")]
    SearchMediaListed {
        timestamp: String,
        query: String,
        matches: Vec<SearchMediaMatch>,
    },
    #[serde(rename = "mcp.listed")]
    McpListed {
        timestamp: String,
        servers: Vec<Value>,
    },
    #[serde(rename = "mcp.apps")]
    McpAppsListed { timestamp: String, apps: Vec<Value> },
    #[serde(rename = "mcp.oauth")]
    McpOauthChanged {
        timestamp: String,
        server: String,
        #[serde(
            rename = "authorizationUrl",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        authorization_url: Option<String>,
        #[serde(default)]
        removed: bool,
    },
    #[serde(rename = "mcp.refreshed")]
    McpRefreshed { timestamp: String },
    #[serde(rename = "mcp.toolResult")]
    McpToolResult {
        timestamp: String,
        server: String,
        tool: String,
        result: Value,
    },
    #[serde(rename = "settings.changed")]
    SettingsChanged {
        timestamp: String,
        settings: ProductHostSettings,
    },
    #[serde(rename = "audit.listed")]
    AuditListed {
        timestamp: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        records: Vec<Value>,
    },
    #[serde(rename = "listener.listed")]
    ListenerListed {
        timestamp: String,
        integrations: Vec<ListenerIntegrationSummary>,
    },
    #[serde(rename = "listener.changed")]
    ListenerChanged {
        timestamp: String,
        integration: ListenerIntegrationSummary,
    },
    #[serde(rename = "update.changed")]
    UpdateChanged {
        timestamp: String,
        state: UpdateState,
    },
    #[serde(rename = "agent.step")]
    AgentStep {
        timestamp: String,
        #[serde(
            rename = "operationId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        operation_id: Option<String>,
        #[serde(rename = "stepId")]
        step_id: String,
        kind: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        status: AgentStepStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total: Option<u64>,
    },
    #[serde(rename = "model.routed")]
    ModelRouted {
        timestamp: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        provider: String,
        model: String,
        mode: AgentMode,
    },
    #[serde(rename = "usage.updated")]
    UsageUpdated {
        timestamp: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        #[serde(rename = "inputTokens")]
        input_tokens: i64,
        #[serde(rename = "cachedInputTokens")]
        cached_input_tokens: i64,
        #[serde(rename = "outputTokens")]
        output_tokens: i64,
        #[serde(rename = "reasoningTokens")]
        reasoning_tokens: i64,
        #[serde(rename = "totalTokens")]
        total_tokens: i64,
        #[serde(
            rename = "contextWindow",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        context_window: Option<i64>,
    },
    #[serde(rename = "marketplace.installed")]
    MarketplaceInstalled {
        timestamp: String,
        #[serde(rename = "miniAppId")]
        mini_app_id: String,
        version: String,
    },
    #[serde(rename = "miniapp.opened")]
    MiniAppOpened {
        timestamp: String,
        #[serde(rename = "miniAppId")]
        mini_app_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        html: Option<String>,
    },
    #[serde(rename = "approval.requested")]
    ApprovalRequested {
        timestamp: String,
        #[serde(rename = "approvalId")]
        approval_id: String,
        #[serde(rename = "miniAppId")]
        mini_app_id: String,
        capability: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(
            rename = "proposedRule",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        proposed_rule: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<String>,
    },
    #[serde(rename = "approval.resolved")]
    ApprovalResolved {
        timestamp: String,
        #[serde(rename = "approvalId")]
        approval_id: String,
        decision: ApprovalDecision,
    },
    #[serde(rename = "operation.started")]
    OperationStarted {
        timestamp: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        label: String,
        interruptible: bool,
    },
    #[serde(rename = "operation.interrupted")]
    OperationInterrupted {
        timestamp: String,
        #[serde(rename = "operationId")]
        operation_id: String,
    },
    #[serde(rename = "operation.completed")]
    OperationCompleted {
        timestamp: String,
        #[serde(rename = "operationId")]
        operation_id: String,
    },
    #[serde(rename = "operation.failed")]
    OperationFailed {
        timestamp: String,
        #[serde(rename = "operationId")]
        operation_id: String,
        code: String,
        message: String,
    },
    #[serde(rename = "session.cleared")]
    SessionCleared { timestamp: String },
    #[serde(rename = "host.closed")]
    HostClosed { timestamp: String },
}

impl HostEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::HostReady { .. } => "host.ready",
            Self::ChatMessage { .. } => "chat.message",
            Self::ChatDelta { .. } => "chat.delta",
            Self::TranscriptCard { .. } => "transcript.card",
            Self::DraftChanged { .. } => "draft.changed",
            Self::SecretProvided { .. } => "secret.provided",
            Self::ConversationListed { .. } => "conversation.listed",
            Self::ConversationOpened { .. } => "conversation.opened",
            Self::MessagingEvent { .. } => "messaging.event",
            Self::CapabilityListed { .. } => "capability.listed",
            Self::AutomationListed { .. } => "automation.listed",
            Self::AutomationChanged { .. } => "automation.changed",
            Self::ConnectorListed { .. } => "connector.listed",
            Self::ConnectorChanged { .. } => "connector.changed",
            Self::ConnectorOauthRequested { .. } => "connector.oauthRequested",
            Self::SkillListed { .. } => "skill.listed",
            Self::SkillChanged { .. } => "skill.changed",
            Self::BotListed { .. } => "bot.listed",
            Self::BotChanged { .. } => "bot.changed",
            Self::GroupListed { .. } => "group.listed",
            Self::GroupChanged { .. } => "group.changed",
            Self::GroupDelta { .. } => "group.delta",
            Self::AgentPeerMessageChanged { .. } => "agent.peerMessage",
            Self::AgentPeerHistoryListed { .. } => "agent.peerHistory",
            Self::AgentBroadcasted { .. } => "agent.broadcasted",
            Self::AgentBackgroundStarted { .. } => "agent.backgroundStarted",
            Self::AgentBackgroundDelta { .. } => "agent.backgroundDelta",
            Self::AgentBackgroundMessage { .. } => "agent.backgroundMessage",
            Self::AgentBackgroundFinished { .. } => "agent.backgroundFinished",
            Self::SubagentListed { .. } => "subagent.listed",
            Self::SubagentChanged { .. } => "subagent.changed",
            Self::AsyncTaskListed { .. } => "asyncTask.listed",
            Self::AsyncTaskChanged { .. } => "asyncTask.changed",
            Self::TeachChanged { .. } => "teach.changed",
            Self::ComputerStatusChanged { .. } => "computer.status",
            Self::ComputerSnapshotCaptured { .. } => "computer.snapshot",
            Self::ComputerActionCompleted { .. } => "computer.result",
            Self::RemoteComputerChanged { .. } => "remoteComputer.changed",
            Self::MemoryListed { .. } => "memory.listed",
            Self::MemoryChanged { .. } => "memory.changed",
            Self::TrayListed { .. } => "tray.listed",
            Self::TrayChanged { .. } => "tray.changed",
            Self::WorkflowListed { .. } => "workflow.listed",
            Self::WorkflowChanged { .. } => "workflow.changed",
            Self::AttachmentStored { .. } => "attachment.stored",
            Self::AttachmentTextRead { .. } => "attachment.text",
            Self::AttachmentChunkRead { .. } => "attachment.chunk",
            Self::AttachmentImageRead { .. } => "attachment.image",
            Self::SearchMessagesListed { .. } => "search.messages",
            Self::SearchMediaListed { .. } => "search.media",
            Self::McpListed { .. } => "mcp.listed",
            Self::McpAppsListed { .. } => "mcp.apps",
            Self::McpOauthChanged { .. } => "mcp.oauth",
            Self::McpRefreshed { .. } => "mcp.refreshed",
            Self::McpToolResult { .. } => "mcp.toolResult",
            Self::SettingsChanged { .. } => "settings.changed",
            Self::AuditListed { .. } => "audit.listed",
            Self::ListenerListed { .. } => "listener.listed",
            Self::ListenerChanged { .. } => "listener.changed",
            Self::UpdateChanged { .. } => "update.changed",
            Self::AgentStep { .. } => "agent.step",
            Self::ModelRouted { .. } => "model.routed",
            Self::UsageUpdated { .. } => "usage.updated",
            Self::MarketplaceInstalled { .. } => "marketplace.installed",
            Self::MiniAppOpened { .. } => "miniapp.opened",
            Self::ApprovalRequested { .. } => "approval.requested",
            Self::ApprovalResolved { .. } => "approval.resolved",
            Self::OperationStarted { .. } => "operation.started",
            Self::OperationInterrupted { .. } => "operation.interrupted",
            Self::OperationCompleted { .. } => "operation.completed",
            Self::OperationFailed { .. } => "operation.failed",
            Self::SessionCleared { .. } => "session.cleared",
            Self::HostClosed { .. } => "host.closed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_json_is_compatible_with_the_react_contract() {
        let command: FeatureCommand = serde_json::from_str(
            r#"{"type":"capability.request","requestId":"req-1","miniAppId":"global-dharma","capability":"camera","reason":"scan scripture"}"#,
        )
        .expect("decode command");
        assert_eq!(command.request_id(), "req-1");
        assert_eq!(
            serde_json::to_value(command).expect("encode command")["type"],
            "capability.request"
        );
    }

    #[test]
    fn product_settings_default_legacy_payloads_and_round_trip_router_choices() {
        let legacy: ProductHostSettings = serde_json::from_str(
            r#"{"notifications":true,"autoUpdateWhenIdle":true,"localExecution":true,"routeEgressLocally":false,"securityKeys":false,"webauthnProxyEnabled":false,"localToolPermission":"ask"}"#,
        )
        .expect("decode legacy settings");
        assert_eq!(legacy.inference_provider, InferenceProvider::Fabushi);
        assert_eq!(legacy.sandbox_runtime, SandboxRuntime::Host);

        let configured = ProductHostSettings {
            inference_provider: InferenceProvider::ClaudeCode,
            sandbox_runtime: SandboxRuntime::LocalDocker,
            ..Default::default()
        };
        let value = serde_json::to_value(configured).expect("encode router settings");
        assert_eq!(value["inferenceProvider"], "claude-code");
        assert_eq!(value["sandboxRuntime"], "local-docker");
    }

    #[test]
    fn mcp_remove_command_round_trips_with_request_id() {
        let command: FeatureCommand = serde_json::from_str(
            r#"{"type":"mcp.remove","requestId":"mcp-remove-1","server":"docs"}"#,
        )
        .expect("decode MCP remove command");
        assert_eq!(command.request_id(), "mcp-remove-1");
        assert!(matches!(
            command,
            FeatureCommand::McpRemove { server, .. } if server == "docs"
        ));
    }

    #[test]
    fn legacy_computer_commands_default_to_versioned_local_desktop_target() {
        let command: FeatureCommand = serde_json::from_str(
            r#"{"type":"computer.screenshot","requestId":"screen-1","origin":"local-ui"}"#,
        )
        .expect("decode legacy computer command");
        match command {
            FeatureCommand::ComputerScreenshot { target, .. } => {
                assert_eq!(target.protocol_version, COMPUTER_CONTROL_PROTOCOL_VERSION);
                assert_eq!(target.kind, ComputerTargetKind::Desktop);
                assert_eq!(target.generation, 0);
                assert!(target.device_id.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn remote_computer_target_round_trips_device_and_generation() {
        let target = ComputerControlTarget::remote_desktop("desktop-a", 42);
        let value = serde_json::to_value(&target).expect("encode target");
        assert_eq!(value["protocolVersion"], COMPUTER_CONTROL_PROTOCOL_VERSION);
        assert_eq!(value["kind"], "desktop");
        assert_eq!(value["deviceId"], "desktop-a");
        assert_eq!(value["generation"], 42);
        assert_eq!(
            serde_json::from_value::<ComputerControlTarget>(value).unwrap(),
            target
        );
    }

    #[test]
    fn computer_action_duration_and_scroll_amount_match_the_react_contract() {
        let action: ComputerAction = serde_json::from_str(
            r#"{"action":"drag","x":1,"y":2,"x2":3,"y2":4,"durationMs":375,"amount":7}"#,
        )
        .expect("decode computer action");
        assert_eq!(action.wait_ms, Some(375));
        assert_eq!(action.amount, Some(7));
        let value = serde_json::to_value(&action).expect("encode computer action");
        assert_eq!(value["durationMs"], 375);
        assert_eq!(value["amount"], 7);
        assert!(value.get("waitMs").is_none());
    }

    #[test]
    fn legacy_wait_ms_alias_remains_accepted() {
        let action: ComputerAction = serde_json::from_str(r#"{"action":"wait","waitMs":250}"#)
            .expect("decode legacy wait action");
        assert_eq!(action.wait_ms, Some(250));
    }

    #[test]
    fn event_json_uses_camel_case_fields() {
        let event = HostEvent::OperationStarted {
            timestamp: "0".into(),
            operation_id: "operation-1".into(),
            label: "sync".into(),
            interruptible: true,
        };
        let value = serde_json::to_value(event).expect("encode event");
        assert_eq!(value["type"], "operation.started");
        assert_eq!(value["operationId"], "operation-1");
    }

    #[test]
    fn agent_chat_context_is_additive_and_backwards_compatible() {
        let legacy: FeatureCommand =
            serde_json::from_str(r#"{"type":"chat.send","requestId":"req-1","text":"hello"}"#)
                .expect("decode legacy chat command");
        assert!(matches!(
            legacy,
            FeatureCommand::ChatSend {
                mode: AgentMode::Agent,
                mode_statement: None,
                attachments,
                ..
            } if attachments.is_empty()
        ));

        let event = HostEvent::UsageUpdated {
            timestamp: "0".into(),
            operation_id: "operation-1".into(),
            input_tokens: 10,
            cached_input_tokens: 2,
            output_tokens: 5,
            reasoning_tokens: 1,
            total_tokens: 15,
            context_window: Some(128_000),
        };
        let value = serde_json::to_value(event).expect("encode usage event");
        assert_eq!(value["type"], "usage.updated");
        assert_eq!(value["contextWindow"], 128_000);
    }

    #[test]
    fn automation_commands_and_events_match_the_frontend_contract() {
        let command: FeatureCommand = serde_json::from_str(
            r#"{"type":"automation.upsert","requestId":"routine-1","name":"Daily review","prompt":"Summarize progress","schedule":"@daily"}"#,
        )
        .expect("decode automation command");
        assert_eq!(command.request_id(), "routine-1");
        assert!(matches!(
            command,
            FeatureCommand::AutomationUpsert { enabled: true, .. }
        ));

        let event = HostEvent::AutomationListed {
            timestamp: "0".into(),
            automations: vec![AutomationSummary {
                id: "daily-review".into(),
                agent_id: None,
                name: "Daily review".into(),
                prompt: "Summarize progress".into(),
                schedule: "@daily".into(),
                trigger: Some(AutomationTrigger::Schedule {
                    schedule: "@daily".into(),
                }),
                enabled: true,
                created_at_ms: 1,
                last_run_at_ms: None,
                next_run_at_ms: Some(2),
            }],
        };
        let value = serde_json::to_value(event).expect("encode automation event");
        assert_eq!(value["type"], "automation.listed");
        assert_eq!(value["automations"][0]["nextRunAtMs"], 2);
        assert_eq!(value["automations"][0]["trigger"]["kind"], "schedule");
    }

    #[test]
    fn event_routine_and_product_surface_commands_round_trip() {
        let command: FeatureCommand = serde_json::from_str(
            r#"{"type":"automation.upsert","requestId":"event-1","name":"Triage regressions","prompt":"Inspect and summarize","schedule":"event:sentry:issue.regressed","trigger":{"kind":"event","source":"sentry","event":"issue.regressed","filter":"project == web"}}"#,
        )
        .expect("decode event routine");
        assert!(matches!(
            command,
            FeatureCommand::AutomationUpsert {
                trigger: Some(AutomationTrigger::Event {
                    source: ListenerPlatform::Sentry,
                    ..
                }),
                ..
            }
        ));

        let connector: FeatureCommand = serde_json::from_str(
            r#"{"type":"connector.setToolEnabled","requestId":"connector-1","connectorId":"github","toolId":"create_issue","enabled":false}"#,
        )
        .expect("decode connector command");
        assert_eq!(connector.request_id(), "connector-1");

        let secret: FeatureCommand = serde_json::from_str(
            r#"{"type":"secret.provide","requestId":"secret-1","secretRequestId":"token","value":"redacted"}"#,
        )
        .expect("decode secret command");
        assert_eq!(secret.request_id(), "secret-1");
    }

    #[test]
    fn structured_transcript_cards_use_the_frontend_shape() {
        let event = HostEvent::TranscriptCard {
            timestamp: "0".into(),
            entry_id: "entry-1".into(),
            operation_id: Some("operation-1".into()),
            card: TranscriptCard::SecretRequest {
                request_id: "api-key".into(),
                label: "API key".into(),
                description: Some("Needed to connect the service".into()),
                provided: false,
            },
        };
        let value = serde_json::to_value(event).expect("encode transcript card");
        assert_eq!(value["type"], "transcript.card");
        assert_eq!(value["entryId"], "entry-1");
        assert_eq!(value["card"]["kind"], "secretRequest");
        assert_eq!(value["card"]["requestId"], "api-key");
    }

    #[test]
    fn mini_app_transcript_card_uses_frontend_camel_case_shape() {
        let event = HostEvent::TranscriptCard {
            timestamp: "0".into(),
            entry_id: "entry-miniapp".into(),
            operation_id: Some("operation-miniapp".into()),
            card: TranscriptCard::MiniApp {
                mini_app_id: "counter-demo".into(),
                name: "Counter".into(),
                html: "<!doctype html><html></html>".into(),
                description: Some("Runnable demo".into()),
            },
        };
        let value = serde_json::to_value(event).expect("encode Mini App card");
        assert_eq!(value["card"]["kind"], "miniApp");
        assert_eq!(value["card"]["miniAppId"], "counter-demo");
        assert_eq!(value["card"]["name"], "Counter");
    }
}
