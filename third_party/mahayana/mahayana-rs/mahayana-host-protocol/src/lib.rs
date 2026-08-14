//! Versioned product-level commands and events shared by React, Tauri, mobile
//! shells, deterministic tests, and future WebAssembly hosts.

use serde::Deserialize;
use serde::Serialize;

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
    Tauri,
    Wasm,
    Flutter,
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
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
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
    },
    #[serde(rename = "automation.upsert")]
    AutomationUpsert {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
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
        enabled: bool,
    },
    #[serde(rename = "automation.delete")]
    AutomationDelete {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
    },
    #[serde(rename = "automation.run")]
    AutomationRun {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
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
    #[serde(rename = "bot.setHidden")]
    BotSetHidden {
        #[serde(rename = "requestId")]
        request_id: String,
        id: String,
        hidden: bool,
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
            | Self::AutomationList { request_id }
            | Self::AutomationUpsert { request_id, .. }
            | Self::AutomationSetEnabled { request_id, .. }
            | Self::AutomationDelete { request_id, .. }
            | Self::AutomationRun { request_id, .. }
            | Self::MarketplaceInstall { request_id, .. }
            | Self::MiniAppOpen { request_id, .. }
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
            | Self::BotSetHidden { request_id, .. }
            | Self::DraftResolve { request_id, .. }
            | Self::SecretProvide { request_id, .. }
            | Self::ListenerList { request_id }
            | Self::ListenerConnect { request_id, .. }
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
    BotChanged { timestamp: String, bot: BotSummary },
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
}
