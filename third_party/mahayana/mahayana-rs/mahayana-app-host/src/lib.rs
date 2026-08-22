use mahayana_core::RuntimeConfig;
use mahayana_feature_host::FeatureHostController;
use mahayana_host::HostCreateConfig;
use mahayana_host_protocol::{
    ApprovalResolution, FeatureCommand, HostConfig, HostMode, SurfacePlatform,
};
use mahayana_js_runtime::{DeepSeekJsHost, scan_package_compatibility};
use mahayana_plugin_runtime::{ExternalReleaseManifest, PermissionManager, PluginInstaller};
use mahayana_product::MahayanaProductClient;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppHostFeatureMode {
    Production,
    Test,
}

impl From<AppHostFeatureMode> for HostMode {
    fn from(value: AppHostFeatureMode) -> Self {
        match value {
            AppHostFeatureMode::Production => HostMode::Production,
            AppHostFeatureMode::Test => HostMode::Test,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppHostError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("host operation failed: {0}")]
    Operation(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRequest {
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResponse {
    pub id: Option<Value>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct AppHost {
    app_data_dir: PathBuf,
    product: MahayanaProductClient,
    js: Mutex<DeepSeekJsHost>,
    feature: FeatureHostController,
}

impl AppHost {
    pub fn new(app_data_dir: impl Into<PathBuf>) -> Result<Self, AppHostError> {
        let feature_mode = configured_feature_host_mode()?;
        Self::new_with_feature_mode(app_data_dir, feature_mode)
    }

    pub fn new_with_feature_mode(
        app_data_dir: impl Into<PathBuf>,
        feature_mode: AppHostFeatureMode,
    ) -> Result<Self, AppHostError> {
        let app_data_dir = app_data_dir.into();
        std::fs::create_dir_all(&app_data_dir)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        let feature_root = feature_host_root(&app_data_dir);
        let feature = create_feature_host(&app_data_dir, feature_mode)?;
        let product = MahayanaProductClient::new_with_default_api_base_url(
            feature_root.join("account-session.json"),
            feature_root.join("product-surface.json"),
        );
        Ok(Self {
            app_data_dir,
            product,
            js: Mutex::new(
                DeepSeekJsHost::new()
                    .map_err(|error| AppHostError::Operation(error.to_string()))?,
            ),
            feature,
        })
    }

    pub fn dispatch(&self, request: HostRequest) -> HostResponse {
        let id = request.id.clone();
        match self.handle(&request.method, request.params) {
            Ok(result) => HostResponse {
                id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => HostResponse {
                id,
                ok: false,
                result: None,
                error: Some(error.to_string()),
            },
        }
    }

    fn handle(&self, method: &str, params: Value) -> Result<Value, AppHostError> {
        match method {
            "host.platform" => Ok(json!({"platform": host_platform()})),
            method if method.starts_with("feature.") => self.handle_feature(method, params),
            "marketplace.browse" => {
                let query = params.get("query").and_then(Value::as_str);
                let requested_platform = params
                    .get("platform")
                    .and_then(Value::as_str)
                    .unwrap_or(host_platform());
                let marketplace_platform = match requested_platform {
                    "ios" | "android" => "mobile",
                    other => other,
                };
                self.product
                    .marketplace_browse(query, Some(marketplace_platform))
                    .map_err(|error| AppHostError::Operation(error.to_string()))
            }
            "marketplace.release" => {
                let plugin_id = string_param(&params, "pluginId")?;
                let version = string_param(&params, "version")?;
                self.product
                    .marketplace_release_metadata(plugin_id, version)
                    .map_err(|error| AppHostError::Operation(error.to_string()))
            }
            "platform.request" => self
                .product
                .execute("mahayana.platform.request", &params)
                .map_err(|error| AppHostError::Operation(error.to_string())),
            "plugin.install" => self.install_plugin(params),
            "plugin.uninstall" => self.uninstall_plugin(params),
            "plugin.active" => self.active_plugin(params),
            "plugin.permissions" => self.plugin_permissions(params),
            "plugin.permission.grant" => self.set_permission(params, true),
            "plugin.permission.revoke" => self.set_permission(params, false),
            "plugin.compatibility" => self.plugin_compatibility(params),
            "plugin.uiDocument" => self.plugin_ui_document(params),
            "runtime.start" => self.start_runtime(params),
            "runtime.stop" => self.stop_runtime(params),
            "runtime.tools" => self.runtime_tools(),
            other => Err(AppHostError::InvalidRequest(format!(
                "unknown method {other}"
            ))),
        }
    }

    fn handle_feature(&self, method: &str, params: Value) -> Result<Value, AppHostError> {
        match method {
            "feature.info" => serde_json::to_value(self.feature.info())
                .map_err(|error| AppHostError::Operation(error.to_string())),
            "feature.execute" => self.feature_execute(params),
            "feature.receive" => self.feature_receive(),
            "feature.approval.resolve" => self.feature_resolve_approval(params),
            "feature.interrupt" => self.feature_interrupt(params),
            "feature.auth.status" => self
                .feature
                .auth_status()
                .map_err(|error| AppHostError::Operation(error.to_string())),
            "feature.auth.providers" => self
                .feature
                .auth_providers()
                .map_err(|error| AppHostError::Operation(error.to_string())),
            "feature.auth.passwordLogin" => self.feature_password_login(params),
            "feature.auth.browserStart" => self
                .feature
                .browser_login_start()
                .map_err(|error| AppHostError::Operation(error.to_string())),
            "feature.auth.browserPoll" => self.feature_browser_login_poll(params),
            "feature.auth.browserCancel" => self.feature_browser_login_cancel(params),
            "feature.auth.browserReopen" => self.feature_browser_login_reopen(params),
            "feature.auth.oauthStart" => self.feature_oauth_start(params),
            "feature.auth.oauthPoll" => self.feature_oauth_poll(params),
            "feature.auth.logout" => self
                .feature
                .logout()
                .map_err(|error| AppHostError::Operation(error.to_string())),
            "feature.messaging.access.issue" => self.feature_messaging_access_issue(params),
            other => Err(AppHostError::InvalidRequest(format!(
                "unknown feature method {other}"
            ))),
        }
    }

    fn feature_execute(&self, params: Value) -> Result<Value, AppHostError> {
        let command_value = params.get("command").cloned().unwrap_or(params);
        let command: FeatureCommand = serde_json::from_value(command_value).map_err(|error| {
            AppHostError::InvalidRequest(format!("invalid feature command: {error}"))
        })?;
        let accepted = self
            .feature
            .execute(command)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_value(accepted).map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_receive(&self) -> Result<Value, AppHostError> {
        let event = self
            .feature
            .receive()
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_value(event).map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_resolve_approval(&self, params: Value) -> Result<Value, AppHostError> {
        let resolution_value = params.get("resolution").cloned().unwrap_or(params);
        let resolution: ApprovalResolution =
            serde_json::from_value(resolution_value).map_err(|error| {
                AppHostError::InvalidRequest(format!("invalid approval resolution: {error}"))
            })?;
        self.feature
            .resolve_approval(resolution)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        Ok(Value::Null)
    }

    fn feature_interrupt(&self, params: Value) -> Result<Value, AppHostError> {
        let operation_id = string_param(&params, "operationId")?;
        self.feature
            .interrupt(operation_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        Ok(Value::Null)
    }

    fn feature_messaging_access_issue(&self, params: Value) -> Result<Value, AppHostError> {
        let device_id = string_param(&params, "deviceId")?.to_string();
        let session_id = string_param(&params, "sessionId")?.to_string();
        let scopes = params
            .get("scopes")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        item.as_str().map(str::to_string).ok_or_else(|| {
                            AppHostError::InvalidRequest("scopes must contain strings".into())
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let ttl_ms = params
            .get("ttlMs")
            .and_then(Value::as_i64)
            .unwrap_or(24 * 60 * 60 * 1000);
        self.feature
            .issue_messaging_access(device_id, session_id, scopes, ttl_ms)
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_password_login(&self, params: Value) -> Result<Value, AppHostError> {
        let username = string_param(&params, "username")?.to_string();
        let password = string_param(&params, "password")?.to_string();
        self.feature
            .password_login(username, password)
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_browser_login_reopen(&self, params: Value) -> Result<Value, AppHostError> {
        self.feature
            .browser_login_reopen(string_param(&params, "attemptId")?.to_string())
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_browser_login_cancel(&self, params: Value) -> Result<Value, AppHostError> {
        self.feature
            .browser_login_cancel(string_param(&params, "attemptId")?.to_string())
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_browser_login_poll(&self, params: Value) -> Result<Value, AppHostError> {
        self.feature
            .browser_login_poll(string_param(&params, "attemptId")?.to_string())
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_oauth_start(&self, params: Value) -> Result<Value, AppHostError> {
        self.feature
            .oauth_start(string_param(&params, "provider")?.to_string())
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn feature_oauth_poll(&self, params: Value) -> Result<Value, AppHostError> {
        self.feature
            .oauth_poll(string_param(&params, "attemptId")?.to_string())
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn plugin_root(&self) -> PathBuf {
        self.app_data_dir.join("plugins")
    }

    fn permission_store(&self) -> PathBuf {
        self.plugin_root().join("permissions.json")
    }

    fn installer(&self) -> Result<PluginInstaller, AppHostError> {
        PluginInstaller::new(self.plugin_root())
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn install_plugin(&self, params: Value) -> Result<Value, AppHostError> {
        let release_value = params
            .get("release")
            .cloned()
            .ok_or_else(|| AppHostError::InvalidRequest("release is required".into()))?;
        let release: ExternalReleaseManifest = serde_json::from_value(release_value)
            .map_err(|error| AppHostError::InvalidRequest(error.to_string()))?;
        let platform = params
            .get("platform")
            .and_then(Value::as_str)
            .unwrap_or(host_platform());
        let preferred: &[&str] = match platform {
            "ios" | "android" | "mobile" => &[
                "deepseek-js",
                "javascript",
                "cordis-js",
                "web-wasm",
                "userscript",
                "mcp",
                "local-web",
            ],
            _ => &[
                "deepseek-js",
                "javascript",
                "cordis-js",
                "local-web",
                "web-wasm",
                "native",
                "desktop-stdio",
                "mcp",
            ],
        };
        let installer = self.installer()?;
        let pointer = installer
            .install(&release, platform, preferred)
            .or_else(|error| {
                if matches!(platform, "ios" | "android") {
                    installer.install(&release, "mobile", preferred)
                } else {
                    Err(error)
                }
            })
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_value(pointer).map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn uninstall_plugin(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        if let Ok(mut host) = self.js.lock() {
            let _ = host.disable_plugin(plugin_id);
        }
        let removed = self
            .installer()?
            .uninstall(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        let permissions_removed = PermissionManager::load(self.permission_store())
            .map_err(|error| AppHostError::Operation(error.to_string()))?
            .remove_plugin(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        Ok(json!({
            "pluginId": plugin_id,
            "removed": removed,
            "permissionsRemoved": permissions_removed
        }))
    }

    fn active_plugin(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        let pointer = self
            .installer()?
            .active(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_value(pointer).map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn plugin_permissions(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        let pointer = self
            .installer()?
            .active(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?
            .ok_or_else(|| {
                AppHostError::Operation(format!("plugin {plugin_id} is not installed"))
            })?;
        let mut manager = PermissionManager::load(self.permission_store())
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        manager
            .retain_requested(plugin_id, &pointer.requested_permissions)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        let granted = manager.grants_for(plugin_id);
        let missing = pointer
            .requested_permissions
            .iter()
            .filter(|permission| !granted.contains(permission))
            .cloned()
            .collect::<Vec<_>>();
        Ok(
            json!({"requested": pointer.requested_permissions, "granted": granted, "missing": missing}),
        )
    }

    fn set_permission(&self, params: Value, grant: bool) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        let permission = string_param(&params, "permission")?;
        let pointer = self
            .installer()?
            .active(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?
            .ok_or_else(|| {
                AppHostError::Operation(format!("plugin {plugin_id} is not installed"))
            })?;
        let mut manager = PermissionManager::load(self.permission_store())
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        if grant {
            manager
                .grant(plugin_id, &pointer.requested_permissions, permission)
                .map_err(|error| AppHostError::Operation(error.to_string()))?;
        } else {
            manager
                .revoke(plugin_id, permission)
                .map_err(|error| AppHostError::Operation(error.to_string()))?;
        }
        self.plugin_permissions(json!({"pluginId": plugin_id}))
    }

    fn plugin_compatibility(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        let pointer = self
            .installer()?
            .active(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?
            .ok_or_else(|| {
                AppHostError::Operation(format!("plugin {plugin_id} is not installed"))
            })?;
        let report = scan_package_compatibility(Path::new(&pointer.installed_path))
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_value(report).map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn plugin_ui_document(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        let pointer = self
            .installer()?
            .active(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?
            .ok_or_else(|| {
                AppHostError::Operation(format!("plugin {plugin_id} is not installed"))
            })?;
        if pointer.runtime != "local-web" {
            return Err(AppHostError::Operation(format!(
                "plugin runtime {} does not expose a local Web document",
                pointer.runtime
            )));
        }
        let root = std::fs::canonicalize(&pointer.installed_path)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        let entry = pointer
            .entry
            .as_deref()
            .unwrap_or("index.html")
            .trim_start_matches("./");
        let relative = PathBuf::from(entry);
        if relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return Err(AppHostError::InvalidRequest(
                "plugin UI entry escaped installed root".into(),
            ));
        }
        let document = std::fs::canonicalize(root.join(relative))
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        if !document.starts_with(&root) {
            return Err(AppHostError::InvalidRequest(
                "plugin UI entry escaped installed root".into(),
            ));
        }
        let html = std::fs::read_to_string(document)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        Ok(json!({"pluginId": plugin_id, "html": html}))
    }

    fn start_runtime(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?.to_string();
        let pointer = self
            .installer()?
            .active(&plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?
            .ok_or_else(|| {
                AppHostError::Operation(format!("plugin {plugin_id} is not installed"))
            })?;
        if !matches!(
            pointer.runtime.as_str(),
            "deepseek-js" | "javascript" | "cordis-js"
        ) {
            return Err(AppHostError::Operation(format!(
                "runtime {} is not supported by the portable JS host",
                pointer.runtime
            )));
        }
        let root = PathBuf::from(&pointer.installed_path);
        let entry = discover_js_entry(&pointer.entry, &root)?;
        let report = scan_package_compatibility(&root)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        if !report.portable_compatible {
            return Err(AppHostError::Operation(report.blockers().join("; ")));
        }
        let grants = PermissionManager::load(self.permission_store())
            .map_err(|error| AppHostError::Operation(error.to_string()))?
            .grants_for(&plugin_id);
        let config = params.get("config").cloned().unwrap_or_else(|| json!({}));
        let mut host = self
            .js
            .lock()
            .map_err(|_| AppHostError::Operation("JavaScript host lock poisoned".into()))?;
        let state = host
            .register_plugin_with_grants(&plugin_id, &root, &entry, &config, &grants)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_value(state).map_err(|error| AppHostError::Operation(error.to_string()))
    }

    fn stop_runtime(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        self.js
            .lock()
            .map_err(|_| AppHostError::Operation("JavaScript host lock poisoned".into()))?
            .disable_plugin(plugin_id)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        Ok(Value::Null)
    }

    fn runtime_tools(&self) -> Result<Value, AppHostError> {
        let tools = self
            .js
            .lock()
            .map_err(|_| AppHostError::Operation("JavaScript host lock poisoned".into()))?
            .registered_tools()
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_value(tools).map_err(|error| AppHostError::Operation(error.to_string()))
    }
}

fn configured_feature_host_mode() -> Result<AppHostFeatureMode, AppHostError> {
    match std::env::var("FABUSHI_FEATURE_HOST_MODE") {
        Ok(value) if value.eq_ignore_ascii_case("test") => Ok(AppHostFeatureMode::Test),
        Ok(value) if value.eq_ignore_ascii_case("production") => Ok(AppHostFeatureMode::Production),
        Ok(value) if value.trim().is_empty() => Ok(AppHostFeatureMode::Production),
        Ok(value) => Err(AppHostError::InvalidRequest(format!(
            "unsupported FABUSHI_FEATURE_HOST_MODE {value:?}; expected test or production"
        ))),
        Err(std::env::VarError::NotPresent) => Ok(AppHostFeatureMode::Production),
        Err(error) => Err(AppHostError::InvalidRequest(format!(
            "invalid FABUSHI_FEATURE_HOST_MODE: {error}"
        ))),
    }
}

fn feature_host_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("feature-host")
}

fn create_feature_host(
    app_data_dir: &Path,
    feature_mode: AppHostFeatureMode,
) -> Result<FeatureHostController, AppHostError> {
    let root = feature_host_root(app_data_dir);
    std::fs::create_dir_all(&root).map_err(|error| AppHostError::Operation(error.to_string()))?;
    let host_config = HostCreateConfig {
        runtime: RuntimeConfig {
            data_dir: Some(root.join("runtime")),
            ..RuntimeConfig::default()
        },
        product_session_path: Some(root.join("account-session.json")),
        product_surface_state_path: Some(root.join("product-surface.json")),
        automation_path: Some(root.join("automations.json")),
        ..HostCreateConfig::default()
    };
    FeatureHostController::create_with_host_config(
        HostConfig {
            profile_id: "default".to_string(),
            mode: feature_mode.into(),
        },
        surface_platform(),
        host_config,
    )
    .map_err(|error| {
        AppHostError::Operation(format!("feature host initialization failed: {error}"))
    })
}

fn discover_js_entry(entry: &Option<String>, root: &Path) -> Result<PathBuf, AppHostError> {
    if let Some(entry) = entry.as_deref() {
        let relative = PathBuf::from(entry.trim_start_matches("./"));
        if relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return Err(AppHostError::InvalidRequest(
                "plugin entry escaped installed root".into(),
            ));
        }
        if root.join(&relative).is_file() {
            return Ok(relative);
        }
    }
    for candidate in ["index.mjs", "index.js", "lib/index.js", "dist/index.js"] {
        if root.join(candidate).is_file() {
            return Ok(PathBuf::from(candidate));
        }
    }
    Err(AppHostError::Operation(
        "installed plugin has no runnable JavaScript entry".into(),
    ))
}

fn string_param<'a>(params: &'a Value, name: &str) -> Result<&'a str, AppHostError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppHostError::InvalidRequest(format!("{name} is required")))
}

fn surface_platform() -> SurfacePlatform {
    if cfg!(target_os = "ios") {
        SurfacePlatform::Ios
    } else if cfg!(target_os = "android") {
        SurfacePlatform::Android
    } else {
        SurfacePlatform::Electron
    }
}

pub fn host_platform() -> &'static str {
    if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "desktop"
    }
}

pub fn default_app_data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("FABUSHI_APP_DATA") {
        return PathBuf::from(path);
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/com.ombhrum.fabushi");
    }
    #[cfg(target_os = "windows")]
    if let Some(data) = std::env::var_os("APPDATA") {
        return PathBuf::from(data).join("Fabushi");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fabushi")
}

pub fn dispatch_json(host: &AppHost, input: &str) -> String {
    let response = match serde_json::from_str::<HostRequest>(input) {
        Ok(request) => host.dispatch(request),
        Err(error) => HostResponse {
            id: None,
            ok: false,
            result: None,
            error: Some(format!("invalid JSON request: {error}")),
        },
    };
    serde_json::to_string(&response).unwrap_or_else(|error| {
        format!("{{\"ok\":false,\"error\":\"serialization failed: {error}\"}}")
    })
}
