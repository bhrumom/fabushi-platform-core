//! First-party Mahayana platform client.

use codex_login::token_data::parse_jwt_expiration;
use codex_secrets::LocalSecretsNamespace;
use codex_secrets::SecretName;
use codex_secrets::SecretScope;
use codex_secrets::SecretsBackendKind;
use codex_secrets::SecretsManager;
use mahayana_host_protocol::BotSummary;
use mahayana_host_protocol::ConnectorAccountSummary;
use mahayana_host_protocol::ConnectorStatus;
use mahayana_host_protocol::ConnectorSummary;
use mahayana_host_protocol::ConnectorToolSummary;
use mahayana_host_protocol::ConnectorTransport;
use mahayana_host_protocol::DraftAction;
use mahayana_host_protocol::DraftSendState;
use mahayana_host_protocol::FeatureCommand;
use mahayana_host_protocol::ListenerIntegrationSummary;
use mahayana_host_protocol::ListenerPlatform;
use mahayana_host_protocol::MessageDraft;
use mahayana_host_protocol::SkillPublishState;
use mahayana_host_protocol::SkillSource;
use mahayana_host_protocol::SkillSummary;
use mahayana_host_protocol::SkillTeamSummary;
use mahayana_host_protocol::UpdateDisabledReason;
use mahayana_host_protocol::UpdateState;
use mahayana_platform_core::AccountUsageStatus;
use mahayana_platform_core::Currency;
use mahayana_platform_core::DelegatedTokenRequest;
use mahayana_platform_core::Entitlement;
use mahayana_platform_core::PurchaseRequest;
use mahayana_platform_core::Quote;
use mahayana_platform_core::canonical_json_bytes;
use mahayana_platform_core::canonical_json_sha256;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use std::env;
use std::io::Read;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const DEFAULT_API_BASE_URL: &str = "https://api.ombhrum.com";
const MAHAYANA_ACCOUNT_SESSION_SECRET: &str = "MAHAYANA_ACCOUNT_SESSION";
const MAHAYANA_TEST_ACCOUNT_TOKEN_ENV: &str = "MAHAYANA_TEST_ACCOUNT_TOKEN";
const MAHAYANA_TEST_ACCOUNT_MARKER: &str = "test-account-login.sha256";
const FABUSHI_CI_ACCOUNT_SESSION_FILE_ENV: &str = "FABUSHI_CI_ACCOUNT_SESSION_FILE";
const GITHUB_ACTIONS_ENV: &str = "GITHUB_ACTIONS";
const CI_ACCOUNT_SESSION_MAX_BYTES: u64 = 64 * 1024;
const ACCESS_TOKEN_REFRESH_SKEW_SECONDS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalance {
    pub currency: Currency,
    pub available: i64,
    pub reserved: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletHistoryPage {
    pub entries: Vec<Value>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseReceipt {
    pub order_id: String,
    pub status: String,
    pub entitlement: Option<Entitlement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchasePage {
    pub purchases: Vec<Value>,
    pub next_cursor: Option<String>,
}

const PRODUCT_SURFACE_STATE_SCHEMA: u32 = 1;
const PRODUCT_SURFACE_STATE_FILENAME: &str = "product-surface.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductSurfaceState {
    schema_version: u32,
    sequence: u64,
    connectors: Vec<ConnectorSummary>,
    skills: Vec<SkillSummary>,
    skill_teams: Vec<SkillTeamSummary>,
    bots: Vec<BotSummary>,
    listeners: Vec<ListenerIntegrationSummary>,
    update_state: UpdateState,
}

impl Default for ProductSurfaceState {
    fn default() -> Self {
        Self {
            schema_version: PRODUCT_SURFACE_STATE_SCHEMA,
            sequence: 0,
            connectors: default_surface_connectors(),
            skills: default_surface_skills(),
            skill_teams: Vec::new(),
            bots: default_surface_bots(),
            listeners: default_surface_listeners(),
            // The desktop bundle currently has no signed updater endpoint or
            // updater plugin. Report the same explicit disabled state used by
            // recovered unpackaged development builds instead of pretending a
            // network update check succeeded.
            update_state: UpdateState::Disabled {
                reason: UpdateDisabledReason::NotPackaged,
            },
        }
    }
}

fn surface_connector_tool(id: &str, name: &str, description: &str) -> ConnectorToolSummary {
    ConnectorToolSummary {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        enabled: true,
        requires_approval: Some(true),
    }
}

fn surface_connector(
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
        can_add_account: transport != ConnectorTransport::Command,
        transport,
        source: Some("Mahayana / Codex MCP".into()),
        teammate_count: None,
        accounts: Vec::new(),
        tools,
    }
}

fn default_surface_connectors() -> Vec<ConnectorSummary> {
    vec![
        surface_connector(
            "gmail",
            "Gmail",
            "Search, read, draft, label, and manage email through the installed Gmail MCP plugin.",
            ConnectorTransport::Http,
            vec![
                surface_connector_tool("search", "Search mail", "Search messages and threads."),
                surface_connector_tool(
                    "draft",
                    "Draft mail",
                    "Create an email draft for user approval.",
                ),
                surface_connector_tool("send", "Send mail", "Send an approved email draft."),
            ],
        ),
        surface_connector(
            "github",
            "GitHub",
            "Repositories, pull requests, issues, comments, and CI.",
            ConnectorTransport::Http,
            vec![
                surface_connector_tool(
                    "read_repository",
                    "Read repository",
                    "Read repository files and metadata.",
                ),
                surface_connector_tool(
                    "create_issue",
                    "Create issue",
                    "Create and update GitHub issues.",
                ),
                surface_connector_tool(
                    "comment_pull_request",
                    "Comment on pull request",
                    "Post review comments on pull requests.",
                ),
            ],
        ),
        surface_connector(
            "slack",
            "Slack",
            "Messages, mentions, reactions, and approved drafts.",
            ConnectorTransport::Http,
            vec![
                surface_connector_tool(
                    "search_messages",
                    "Search messages",
                    "Search workspace messages and threads.",
                ),
                surface_connector_tool(
                    "post_message",
                    "Post message",
                    "Send an approved message or thread reply.",
                ),
                surface_connector_tool(
                    "add_reaction",
                    "Add reaction",
                    "Add a reaction to a message.",
                ),
            ],
        ),
        surface_connector(
            "teams",
            "Microsoft Teams",
            "Teams messages, mentions, channels, and approved drafts.",
            ConnectorTransport::Http,
            vec![
                surface_connector_tool(
                    "search_messages",
                    "Search messages",
                    "Search Teams channels and chats.",
                ),
                surface_connector_tool(
                    "post_message",
                    "Post message",
                    "Send an approved Teams message.",
                ),
            ],
        ),
        surface_connector(
            "linear",
            "Linear",
            "Issues, comments, status changes, and projects.",
            ConnectorTransport::Http,
            vec![
                surface_connector_tool(
                    "read_issues",
                    "Read issues",
                    "Read Linear issues and projects.",
                ),
                surface_connector_tool(
                    "update_issue",
                    "Update issue",
                    "Update issue state, assignee, and fields.",
                ),
            ],
        ),
        surface_connector(
            "sentry",
            "Sentry",
            "Errors, regressions, releases, and issue ownership.",
            ConnectorTransport::Http,
            vec![
                surface_connector_tool(
                    "read_issues",
                    "Read issues",
                    "Read Sentry issues and events.",
                ),
                surface_connector_tool(
                    "resolve_issue",
                    "Resolve issue",
                    "Resolve or assign a Sentry issue.",
                ),
            ],
        ),
        surface_connector(
            "pagerduty",
            "PagerDuty",
            "Incidents, acknowledgements, responders, and escalation.",
            ConnectorTransport::Http,
            vec![
                surface_connector_tool(
                    "read_incidents",
                    "Read incidents",
                    "Read incident details and timelines.",
                ),
                surface_connector_tool(
                    "acknowledge_incident",
                    "Acknowledge incident",
                    "Acknowledge an incident after approval.",
                ),
            ],
        ),
        surface_connector(
            "git",
            "Git",
            "Local commits, branches, and repository state.",
            ConnectorTransport::Command,
            vec![
                surface_connector_tool(
                    "read_status",
                    "Read status",
                    "Read local repository status.",
                ),
                surface_connector_tool(
                    "read_history",
                    "Read history",
                    "Read commit and branch history.",
                ),
            ],
        ),
    ]
}

fn default_surface_skills() -> Vec<SkillSummary> {
    vec![SkillSummary {
        id: "skill-research-brief".into(),
        name: "Research brief".into(),
        description: "Turn verified sources into a concise research brief.".into(),
        use_when: "Use when a task needs sourced research and a decision-ready summary.".into(),
        instructions:
            "Verify sources, distinguish facts from inference, and end with actionable conclusions."
                .into(),
        source: SkillSource::Private,
        publish_state: SkillPublishState::Local,
        owner_agent_id: Some("mahayana-assistant".into()),
        team_id: None,
        team_name: None,
        read_only: Some(false),
        updated_at_ms: 0,
    }]
}

fn default_surface_bots() -> Vec<BotSummary> {
    vec![
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
            conversation_id: Some("mahayana-ai:agent:assistant".into()),
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
}

fn surface_listener(
    platform: ListenerPlatform,
    display_name: &str,
    blurb: &str,
) -> ListenerIntegrationSummary {
    ListenerIntegrationSummary {
        platform,
        display_name: display_name.into(),
        blurb: blurb.into(),
        is_connected: platform == ListenerPlatform::Git,
        account_label: (platform == ListenerPlatform::Git).then(|| "Local repository".into()),
        error: None,
    }
}

fn default_surface_listeners() -> Vec<ListenerIntegrationSummary> {
    vec![
        surface_listener(
            ListenerPlatform::Github,
            "GitHub",
            "Watch repository PRs, comments, issues, and CI.",
        ),
        surface_listener(
            ListenerPlatform::Git,
            "Git",
            "Wake routines on local repository activity.",
        ),
        surface_listener(
            ListenerPlatform::Slack,
            "Slack",
            "Wake routines on messages, mentions, and reactions.",
        ),
        surface_listener(
            ListenerPlatform::Teams,
            "Microsoft Teams",
            "Wake routines on Teams activity.",
        ),
        surface_listener(
            ListenerPlatform::Linear,
            "Linear",
            "Wake routines on issue and project activity.",
        ),
        surface_listener(
            ListenerPlatform::Sentry,
            "Sentry",
            "Wake routines on issue and regression activity.",
        ),
        surface_listener(
            ListenerPlatform::Pagerduty,
            "PagerDuty",
            "Wake routines on incident activity.",
        ),
    ]
}

/// First-party product API client shared by the CLI and native application
/// shells. Authentication is stored once by Rust so every surface observes the
/// same Mahayana account session.
#[derive(Clone)]
pub struct MahayanaProductClient {
    api_base_url: String,
    /// Stable Mahayana home anchor used to locate Codex encrypted secrets.
    session_path: PathBuf,
    /// Shared non-secret product state used by CLI and native application shells.
    surface_state_path: PathBuf,
    /// Short-lived browser-login poll proofs live in caller-owned private files.
    /// They never need OS keyring access and are removed on terminal auth states.
    browser_login_poll_dir: PathBuf,
    skills_root: PathBuf,
    secrets_manager: SecretsManager,
    managed_secrets: SecretsManager,
}

impl std::fmt::Debug for MahayanaProductClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MahayanaProductClient")
            .field("api_base_url", &self.api_base_url)
            .field("legacy_session_path", &self.session_path)
            .field("surface_state_path", &self.surface_state_path)
            .finish_non_exhaustive()
    }
}

impl Default for MahayanaProductClient {
    fn default() -> Self {
        let home = default_mahayana_home();
        Self::new_with_default_api_base_url(
            home.join("session.json"),
            home.join(PRODUCT_SURFACE_STATE_FILENAME),
        )
    }
}

/// Shared Mahayana data directory used by the signed application and CLI.
/// Platform runtimes should derive their own subdirectories from this path so
/// account state and Codex conversations never split across host surfaces.
pub fn default_product_surface_state_path() -> PathBuf {
    default_mahayana_home().join(PRODUCT_SURFACE_STATE_FILENAME)
}

fn default_codex_skills_root() -> PathBuf {
    let codex_home = env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"));
    codex_home.join("skills")
}

fn safe_skill_directory_name(skill_id: &str) -> Result<String, ProductError> {
    let value = skill_id.trim();
    if value.is_empty()
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err(ProductError::InvalidParameter("skillId"));
    }
    Ok(value.to_string())
}

fn product_auth_secrets_namespace(configured: Option<&str>) -> LocalSecretsNamespace {
    match configured.map(str::trim) {
        Some("fabushi-desktop-v2") => LocalSecretsNamespace::FabushiDesktopAuth,
        _ => LocalSecretsNamespace::MahayanaAuth,
    }
}

fn product_managed_secrets_namespace(configured: Option<&str>) -> LocalSecretsNamespace {
    match configured.map(str::trim) {
        Some("fabushi-desktop-v2") => LocalSecretsNamespace::FabushiDesktopManagedSecrets,
        _ => LocalSecretsNamespace::ManagedSecrets,
    }
}

pub fn default_mahayana_home() -> PathBuf {
    if let Some(path) = env::var_os("MAHAYANA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path;
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = env::var_os("HOME") {
        // The signed App and its bundled command-line tool use the existing
        // Fabushi application group. This avoids a release-sandbox copy of
        // the account session diverging from the user's terminal CLI.
        return PathBuf::from(home)
            .join("Library")
            .join("Group Containers")
            .join("group.com.ombhrum.fabushi.share")
            .join("mahayana");
    }
    #[cfg(target_os = "windows")]
    if let Some(app_data) = env::var_os("APPDATA") {
        return PathBuf::from(app_data).join("Fabushi").join("Mahayana");
    }
    env::var_os("HOME")
        .map(|value| PathBuf::from(value).join(".mahayana"))
        .unwrap_or_else(|| PathBuf::from(".mahayana"))
}

impl MahayanaProductClient {
    /// Creates a product client with the configured first-party API while keeping
    /// its encrypted account state anchored to caller-owned paths. Desktop and
    /// native app hosts use this instead of `Default` so a stale shared-container
    /// credential file cannot block startup of an otherwise independent app data
    /// directory.
    pub fn new_with_default_api_base_url(
        session_path: impl Into<PathBuf>,
        surface_state_path: impl Into<PathBuf>,
    ) -> Self {
        let api_base_url = env::var("MAHAYANA_API_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string());
        Self::new_with_surface_state_path(api_base_url, session_path, surface_state_path)
    }

    pub fn new(api_base_url: impl Into<String>, session_path: impl Into<PathBuf>) -> Self {
        let session_path = session_path.into();
        let surface_state_path = session_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join(PRODUCT_SURFACE_STATE_FILENAME);
        Self::new_with_surface_state_path(api_base_url, session_path, surface_state_path)
    }

    pub fn new_with_surface_state_path(
        api_base_url: impl Into<String>,
        session_path: impl Into<PathBuf>,
        surface_state_path: impl Into<PathBuf>,
    ) -> Self {
        let session_path = session_path.into();
        let mahayana_home = session_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let browser_login_poll_dir = mahayana_home.join("browser-login");
        let client = Self {
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
            session_path,
            surface_state_path: surface_state_path.into(),
            browser_login_poll_dir,
            skills_root: default_codex_skills_root(),
            secrets_manager: SecretsManager::new_with_namespace(
                mahayana_home.clone(),
                SecretsBackendKind::Local,
                product_auth_secrets_namespace(
                    env::var("MAHAYANA_AUTH_STORAGE_NAMESPACE").ok().as_deref(),
                ),
            ),
            managed_secrets: SecretsManager::new_with_namespace(
                mahayana_home,
                SecretsBackendKind::Local,
                product_managed_secrets_namespace(
                    env::var("MAHAYANA_AUTH_STORAGE_NAMESPACE").ok().as_deref(),
                ),
            ),
        };
        client.import_legacy_session_if_needed();
        client
    }

    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }

    /// Returns a previously stored first-party requested secret without ever
    /// serializing it through the renderer/product command response. Callers
    /// must already know the opaque request id that created the secret.
    pub fn managed_secret(&self, secret_request_id: &str) -> Result<Option<String>, ProductError> {
        let name = managed_secret_name(secret_request_id)?;
        self.managed_secrets
            .get(&SecretScope::Global, &name)
            .map_err(secrets_error)
    }

    /// Revokes a first-party requested secret from the encrypted managed
    /// secrets namespace. The returned boolean indicates whether a value was
    /// present before revocation.
    pub fn revoke_managed_secret(&self, secret_request_id: &str) -> Result<bool, ProductError> {
        let name = managed_secret_name(secret_request_id)?;
        self.managed_secrets
            .delete(&SecretScope::Global, &name)
            .map_err(secrets_error)
    }

    pub fn session_path(&self) -> &Path {
        &self.session_path
    }

    pub fn skills_root(&self) -> &Path {
        &self.skills_root
    }

    fn persist_private_skill(&self, skill: &SkillSummary) -> Result<(), ProductError> {
        let dir_name = safe_skill_directory_name(&skill.id)?;
        let directory = self.skills_root.join(dir_name);
        std::fs::create_dir_all(&directory).map_err(|error| {
            ProductError::State(format!(
                "create skill directory {}: {error}",
                directory.display()
            ))
        })?;
        let name = serde_json::to_string(&skill.name)
            .map_err(|error| ProductError::State(format!("encode skill name: {error}")))?;
        let description = serde_json::to_string(
            &format!("{} {}", skill.description.trim(), skill.use_when.trim())
                .trim()
                .to_string(),
        )
        .map_err(|error| ProductError::State(format!("encode skill description: {error}")))?;
        let contents = format!(
            "---\nname: {name}\ndescription: {description}\n---\n\n{}\n",
            skill.instructions.trim()
        );
        std::fs::write(directory.join("SKILL.md"), contents)
            .map_err(|error| ProductError::State(format!("write skill {}: {error}", skill.id)))
    }

    fn remove_private_skill(&self, skill_id: &str) -> Result<(), ProductError> {
        let directory = self.skills_root.join(safe_skill_directory_name(skill_id)?);
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ProductError::State(format!(
                "remove skill directory {}: {error}",
                directory.display()
            ))),
        }
    }

    /// Validates an explicitly provisioned GitHub Actions account session.
    /// The session is short-lived, contains no refresh token, and is read from
    /// a private file owned by the workflow. Ordinary application launches do
    /// not accept this path, even if an inherited environment variable exists.
    pub fn bootstrap_ci_test_account_session(&self) -> Result<bool, ProductError> {
        if env::var(GITHUB_ACTIONS_ENV).ok().as_deref() != Some("true") {
            return Ok(false);
        }
        let Some(path) =
            env::var_os(FABUSHI_CI_ACCOUNT_SESSION_FILE_ENV).filter(|value| !value.is_empty())
        else {
            return Ok(false);
        };
        load_ci_account_session_file(Path::new(&path), now_seconds())?;
        Ok(true)
    }

    /// Returns the current account access credential to the trusted desktop
    /// main process so the installed application can register its own remote
    /// device. Refresh credentials never leave the Rust-owned session store.
    pub fn device_agent_session(&self) -> Result<Value, ProductError> {
        let session = self.required_session()?;
        let access_token = self.active_session_token(session)?;
        let current = self.required_session()?;
        let session_id = optional_string(&current, "sessionId")
            .ok_or_else(|| ProductError::Session("account session is missing sessionId".into()))?;
        let device_id = optional_string(&current, "deviceId")
            .ok_or_else(|| ProductError::Session("account session is missing deviceId".into()))?;
        let user = current.get("user").cloned().unwrap_or(Value::Null);
        let user_id = current
            .get("userId")
            .cloned()
            .or_else(|| user.get("id").cloned())
            .ok_or_else(|| ProductError::Session("account session is missing userId".into()))?;
        Ok(json!({
            "accessToken": access_token,
            "accessTokenExpiresAt": explicit_expiration_seconds(&current),
            "sessionId": session_id,
            "deviceId": device_id,
            "username": current.get("username").cloned().unwrap_or(Value::Null),
            "userId": user_id,
            "user": user,
            "provider": current.get("provider").cloned().unwrap_or(Value::String("official".into())),
            "ciRunner": current.get("ciRunner").and_then(Value::as_bool).unwrap_or(false),
        }))
    }

    /// Stores the environment-provisioned smoke-test credential in the same
    /// encrypted Mahayana session backend used by normal account logins. On a
    /// headless CI runner without an OS keyring, only a SHA-256 login marker is
    /// written; later CLI processes read the credential from the existing
    /// GitHub Actions secret environment instead of persisting it on disk.
    pub fn store_test_account_session(&self, token: &str) -> Result<(), ProductError> {
        let token = safe_test_account_token(token)?;
        let session = json!({
            "accessToken": token,
            "provider": "test",
            "username": "TestAccount",
            "user": {
                "id": "user:test_account",
                "userId": "user:test_account",
                "username": "TestAccount",
                "membership": {"type": "lifetime", "active": true},
                "isTestAccount": true,
            },
        });
        match self.save_session(&session) {
            Ok(()) => {
                let _ = std::fs::remove_file(self.test_account_marker_path());
                Ok(())
            }
            Err(error) => {
                let environment_token = env::var(MAHAYANA_TEST_ACCOUNT_TOKEN_ENV)
                    .ok()
                    .map(|value| value.trim().to_string());
                if environment_token.as_deref() != Some(token) {
                    return Err(error);
                }
                write_private_file(
                    &self.test_account_marker_path(),
                    &format!("{:x}", sha2::Sha256::digest(token.as_bytes())),
                )
            }
        }
    }

    pub fn marketplace_browse(
        &self,
        query: Option<&str>,
        platform: Option<&str>,
    ) -> Result<Value, ProductError> {
        let query = query.map(str::trim).filter(|query| !query.is_empty());
        let platform = platform.map(safe_marketplace_platform).transpose()?;
        let mut parameters = Vec::new();
        if let Some(query) = query {
            parameters.push(("q", query));
        }
        if let Some(platform) = platform {
            parameters.push(("platform", platform));
        }
        // Public marketplace discovery must never be coupled to account-token
        // refresh. A revoked login must not hide publicly approved Mini Apps.
        self.get_json("/v1/marketplace/plugins", &parameters, None)
    }

    pub fn marketplace_release_metadata(
        &self,
        plugin_id: &str,
        version: &str,
    ) -> Result<Value, ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let version = safe_marketplace_version(version)?;
        self.get_json(
            &format!("/v1/marketplace/plugins/{plugin_id}/releases/{version}"),
            &[],
            None,
        )
    }

    pub fn download_marketplace_plugin(
        &self,
        plugin_id: &str,
        version: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let version = safe_marketplace_version(version)?;
        let client = http_client()?;
        let response = client
            .get(format!(
                "{}/v1/marketplace/plugins/{plugin_id}/releases/{version}/download",
                self.api_base_url
            ))
            .header("Accept", "application/gzip, application/octet-stream")
            .send()
            .map_err(|error| ProductError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response
                .text()
                .unwrap_or_else(|_| "marketplace download failed".to_string());
            return Err(ProductError::HttpStatus { status, message });
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(ProductError::Response(
                "marketplace plugin package exceeds the local size limit".into(),
            ));
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .and_then(|length| usize::try_from(length).ok())
                .unwrap_or(0)
                .min(max_bytes),
        );
        response
            .take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| ProductError::Transport(error.to_string()))?;
        if bytes.len() > max_bytes {
            return Err(ProductError::Response(
                "marketplace plugin package exceeds the local size limit".into(),
            ));
        }
        Ok(bytes)
    }

    /// Wait until a newly deployed Cloudflare plugin site serves the exact
    /// release manifest and package. Cloudflare deployments can report success
    /// before static assets are readable from every edge, while the platform
    /// intentionally performs an immediate authoritative verification.
    pub fn wait_for_marketplace_deployment(
        &self,
        plugin_id: &str,
        version: &str,
        deployment_url: &str,
        package_sha256: &str,
        package_size: u64,
        source: &Value,
        release_manifest: &Value,
    ) -> Result<(), ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let version = safe_marketplace_version(version)?;
        let deployment_url = https_deployment_url(deployment_url)?;
        let package_sha256 = safe_sha256(package_sha256)?;
        if package_size == 0 || package_size > 50 * 1024 * 1024 {
            return Err(ProductError::InvalidParameter("packageSize"));
        }
        let package_size = usize::try_from(package_size)
            .map_err(|_| ProductError::InvalidParameter("packageSize"))?;
        let client = http_client()?;
        let manifest_url = format!("{deployment_url}/mahayana/plugin.json");
        let package_url = format!("{deployment_url}/mahayana/plugin.tar.gz");
        let release_manifest_url = format!("{deployment_url}/mahayana/release-manifest.json");
        let mut last_error = "deployment assets are not available yet".to_string();
        for attempt in 1..=24 {
            match verify_marketplace_deployment_once(
                &client,
                &manifest_url,
                &package_url,
                &release_manifest_url,
                plugin_id,
                version,
                package_sha256,
                package_size,
                source,
                release_manifest,
            ) {
                Ok(()) => return Ok(()),
                Err(error) => last_error = error,
            }
            if attempt < 24 {
                thread::sleep(Duration::from_secs(5));
            }
        }
        Err(ProductError::Response(format!(
            "Cloudflare plugin deployment did not become verifiable: {last_error}"
        )))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_plugin(
        &self,
        plugin_id: &str,
        version: &str,
        deployment_url: &str,
        package_sha256: &str,
        package_size: u64,
        platforms: &[String],
        package: &[u8],
        source: &Value,
        release_manifest: &Value,
    ) -> Result<Value, ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let version = safe_marketplace_version(version)?;
        let deployment_url = https_deployment_url(deployment_url)?;
        let package_sha256 = safe_sha256(package_sha256)?;
        let platforms = safe_marketplace_platforms(platforms)?;
        if package_size == 0
            || package_size > 50 * 1024 * 1024
            || package.len() as u64 != package_size
        {
            return Err(ProductError::InvalidParameter("packageSize"));
        }
        let actual_sha256 = format!("{:x}", sha2::Sha256::digest(package));
        if !actual_sha256.eq_ignore_ascii_case(package_sha256) {
            return Err(ProductError::InvalidParameter("packageSha256"));
        }
        let release_manifest_json = String::from_utf8(
            canonical_json_bytes(release_manifest)
                .map_err(|error| ProductError::Configuration(error.to_string()))?,
        )
        .map_err(|error| ProductError::Configuration(error.to_string()))?;
        let token = self.authorization_token(&Value::Null)?;
        let package_part = reqwest::blocking::multipart::Part::bytes(package.to_vec())
            .file_name(format!("{plugin_id}-{version}.tar.gz"));
        let form = reqwest::blocking::multipart::Form::new()
            .text("pluginId", plugin_id.to_string())
            .text("version", version.to_string())
            .text("deploymentUrl", deployment_url)
            .text("packageSha256", package_sha256.to_string())
            .text("packageSize", package_size.to_string())
            .text(
                "platforms",
                serde_json::to_string(&platforms)
                    .map_err(|error| ProductError::Configuration(error.to_string()))?,
            )
            .text(
                "source",
                serde_json::to_string(source)
                    .map_err(|error| ProductError::Configuration(error.to_string()))?,
            )
            .text("releaseManifest", release_manifest_json)
            .part("package", package_part);
        let client = http_client()?;
        decode_response(
            client
                .post(format!("{}/v1/marketplace/releases", self.api_base_url))
                .header("Accept", "application/json")
                .bearer_auth(token)
                .multipart(form)
                .send(),
        )
    }

    pub fn publish_external_plugin(
        &self,
        plugin_id: &str,
        version: &str,
        display_name: &str,
        description: &str,
        platforms: &[String],
        source: &Value,
        release_manifest: &Value,
    ) -> Result<Value, ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let version = safe_marketplace_version(version)?;
        let platforms = platforms
            .iter()
            .map(|platform| {
                if matches!(
                    platform.as_str(),
                    "cli" | "desktop" | "mobile" | "web" | "ios" | "android"
                ) {
                    Ok(platform.clone())
                } else {
                    Err(ProductError::InvalidParameter("platforms"))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        if platforms.is_empty() {
            return Err(ProductError::InvalidParameter("platforms"));
        }
        let token = self.authorization_token(&Value::Null)?;
        self.post_json(
            "/v1/marketplace/external-releases",
            json!({
                "pluginId": plugin_id,
                "version": version,
                "displayName": non_empty(display_name, "displayName")?,
                "description": description,
                "platforms": platforms,
                "source": source,
                "releaseManifest": release_manifest,
            }),
            Some(&token),
        )
    }

    pub fn wallet_balance(&self) -> Result<WalletBalance, ProductError> {
        let token = self.authorization_token(&Value::Null)?;
        decode_value(self.get_json("/v1/wallet/balance", &[], Some(&token))?)
    }

    /// Returns the server-authoritative model-token budget for the signed-in
    /// Mahayana account. Client-observed usage events are intentionally not
    /// accepted here: only the trusted model gateway may reserve and capture
    /// billable usage.
    pub fn model_usage(&self) -> Result<AccountUsageStatus, ProductError> {
        let token = self.authorization_token(&Value::Null)?;
        decode_value(self.get_json("/v1/ai/usage", &[], Some(&token))?)
    }

    pub fn wallet_history(&self, cursor: Option<&str>) -> Result<WalletHistoryPage, ProductError> {
        let query = cursor
            .map(str::trim)
            .filter(|cursor| !cursor.is_empty())
            .map(|cursor| vec![("cursor", cursor)])
            .unwrap_or_default();
        let token = self.authorization_token(&Value::Null)?;
        decode_value(self.get_json("/v1/wallet/history", &query, Some(&token))?)
    }

    pub fn wallet_top_up(&self, sku: &str, idempotency_key: &str) -> Result<Value, ProductError> {
        let token = self.authorization_token(&Value::Null)?;
        self.post_json(
            "/v1/wallet/top-up",
            json!({
                "sku": non_empty(sku, "sku")?,
                "idempotencyKey": non_empty(idempotency_key, "idempotencyKey")?,
            }),
            Some(&token),
        )
    }

    pub fn purchases(&self, cursor: Option<&str>) -> Result<PurchasePage, ProductError> {
        let query = cursor
            .map(str::trim)
            .filter(|cursor| !cursor.is_empty())
            .map(|cursor| vec![("cursor", cursor)])
            .unwrap_or_default();
        let token = self.authorization_token(&Value::Null)?;
        decode_value(self.get_json("/v1/purchases", &query, Some(&token))?)
    }

    pub fn restore_purchases(&self) -> Result<PurchasePage, ProductError> {
        let token = self.authorization_token(&Value::Null)?;
        decode_value(self.post_json("/v1/purchases/restore", json!({}), Some(&token))?)
    }

    pub fn quote(&self, plugin_id: &str, sku: &str) -> Result<Quote, ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let token = self.authorization_token(&Value::Null)?;
        decode_value(self.post_json(
            &format!("/v1/plugins/{plugin_id}/commerce/quote"),
            json!({"sku": non_empty(sku, "sku")?}),
            Some(&token),
        )?)
    }

    pub fn purchase(
        &self,
        plugin_id: &str,
        request: &PurchaseRequest,
    ) -> Result<PurchaseReceipt, ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let token = self.authorization_token(&Value::Null)?;
        let body = serde_json::to_value(request)
            .map_err(|error| ProductError::Response(error.to_string()))?;
        decode_value(self.post_json(
            &format!("/v1/plugins/{plugin_id}/commerce/purchase"),
            body,
            Some(&token),
        )?)
    }

    pub fn entitlement(
        &self,
        plugin_id: &str,
        capability: &str,
    ) -> Result<Option<Entitlement>, ProductError> {
        let plugin_id = safe_path_identifier(plugin_id, "pluginId")?;
        let capability = safe_path_identifier(capability, "capability")?;
        let token = self.authorization_token(&Value::Null)?;
        let response = self.get_json(
            &format!("/v1/plugins/{plugin_id}/entitlements/{capability}"),
            &[],
            Some(&token),
        )?;
        response
            .get("entitlement")
            .cloned()
            .filter(|value| !value.is_null())
            .map(decode_value)
            .transpose()
    }

    pub fn delegated_plugin_token(
        &self,
        request: &DelegatedTokenRequest,
    ) -> Result<Value, ProductError> {
        let token = self.authorization_token(&Value::Null)?;
        let body = serde_json::to_value(request)
            .map_err(|error| ProductError::Response(error.to_string()))?;
        self.post_json("/v1/auth/plugin-token", body, Some(&token))
    }

    /// Returns the locally stored Fabushi/Alipay session token used by the
    /// first-party Responses provider. The value must stay in memory and must
    /// not be copied into Codex `auth.json` or logs.
    pub fn session_token(&self) -> Result<String, ProductError> {
        self.authorization_token(&Value::Null)
    }

    fn load_surface_state(&self) -> Result<ProductSurfaceState, ProductError> {
        let mut state = match std::fs::read(&self.surface_state_path) {
            Ok(bytes) => {
                serde_json::from_slice::<ProductSurfaceState>(&bytes).map_err(|error| {
                    ProductError::State(format!("decode product surface state: {error}"))
                })?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ProductSurfaceState::default()
            }
            Err(error) => {
                return Err(ProductError::State(format!(
                    "read {}: {error}",
                    self.surface_state_path.display()
                )));
            }
        };
        if state.schema_version != PRODUCT_SURFACE_STATE_SCHEMA {
            state = ProductSurfaceState::default();
        }
        for connector in default_surface_connectors() {
            if !state.connectors.iter().any(|item| item.id == connector.id) {
                state.connectors.push(connector);
            }
        }
        for bot in default_surface_bots() {
            if !state.bots.iter().any(|item| item.id == bot.id) {
                state.bots.push(bot);
            }
        }
        for integration in default_surface_listeners() {
            if !state
                .listeners
                .iter()
                .any(|item| item.platform == integration.platform)
            {
                state.listeners.push(integration);
            }
        }
        Ok(state)
    }

    fn save_surface_state(&self, state: &ProductSurfaceState) -> Result<(), ProductError> {
        if let Some(parent) = self.surface_state_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ProductError::State(format!("create {}: {error}", parent.display()))
            })?;
        }
        let temp = self
            .surface_state_path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(state).map_err(|error| {
            ProductError::State(format!("encode product surface state: {error}"))
        })?;
        std::fs::write(&temp, bytes)
            .map_err(|error| ProductError::State(format!("write {}: {error}", temp.display())))?;
        std::fs::rename(&temp, &self.surface_state_path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            ProductError::State(format!(
                "commit {}: {error}",
                self.surface_state_path.display()
            ))
        })
    }

    fn execute_surface_command(
        &self,
        request_type: &str,
        request: &Value,
    ) -> Result<Value, ProductError> {
        if request_type == "mahayana.marketplace.list" {
            return self.marketplace_browse(
                request.get("query").and_then(Value::as_str),
                request
                    .get("platform")
                    .and_then(Value::as_str)
                    .or(Some("desktop")),
            );
        }
        let command: FeatureCommand = serde_json::from_value(request.clone()).map_err(|error| {
            ProductError::State(format!("decode product surface command: {error}"))
        })?;
        let mut state = self.load_surface_state()?;
        let mut changed = false;
        let response = match command {
            FeatureCommand::ConnectorList { .. } => json!({"connectors": state.connectors}),
            FeatureCommand::ConnectorConnect { connector_id, .. } => {
                let connector = state
                    .connectors
                    .iter_mut()
                    .find(|connector| connector.id == connector_id)
                    .ok_or_else(|| {
                        ProductError::State(format!("unknown connector: {connector_id}"))
                    })?;
                if connector.transport == ConnectorTransport::Command {
                    connector.status = ConnectorStatus::Connected;
                    changed = true;
                    json!({"connector": connector})
                } else {
                    connector.status = ConnectorStatus::AuthRequired;
                    changed = true;
                    json!({"connector": connector, "requiresOAuth": true})
                }
            }
            FeatureCommand::ConnectorRenameAccount {
                connector_id,
                account_id,
                label,
                ..
            } => {
                let label = label.trim();
                if label.is_empty() {
                    return Err(ProductError::State(
                        "account label must not be empty".into(),
                    ));
                }
                let connector = state
                    .connectors
                    .iter_mut()
                    .find(|connector| connector.id == connector_id)
                    .ok_or_else(|| {
                        ProductError::State(format!("unknown connector: {connector_id}"))
                    })?;
                if !connector
                    .accounts
                    .iter()
                    .any(|account| account.id == account_id)
                    && account_id.starts_with(&format!("mcp:{connector_id}"))
                {
                    connector.accounts.push(ConnectorAccountSummary {
                        id: account_id.clone(),
                        label: connector.display_name.clone(),
                        status: ConnectorStatus::Connected,
                        email: None,
                        team_managed: Some(false),
                        error: None,
                    });
                }
                let account = connector
                    .accounts
                    .iter_mut()
                    .find(|account| account.id == account_id)
                    .ok_or_else(|| {
                        ProductError::State(format!("unknown connector account: {account_id}"))
                    })?;
                if account.team_managed == Some(true) {
                    return Err(ProductError::State(
                        "team-managed accounts cannot be renamed".into(),
                    ));
                }
                account.label = label.to_string();
                changed = true;
                json!({"connector": connector})
            }
            FeatureCommand::ConnectorRemoveAccount {
                connector_id,
                account_id,
                ..
            } => {
                let connector = state
                    .connectors
                    .iter_mut()
                    .find(|connector| connector.id == connector_id)
                    .ok_or_else(|| {
                        ProductError::State(format!("unknown connector: {connector_id}"))
                    })?;
                if connector
                    .accounts
                    .iter()
                    .any(|account| account.id == account_id && account.team_managed == Some(true))
                {
                    return Err(ProductError::State(
                        "team-managed accounts cannot be removed".into(),
                    ));
                }
                let before = connector.accounts.len();
                connector
                    .accounts
                    .retain(|account| account.id != account_id);
                if connector.accounts.len() == before
                    && !account_id.starts_with(&format!("mcp:{connector_id}"))
                {
                    return Err(ProductError::State(format!(
                        "unknown connector account: {account_id}"
                    )));
                }
                if connector.accounts.is_empty() {
                    connector.status = ConnectorStatus::Disconnected;
                }
                changed = true;
                json!({"connector": connector})
            }
            FeatureCommand::ConnectorSetToolEnabled {
                connector_id,
                tool_id,
                enabled,
                ..
            } => {
                let connector = state
                    .connectors
                    .iter_mut()
                    .find(|connector| connector.id == connector_id)
                    .ok_or_else(|| {
                        ProductError::State(format!("unknown connector: {connector_id}"))
                    })?;
                if !connector.tools.iter().any(|tool| tool.id == tool_id) {
                    connector.tools.push(ConnectorToolSummary {
                        id: tool_id.clone(),
                        name: tool_id.clone(),
                        description: "Discovered from the live MCP server.".into(),
                        enabled,
                        requires_approval: Some(true),
                    });
                } else if let Some(tool) =
                    connector.tools.iter_mut().find(|tool| tool.id == tool_id)
                {
                    tool.enabled = enabled;
                }
                changed = true;
                json!({"connector": connector})
            }
            FeatureCommand::SkillList { agent_id, .. } => {
                let skills = state
                    .skills
                    .iter()
                    .filter(|skill| {
                        agent_id.as_ref().is_none_or(|agent_id| {
                            skill
                                .owner_agent_id
                                .as_ref()
                                .is_none_or(|owner| owner == agent_id)
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                json!({"skills": skills, "teams": state.skill_teams})
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
                let name = name.trim();
                let use_when = use_when.trim();
                if name.is_empty() || use_when.is_empty() || instructions.trim().is_empty() {
                    return Err(ProductError::State(
                        "skill name, useWhen, and instructions are required".into(),
                    ));
                }
                let id = id.unwrap_or_else(|| {
                    state.sequence += 1;
                    format!("skill-{}-{}", surface_now_millis(), state.sequence)
                });
                let existing = state.skills.iter().position(|skill| skill.id == id);
                if existing.is_some_and(|index| state.skills[index].read_only == Some(true)) {
                    return Err(ProductError::State(
                        "managed skills cannot be edited".into(),
                    ));
                }
                let previous = existing.map(|index| state.skills[index].clone());
                let skill = SkillSummary {
                    id: id.clone(),
                    name: name.to_string(),
                    description: description.trim().to_string(),
                    use_when: use_when.to_string(),
                    instructions,
                    source: previous
                        .as_ref()
                        .map_or(SkillSource::Private, |skill| skill.source),
                    publish_state: previous
                        .as_ref()
                        .map_or(SkillPublishState::Local, |skill| skill.publish_state),
                    owner_agent_id,
                    team_id: previous.as_ref().and_then(|skill| skill.team_id.clone()),
                    team_name: previous.as_ref().and_then(|skill| skill.team_name.clone()),
                    read_only: previous.as_ref().and_then(|skill| skill.read_only),
                    updated_at_ms: surface_now_millis(),
                };
                self.persist_private_skill(&skill)?;
                if let Some(index) = existing {
                    state.skills[index] = skill.clone();
                } else {
                    state.skills.push(skill.clone());
                }
                changed = true;
                json!({"action": if previous.is_some() {"updated"} else {"created"}, "skill": skill})
            }
            FeatureCommand::SkillDelete { id, .. } => {
                let index = state
                    .skills
                    .iter()
                    .position(|skill| skill.id == id)
                    .ok_or_else(|| ProductError::State(format!("unknown skill: {id}")))?;
                if state.skills[index].read_only == Some(true)
                    || state.skills[index].publish_state == SkillPublishState::Managed
                {
                    return Err(ProductError::State(
                        "managed skills cannot be deleted".into(),
                    ));
                }
                let skill = state.skills.remove(index);
                self.remove_private_skill(&skill.id)?;
                changed = true;
                json!({"skill": skill})
            }
            FeatureCommand::SkillPublish { id, team_id, .. } => {
                let team = state
                    .skill_teams
                    .iter()
                    .find(|team| team.id == team_id)
                    .cloned()
                    .ok_or_else(|| {
                        ProductError::State(
                            "no publishable Mahayana team is available for this account".into(),
                        )
                    })?;
                let skill = state
                    .skills
                    .iter_mut()
                    .find(|skill| skill.id == id)
                    .ok_or_else(|| ProductError::State(format!("unknown skill: {id}")))?;
                if skill.description.trim().is_empty() {
                    return Err(ProductError::State(
                        "add a skill description before publishing".into(),
                    ));
                }
                skill.source = SkillSource::Team;
                skill.publish_state = SkillPublishState::Published;
                skill.team_id = Some(team.id);
                skill.team_name = Some(team.name);
                skill.updated_at_ms = surface_now_millis();
                changed = true;
                json!({"skill": skill})
            }
            FeatureCommand::SkillUnpublish { id, .. } => {
                let skill = state
                    .skills
                    .iter_mut()
                    .find(|skill| skill.id == id)
                    .ok_or_else(|| ProductError::State(format!("unknown skill: {id}")))?;
                if skill.publish_state == SkillPublishState::Managed {
                    return Err(ProductError::State(
                        "managed skills cannot be unpublished".into(),
                    ));
                }
                skill.source = SkillSource::Private;
                skill.publish_state = SkillPublishState::Local;
                skill.team_id = None;
                skill.team_name = None;
                skill.updated_at_ms = surface_now_millis();
                changed = true;
                json!({"skill": skill})
            }
            FeatureCommand::SkillSync { id, .. } => {
                let skill = state
                    .skills
                    .iter_mut()
                    .find(|skill| skill.id == id)
                    .ok_or_else(|| ProductError::State(format!("unknown skill: {id}")))?;
                if skill.team_id.is_none() {
                    return Err(ProductError::State(
                        "only published skills can be synced".into(),
                    ));
                }
                skill.publish_state = SkillPublishState::Synced;
                skill.updated_at_ms = surface_now_millis();
                changed = true;
                json!({"skill": skill})
            }
            FeatureCommand::BotList { .. } => json!({"bots": state.bots}),
            FeatureCommand::BotSetHidden { id, hidden, .. } => {
                let bot = state
                    .bots
                    .iter_mut()
                    .find(|bot| bot.id == id)
                    .ok_or_else(|| ProductError::State(format!("unknown bot: {id}")))?;
                bot.hidden = hidden;
                changed = true;
                json!({"bot": bot})
            }
            FeatureCommand::DraftResolve { draft, action, .. } => {
                validate_surface_draft(&draft)?;
                let status = match action {
                    DraftAction::Discard => DraftSendState::Discarded,
                    DraftAction::Send => {
                        return Err(ProductError::State(
                            "draft sending requires a connected Gmail or Slack MCP runtime".into(),
                        ));
                    }
                };
                json!({"draftId": draft.id(), "status": status})
            }
            FeatureCommand::SecretProvide {
                secret_request_id,
                value,
                ..
            } => {
                if value.is_empty() {
                    return Err(ProductError::State("secret value must not be empty".into()));
                }
                let name = managed_secret_name(&secret_request_id)?;
                self.managed_secrets
                    .set(&SecretScope::Global, &name, &value)
                    .map_err(secrets_error)?;
                // Never echo the secret or its encrypted storage key. The
                // opaque request id is the only renderer-visible handle.
                json!({"provided": true, "secretRequestId": secret_request_id})
            }
            FeatureCommand::ListenerList { .. } => json!({"integrations": state.listeners}),
            FeatureCommand::ListenerConnect { platform, .. } => {
                if platform != ListenerPlatform::Git {
                    return Err(ProductError::State(
                        "listener connection requires the matching MCP OAuth flow".into(),
                    ));
                }
                let integration = state
                    .listeners
                    .iter_mut()
                    .find(|integration| integration.platform == platform)
                    .ok_or_else(|| ProductError::State("unsupported listener platform".into()))?;
                integration.is_connected = true;
                integration.account_label = Some("Local repository".into());
                integration.error = None;
                changed = true;
                json!({"integration": integration})
            }
            FeatureCommand::ListenerDisconnect { platform, .. } => {
                let integration = state
                    .listeners
                    .iter_mut()
                    .find(|integration| integration.platform == platform)
                    .ok_or_else(|| ProductError::State("unsupported listener platform".into()))?;
                integration.is_connected = false;
                integration.account_label = None;
                integration.error = None;
                changed = true;
                json!({"integration": integration})
            }
            FeatureCommand::UpdateStatus { .. } => json!({"state": state.update_state}),
            FeatureCommand::UpdateCheck { .. } => json!({"state": state.update_state}),
            FeatureCommand::UpdateInstall { .. } => {
                return Err(ProductError::State(
                    "this Fabushi build is not packaged with a desktop updater".into(),
                ));
            }
            _ => {
                return Err(ProductError::UnsupportedRequest(request_type.to_string()));
            }
        };
        if changed {
            self.save_surface_state(&state)?;
        }
        Ok(response)
    }

    pub fn execute(&self, request_type: &str, request: &Value) -> Result<Value, ProductError> {
        if request_type.starts_with("mahayana.connector.")
            || request_type.starts_with("mahayana.skill.")
            || request_type.starts_with("mahayana.bot.")
            || request_type.starts_with("mahayana.draft.")
            || request_type.starts_with("mahayana.secret.")
            || request_type.starts_with("mahayana.listener.")
            || request_type.starts_with("mahayana.update.")
            || request_type == "mahayana.marketplace.list"
        {
            return self.execute_surface_command(request_type, request);
        }
        match request_type {
            "mahayana.auth.status" => self.auth_status(request),
            "mahayana.auth.session.restore" => self.restore_session(),
            "mahayana.auth.password.login" => self.password_login(request),
            "mahayana.auth.browser.start" => self.browser_login_start(request),
            "mahayana.auth.browser.poll" => self.browser_login_poll(request),
            "mahayana.auth.browser.cancel" => self.browser_login_cancel(request),
            "mahayana.auth.browser.reopen" => self.browser_login_reopen(request),
            "mahayana.auth.oauth.providers" => self.oauth_providers(),
            "mahayana.auth.oauth.start" => self.oauth_start(request),
            "mahayana.auth.oauth.poll" => self.oauth_poll(request),
            "mahayana.auth.register" => self.register(request),
            "mahayana.auth.verification.send" => self.verification_send(request),
            "mahayana.auth.password.forgot" => self.password_forgot(request),
            "mahayana.auth.password.reset" => self.password_reset(request),
            "mahayana.auth.alipay.start" => self.alipay_start(request),
            "mahayana.auth.alipay.complete" => self.alipay_complete(request),
            "mahayana.auth.alipay.poll" => self.alipay_poll(request),
            "mahayana.auth.alipay.sdk.start" => {
                self.get_json("/api/auth/alipay/auth-string", &[], None)
            }
            "mahayana.auth.alipay.sdk.complete" => self.alipay_sdk_complete(request),
            "mahayana.auth.alipay.register" => self.alipay_register(request),
            "mahayana.auth.apple.complete" => self.apple_complete(request),
            "mahayana.auth.firebase.phone.complete" => self.firebase_phone_complete(request),
            "mahayana.auth.logout" => self.logout(),
            "mahayana.platform.request" => self.platform_request(request),
            "mahayana.contacts.list" => self.authorized_get(request, "/api/social/friends", &[]),
            "mahayana.contacts.search" => {
                let query = required_string(request, "query")?;
                self.authorized_get(request, "/api/social/users/search", &[("q", query)])
            }
            "mahayana.contacts.add" => {
                let contact = required_string(request, "contact")?;
                let mut body = json!({"targetUserId": contact});
                if let Some(message) = optional_string(request, "message") {
                    body["message"] = Value::String(message.to_string());
                }
                self.authorized_post(request, "/api/social/friend-requests", body)
            }
            "mahayana.contacts.requests" => {
                self.authorized_get(request, "/api/social/friend-requests/incoming", &[])
            }
            "mahayana.contacts.accept" => {
                let request_id = required_identifier(request, "requestId")?;
                self.authorized_post(
                    request,
                    &format!("/api/social/friend-requests/{request_id}/accept"),
                    json!({}),
                )
            }
            "mahayana.messages.list" => {
                let contact = required_string(request, "contact")?;
                let limit = request
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 200)
                    .to_string();
                self.authorized_get(
                    request,
                    "/api/social/messages",
                    &[("contactId", contact), ("limit", &limit)],
                )
            }
            "mahayana.messages.send" => {
                let contact = required_string(request, "contact")?;
                let text = required_string(request, "text")?;
                let mut body = json!({"contactId": contact, "text": text});
                if let Some(client_request_id) = optional_string(request, "clientRequestId") {
                    body["clientRequestId"] = Value::String(client_request_id.to_string());
                }
                self.authorized_post(request, "/api/social/messages", body)
            }
            "mahayana.remote.computers.list" => self.authorized_get(request, "/v1/computers", &[]),
            "mahayana.remote.computer.register" => {
                let device_id = required_identifier(request, "deviceId")?;
                let label = required_string(request, "label")?;
                let device_secret = required_string(request, "deviceSecret")?;
                self.authorized_post(
                    request,
                    "/v1/computers/register",
                    json!({"deviceId": device_id, "label": label, "deviceSecret": device_secret}),
                )
            }
            "mahayana.remote.computer.heartbeat" => {
                let device_id = required_identifier(request, "deviceId")?;
                let device_secret = required_string(request, "deviceSecret")?;
                self.authorized_post(
                    request,
                    "/v1/computers/heartbeat",
                    json!({"deviceId": device_id, "deviceSecret": device_secret}),
                )
            }
            "mahayana.remote.computer.pair" => {
                let pairing_code = required_string(request, "pairingCode")?;
                let label = required_string(request, "label")?;
                self.authorized_post(
                    request,
                    "/v1/computers/pair",
                    json!({"pairingCode": pairing_code, "label": label}),
                )
            }
            "mahayana.remote.computer.clients" => {
                let device_id = required_identifier(request, "deviceId")?;
                self.authorized_get(request, &format!("/v1/computers/{device_id}/clients"), &[])
            }
            "mahayana.remote.computer.client.revoke" => {
                let device_id = required_identifier(request, "deviceId")?;
                let client_id = required_identifier(request, "clientId")?;
                self.authorized_post(
                    request,
                    &format!("/v1/computers/{device_id}/clients/{client_id}/revoke"),
                    json!({}),
                )
            }
            "mahayana.remote.computer.sessions" => {
                let device_id = required_identifier(request, "deviceId")?;
                let device_secret = required_string(request, "deviceSecret")?;
                self.authorized_post(
                    request,
                    &format!("/v1/computers/{device_id}/sessions/list"),
                    json!({"deviceSecret": device_secret}),
                )
            }
            "mahayana.remote.computer.session.create" => {
                let device_id = required_identifier(request, "deviceId")?;
                let client_id = required_identifier(request, "clientId")?;
                self.authorized_post(
                    request,
                    &format!("/v1/computers/{device_id}/sessions"),
                    json!({"clientId": client_id}),
                )
            }
            "mahayana.remote.computer.session.activate" => {
                let device_id = required_identifier(request, "deviceId")?;
                let session_id = required_identifier(request, "sessionId")?;
                let device_secret = required_string(request, "deviceSecret")?;
                self.authorized_post(
                    request,
                    &format!("/v1/computers/{device_id}/sessions/{session_id}/activate"),
                    json!({"deviceSecret": device_secret}),
                )
            }
            "mahayana.remote.computer.session.close" => {
                let device_id = required_identifier(request, "deviceId")?;
                let session_id = required_identifier(request, "sessionId")?;
                let role = required_identifier(request, "role")?;
                let mut body = json!({"role": role});
                if let Some(device_secret) = optional_string(request, "deviceSecret") {
                    body["deviceSecret"] = Value::String(device_secret.to_string());
                }
                if let Some(client_id) = optional_string(request, "clientId") {
                    body["clientId"] = Value::String(client_id.to_string());
                }
                if let Some(mobile_token) = optional_string(request, "mobileToken") {
                    body["mobileToken"] = Value::String(mobile_token.to_string());
                }
                self.authorized_post(
                    request,
                    &format!("/v1/computers/{device_id}/sessions/{session_id}/close"),
                    body,
                )
            }
            "mahayana.remote.computer.signal" => {
                let device_id = required_identifier(request, "deviceId")?;
                let session_id = required_identifier(request, "sessionId")?;
                let sender_role = required_identifier(request, "senderRole")?;
                let kind = required_identifier(request, "kind")?;
                let payload = request.get("payload").cloned().unwrap_or(Value::Null);
                let mut body = json!({
                    "sessionId": session_id,
                    "senderRole": sender_role,
                    "kind": kind,
                    "payload": payload,
                });
                if let Some(device_secret) = optional_string(request, "deviceSecret") {
                    body["deviceSecret"] = Value::String(device_secret.to_string());
                }
                if let Some(client_id) = optional_string(request, "clientId") {
                    body["clientId"] = Value::String(client_id.to_string());
                }
                if let Some(mobile_token) = optional_string(request, "mobileToken") {
                    body["mobileToken"] = Value::String(mobile_token.to_string());
                }
                self.authorized_post(request, &format!("/v1/computers/{device_id}/signals"), body)
            }
            "mahayana.remote.computer.signals.drain" => {
                let device_id = required_identifier(request, "deviceId")?;
                let session_id = required_identifier(request, "sessionId")?;
                let receiver_role = required_identifier(request, "receiverRole")?;
                let mut body = json!({
                    "sessionId": session_id,
                    "receiverRole": receiver_role,
                    "afterSignalId": request.get("afterSignalId").and_then(Value::as_i64),
                });
                if let Some(device_secret) = optional_string(request, "deviceSecret") {
                    body["deviceSecret"] = Value::String(device_secret.to_string());
                }
                if let Some(client_id) = optional_string(request, "clientId") {
                    body["clientId"] = Value::String(client_id.to_string());
                }
                if let Some(mobile_token) = optional_string(request, "mobileToken") {
                    body["mobileToken"] = Value::String(mobile_token.to_string());
                }
                self.authorized_post(
                    request,
                    &format!("/v1/computers/{device_id}/signals/drain"),
                    body,
                )
            }
            "mahayana.miniapps.registry" => self.miniapp_registry(request),
            other => Err(ProductError::UnsupportedRequest(other.to_string())),
        }
    }

    fn auth_status(&self, request: &Value) -> Result<Value, ProductError> {
        let command_token = access_token(request);
        let session = self.load_session()?;
        let token = match command_token {
            Some(token) => Some(token.to_string()),
            None => match session.clone() {
                Some(session) => match self.active_session_token(session) {
                    Ok(token) => Some(token),
                    Err(error) if terminal_session_error(&error) => {
                        self.remove_session()?;
                        None
                    }
                    Err(error) => return Err(error),
                },
                None => None,
            },
        };
        let Some(token) = token else {
            return Ok(json!({
                "@type": "mahayana.auth.status",
                "loggedIn": false,
                "provider": "alipay",
            }));
        };
        let provider = session
            .as_ref()
            .and_then(|value| value.get("provider"))
            .and_then(Value::as_str)
            .unwrap_or("official");
        match self.get_json("/api/auth/user-info", &[], Some(&token)) {
            Ok(user) => Ok(json!({
                "@type": "mahayana.auth.status",
                "loggedIn": true,
                "provider": provider,
                "user": user,
            })),
            Err(ProductError::HttpStatus { status: 401, .. }) => {
                if command_token.is_none() {
                    self.remove_session()?;
                }
                Ok(json!({
                    "@type": "mahayana.auth.status",
                    "loggedIn": false,
                    "provider": "alipay",
                    "expired": true,
                }))
            }
            Err(error) => Err(error),
        }
    }

    fn password_login(&self, request: &Value) -> Result<Value, ProductError> {
        let mut body = json!({
            "username": required_string(request, "username")?,
            "password": required_string(request, "password")?,
        });
        if let Some(device_id) = optional_string(request, "deviceId") {
            body["deviceId"] = Value::String(device_id.to_string());
        }
        let response = self.post_json("/api/auth/login", body, None)?;
        self.store_login_response(&response, "password")?;
        typed_session(response, "password", true)
    }

    fn oauth_providers(&self) -> Result<Value, ProductError> {
        let response = self.get_json("/api/auth/oauth/providers", &[], None)?;
        Ok(response.get("providers").cloned().unwrap_or(response))
    }

    fn oauth_start(&self, request: &Value) -> Result<Value, ProductError> {
        let provider = required_identifier(request, "provider")?;
        self.post_json(
            "/api/auth/oauth/start",
            json!({
                "provider": provider,
                "platform": optional_string(request, "platform").unwrap_or("desktop"),
                "deviceId": optional_string(request, "deviceId"),
            }),
            None,
        )
    }

    fn finish_polled_login(&self, response: Value) -> Result<Value, ProductError> {
        if response.get("status").and_then(Value::as_str) != Some("completed") {
            return Ok(response);
        }
        let session = response.get("session").unwrap_or(&response);
        let provider = response
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("browser");
        self.store_login_response(session, provider)?;
        Ok(json!({
            "status": "completed",
            "provider": provider,
            "auth": typed_session(session.clone(), provider, true)?,
        }))
    }

    fn browser_login_start(&self, request: &Value) -> Result<Value, ProductError> {
        let mut response = self.post_json(
            "/api/auth/browser/start",
            json!({
                "platform": optional_string(request, "platform").unwrap_or("desktop"),
                "deviceId": optional_string(request, "deviceId"),
            }),
            None,
        )?;
        let attempt_id = required_identifier(&response, "attemptId")?.to_string();
        let poll_secret = required_string(&response, "pollSecret")?.to_string();
        self.save_browser_login_poll_secret(&attempt_id, &poll_secret)?;
        if let Some(object) = response.as_object_mut() {
            object.remove("pollSecret");
        }
        Ok(response)
    }

    fn browser_login_reopen(&self, request: &Value) -> Result<Value, ProductError> {
        let attempt_id = required_identifier(request, "attemptId")?;
        let Some(poll_secret) = self.load_browser_login_poll_secret(&attempt_id)? else {
            return Ok(json!({"status": "expired"}));
        };
        self.post_json(
            &format!("/api/auth/browser/attempts/{attempt_id}/reopen"),
            json!({"pollSecret": poll_secret}),
            None,
        )
    }

    fn browser_login_cancel(&self, request: &Value) -> Result<Value, ProductError> {
        let attempt_id = required_identifier(request, "attemptId")?;
        let Some(poll_secret) = self.load_browser_login_poll_secret(&attempt_id)? else {
            return Ok(json!({"status": "expired"}));
        };
        let response = self.post_json(
            &format!("/api/auth/browser/attempts/{attempt_id}/cancel"),
            json!({"pollSecret": poll_secret}),
            None,
        )?;
        let terminal = matches!(
            response.get("status").and_then(Value::as_str),
            Some("cancelled" | "expired" | "failed")
        );
        if terminal {
            let _ = self.remove_browser_login_poll_secret(&attempt_id);
        }
        Ok(response)
    }

    fn browser_login_poll(&self, request: &Value) -> Result<Value, ProductError> {
        let attempt_id = required_identifier(request, "attemptId")?;
        let Some(poll_secret) = self.load_browser_login_poll_secret(&attempt_id)? else {
            return Ok(json!({"status": "expired"}));
        };
        let response = self.post_json(
            &format!("/api/auth/browser/attempts/{attempt_id}"),
            json!({"pollSecret": poll_secret}),
            None,
        )?;
        let terminal = matches!(
            response.get("status").and_then(Value::as_str),
            Some("completed" | "expired" | "cancelled" | "failed")
        );
        let result = self.finish_polled_login(response);
        if terminal {
            let _ = self.remove_browser_login_poll_secret(&attempt_id);
        }
        result
    }

    fn oauth_poll(&self, request: &Value) -> Result<Value, ProductError> {
        let attempt_id = required_identifier(request, "attemptId")?;
        let response =
            self.get_json(&format!("/api/auth/oauth/attempts/{attempt_id}"), &[], None)?;
        self.finish_polled_login(response)
    }

    fn register(&self, request: &Value) -> Result<Value, ProductError> {
        self.post_json(
            "/api/auth/register",
            json!({
                "username": required_string(request, "username")?,
                "email": required_string(request, "email")?,
                "password": required_string(request, "password")?,
                "verificationCode": required_string(request, "verificationCode")?,
            }),
            None,
        )
    }

    fn verification_send(&self, request: &Value) -> Result<Value, ProductError> {
        self.post_json(
            "/api/auth/send-verification-code",
            json!({
                "email": required_string(request, "email")?,
                "type": required_string(request, "type")?,
            }),
            None,
        )
    }

    fn password_forgot(&self, request: &Value) -> Result<Value, ProductError> {
        self.post_json(
            "/api/auth/forgot-password",
            json!({"email": required_string(request, "email")?}),
            None,
        )
    }

    fn password_reset(&self, request: &Value) -> Result<Value, ProductError> {
        self.post_json(
            "/api/auth/reset-password",
            json!({
                "email": required_string(request, "email")?,
                "token": required_string(request, "resetToken")?,
                "newPassword": required_string(request, "newPassword")?,
            }),
            None,
        )
    }

    fn alipay_start(&self, request: &Value) -> Result<Value, ProductError> {
        let platform = optional_string(request, "platform").unwrap_or("cli");
        let response = self.get_json(
            "/api/auth/alipay/login-url",
            &[("platform", platform)],
            None,
        )?;
        Ok(json!({
            "@type": "mahayana.auth.alipay.authorization",
            "provider": "alipay",
            "loginUrl": response.get("authUrl").or_else(|| response.get("loginUrl")),
            "state": response.get("state"),
            "appId": response.get("appId"),
            "platform": response.get("platform").cloned().unwrap_or_else(|| Value::String(platform.to_string())),
        }))
    }

    /// Restores UI-safe account state. Access and refresh credentials never
    /// cross the Rust ABI into a native host shell.
    fn restore_session(&self) -> Result<Value, ProductError> {
        let session = self.required_session()?;
        let mut output = session.as_object().cloned().unwrap_or_default();
        strip_credentials(&mut output);
        output.insert(
            "@type".to_string(),
            Value::String("mahayana.auth.session".to_string()),
        );
        output.insert("loggedIn".to_string(), Value::Bool(true));
        output.insert("sessionStored".to_string(), Value::Bool(true));
        Ok(Value::Object(output))
    }

    /// Import the pre-Secret-Store JSON once. Legacy credentials remain
    /// readable only here and are normalized before entering the encrypted
    /// Mahayana auth namespace; no token alias is accepted by normal APIs.
    fn import_legacy_session_if_needed(&self) {
        if !matches!(self.load_session(), Ok(None)) {
            return;
        }
        let Ok(contents) = std::fs::read_to_string(&self.session_path) else {
            return;
        };
        let Ok(mut session) = serde_json::from_str::<Value>(&contents) else {
            return;
        };
        let Some(object) = session.as_object_mut() else {
            return;
        };
        if object.get("accessToken").is_none() {
            let Some(token) = object.remove("token").filter(Value::is_string) else {
                return;
            };
            object.insert("accessToken".to_string(), token);
        }
        let _ = self.save_session(&session);
    }

    fn alipay_complete(&self, request: &Value) -> Result<Value, ProductError> {
        let auth_code = required_string(request, "authCode")?;
        let mut body = json!({"auth_code": auth_code});
        if let Some(state) = optional_string(request, "state") {
            body["state"] = Value::String(state.to_string());
        }
        let response = self.post_json("/api/auth/alipay/login", body, None)?;
        self.store_login_response(&response, "alipay")?;
        typed_session(response, "alipay", false)
    }

    fn alipay_poll(&self, request: &Value) -> Result<Value, ProductError> {
        let state = required_string(request, "state")?;
        let response = self.get_json("/api/auth/alipay/cli-session", &[("state", state)], None)?;
        let response = normalize_alipay_cli_response(response);
        if response.get("status").and_then(Value::as_str) == Some("complete") {
            self.store_login_response(&response, "alipay")?;
        }
        Ok(response)
    }

    fn alipay_sdk_complete(&self, request: &Value) -> Result<Value, ProductError> {
        let auth_code = required_string(request, "authCode")?;
        let mut body = json!({"auth_code": auth_code});
        if let Some(target_id) = optional_string(request, "targetId") {
            body["target_id"] = Value::String(target_id.to_string());
        }
        let response = self.post_json("/api/auth/alipay/sdk-login", body, None)?;
        self.store_login_response(&response, "alipay")?;
        typed_session(response, "alipay", false)
    }

    fn alipay_register(&self, request: &Value) -> Result<Value, ProductError> {
        let mut body = json!({
            "alipayProviderSubject": required_string(request, "alipayProviderSubject")?,
        });
        copy_optional_fields(
            request,
            &mut body,
            &[
                "alipaySubjectType",
                "username",
                "password",
                "nickname",
                "avatar",
                "email",
                "alipayNickname",
                "alipayAvatar",
            ],
        );
        if request.get("oneClick").and_then(Value::as_bool) == Some(true) {
            body["oneClick"] = Value::Bool(true);
        }
        let response = self.post_json("/api/auth/alipay/register", body, None)?;
        self.store_login_response(&response, "alipay")?;
        typed_session(response, "alipay", false)
    }

    fn apple_complete(&self, request: &Value) -> Result<Value, ProductError> {
        let mut body = json!({
            "identityToken": required_string(request, "identityToken")?,
            "authorizationCode": required_string(request, "authorizationCode")?,
        });
        copy_optional_fields(request, &mut body, &["email", "givenName", "familyName"]);
        let response = self.post_json("/api/auth/apple-login", body, None)?;
        self.store_login_response(&response, "apple")?;
        typed_session(response, "apple", false)
    }

    fn firebase_phone_complete(&self, request: &Value) -> Result<Value, ProductError> {
        let response = self.post_json(
            "/api/auth/firebase-phone-login",
            json!({
                "idToken": required_string(request, "idToken")?,
                "phoneNumber": required_string(request, "phoneNumber")?,
                "firebaseUid": required_string(request, "firebaseUid")?,
                "isNewUser": request.get("isNewUser").and_then(Value::as_bool).unwrap_or(false),
            }),
            None,
        )?;
        self.store_login_response(&response, "firebase-phone")?;
        typed_session(response, "firebase-phone", false)
    }

    fn store_login_response(&self, response: &Value, provider: &str) -> Result<(), ProductError> {
        if let Some(token) = access_token(response) {
            let session = json!({
                "token": token,
                "accessToken": token,
                "refreshToken": response.get("refreshToken"),
                "accessTokenExpiresAt": response.get("accessTokenExpiresAt"),
                "refreshTokenExpiresAt": response.get("refreshTokenExpiresAt"),
                "sessionId": response.get("sessionId"),
                "deviceId": response.get("deviceId"),
                "tokenType": response.get("tokenType"),
                "provider": provider,
                "user": response.get("user"),
                "username": response.get("username"),
                "email": response.get("email"),
            });
            self.save_session(&session)?;
        }
        Ok(())
    }

    fn logout(&self) -> Result<Value, ProductError> {
        let server_session_revoked = match self.load_session()? {
            Some(session) => self
                .active_session_token(session)
                .and_then(|token| self.post_json("/api/auth/logout", json!({}), Some(&token)))
                .is_ok(),
            None => true,
        };
        self.remove_session()?;
        Ok(json!({
            "@type": "mahayana.auth.loggedOut",
            "loggedIn": false,
            "provider": "official",
            "serverSessionRevoked": server_session_revoked,
        }))
    }

    fn miniapp_registry(&self, request: &Value) -> Result<Value, ProductError> {
        let token = self.optional_authorization_token(request)?;
        self.get_json("/api/plugins/registry", &[], token.as_deref())
    }

    /// Executes one same-origin platform request with the active Rust-owned
    /// session. UI shells supply business data but never receive bearer or
    /// refresh credentials.
    fn platform_request(&self, request: &Value) -> Result<Value, ProductError> {
        let method = required_string(request, "method")?.to_ascii_uppercase();
        if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
            return Err(ProductError::InvalidParameter("method"));
        }
        let path = safe_platform_path(required_string(request, "path")?)?;
        let mut url = url::Url::parse(&format!("{}{}", self.api_base_url, path))
            .map_err(|error| ProductError::Configuration(error.to_string()))?;
        if let Some(query) = request.get("query").and_then(Value::as_object) {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                let value = value
                    .as_str()
                    .ok_or(ProductError::InvalidParameter("query"))?;
                pairs.append_pair(name, value);
            }
        }
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| ProductError::InvalidParameter("method"))?;
        let client = http_client()?;
        let mut builder = client
            .request(method, url)
            .header("Accept", "application/json");
        if request
            .get("authenticated")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            builder = builder.bearer_auth(self.authorization_token(&Value::Null)?);
        }
        if let Some(body) = request.get("body").filter(|body| !body.is_null()) {
            builder = builder.json(body);
        }
        let response = builder
            .send()
            .map_err(|error| ProductError::Transport(error.to_string()))?;
        let status_code = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let raw_body_text = response
            .text()
            .map_err(|error| ProductError::Transport(error.to_string()))?;
        let decoded = serde_json::from_str::<Value>(&raw_body_text)
            .unwrap_or_else(|_| Value::String(raw_body_text.clone()));
        let data = redact_secrets(&decoded);
        let body_text = if decoded.is_object() || decoded.is_array() {
            serde_json::to_string(&data)
                .map_err(|error| ProductError::Response(error.to_string()))?
        } else {
            raw_body_text
        };
        Ok(json!({
            "@type": "mahayana.platform.response",
            "ok": (200..300).contains(&status_code),
            "statusCode": status_code,
            "contentType": content_type,
            "bodyText": body_text,
            "data": data,
        }))
    }

    /// Publishes a locally generated, scan-ready mini-app to the user's
    /// personal sandbox. The backend intentionally accepts anonymous users,
    /// so login enriches ownership but is never a prerequisite for creation.
    fn authorized_get(
        &self,
        command: &Value,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, ProductError> {
        let token = self.authorization_token(command)?;
        self.get_json(path, query, Some(&token))
    }

    fn authorized_post(
        &self,
        command: &Value,
        path: &str,
        body: Value,
    ) -> Result<Value, ProductError> {
        let token = self.authorization_token(command)?;
        self.post_json(path, body, Some(&token))
    }

    fn authorization_token(&self, command: &Value) -> Result<String, ProductError> {
        if let Some(token) = access_token(command) {
            return Ok(token.to_string());
        }
        if let Some(token) = self.environment_test_account_token()? {
            return Ok(token);
        }
        let session = self.required_session()?;
        self.active_session_token(session)
    }

    fn optional_authorization_token(
        &self,
        command: &Value,
    ) -> Result<Option<String>, ProductError> {
        if let Some(token) = access_token(command) {
            return Ok(Some(token.to_string()));
        }
        if let Some(token) = self.environment_test_account_token()? {
            return Ok(Some(token));
        }
        self.load_session()?
            .map(|session| self.active_session_token(session))
            .transpose()
    }

    fn get_json(
        &self,
        path: &str,
        query: &[(&str, &str)],
        token: Option<&str>,
    ) -> Result<Value, ProductError> {
        let mut url = url::Url::parse(&format!("{}{}", self.api_base_url, path))
            .map_err(|error| ProductError::Configuration(error.to_string()))?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        let response = send_with_ipv4_fallback(|client| {
            let mut request = client.get(url.clone()).header("Accept", "application/json");
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send()
        })?;
        decode_response(Ok(response))
    }

    fn post_json(
        &self,
        path: &str,
        body: Value,
        token: Option<&str>,
    ) -> Result<Value, ProductError> {
        let url = format!("{}{}", self.api_base_url, path);
        let response = send_with_ipv4_fallback(|client| {
            let mut request = client
                .post(&url)
                .header("Accept", "application/json")
                .json(&body);
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send()
        })?;
        decode_response(Ok(response))
    }

    fn required_session(&self) -> Result<Value, ProductError> {
        self.load_session()?.ok_or(ProductError::NotLoggedIn)
    }

    fn active_session_token(&self, session: Value) -> Result<String, ProductError> {
        let token = access_token(&session)
            .map(str::to_string)
            .ok_or(ProductError::NotLoggedIn)?;
        if !session_needs_refresh(&session, &token) {
            return Ok(token);
        }

        let refresh_token =
            optional_string(&session, "refreshToken").ok_or(ProductError::SessionExpired)?;
        let mut body = json!({"refreshToken": refresh_token});
        if let Some(device_id) = optional_string(&session, "deviceId") {
            body["deviceId"] = Value::String(device_id.to_string());
        }
        let response = match self.post_json("/api/auth/refresh", body, None) {
            Ok(response) => response,
            Err(error) if terminal_session_error(&error) => {
                self.remove_session()?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let refreshed_token = access_token(&response).map(str::to_string).ok_or_else(|| {
            ProductError::Response("refresh response did not include an access token".to_string())
        })?;
        let updated = merge_refreshed_session(session, response, &refreshed_token);
        self.save_session(&updated)?;
        Ok(refreshed_token)
    }

    fn load_session(&self) -> Result<Option<Value>, ProductError> {
        if let Some(path) =
            env::var_os(FABUSHI_CI_ACCOUNT_SESSION_FILE_ENV).filter(|value| !value.is_empty())
        {
            if env::var(GITHUB_ACTIONS_ENV).ok().as_deref() != Some("true") {
                return Err(ProductError::Session(
                    "CI account session files are accepted only inside GitHub Actions".into(),
                ));
            }
            return load_ci_account_session_file(Path::new(&path), now_seconds()).map(Some);
        }
        let name = account_session_secret_name()?;
        let stored = match self.secrets_manager.get(&SecretScope::Global, &name) {
            Ok(stored) => stored,
            Err(error) if unreadable_auth_store_error(&error) => {
                // A mobile app update can preserve the encrypted auth file while
                // the OS keyring entry is rotated or becomes unavailable. The
                // old session is unrecoverable in that state, so quarantine only
                // the auth namespace and continue as signed out. Managed/requested
                // secrets use an independent namespace and are left untouched.
                self.secrets_manager
                    .quarantine_unreadable_store()
                    .map_err(secrets_error)?;
                None
            }
            Err(error) => return Err(secrets_error(error)),
        };
        if let Some(raw) = stored {
            return serde_json::from_str(&raw)
                .map(Some)
                .map_err(|error| ProductError::Session(error.to_string()));
        }
        Ok(None)
    }

    fn browser_login_poll_secret_path(&self, attempt_id: &str) -> Result<PathBuf, ProductError> {
        let attempt_id = safe_path_identifier(attempt_id, "attemptId")?;
        Ok(self
            .browser_login_poll_dir
            .join(format!("{attempt_id}.poll-secret")))
    }

    fn save_browser_login_poll_secret(
        &self,
        attempt_id: &str,
        poll_secret: &str,
    ) -> Result<(), ProductError> {
        if poll_secret.len() < 32 || poll_secret.len() > 256 {
            return Err(ProductError::Response(
                "browser login poll secret is invalid".into(),
            ));
        }
        let path = self.browser_login_poll_secret_path(attempt_id)?;
        // This verifier is short-lived attempt state, not an account credential.
        // Keeping it in a mode-0600 app-data file preserves secure browser-login
        // resume without asking macOS Keychain to authorize a frequently changing
        // helper binary. It never crosses the renderer or appears in a URL.
        write_private_file(&path, poll_secret)
    }

    fn load_browser_login_poll_secret(
        &self,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductError> {
        let path = self.browser_login_poll_secret_path(attempt_id)?;
        match std::fs::read_to_string(&path) {
            Ok(value) => {
                let value = value.trim();
                if value.len() < 32 || value.len() > 256 {
                    return Err(ProductError::Session(
                        "stored browser login poll secret is invalid".into(),
                    ));
                }
                Ok(Some(value.to_string()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ProductError::Session(format!(
                "read browser login poll state: {error}"
            ))),
        }
    }

    fn remove_browser_login_poll_secret(&self, attempt_id: &str) -> Result<(), ProductError> {
        let path = self.browser_login_poll_secret_path(attempt_id)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ProductError::Session(format!(
                "remove browser login poll state: {error}"
            ))),
        }
    }

    fn save_session(&self, session: &Value) -> Result<(), ProductError> {
        let name = account_session_secret_name()?;
        let contents = serde_json::to_string(session)
            .map_err(|error| ProductError::Session(error.to_string()))?;
        self.secrets_manager
            .set(&SecretScope::Global, &name, &contents)
            .map_err(secrets_error)
    }

    fn remove_session(&self) -> Result<(), ProductError> {
        let name = account_session_secret_name()?;
        let encrypted_result = self
            .secrets_manager
            .delete(&SecretScope::Global, &name)
            .map(|_| ())
            .map_err(secrets_error);
        let marker_result = match std::fs::remove_file(self.test_account_marker_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ProductError::Session(error.to_string())),
        };
        encrypted_result.and(marker_result)
    }

    fn test_account_marker_path(&self) -> PathBuf {
        self.session_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(MAHAYANA_TEST_ACCOUNT_MARKER)
    }

    fn environment_test_account_token(&self) -> Result<Option<String>, ProductError> {
        let token = match env::var(MAHAYANA_TEST_ACCOUNT_TOKEN_ENV) {
            Ok(token) if !token.trim().is_empty() => token,
            _ => return Ok(None),
        };
        let token = safe_test_account_token(&token)?;
        let marker = match std::fs::read_to_string(self.test_account_marker_path()) {
            Ok(marker) => marker,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ProductError::Session(error.to_string())),
        };
        let digest = format!("{:x}", sha2::Sha256::digest(token.as_bytes()));
        if marker.trim() != digest {
            return Err(ProductError::Session(
                "test account environment token does not match the completed CLI login".into(),
            ));
        }
        Ok(Some(token.to_string()))
    }
}

fn verify_marketplace_deployment_once(
    client: &reqwest::blocking::Client,
    manifest_url: &str,
    package_url: &str,
    release_manifest_url: &str,
    plugin_id: &str,
    version: &str,
    package_sha256: &str,
    package_size: usize,
    source: &Value,
    release_manifest: &Value,
) -> Result<(), String> {
    let manifest_response = client
        .get(manifest_url)
        .header("Accept", "application/json")
        .send()
        .map_err(|error| format!("failed to fetch plugin manifest: {error}"))?;
    if !manifest_response.status().is_success() {
        return Err(format!(
            "plugin manifest returned HTTP {}",
            manifest_response.status()
        ));
    }
    if manifest_response
        .content_length()
        .is_some_and(|length| length > 64 * 1024)
    {
        return Err("plugin manifest exceeds 64 KiB".into());
    }
    let mut manifest_bytes = Vec::with_capacity(
        manifest_response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(64 * 1024),
    );
    manifest_response
        .take(64 * 1024 + 1)
        .read_to_end(&mut manifest_bytes)
        .map_err(|error| format!("failed to read plugin manifest: {error}"))?;
    if manifest_bytes.len() > 64 * 1024 {
        return Err("plugin manifest exceeds 64 KiB".into());
    }
    let manifest = serde_json::from_slice::<Value>(&manifest_bytes)
        .map_err(|error| format!("plugin manifest is invalid JSON: {error}"))?;
    validate_marketplace_site_manifest(
        &manifest,
        plugin_id,
        version,
        package_sha256,
        package_size,
        source,
        release_manifest,
    )?;

    let release_response = client
        .get(release_manifest_url)
        .header("Accept", "application/json")
        .send()
        .map_err(|error| format!("failed to fetch release manifest: {error}"))?;
    if !release_response.status().is_success() {
        return Err(format!(
            "release manifest returned HTTP {}",
            release_response.status()
        ));
    }
    let release_bytes = release_response
        .bytes()
        .map_err(|error| format!("failed to read release manifest: {error}"))?;
    if release_bytes.len() > 256 * 1024 {
        return Err("release manifest exceeds 256 KiB".into());
    }
    let expected_release_bytes =
        canonical_json_bytes(release_manifest).map_err(|error| error.to_string())?;
    if release_bytes.as_ref() != expected_release_bytes.as_slice() {
        return Err("deployed release manifest differs from the source-bound manifest".into());
    }

    let package_response = client
        .get(package_url)
        .send()
        .map_err(|error| format!("failed to fetch plugin package: {error}"))?;
    if !package_response.status().is_success() {
        return Err(format!(
            "plugin package returned HTTP {}",
            package_response.status()
        ));
    }
    if package_response
        .content_length()
        .is_some_and(|length| length != package_size as u64)
    {
        return Err("plugin package size does not match release metadata".into());
    }
    let mut package = Vec::with_capacity(package_size);
    package_response
        .take(package_size as u64 + 1)
        .read_to_end(&mut package)
        .map_err(|error| format!("failed to read plugin package: {error}"))?;
    if package.len() != package_size {
        return Err("plugin package size does not match release metadata".into());
    }
    let actual_sha256 = format!("{:x}", sha2::Sha256::digest(&package));
    if !actual_sha256.eq_ignore_ascii_case(package_sha256) {
        return Err("plugin package SHA-256 does not match release metadata".into());
    }
    Ok(())
}

fn validate_marketplace_site_manifest(
    manifest: &Value,
    plugin_id: &str,
    version: &str,
    package_sha256: &str,
    package_size: usize,
    source: &Value,
    release_manifest: &Value,
) -> Result<(), String> {
    let release_manifest_sha256 =
        canonical_json_sha256(release_manifest).map_err(|error| error.to_string())?;
    let matches = manifest.get("schemaVersion").and_then(Value::as_u64) == Some(2)
        && manifest.get("pluginId").and_then(Value::as_str) == Some(plugin_id)
        && manifest.get("version").and_then(Value::as_str) == Some(version)
        && manifest.get("packagePath").and_then(Value::as_str) == Some("/mahayana/plugin.tar.gz")
        && manifest
            .get("packageSha256")
            .and_then(Value::as_str)
            .is_some_and(|digest| digest.eq_ignore_ascii_case(package_sha256))
        && manifest.get("packageSize").and_then(Value::as_u64) == Some(package_size as u64)
        && manifest.get("runtime").and_then(Value::as_str) == Some("independent-worker-or-pages")
        && manifest.get("source") == Some(source)
        && manifest.get("releaseManifestPath").and_then(Value::as_str)
            == Some("/mahayana/release-manifest.json")
        && manifest
            .get("releaseManifestSha256")
            .and_then(Value::as_str)
            .is_some_and(|digest| digest.eq_ignore_ascii_case(&release_manifest_sha256));
    matches
        .then_some(())
        .ok_or_else(|| "plugin manifest does not match release metadata".to_string())
}

fn https_deployment_url(value: &str) -> Result<String, ProductError> {
    let mut url = url::Url::parse(value.trim())
        .map_err(|_| ProductError::InvalidParameter("deploymentUrl"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProductError::InvalidParameter("deploymentUrl"));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !(host.ends_with(".workers.dev") || host.ends_with(".pages.dev")) {
        return Err(ProductError::InvalidParameter("deploymentUrl"));
    }
    let normalized = url.path().trim_end_matches('/').to_string();
    url.set_path(&normalized);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn safe_marketplace_version(value: &str) -> Result<&str, ProductError> {
    let value = non_empty(value, "version")?;
    ((1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+')))
    .then_some(value)
    .ok_or(ProductError::InvalidParameter("version"))
}

fn safe_sha256(value: &str) -> Result<&str, ProductError> {
    let value = value.trim();
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(ProductError::InvalidParameter("packageSha256"))
    }
}

fn safe_marketplace_platform(value: &str) -> Result<&str, ProductError> {
    match value.trim() {
        "cli" => Ok("cli"),
        "desktop" => Ok("desktop"),
        "mobile" => Ok("mobile"),
        "web" => Ok("web"),
        "ios" => Ok("ios"),
        "android" => Ok("android"),
        _ => Err(ProductError::InvalidParameter("platform")),
    }
}

fn safe_marketplace_platforms(platforms: &[String]) -> Result<Vec<&str>, ProductError> {
    let mut normalized = Vec::new();
    for platform in platforms {
        let platform = safe_marketplace_platform(platform)?;
        if !normalized.contains(&platform) {
            normalized.push(platform);
        }
    }
    if normalized.is_empty() {
        return Err(ProductError::InvalidParameter("platforms"));
    }
    Ok(normalized)
}

fn write_private_file(path: &Path, contents: &str) -> Result<(), ProductError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ProductError::Session(error.to_string()))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| ProductError::Session(error.to_string()))?;
    std::io::Write::write_all(&mut file, contents.as_bytes())
        .map_err(|error| ProductError::Session(error.to_string()))
}

fn safe_test_account_token(value: &str) -> Result<&str, ProductError> {
    let value = value.trim();
    if (32..=512).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_graphic()) {
        Ok(value)
    } else {
        Err(ProductError::InvalidParameter("testAccountToken"))
    }
}

fn account_session_secret_name() -> Result<SecretName, ProductError> {
    SecretName::new(MAHAYANA_ACCOUNT_SESSION_SECRET).map_err(secrets_error)
}

fn managed_secret_name(secret_request_id: &str) -> Result<SecretName, ProductError> {
    let id = secret_request_id.trim();
    if id.is_empty() {
        return Err(ProductError::InvalidParameter("secretRequestId"));
    }
    let digest = sha2::Sha256::digest(id.as_bytes());
    SecretName::new(&format!("MAHAYANA_REQUESTED_SECRET_{digest:X}")).map_err(secrets_error)
}

fn ci_session_identifier(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) if !value.trim().is_empty() && value.len() <= 200 => {
            Some(value.trim().to_string())
        }
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn validate_ci_account_session(value: Value, now: i64) -> Result<Value, ProductError> {
    let token = access_token(&value)
        .ok_or_else(|| ProductError::Session("CI account session is missing accessToken".into()))?;
    let token_is_safe = (32..=16 * 1024).contains(&token.len())
        && !token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control());
    let session_id = optional_string(&value, "sessionId").unwrap_or_default();
    let device_id = optional_string(&value, "deviceId").unwrap_or_default();
    let username = optional_string(&value, "username").unwrap_or_default();
    let user_id = ci_session_identifier(value.get("userId"));
    let nested_user_id = ci_session_identifier(value.get("user").and_then(|user| user.get("id")));
    let expires_at = explicit_expiration_seconds(&value).unwrap_or_default();
    let valid = value.get("provider").and_then(Value::as_str) == Some("github-actions")
        && value.get("ciRunner").and_then(Value::as_bool) == Some(true)
        && value.get("tokenType").and_then(Value::as_str) == Some("Bearer")
        && value.get("refreshToken").is_none()
        && token_is_safe
        && session_id.starts_with("ci-runner:")
        && device_id.starts_with("gha-")
        && (device_id.ends_with("-interactive") || device_id.ends_with("-macos-app"))
        && !username.is_empty()
        && username.chars().count() <= 320
        && user_id.is_some()
        && user_id == nested_user_id
        && expires_at > now.saturating_add(30)
        && expires_at <= now.saturating_add(5 * 60 * 60);
    if !valid {
        return Err(ProductError::Session(
            "CI account session failed its provenance, identity, or lifetime contract".into(),
        ));
    }
    Ok(value)
}

fn load_ci_account_session_file(path: &Path, now: i64) -> Result<Value, ProductError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        ProductError::Session(format!("read CI account session metadata: {error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ProductError::Session(
            "CI account session path must be a regular non-symlink file".into(),
        ));
    }
    if metadata.len() == 0 || metadata.len() > CI_ACCOUNT_SESSION_MAX_BYTES {
        return Err(ProductError::Session(
            "CI account session file has an invalid size".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ProductError::Session(
                "CI account session file must not be readable by group or other users".into(),
            ));
        }
    }
    let bytes = std::fs::read(path)
        .map_err(|error| ProductError::Session(format!("read CI account session: {error}")))?;
    if bytes.len() as u64 > CI_ACCOUNT_SESSION_MAX_BYTES {
        return Err(ProductError::Session(
            "CI account session file exceeds the size limit".into(),
        ));
    }
    let value = serde_json::from_slice::<Value>(&bytes)
        .map_err(|error| ProductError::Session(format!("parse CI account session: {error}")))?;
    validate_ci_account_session(value, now)
}

fn merge_refreshed_session(session: Value, response: Value, access_token: &str) -> Value {
    let mut updated = session.as_object().cloned().unwrap_or_default();
    if let Some(response) = response.as_object() {
        for (key, value) in response {
            if !value.is_null() {
                updated.insert(key.clone(), value.clone());
            }
        }
    }
    updated.insert(
        "accessToken".to_string(),
        Value::String(access_token.to_string()),
    );
    if !updated.contains_key("accessTokenExpiresAt")
        && let Some(expires_at) = jwt_expiration_seconds(access_token)
    {
        updated.insert("accessTokenExpiresAt".to_string(), expires_at.into());
    }
    Value::Object(updated)
}

fn session_needs_refresh(session: &Value, token: &str) -> bool {
    let expires_at = explicit_expiration_seconds(session).or_else(|| jwt_expiration_seconds(token));
    expires_at.is_some_and(|expires_at| {
        expires_at <= now_seconds().saturating_add(ACCESS_TOKEN_REFRESH_SKEW_SECONDS)
    })
}

fn explicit_expiration_seconds(session: &Value) -> Option<i64> {
    let value = session.get("accessTokenExpiresAt")?;
    let raw = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))?;
    // Accept the millisecond timestamp emitted by older application shells.
    Some(if raw > 10_000_000_000 {
        raw / 1_000
    } else {
        raw
    })
}

fn jwt_expiration_seconds(token: &str) -> Option<i64> {
    parse_jwt_expiration(token)
        .ok()
        .flatten()
        .map(|expires_at| expires_at.timestamp())
}

fn surface_now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn validate_surface_draft(draft: &MessageDraft) -> Result<(), ProductError> {
    match draft {
        MessageDraft::Email {
            to, subject, body, ..
        } => {
            if to.is_empty()
                || to
                    .iter()
                    .any(|recipient| !recipient.contains('@') || recipient.trim().is_empty())
            {
                return Err(ProductError::State(
                    "email draft requires valid recipients".into(),
                ));
            }
            if subject.trim().is_empty() || body.trim().is_empty() {
                return Err(ProductError::State(
                    "email draft requires a subject and body".into(),
                ));
            }
        }
        MessageDraft::Slack { target, body, .. } => {
            if target.trim().is_empty() || body.trim().is_empty() {
                return Err(ProductError::State(
                    "Slack draft requires a target and body".into(),
                ));
            }
        }
    }
    Ok(())
}

fn unreadable_auth_store_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string();
        message.contains("failed to decrypt secrets file")
            || message.contains("failed to decode secrets file at")
    })
}

fn secrets_error(error: anyhow::Error) -> ProductError {
    ProductError::Session(error.to_string())
}

fn typed_session(
    response: Value,
    provider: &str,
    require_token: bool,
) -> Result<Value, ProductError> {
    let mut output = response.as_object().cloned().unwrap_or_default();
    let session_stored = access_token(&Value::Object(output.clone())).is_some();
    if require_token && !session_stored {
        return Err(ProductError::Response(
            "login response did not include a session token".to_string(),
        ));
    }
    output.insert(
        "@type".to_string(),
        Value::String("mahayana.auth.session".to_string()),
    );
    output.insert("provider".to_string(), Value::String(provider.to_string()));
    output.insert("sessionStored".to_string(), Value::Bool(session_stored));
    output.insert("loggedIn".to_string(), Value::Bool(session_stored));
    strip_credentials(&mut output);
    Ok(Value::Object(output))
}

fn strip_credentials(output: &mut Map<String, Value>) {
    for key in [
        "token",
        "accessToken",
        "refreshToken",
        "accessTokenExpiresAt",
        "refreshTokenExpiresAt",
        "tokenType",
    ] {
        output.remove(key);
    }
}

fn copy_optional_fields(request: &Value, body: &mut Value, fields: &[&str]) {
    for field in fields {
        if let Some(value) = request
            .get(*field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body[*field] = Value::String(value.to_string());
        }
    }
}

fn http_client() -> Result<reqwest::blocking::Client, ProductError> {
    build_http_client(false)
}

fn ipv4_http_client() -> Result<reqwest::blocking::Client, ProductError> {
    build_http_client(true)
}

fn build_http_client(force_ipv4: bool) -> Result<reqwest::blocking::Client, ProductError> {
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30));
    if force_ipv4 {
        builder = builder.local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
    builder
        .build()
        .map_err(|error| ProductError::Configuration(error.to_string()))
}

fn send_with_ipv4_fallback<F>(send: F) -> Result<reqwest::blocking::Response, ProductError>
where
    F: Fn(&reqwest::blocking::Client) -> Result<reqwest::blocking::Response, reqwest::Error>,
{
    let client = http_client()?;
    match send(&client) {
        Ok(response) => Ok(response),
        Err(error) if error.is_connect() => {
            let ipv4_client = ipv4_http_client()?;
            send(&ipv4_client).map_err(|fallback_error| {
                ProductError::Transport(format!(
                    "{fallback_error}; IPv4 fallback after initial connection error: {error}"
                ))
            })
        }
        Err(error) => Err(ProductError::Transport(error.to_string())),
    }
}

fn decode_response(
    response: Result<reqwest::blocking::Response, reqwest::Error>,
) -> Result<Value, ProductError> {
    let response = response.map_err(|error| ProductError::Transport(error.to_string()))?;
    let status = response.status();
    if status.is_success() {
        return response
            .json::<Value>()
            .map_err(|error| ProductError::Response(error.to_string()));
    }
    let body = response
        .text()
        .unwrap_or_else(|_| "request failed".to_string());
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| {
            let code = value.get("error").and_then(Value::as_str);
            let detail = value.get("message").and_then(Value::as_str);
            match (code, detail) {
                (Some(code), Some(detail)) if code != detail => Some(format!("{code}: {detail}")),
                (Some(code), _) => Some(code.to_string()),
                (_, Some(detail)) => Some(detail.to_string()),
                _ => None,
            }
        })
        .unwrap_or(body);
    Err(ProductError::HttpStatus {
        status: status.as_u16(),
        message,
    })
}

fn required_string<'a>(request: &'a Value, name: &'static str) -> Result<&'a str, ProductError> {
    request
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ProductError::InvalidParameter(name))
}

fn non_empty<'a>(value: &'a str, name: &'static str) -> Result<&'a str, ProductError> {
    let value = value.trim();
    (!value.is_empty())
        .then_some(value)
        .ok_or(ProductError::InvalidParameter(name))
}

fn safe_path_identifier<'a>(value: &'a str, name: &'static str) -> Result<&'a str, ProductError> {
    let value = non_empty(value, name)?;
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        .then_some(value)
        .ok_or(ProductError::InvalidParameter(name))
}

fn safe_platform_path(value: &str) -> Result<&str, ProductError> {
    let value = non_empty(value, "path")?;
    let allowed_prefix = value.starts_with("/api/") || value.starts_with("/v1/");
    let safe = allowed_prefix
        && !value.contains(['\r', '\n', '\\'])
        && !value.split('/').any(|segment| segment == "..")
        && !value.starts_with("//")
        && !value.contains('?')
        && !value.contains('#');
    safe.then_some(value)
        .ok_or(ProductError::InvalidParameter("path"))
}

fn decode_value<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ProductError> {
    serde_json::from_value(value).map_err(|error| ProductError::Response(error.to_string()))
}

fn optional_string<'a>(request: &'a Value, name: &str) -> Option<&'a str> {
    request
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn access_token(value: &Value) -> Option<&str> {
    optional_string(value, "accessToken")
}

fn normalize_alipay_cli_response(mut response: Value) -> Value {
    // Older account workers returned the one-time CLI credential as `token`,
    // while shared sessions deliberately accept only `accessToken`. Normalize
    // only this trusted completed response instead of weakening the general
    // authorization parser.
    if response.get("status").and_then(Value::as_str) == Some("complete")
        && response.get("accessToken").is_none()
        && let Some(token) = optional_string(&response, "token").map(str::to_string)
    {
        response["accessToken"] = Value::String(token);
        if let Some(object) = response.as_object_mut() {
            object.remove("token");
        }
    }
    response
}

fn required_identifier(request: &Value, name: &'static str) -> Result<String, ProductError> {
    match request.get(name) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        _ => Err(ProductError::InvalidParameter(name)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductError {
    InvalidParameter(&'static str),
    UnsupportedRequest(String),
    Configuration(String),
    NotLoggedIn,
    SessionExpired,
    Session(String),
    State(String),
    Transport(String),
    HttpStatus { status: u16, message: String },
    Response(String),
}

impl std::fmt::Display for ProductError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParameter(name) => {
                write!(formatter, "request parameter {name} is invalid")
            }
            Self::UnsupportedRequest(name) => {
                write!(formatter, "unsupported product request: {name}")
            }
            Self::Configuration(error) => {
                write!(formatter, "product API configuration failed: {error}")
            }
            Self::NotLoggedIn => write!(
                formatter,
                "请先登录大乘软件账号：mahayana login（支持支付宝或官方账号）"
            ),
            Self::SessionExpired => write!(
                formatter,
                "大乘登录已过期且没有可轮换的 refresh token，请重新运行 mahayana login"
            ),
            Self::Session(error) => write!(formatter, "Mahayana account session failed: {error}"),
            Self::State(error) => write!(formatter, "Mahayana product state failed: {error}"),
            Self::Transport(error) => {
                write!(formatter, "Mahayana product API transport failed: {error}")
            }
            Self::HttpStatus { status, message } => write!(
                formatter,
                "Mahayana product API returned HTTP {status}: {message}"
            ),
            Self::Response(error) => {
                write!(formatter, "Mahayana product API response failed: {error}")
            }
        }
    }
}

impl std::error::Error for ProductError {}

fn terminal_session_error(error: &ProductError) -> bool {
    matches!(
        error,
        ProductError::NotLoggedIn
            | ProductError::SessionExpired
            | ProductError::HttpStatus { status: 401, .. }
    )
}

pub fn redact_secrets(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut redacted = Map::new();
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "token" | "apiKey" | "accessToken" | "refreshToken" | "productSessionToken"
                ) {
                    redacted.insert(
                        key.clone(),
                        Value::String("[stored by Mahayana]".to_string()),
                    );
                } else {
                    redacted.insert(key.clone(), redact_secrets(value));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(values) => Value::Array(values.iter().map(redact_secrets).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_removes_nested_account_tokens() {
        let value = json!({
            "token": "secret",
            "nested": {"accessToken": "also-secret", "name": "kept"},
        });
        let output = redact_secrets(&value);
        assert_eq!(output["token"], "[stored by Mahayana]");
        assert_eq!(output["nested"]["accessToken"], "[stored by Mahayana]");
        assert_eq!(output["nested"]["name"], "kept");
    }

    #[test]
    fn session_refresh_uses_explicit_expiry_and_preserves_rotating_credentials() {
        let future = now_seconds() + 3_600;
        let current = json!({
            "accessToken": "old-access",
            "refreshToken": "old-refresh",
            "accessTokenExpiresAt": future,
            "provider": "password",
            "user": {"id": "user-1"}
        });
        assert!(!session_needs_refresh(&current, "old-access"));

        let expired = json!({
            "accessToken": "old-access",
            "refreshToken": "old-refresh",
            "accessTokenExpiresAt": (now_seconds() - 1) * 1_000,
        });
        assert!(session_needs_refresh(&expired, "old-access"));

        let refreshed = merge_refreshed_session(
            current,
            json!({
                "accessToken": "new-access",
                "refreshToken": "new-refresh",
                "accessTokenExpiresAt": future + 3_600,
            }),
            "new-access",
        );
        assert!(refreshed.get("token").is_none());
        assert_eq!(refreshed["accessToken"], "new-access");
        assert_eq!(refreshed["refreshToken"], "new-refresh");
        assert_eq!(refreshed["provider"], "password");
        assert_eq!(refreshed["user"]["id"], "user-1");
    }

    #[test]
    fn path_identifiers_allow_product_ids_but_reject_traversal() {
        assert_eq!(
            safe_path_identifier("sandbox.test-1", "miniAppId").as_deref(),
            Ok("sandbox.test-1")
        );
        assert_eq!(
            safe_path_identifier("../admin", "miniAppId"),
            Err(ProductError::InvalidParameter("miniAppId"))
        );
        assert_eq!(
            safe_path_identifier("app/submit", "miniAppId"),
            Err(ProductError::InvalidParameter("miniAppId"))
        );
        assert_eq!(
            safe_platform_path("/api/social/friends"),
            Ok("/api/social/friends")
        );
        assert_eq!(
            safe_platform_path("https://evil.example/api/social/friends"),
            Err(ProductError::InvalidParameter("path"))
        );
        assert_eq!(
            safe_platform_path("/api/../admin"),
            Err(ProductError::InvalidParameter("path"))
        );
    }

    #[test]
    fn marketplace_browse_is_public_and_omits_authorization() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind marketplace test server");
        let address = listener.local_addr().expect("marketplace test address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept marketplace request");
            let mut request = [0_u8; 4096];
            let size = stream.read(&mut request).expect("read marketplace request");
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("GET /v1/marketplace/plugins?q=global&platform=desktop "));
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
            let body = r#"{"plugins":[{"pluginId":"global-dharma","displayName":"全球法布施"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write marketplace response");
        });

        let unique = format!(
            "mahayana-public-marketplace-test-{}-{}",
            std::process::id(),
            surface_now_millis()
        );
        let root = std::env::temp_dir().join(unique);
        let client = MahayanaProductClient::new_with_surface_state_path(
            format!("http://{address}"),
            root.join("session.json"),
            root.join("product-surface.json"),
        );
        let catalog = client
            .marketplace_browse(Some("global"), Some("desktop"))
            .expect("public marketplace must not require account authorization");
        assert_eq!(catalog["plugins"][0]["pluginId"], "global-dharma");
        server.join().expect("join marketplace test server");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_session_errors_are_classified_for_local_eviction() {
        assert!(terminal_session_error(&ProductError::NotLoggedIn));
        assert!(terminal_session_error(&ProductError::SessionExpired));
        assert!(terminal_session_error(&ProductError::HttpStatus {
            status: 401,
            message: "refresh_token_reused".into(),
        }));
        assert!(!terminal_session_error(&ProductError::HttpStatus {
            status: 503,
            message: "upstream unavailable".into(),
        }));
        assert!(!terminal_session_error(&ProductError::Transport(
            "timeout".into()
        )));
    }

    #[test]
    fn marketplace_site_manifest_must_match_release_metadata() {
        let digest = "a".repeat(64);
        let manifest = json!({
            "schemaVersion": 2,
            "pluginId": "cloud-market-hello",
            "version": "1.0.0",
            "packagePath": "/mahayana/plugin.tar.gz",
            "packageSha256": digest,
            "packageSize": 4706,
            "runtime": "independent-worker-or-pages",
            "source": {"provider":"github","repository":"bhrumom/fabushi"},
            "releaseManifestPath": "/mahayana/release-manifest.json",
            "releaseManifestSha256": canonical_json_sha256(&json!({
                "protocol": "mahayana.multi-artifact-release.v1"
            })).unwrap(),
        });
        let source = json!({"provider":"github","repository":"bhrumom/fabushi"});
        let release_manifest = json!({"protocol":"mahayana.multi-artifact-release.v1"});
        assert_eq!(
            validate_marketplace_site_manifest(
                &manifest,
                "cloud-market-hello",
                "1.0.0",
                &"a".repeat(64),
                4706,
                &source,
                &release_manifest,
            ),
            Ok(())
        );
        let mut wrong_runtime = manifest;
        wrong_runtime["runtime"] = Value::String("r2-package".into());
        assert!(
            validate_marketplace_site_manifest(
                &wrong_runtime,
                "cloud-market-hello",
                "1.0.0",
                &"a".repeat(64),
                4706,
                &source,
                &release_manifest,
            )
            .is_err()
        );
    }

    #[test]
    fn marketplace_deployment_metadata_requires_public_https_and_sha256() {
        assert_eq!(
            https_deployment_url("https://plugin-example.pages.dev/"),
            Ok("https://plugin-example.pages.dev".to_string())
        );
        assert_eq!(
            https_deployment_url("https://plugin.example"),
            Err(ProductError::InvalidParameter("deploymentUrl"))
        );
        assert_eq!(
            https_deployment_url("http://localhost:8787"),
            Err(ProductError::InvalidParameter("deploymentUrl"))
        );
        assert!(safe_sha256(&"a".repeat(64)).is_ok());
        assert_eq!(
            safe_sha256("not-a-digest"),
            Err(ProductError::InvalidParameter("packageSha256"))
        );
        assert_eq!(safe_marketplace_platform("desktop"), Ok("desktop"));
        assert_eq!(safe_marketplace_platform("ios"), Ok("ios"));
        assert_eq!(safe_marketplace_platform("android"), Ok("android"));
        assert_eq!(
            safe_marketplace_platforms(&[
                "desktop".into(),
                "ios".into(),
                "android".into(),
                "desktop".into(),
                "cli".into(),
            ]),
            Ok(vec!["desktop", "ios", "android", "cli"])
        );
    }

    #[test]
    fn ci_account_session_requires_short_lived_github_actions_provenance() {
        let now = now_seconds();
        let session = json!({
            "accessToken": "a".repeat(64),
            "tokenType": "Bearer",
            "accessTokenExpiresAt": now + 4 * 60 * 60,
            "sessionId": "ci-runner:12345:1",
            "deviceId": "gha-12345-1-interactive",
            "username": "linked-github-user",
            "userId": "42",
            "user": {"id": "42", "username": "linked-github-user"},
            "provider": "github-actions",
            "ciRunner": true,
        });
        assert_eq!(
            validate_ci_account_session(session.clone(), now),
            Ok(session.clone())
        );
        let mut macos_app = session.clone();
macos_app["deviceId"] = Value::String("gha-12345-1-macos-app".into());
assert_eq!(
    validate_ci_account_session(macos_app.clone(), now),
    Ok(macos_app)
);
        let mut with_refresh = session.clone();
        with_refresh["refreshToken"] = Value::String("forbidden".into());
        assert!(validate_ci_account_session(with_refresh, now).is_err());
        let mut wrong_device = session.clone();
        wrong_device["deviceId"] = Value::String("desktop-user-device".into());
        assert!(validate_ci_account_session(wrong_device, now).is_err());
        let mut wrong_user = session.clone();
        wrong_user["user"]["id"] = Value::String("43".into());
        assert!(validate_ci_account_session(wrong_user, now).is_err());
        let mut too_long = session;
        too_long["accessTokenExpiresAt"] = Value::Number((now + 6 * 60 * 60).into());
        assert!(validate_ci_account_session(too_long, now).is_err());
    }

    #[test]
    fn test_account_tokens_must_be_secret_strength_and_never_contain_whitespace() {
        assert!(safe_test_account_token(&"a".repeat(64)).is_ok());
        assert_eq!(
            safe_test_account_token("short"),
            Err(ProductError::InvalidParameter("testAccountToken"))
        );
        assert_eq!(
            safe_test_account_token(&format!("{} token", "a".repeat(32))),
            Err(ProductError::InvalidParameter("testAccountToken"))
        );
    }

    #[test]
    fn ui_sessions_never_expose_account_credentials_or_legacy_token_aliases() {
        assert!(access_token(&json!({"token": "legacy"})).is_none());
        let session = typed_session(
            json!({
                "accessToken": "access",
                "refreshToken": "refresh",
                "accessTokenExpiresAt": 123,
                "user": {"username": "tester"},
            }),
            "password",
            true,
        )
        .unwrap();
        assert_eq!(session["sessionStored"], true);
        assert_eq!(session["loggedIn"], true);
        assert_eq!(session["user"]["username"], "tester");
        assert!(session.get("token").is_none());
        assert!(session.get("accessToken").is_none());
        assert!(session.get("refreshToken").is_none());
    }

    #[test]
    fn desktop_auth_storage_uses_prompt_free_namespace() {
        assert_eq!(
            product_auth_secrets_namespace(Some("fabushi-desktop-v2")),
            LocalSecretsNamespace::FabushiDesktopAuth
        );
        assert_eq!(
            product_auth_secrets_namespace(None),
            LocalSecretsNamespace::MahayanaAuth
        );
        assert_eq!(
            product_managed_secrets_namespace(Some("fabushi-desktop-v2")),
            LocalSecretsNamespace::FabushiDesktopManagedSecrets
        );
        assert_eq!(
            product_managed_secrets_namespace(None),
            LocalSecretsNamespace::ManagedSecrets
        );
    }

    #[test]
    fn browser_login_poll_secrets_use_private_files_without_keyring() {
        let unique = format!(
            "mahayana-browser-poll-test-{}-{}",
            std::process::id(),
            surface_now_millis()
        );
        let root = std::env::temp_dir().join(unique);
        let client = MahayanaProductClient::new_with_surface_state_path(
            "https://example.invalid",
            root.join("session.json"),
            root.join("product-surface.json"),
        );
        let attempt_id = "attempt-private-poll-1234";
        let poll_secret = "p".repeat(64);

        client
            .save_browser_login_poll_secret(attempt_id, &poll_secret)
            .expect("save poll secret");
        let path = client
            .browser_login_poll_secret_path(attempt_id)
            .expect("poll path");
        assert!(path.starts_with(root.join("browser-login")));
        assert!(!path.starts_with(root.join("secrets")));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read poll secret"),
            poll_secret
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path)
                    .expect("poll metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(
            client
                .load_browser_login_poll_secret(attempt_id)
                .expect("load poll secret"),
            Some(poll_secret)
        );
        client
            .remove_browser_login_poll_secret(attempt_id)
            .expect("remove poll secret");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn managed_secret_names_are_stable_and_never_embed_request_ids() {
        let name = managed_secret_name("secret-request-sensitive-value").expect("secret name");
        let rendered = name.as_str();
        assert!(rendered.starts_with("MAHAYANA_REQUESTED_SECRET_"));
        assert!(!rendered.contains("sensitive"));
        assert_eq!(
            managed_secret_name("secret-request-sensitive-value").expect("stable secret name"),
            name
        );
        assert_eq!(
            managed_secret_name("  "),
            Err(ProductError::InvalidParameter("secretRequestId"))
        );
    }

    #[test]
    fn private_skills_are_written_as_real_codex_skill_files() {
        let unique = format!(
            "mahayana-skill-test-{}-{}",
            std::process::id(),
            surface_now_millis()
        );
        let root = std::env::temp_dir().join(unique);
        let skill = SkillSummary {
            id: "skill-real-runtime".into(),
            name: "Real runtime skill".into(),
            description: "Runs against the real Codex skill loader.".into(),
            use_when: "Use when testing skill persistence.".into(),
            instructions: "Perform the requested action and report the result.".into(),
            source: SkillSource::Private,
            publish_state: SkillPublishState::Local,
            owner_agent_id: None,
            team_id: None,
            team_name: None,
            read_only: None,
            updated_at_ms: surface_now_millis(),
        };
        let client = MahayanaProductClient::new_with_surface_state_path(
            "https://example.invalid",
            root.join("session.json"),
            root.join("product-surface.json"),
        );
        let client = MahayanaProductClient {
            skills_root: root.join("codex-skills"),
            ..client
        };
        client.persist_private_skill(&skill).expect("persist skill");
        let contents = std::fs::read_to_string(
            client
                .skills_root()
                .join("skill-real-runtime")
                .join("SKILL.md"),
        )
        .expect("read persisted skill");
        assert!(contents.contains("name: \"Real runtime skill\""));
        assert!(contents.contains("Use when testing skill persistence."));
        assert!(contents.contains("Perform the requested action"));
        client
            .remove_private_skill(&skill.id)
            .expect("remove skill");
        assert!(!client.skills_root().join("skill-real-runtime").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_alipay_cli_tokens_are_normalized_without_weakening_auth_parsing() {
        let normalized = normalize_alipay_cli_response(json!({
            "status": "complete",
            "token": "one-time-access",
        }));
        assert_eq!(access_token(&normalized), Some("one-time-access"));
        assert!(normalized.get("token").is_none());

        let pending = normalize_alipay_cli_response(json!({
            "status": "pending",
            "token": "not-yet-valid",
        }));
        assert!(access_token(&pending).is_none());
        assert_eq!(pending["token"], "not-yet-valid");
    }
}
