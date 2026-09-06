use base64::Engine as _;
use mahayana_core::{ModelProviderMode, RuntimeConfig};
use mahayana_feature_host::FeatureHostController;
use mahayana_host::HostCreateConfig;
use mahayana_host_protocol::{
    ApprovalResolution, FeatureCommand, HostConfig, HostMode, SurfacePlatform,
};
use mahayana_js_runtime::{DeepSeekJsHost, scan_package_compatibility};
use mahayana_native_engine::ProcessExecution;
use mahayana_plugin_runtime::{
    ExternalReleaseManifest, InstalledPluginPointer, PermissionManager, PluginInstaller,
    PluginState,
};
use mahayana_product::MahayanaProductClient;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppHostFeatureMode {
    Production,
    Test,
}

const TEST_MARKETPLACE_PLUGINS: &[(&str, &str, &str)] = &[
    ("global-dharma", "全球法布施", "任务、日志与部署"),
    ("faliu-flashcards", "法流记忆卡", "经文牌组与复习"),
    ("platform-publish", "平台发布", "内容发布与自动化"),
    (
        "hermes-installer",
        "Hermes Installer",
        "插件安装与运行时管理",
    ),
    ("bot-father", "Bot Father", "创建和管理机器人"),
    (
        "chatgpt-auto-confirm",
        "ChatGPT Auto Confirm",
        "受控自动确认与任务协作",
    ),
];

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
    feature_mode: AppHostFeatureMode,
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
        product
            .bootstrap_ci_test_account_session()
            .map_err(|error| {
                AppHostError::Operation(format!(
                    "GitHub Actions test-account bootstrap failed: {error}"
                ))
            })?;
        Ok(Self {
            app_data_dir,
            feature_mode,
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
            "platform.request" => self
                .product
                .execute("mahayana.platform.request", &params)
                .map_err(|error| AppHostError::Operation(error.to_string())),
            "plugin.permissions" => self.plugin_permissions(params),
            "plugin.permission.grant" => self.set_permission(params, true),
            "plugin.permission.revoke" => self.set_permission(params, false),
            "plugin.compatibility" => self.plugin_compatibility(params),
            "runtime.start" => self.start_runtime(params),
            "runtime.stop" => self.stop_runtime(params),
            "runtime.tools" => self.runtime_tools(),
            "runtime.call" => self.runtime_call(params),
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
            "feature.receive" => self.feature_receive(params),
            "feature.approval.resolve" => self.feature_resolve_approval(params),
            "feature.interrupt" => self.feature_interrupt(params),
            "feature.auth.status" => self
                .feature
                .auth_status()
                .map_err(|error| AppHostError::Operation(error.to_string())),
            // Main-process only: this method is intentionally absent from the
            // renderer IPC allowlist. It never returns a refresh credential.
            "feature.auth.deviceAgentSession" => self
                .product
                .device_agent_session()
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
            "feature.marketplace.browse" => self.marketplace_browse(params),
            "feature.marketplace.release" => self.marketplace_release(params),
            "feature.plugin.install" => self.install_plugin(params),
            "feature.plugin.uninstall" => self.uninstall_plugin(params),
            "feature.plugin.active" => self.active_plugin(params),
            "feature.plugin.listInstalled" => self.list_installed_plugins(),
            "feature.plugin.uiDocument" => self.plugin_ui_document(params),
            "feature.messaging.execute" => self.feature_messaging_execute(params),
            "feature.messaging.blob.read" => self.feature_messaging_blob_read(params),
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

    fn feature_receive(&self, params: Value) -> Result<Value, AppHostError> {
        let timeout_ms = params
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(30_000);
        let event = self
            .feature
            .receive_with_timeout(Duration::from_millis(timeout_ms))
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

    fn feature_messaging_execute(&self, params: Value) -> Result<Value, AppHostError> {
        let request_id = string_param(&params, "requestId")?.to_string();
        let envelope = params
            .get("envelope")
            .cloned()
            .ok_or_else(|| AppHostError::InvalidRequest("envelope is required".into()))?;
        let envelopes = self
            .feature
            .execute_messaging_sync(request_id, envelope)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        Ok(json!({"envelopes": envelopes}))
    }

    fn feature_messaging_blob_read(&self, params: Value) -> Result<Value, AppHostError> {
        let blob_id = string_param(&params, "blobId")?;
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let length = params
            .get("length")
            .and_then(Value::as_u64)
            .unwrap_or(1024 * 1024)
            .clamp(1, 1024 * 1024);
        let (metadata, bytes) = self
            .feature
            .read_messaging_blob_range(blob_id, offset, length)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        Ok(json!({
            "metadata": metadata,
            "offset": offset,
            "dataBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
        }))
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

    fn marketplace_browse(&self, params: Value) -> Result<Value, AppHostError> {
        let query = params.get("query").and_then(Value::as_str);
        if self.feature_mode == AppHostFeatureMode::Test {
            let term = query.unwrap_or_default().trim().to_lowercase();
            let plugins = TEST_MARKETPLACE_PLUGINS
                .iter()
                .filter_map(|(plugin_id, display_name, description)| {
                    let searchable =
                        format!("{plugin_id} {display_name} {description}").to_lowercase();
                    if !term.is_empty() && !searchable.contains(&term) {
                        return None;
                    }
                    Some(json!({
                        "pluginId": plugin_id,
                        "displayName": display_name,
                        "description": description,
                        "latestVersion": "1.0.0",
                        "platforms": ["desktop"],
                        "releaseStatus": "approved"
                    }))
                })
                .collect::<Vec<_>>();
            return Ok(json!({"plugins": plugins}));
        }
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

    fn marketplace_release(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        let version = string_param(&params, "version")?;
        if self.feature_mode == AppHostFeatureMode::Test {
            if !TEST_MARKETPLACE_PLUGINS
                .iter()
                .any(|(candidate, _, _)| *candidate == plugin_id)
            {
                return Err(AppHostError::Operation(format!(
                    "test marketplace plugin {plugin_id} was not found"
                )));
            }
            return Ok(json!({
                "pluginId": plugin_id,
                "version": version,
                "releaseStatus": "approved",
                "releaseManifest": {
                    "schemaVersion": 1,
                    "protocol": "mahayana.external-release.v1",
                    "pluginId": plugin_id,
                    "version": version,
                    "permissions": [],
                    "artifacts": []
                }
            }));
        }
        self.product
            .marketplace_release_metadata(plugin_id, version)
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
        if self.feature_mode == AppHostFeatureMode::Test {
            if !TEST_MARKETPLACE_PLUGINS
                .iter()
                .any(|(candidate, _, _)| *candidate == release.plugin_id)
            {
                return Err(AppHostError::Operation(format!(
                    "test marketplace plugin {} was not found",
                    release.plugin_id
                )));
            }
            let plugin_root = self.plugin_root().join(&release.plugin_id);
            let installed_dir = plugin_root
                .join("versions")
                .join(&release.version)
                .join("test-ui");
            std::fs::create_dir_all(&installed_dir)
                .map_err(|error| AppHostError::Operation(error.to_string()))?;
            let display_name = TEST_MARKETPLACE_PLUGINS
                .iter()
                .find(|(candidate, _, _)| *candidate == release.plugin_id)
                .map(|(_, display_name, _)| *display_name)
                .unwrap_or(release.plugin_id.as_str());
            let html = format!(
                "<!doctype html><html><head><meta charset=\"utf-8\"><title>{display_name}</title></head><body><main><h1>{display_name}</h1><p>Installed from the deterministic Mahayana Marketplace test backend.</p></main></body></html>"
            );
            std::fs::write(installed_dir.join("index.html"), html)
                .map_err(|error| AppHostError::Operation(error.to_string()))?;
            let pointer = InstalledPluginPointer {
                plugin_id: release.plugin_id.clone(),
                version: release.version.clone(),
                artifact_id: "test-ui".to_string(),
                artifact_sha256: "0".repeat(64),
                runtime: "local-web".to_string(),
                entry: Some("index.html".to_string()),
                requested_permissions: release.permissions.clone(),
                installed_path: installed_dir.to_string_lossy().into_owned(),
            };
            std::fs::create_dir_all(&plugin_root)
                .map_err(|error| AppHostError::Operation(error.to_string()))?;
            let active = serde_json::to_vec_pretty(&pointer)
                .map_err(|error| AppHostError::Operation(error.to_string()))?;
            std::fs::write(plugin_root.join("active.json"), active)
                .map_err(|error| AppHostError::Operation(error.to_string()))?;
            return serde_json::to_value(pointer)
                .map_err(|error| AppHostError::Operation(error.to_string()));
        }
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

    fn list_installed_plugins(&self) -> Result<Value, AppHostError> {
        let root = self.plugin_root();
        let installer = self.installer()?;
        let mut plugins = Vec::new();
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"plugins": plugins}));
            }
            Err(error) => return Err(AppHostError::Operation(error.to_string())),
        };
        for entry in entries {
            let entry = entry.map_err(|error| AppHostError::Operation(error.to_string()))?;
            if !entry
                .file_type()
                .map_err(|error| AppHostError::Operation(error.to_string()))?
                .is_dir()
            {
                continue;
            }
            let plugin_id = entry.file_name().to_string_lossy().into_owned();
            if let Some(pointer) = installer
                .active(&plugin_id)
                .map_err(|error| AppHostError::Operation(error.to_string()))?
            {
                plugins.push(pointer);
            }
        }
        plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
        Ok(json!({"plugins": plugins}))
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
        let root = std::fs::canonicalize(&pointer.installed_path)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        let requested_entry = pointer
            .entry
            .as_deref()
            .map(|value| value.trim_start_matches("./"));
        let entry = requested_entry
            .filter(|value| value.to_ascii_lowercase().ends_with(".html"))
            .map(str::to_string)
            .or_else(|| {
                ["ui/index.html", "web/index.html", "index.html"]
                    .into_iter()
                    .find(|candidate| root.join(candidate).is_file())
                    .map(str::to_string)
            })
            .ok_or_else(|| {
                AppHostError::Operation(format!(
                    "installed plugin {plugin_id} does not expose an HTML Mini App entry"
                ))
            })?;
        let relative = PathBuf::from(&entry);
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

    fn runtime_call(&self, params: Value) -> Result<Value, AppHostError> {
        let plugin_id = string_param(&params, "pluginId")?;
        let name = string_param(&params, "name")?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return Err(AppHostError::InvalidRequest(
                "runtime.call arguments must be an object".into(),
            ));
        }
        if self.feature_mode == AppHostFeatureMode::Test {
            let installed = self
                .plugin_root()
                .join(plugin_id)
                .join("active.json")
                .is_file();
            if installed
                && TEST_MARKETPLACE_PLUGINS
                    .iter()
                    .any(|(candidate, _, _)| *candidate == plugin_id)
            {
                return deterministic_test_runtime_call(plugin_id, name, &arguments);
            }
        }
        let host = self
            .js
            .lock()
            .map_err(|_| AppHostError::Operation("JavaScript host lock poisoned".into()))?;
        if host.plugin_state(plugin_id) != Some(PluginState::Active) {
            return Err(AppHostError::Operation(format!(
                "plugin {plugin_id} runtime is not active"
            )));
        }
        let tools = host
            .registered_tools()
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        if !tools.iter().any(|candidate| candidate == name) {
            return Err(AppHostError::Operation(format!(
                "tool {name} is not registered in the active runtime"
            )));
        }
        host.call_tool_json(name, &arguments)
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }
}

fn deterministic_test_runtime_call(
    plugin_id: &str,
    name: &str,
    arguments: &Value,
) -> Result<Value, AppHostError> {
    if plugin_id != "global-dharma" {
        return Ok(json!({
            "content": [{"type": "text", "text": format!("{plugin_id} test tool {name} completed") }],
            "structuredContent": {"testMode": true, "tool": name, "arguments": arguments},
        }));
    }
    let (text, structured) = match name {
        "status" => (
            "已读取全球法布施状态。",
            json!({"running": false, "mode": "home", "testMode": true}),
        ),
        "start" => (
            "本地转经轮已通过宿主权限校验并启动。",
            json!({"running": true, "mode": "local-prayer-wheel", "testMode": true}),
        ),
        "stop" => (
            "全球法布施本地模式已停止。",
            json!({"running": false, "mode": "home", "testMode": true}),
        ),
        "logs" => (
            "已读取全球法布施日志。",
            json!({"entries": ["deterministic GitHub Actions test runtime"], "testMode": true}),
        ),
        "send" => (
            "全球发送测试请求已完成。",
            json!({"sent": 1, "testMode": true}),
        ),
        other => {
            return Err(AppHostError::Operation(format!(
                "test runtime tool {other} is not available for global-dharma"
            )));
        }
    };
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
    }))
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
    let provider =
        std::env::var("MAHAYANA_INFERENCE_PROVIDER").unwrap_or_else(|_| "fabushi".into());
    let mut runtime = RuntimeConfig {
        data_dir: Some(root.join("runtime")),
        ..RuntimeConfig::default()
    };
    if provider == "openrouter" {
        runtime.model.provider = ModelProviderMode::UserConfiguredRemote;
        runtime.model.base_url = Some("https://openrouter.ai/api/v1".into());
        runtime.model.model =
            std::env::var("MAHAYANA_OPENROUTER_MODEL").unwrap_or_else(|_| "openai/gpt-5.2".into());
        runtime.model.credential_key = Some("inference/openrouter/api-key".into());
    } else if provider == "claude-code" {
        runtime.model.provider = ModelProviderMode::UserConfiguredRemote;
        runtime.model.base_url = Some("https://api.anthropic.com/v1".into());
        runtime.model.model =
            std::env::var("MAHAYANA_CLAUDE_MODEL").unwrap_or_else(|_| "claude-sonnet-4-6".into());
        runtime.model.credential_key = Some("inference/claude/api-key".into());
    }
    let host_config = HostCreateConfig {
        runtime,
        product_session_path: Some(root.join("account-session.json")),
        product_surface_state_path: Some(root.join("product-surface.json")),
        automation_path: Some(root.join("automations.json")),
        use_codex_account: std::env::var("MAHAYANA_USE_CODEX_ACCOUNT").as_deref() == Ok("1"),
        codex_home: std::env::var_os("MAHAYANA_CODEX_HOME").map(PathBuf::from),
        model_bearer_token: std::env::var("MAHAYANA_MODEL_BEARER_TOKEN")
            .ok()
            .filter(|value| !value.is_empty()),
        model_wire_api: match provider.as_str() {
            "openrouter" => mahayana_model::responses::ResponsesWireApi::ChatCompletions,
            "claude-code" => mahayana_model::responses::ResponsesWireApi::AnthropicMessages,
            _ => mahayana_model::responses::ResponsesWireApi::Responses,
        },
        inherit_installed_plugins: Some(false),
        process_execution: if std::env::var("MAHAYANA_SANDBOX_RUNTIME").as_deref()
            == Ok("local-docker")
        {
            ProcessExecution::LocalDocker {
                docker_path: std::env::var_os("MAHAYANA_DOCKER_BIN")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("docker")),
                image: std::env::var("MAHAYANA_DOCKER_IMAGE").unwrap_or_default(),
            }
        } else {
            ProcessExecution::Host
        },
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
