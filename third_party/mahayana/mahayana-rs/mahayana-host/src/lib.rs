//! Direct Rust host API for the long-lived Mahayana Runtime.
//!
//! Native shells such as Tauri, Swift, and Kotlin should depend on this crate.
//! The C/JSON ABI remains a compatibility adapter for the legacy Flutter host.

use fabushi_official_miniapps::OFFICIAL_PLUGIN_IDS;
use fabushi_official_miniapps::app_definition;
use mahayana_agent::UnavailableAgentBackend;
#[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
use mahayana_agent_codex::CodexAgentBackend;
#[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
use mahayana_agent_codex::CodexAgentConfig;
#[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
use mahayana_conversation::ConversationProvider;
use mahayana_core::ApprovalDecision;
use mahayana_core::ApprovalId;
use mahayana_core::BuildProfile;
use mahayana_core::OperationId;
use mahayana_core::RuntimeCommand;
use mahayana_core::RuntimeConfig;
use mahayana_core::RuntimeEvent;
use mahayana_core::RuntimeResponse;
use mahayana_core::RuntimeStatus;
use mahayana_miniapp::EntitlementChecker;
use mahayana_miniapp::MiniAppConversationProvider;
use mahayana_miniapp::MiniAppDefinition;
use mahayana_platform_core::HostPlatform;
use mahayana_product::MahayanaProductClient;
use mahayana_product::default_mahayana_home;
use mahayana_product::default_product_surface_state_path;
use mahayana_runtime_core::MahayanaRuntime;
use mahayana_runtime_core::RuntimeBuilder;
use mahayana_runtime_core::RuntimeError;
use mahayana_social::MahayanaSocialConversationProvider;
use mahayana_telegram::TelegramConversationProvider;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HostCreateConfig {
    #[serde(flatten)]
    pub runtime: RuntimeConfig,
    pub product_session_path: Option<PathBuf>,
    pub product_surface_state_path: Option<PathBuf>,
    /// Shared automation store used by CLI and native application shells.
    pub automation_path: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    /// Optional Mahayana CLI used only for desktop argv helper dispatch.
    pub codex_executable_path: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    /// Existing embedded Telegram client created by the platform login flow.
    pub telegram_client_id: Option<u64>,
    pub telegram_self_user_id: Option<i64>,
    pub host_platform: Option<HostPlatform>,
    pub mini_apps: Vec<MiniAppDefinition>,
    pub use_codex_account: bool,
    /// Tests and constrained hosts may opt out of inherited local plugins.
    pub inherit_installed_plugins: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HostError {
    message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<RuntimeError> for HostError {
    fn from(error: RuntimeError) -> Self {
        Self::new(error.to_string())
    }
}

/// Long-lived process-local host shared by every presentation surface.
#[derive(Clone)]
pub struct MahayanaHost {
    runtime: Arc<MahayanaRuntime>,
    product_client: MahayanaProductClient,
}

impl MahayanaHost {
    pub fn create(config: HostCreateConfig) -> Result<Self, HostError> {
        let product_client = match (
            config.product_session_path.clone(),
            config.product_surface_state_path.clone(),
        ) {
            (Some(session_path), Some(surface_state_path)) => {
                MahayanaProductClient::new_with_surface_state_path(
                    "https://api.ombhrum.com",
                    session_path,
                    surface_state_path,
                )
            }
            (Some(session_path), None) => {
                MahayanaProductClient::new("https://api.ombhrum.com", session_path)
            }
            (None, Some(surface_state_path)) => MahayanaProductClient::new_with_surface_state_path(
                "https://api.ombhrum.com",
                default_product_session_path(),
                surface_state_path,
            ),
            (None, None) => MahayanaProductClient::default(),
        };
        Ok(Self {
            runtime: Arc::new(build_runtime(config, product_client.clone())?),
            product_client,
        })
    }

    pub fn status(&self) -> RuntimeStatus {
        self.runtime.status()
    }

    pub fn execute(&self, command: RuntimeCommand) -> Result<RuntimeResponse, HostError> {
        self.runtime.execute(command).map_err(HostError::from)
    }

    pub fn receive(&self, timeout: Duration) -> Result<Option<RuntimeEvent>, HostError> {
        self.runtime.receive(timeout).map_err(HostError::from)
    }

    pub fn interrupt(&self, operation_id: OperationId) -> Result<RuntimeResponse, HostError> {
        self.execute(RuntimeCommand::Interrupt { operation_id })
    }

    pub fn resolve_approval(
        &self,
        approval_id: ApprovalId,
        decision: ApprovalDecision,
        payload: serde_json::Value,
    ) -> Result<RuntimeResponse, HostError> {
        self.execute(RuntimeCommand::ResolveApproval {
            approval_id,
            decision,
            payload,
        })
    }

    /// Execute a first-party account, social, or marketplace request while
    /// keeping bearer and refresh credentials inside Rust-owned storage.
    pub fn product_execute(
        &self,
        request_type: &str,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, HostError> {
        self.product_client
            .execute(request_type, request)
            .map_err(|error| HostError::new(error.to_string()))
    }

    /// Revoke and remove the Rust-owned product session without exposing any
    /// bearer or refresh credential to the host UI.
    pub fn clear_session(&self) -> Result<serde_json::Value, HostError> {
        self.product_execute("mahayana.auth.logout", &serde_json::json!({}))
    }
}

/// Canonical Rust-owned account session shared by the Mahayana CLI and native
/// desktop shell. Presentation code receives only UI-safe account fields.
pub fn default_product_surface_path() -> PathBuf {
    default_product_surface_state_path()
}

pub fn default_automation_path() -> PathBuf {
    default_mahayana_home().join("automations.json")
}

pub fn default_product_session_path() -> PathBuf {
    let shared = default_mahayana_home().join("session.json");
    if shared.is_file() {
        return shared;
    }

    // Releases before the native app-group migration stored the account in
    // ~/.mahayana. Keep that signed-in account usable on first launch; the
    // desktop shell copies it into its Rust-owned app-data session and never
    // exposes credentials to React.
    if let Some(home) = std::env::var_os("HOME") {
        let legacy = PathBuf::from(home).join(".mahayana").join("session.json");
        if legacy.is_file() {
            return legacy;
        }
    }
    shared
}

fn build_runtime(
    create: HostCreateConfig,
    product_client: MahayanaProductClient,
) -> Result<MahayanaRuntime, HostError> {
    let runtime_config = create.runtime.clone();
    if runtime_config.remote_agent_enabled {
        return Err(RuntimeError::RemoteAgentForbidden.into());
    }
    #[cfg(all(feature = "mobile-embedded", not(feature = "desktop-full")))]
    let runtime_config = RuntimeConfig {
        build_profile: BuildProfile::MobileEmbedded,
        ..runtime_config
    };
    let host_platform = create
        .host_platform
        .unwrap_or(match runtime_config.build_profile {
            BuildProfile::DesktopFull => HostPlatform::Desktop,
            BuildProfile::MobileEmbedded => HostPlatform::Mobile,
            BuildProfile::WebWasm => HostPlatform::Web,
        });
    // Runtime construction is intentionally network-free. Remote marketplace
    // discovery belongs to an explicit product refresh command, not process
    // startup or every unit test. Official bundled MiniApps are merged below.
    let configured_mini_apps = create.mini_apps.clone();
    let mini_apps = merge_official_mini_apps(configured_mini_apps);
    let session_token = product_client.session_token().ok();
    let mut builder = RuntimeBuilder::new(runtime_config.clone());
    #[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
    let mut codex_conversation_providers: Vec<Arc<dyn ConversationProvider>> = Vec::new();
    if let Some(token) = session_token.as_ref() {
        let provider = Arc::new(MahayanaSocialConversationProvider::new(
            product_client.clone(),
            Some(token.clone()),
        ));
        #[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
        codex_conversation_providers.push(Arc::clone(&provider) as Arc<dyn ConversationProvider>);
        builder = builder.with_provider(provider)?;
    }
    if let Some(telegram_client_id) = create.telegram_client_id {
        let provider = Arc::new(TelegramConversationProvider::from_client_id(
            telegram_client_id,
            create.telegram_self_user_id.unwrap_or_default(),
        ));
        #[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
        codex_conversation_providers.push(Arc::clone(&provider) as Arc<dyn ConversationProvider>);
        builder = builder.with_provider(provider)?;
    }

    #[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
    if matches!(
        runtime_config.build_profile,
        BuildProfile::DesktopFull | BuildProfile::MobileEmbedded
    ) {
        let data_dir = runtime_config.data_dir.clone();
        let cwd = create
            .cwd
            .or_else(|| runtime_config.workspace_roots.first().cloned())
            .or_else(|| data_dir.as_ref().map(|path| path.join("workspace")))
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| HostError::new("current working directory is unavailable"))?;
        let workspace_roots = if runtime_config.workspace_roots.is_empty() {
            vec![cwd.clone()]
        } else {
            runtime_config.workspace_roots.clone()
        };
        let codex_home = create
            .codex_home
            .or_else(|| data_dir.map(|path| path.join("codex")))
            .or_else(default_codex_home_if_available)
            .ok_or_else(|| {
                HostError::new("embedded Mahayana requires an application data directory")
            })?;
        let responses_base_url = create
            .runtime
            .model
            .base_url
            .clone()
            .ok_or_else(|| HostError::new("Dacheng Responses base URL is required"))?;
        let settings = CodexAgentConfig {
            codex_home,
            inherit_installed_plugins: create.inherit_installed_plugins.unwrap_or(
                matches!(runtime_config.build_profile, BuildProfile::DesktopFull) && !cfg!(test),
            ),
            cwd,
            workspace_roots,
            model: runtime_config.model.model.clone(),
            responses_base_url,
            use_codex_account: create.use_codex_account,
            product_session_token: session_token.clone(),
            sandbox_mode: codex_protocol::config_types::SandboxMode::WorkspaceWrite,
            approval_policy: codex_protocol::protocol::AskForApproval::OnRequest,
            codex_executable_path: create.codex_executable_path,
            conversation_providers: codex_conversation_providers,
        };
        return builder
            .build_with_agent_backend_and(
                || async move {
                    let backend = CodexAgentBackend::start(settings).await?;
                    Ok(Arc::new(backend) as Arc<dyn mahayana_agent::AgentBackend>)
                },
                move |builder, backend| {
                    let provider = MiniAppConversationProvider::new_for_platform_with_entitlements(
                        backend,
                        mini_apps,
                        host_platform,
                        Some(Arc::new(PlatformEntitlementChecker {
                            client: product_client,
                        })),
                    )?;
                    builder.with_provider(Arc::new(provider))
                },
            )
            .map_err(HostError::from);
    }

    let unavailable_reason = "this platform build has no embedded Codex backend";
    let backend: Arc<dyn mahayana_agent::AgentBackend> =
        Arc::new(UnavailableAgentBackend::new(unavailable_reason));
    let miniapp = MiniAppConversationProvider::new_for_platform_with_entitlements(
        Arc::clone(&backend),
        mini_apps,
        host_platform,
        Some(Arc::new(PlatformEntitlementChecker {
            client: product_client,
        })),
    )
    .map_err(|error| HostError::new(error.to_string()))?;
    builder
        .with_agent_backend(backend)?
        .with_provider(Arc::new(miniapp))?
        .build()
        .map_err(HostError::from)
}

fn merge_official_mini_apps(
    configured: impl IntoIterator<Item = MiniAppDefinition>,
) -> Vec<MiniAppDefinition> {
    let mut definitions = configured
        .into_iter()
        .map(|definition| (definition.plugin_id.clone(), definition))
        .collect::<BTreeMap<_, _>>();
    for plugin_id in OFFICIAL_PLUGIN_IDS {
        let definition = app_definition(plugin_id).expect("official plugin definition");
        let pinned = definitions
            .get(plugin_id)
            .is_some_and(|definition| definition.pinned);
        definitions.insert(
            plugin_id.to_string(),
            MiniAppDefinition {
                plugin_id: definition.id,
                title: definition.title,
                pinned,
            },
        );
    }
    definitions.into_values().collect()
}

#[cfg(feature = "desktop-full")]
fn default_codex_home() -> PathBuf {
    default_mahayana_home().join("codex")
}

#[cfg(any(feature = "desktop-full", feature = "mobile-embedded"))]
fn default_codex_home_if_available() -> Option<PathBuf> {
    #[cfg(feature = "desktop-full")]
    {
        #[allow(clippy::needless_return)]
        return Some(default_codex_home());
    }
    #[cfg(not(feature = "desktop-full"))]
    {
        None
    }
}

#[derive(Clone)]
struct PlatformEntitlementChecker {
    client: MahayanaProductClient,
}

#[async_trait::async_trait]
impl EntitlementChecker for PlatformEntitlementChecker {
    async fn has_entitlement(&self, plugin_id: &str, capability: &str) -> Result<bool, String> {
        let client = self.client.clone();
        let plugin_id = plugin_id.to_string();
        let capability = capability.to_string();
        tokio::task::spawn_blocking(move || client.entitlement(&plugin_id, &capability))
            .await
            .map_err(|error| error.to_string())?
            .map(|entitlement| entitlement.is_some())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HostCreateConfig {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mahayana-host-test-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create isolated Host root");
        HostCreateConfig {
            runtime: RuntimeConfig {
                data_dir: Some(root.join("runtime")),
                ..RuntimeConfig::default()
            },
            product_session_path: Some(root.join("product-session.json")),
            mini_apps: vec![MiniAppDefinition {
                plugin_id: "test-miniapp".to_string(),
                title: "Test MiniApp".to_string(),
                pinned: false,
            }],
            inherit_installed_plugins: Some(false),
            ..HostCreateConfig::default()
        }
    }

    #[test]
    fn direct_host_creates_executes_receives_and_clones() {
        let host = MahayanaHost::create(test_config()).expect("create host");
        let cloned = host.clone();
        let status = cloned
            .execute(RuntimeCommand::Status)
            .expect("execute status");
        let encoded = serde_json::to_value(status).expect("serialize status");
        assert_eq!(encoded["runtimeAbiVersion"], 1);
        assert_eq!(encoded["remoteAgentEnabled"], false);

        let ready = host
            .receive(Duration::from_millis(10))
            .expect("receive ready")
            .expect("ready event");
        let encoded = serde_json::to_value(ready).expect("serialize event");
        assert_eq!(encoded["@type"], "mahayana.runtime.ready");
    }

    #[test]
    fn create_rejects_remote_agent_gateway() {
        let mut config = test_config();
        config.runtime.remote_agent_enabled = true;
        let error = MahayanaHost::create(config)
            .err()
            .expect("remote agent must be rejected");
        assert!(error.to_string().contains("remote Agent"));
    }
}
