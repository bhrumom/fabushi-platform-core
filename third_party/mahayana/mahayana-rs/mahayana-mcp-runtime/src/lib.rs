//! Mahayana-owned MCP transport/runtime.
//!
//! This crate intentionally implements the MCP wire boundary directly instead
//! of exposing Codex app-server types. It supports local stdio plugins and
//! Streamable HTTP/JSON endpoints used by desktop, mobile, and Web hosts.

pub mod status;

use mahayana_platform_core::HostPlatform;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use url::Url;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone)]
pub struct NativeMcpRegistry {
    plugin_roots: Vec<PathBuf>,
    session_token: Option<String>,
}

impl NativeMcpRegistry {
    pub fn new(
        plugin_roots: impl IntoIterator<Item = PathBuf>,
        session_token: Option<String>,
    ) -> Self {
        Self {
            plugin_roots: plugin_roots.into_iter().collect(),
            session_token,
        }
    }

    pub fn from_workspace(root: impl AsRef<Path>, session_token: Option<String>) -> Self {
        Self::new(
            [root.as_ref().join(".agents/plugins/plugins")],
            session_token,
        )
    }

    pub fn resolve_plugin(
        &self,
        plugin_id: &str,
        platform: HostPlatform,
    ) -> Result<ResolvedMcpPlugin, McpError> {
        validate_plugin_id(plugin_id)?;
        for plugins_root in &self.plugin_roots {
            let plugin_root = plugins_root.join(plugin_id);
            if !plugin_root.is_dir() {
                continue;
            }
            let manifest_path = [
                plugin_root.join(".mahayana-plugin/plugin.json"),
                plugin_root.join(".codex-plugin/plugin.json"),
            ]
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| McpError::PluginManifestMissing(plugin_id.to_string()))?;
            let manifest: PluginManifest = read_json(&manifest_path)?;
            if manifest.name != plugin_id {
                return Err(McpError::InvalidPlugin(format!(
                    "manifest name `{}` does not match `{plugin_id}`",
                    manifest.name
                )));
            }
            let server_name = select_server(&manifest, platform)?;
            let server_path = manifest.mcp_servers.as_deref().unwrap_or("./.mcp.json");
            let config_path = safe_plugin_join(&plugin_root, Path::new(server_path))?;
            let config: McpFile = read_json(&config_path)?;
            let raw = config
                .mcp_servers
                .get(&server_name)
                .cloned()
                .ok_or_else(|| McpError::ServerNotFound(server_name.clone()))?;
            let transport = parse_transport(&plugin_root, raw, self.session_token.as_deref())?;
            return Ok(ResolvedMcpPlugin {
                plugin_id: plugin_id.to_string(),
                plugin_root,
                server_name,
                transport,
            });
        }
        Err(McpError::PluginNotFound(plugin_id.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedMcpPlugin {
    pub plugin_id: String,
    pub plugin_root: PathBuf,
    pub server_name: String,
    pub transport: McpTransport,
}

impl ResolvedMcpPlugin {
    pub fn client(&self) -> NativeMcpClient {
        NativeMcpClient::new(self.transport.clone())
    }
}

#[derive(Debug, Clone)]
pub enum McpTransport {
    Stdio {
        command: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone)]
pub struct NativeMcpClient {
    transport: McpTransport,
}

impl NativeMcpClient {
    pub fn new(transport: McpTransport) -> Self {
        Self { transport }
    }

    pub fn list_tools(&self) -> Result<Vec<Value>, McpError> {
        let result = self.request("tools/list", json!({}))?;
        Ok(result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        if name.trim().is_empty() {
            return Err(McpError::InvalidRequest("tool name is empty".into()));
        }
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    pub fn read_resource(&self, uri: &str) -> Result<Vec<Value>, McpError> {
        let result = self.request("resources/read", json!({"uri": uri}))?;
        Ok(result
            .get("contents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub fn list_resources(&self) -> Result<Vec<Value>, McpError> {
        let result = self.request("resources/list", json!({}))?;
        Ok(result
            .get("resources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        match &self.transport {
            McpTransport::Stdio {
                command,
                args,
                cwd,
                env,
            } => request_stdio(command, args, cwd, env, method, params),
            McpTransport::Http { url, headers } => request_http(url, headers, method, params),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PluginManifest {
    name: String,
    #[serde(rename = "mcpServers")]
    mcp_servers: Option<String>,
    #[serde(default, rename = "runtimeVariants")]
    runtime_variants: Vec<RuntimeVariant>,
}

#[derive(Debug, Deserialize)]
struct RuntimeVariant {
    server: String,
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    priority: i64,
}

#[derive(Debug, Deserialize)]
struct McpFile {
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, Value>,
}

fn select_server(manifest: &PluginManifest, platform: HostPlatform) -> Result<String, McpError> {
    let platform = platform_name(platform);
    if let Some(variant) = manifest
        .runtime_variants
        .iter()
        .filter(|variant| {
            variant.platforms.is_empty()
                || variant
                    .platforms
                    .iter()
                    .any(|candidate| candidate == platform)
        })
        .max_by_key(|variant| variant.priority)
    {
        return Ok(variant.server.clone());
    }
    Err(McpError::InvalidPlugin(format!(
        "plugin `{}` has no MCP runtime for {platform}",
        manifest.name
    )))
}

fn platform_name(platform: HostPlatform) -> &'static str {
    match platform {
        HostPlatform::Cli => "cli",
        HostPlatform::Desktop => "desktop",
        HostPlatform::Mobile => "mobile",
        HostPlatform::Web => "web",
    }
}

fn parse_transport(
    plugin_root: &Path,
    value: Value,
    session_token: Option<&str>,
) -> Result<McpTransport, McpError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if value.get("url").is_some() {
                "http"
            } else {
                "stdio"
            }
        });
    match kind {
        "stdio" => {
            let command = value
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| McpError::InvalidPlugin("stdio server has no command".into()))?;
            let command = if Path::new(command).is_absolute() {
                PathBuf::from(command)
            } else {
                safe_plugin_join(plugin_root, Path::new(command))?
            };
            let cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .map(|cwd| safe_plugin_join(plugin_root, Path::new(cwd)))
                .transpose()?
                .unwrap_or_else(|| plugin_root.to_path_buf());
            let args = value
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let env = value
                .get("env")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|values| values.iter())
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (key.clone(), expand_secret(value, session_token)))
                })
                .collect::<BTreeMap<_, _>>();
            Ok(McpTransport::Stdio {
                command,
                args,
                cwd,
                env,
            })
        }
        "http" | "streamable-http" => {
            let url = value
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| McpError::InvalidPlugin("HTTP server has no URL".into()))?;
            let parsed = Url::parse(url)
                .map_err(|error| McpError::InvalidPlugin(format!("invalid MCP URL: {error}")))?;
            if parsed.scheme() != "https" && !is_loopback(&parsed) {
                return Err(McpError::UnsafeTransport(
                    "remote MCP endpoints must use HTTPS".into(),
                ));
            }
            let mut headers = value
                .get("headers")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|values| values.iter())
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (key.clone(), expand_secret(value, session_token)))
                })
                .collect::<BTreeMap<_, _>>();
            if parsed.host_str() == Some("api.ombhrum.com")
                && let Some(token) = session_token.filter(|token| !token.trim().is_empty())
            {
                headers
                    .entry("Authorization".into())
                    .or_insert_with(|| format!("Bearer {token}"));
            }
            validate_headers(&headers)?;
            Ok(McpTransport::Http {
                url: url.to_string(),
                headers,
            })
        }
        other => Err(McpError::InvalidPlugin(format!(
            "unsupported MCP transport `{other}`"
        ))),
    }
}

fn request_stdio(
    command: &Path,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    method: &str,
    params: Value,
) -> Result<Value, McpError> {
    let mut child = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| McpError::Transport(format!("failed to start MCP server: {error}")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| McpError::Transport("MCP server stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| McpError::Transport("MCP server stdout unavailable".into()))?;
    let mut writer = BufWriter::new(stdin);
    let mut reader = BufReader::new(stdout);

    write_line(&mut writer, initialize_request(1))?;
    let _ = read_jsonrpc_result(&mut reader, 1)?;
    write_line(
        &mut writer,
        json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
    )?;
    write_line(
        &mut writer,
        json!({"jsonrpc":"2.0","id":2,"method":method,"params":params}),
    )?;
    let result = read_jsonrpc_result(&mut reader, 2);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn write_line(writer: &mut impl Write, value: Value) -> Result<(), McpError> {
    serde_json::to_writer(&mut *writer, &value)
        .map_err(|error| McpError::Protocol(error.to_string()))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| McpError::Transport(error.to_string()))
}

fn read_jsonrpc_result(reader: &mut impl BufRead, id: i64) -> Result<Value, McpError> {
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| McpError::Transport(error.to_string()))?;
        if read == 0 {
            return Err(McpError::Protocol(format!(
                "MCP server closed before response {id}"
            )));
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        return jsonrpc_result(value);
    }
}

fn request_http(
    url: &str,
    headers: &BTreeMap<String, String>,
    method: &str,
    params: Value,
) -> Result<Value, McpError> {
    let initialize = send_http(url, headers, None, initialize_request(1))?;
    let session_id = initialize.session_id;
    let mut initialized_headers = headers.clone();
    if let Some(session_id) = session_id.as_deref() {
        initialized_headers.insert("MCP-Session-Id".into(), session_id.to_string());
    }
    let _ = send_http(
        url,
        &initialized_headers,
        session_id.as_deref(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
    )?;
    let response = send_http(
        url,
        &initialized_headers,
        session_id.as_deref(),
        json!({"jsonrpc":"2.0","id":2,"method":method,"params":params}),
    )?;
    if response.body.is_null() {
        return Err(McpError::Protocol(format!(
            "MCP HTTP response for `{method}` was empty"
        )));
    }
    jsonrpc_result(response.body)
}

struct HttpMcpResponse {
    body: Value,
    session_id: Option<String>,
}

fn send_http(
    url: &str,
    headers: &BTreeMap<String, String>,
    session_id: Option<&str>,
    payload: Value,
) -> Result<HttpMcpResponse, McpError> {
    let mut request = ureq::post(url)
        .set("Accept", "application/json, text/event-stream")
        .set("Content-Type", "application/json")
        .set("MCP-Protocol-Version", MCP_PROTOCOL_VERSION);
    for (name, value) in headers {
        request = request.set(name, value);
    }
    if let Some(session_id) = session_id {
        request = request.set("MCP-Session-Id", session_id);
    }
    let response = request.send_json(payload).map_err(|error| match error {
        ureq::Error::Status(status, _) => {
            McpError::Transport(format!("MCP endpoint returned HTTP {status}"))
        }
        ureq::Error::Transport(error) => McpError::Transport(error.to_string()),
    })?;
    let session_id = response
        .header("MCP-Session-Id")
        .or_else(|| response.header("Mcp-Session-Id"))
        .map(str::to_owned);
    if response.status() == 202 || response.status() == 204 {
        return Ok(HttpMcpResponse {
            body: Value::Null,
            session_id,
        });
    }
    let content_type = response
        .header("Content-Type")
        .unwrap_or_default()
        .to_string();
    let mut body = String::new();
    response
        .into_reader()
        .read_to_string(&mut body)
        .map_err(|error| McpError::Transport(error.to_string()))?;
    let body = if content_type.contains("text/event-stream") {
        parse_sse_json(&body)?
    } else {
        serde_json::from_str(&body).map_err(|error| McpError::Protocol(error.to_string()))?
    };
    Ok(HttpMcpResponse { body, session_id })
}

fn parse_sse_json(body: &str) -> Result<Value, McpError> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .next_back()
        .ok_or_else(|| McpError::Protocol("MCP SSE response had no JSON data event".into()))
}

fn initialize_request(id: i64) -> Value {
    json!({
        "jsonrpc":"2.0",
        "id":id,
        "method":"initialize",
        "params":{
            "protocolVersion":MCP_PROTOCOL_VERSION,
            "capabilities":{},
            "clientInfo":{"name":"mahayana","version":env!("CARGO_PKG_VERSION")}
        }
    })
}

fn jsonrpc_result(value: Value) -> Result<Value, McpError> {
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("MCP request failed");
        return Err(McpError::Remote {
            code,
            message: message.to_string(),
        });
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), McpError> {
    if plugin_id.trim().is_empty()
        || plugin_id.contains('/')
        || plugin_id.contains('\\')
        || plugin_id.contains("..")
    {
        return Err(McpError::InvalidPluginId(plugin_id.to_string()));
    }
    Ok(())
}

fn safe_plugin_join(root: &Path, relative: &Path) -> Result<PathBuf, McpError> {
    if relative.is_absolute() {
        return Err(McpError::UnsafeTransport(
            "absolute paths are not allowed in plugin runtime manifests".into(),
        ));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| McpError::Io(error.to_string()))?;
    let mut path = canonical_root.clone();
    for component in relative.components() {
        match component {
            Component::Normal(segment) => path.push(segment),
            Component::CurDir => {}
            _ => {
                return Err(McpError::UnsafeTransport(
                    "plugin runtime path traversal is not allowed".into(),
                ));
            }
        }
    }
    if path.exists() {
        let canonical = path
            .canonicalize()
            .map_err(|error| McpError::Io(error.to_string()))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(McpError::UnsafeTransport(
                "plugin runtime path escapes the plugin root".into(),
            ));
        }
    }
    Ok(path)
}

fn expand_secret(value: &str, session_token: Option<&str>) -> String {
    match session_token {
        Some(token) => value.replace("${MAHAYANA_SESSION_TOKEN}", token),
        None => value.to_string(),
    }
}

fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), McpError> {
    for (name, value) in headers {
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err(McpError::UnsafeTransport(
                "MCP HTTP header contains newline characters".into(),
            ));
        }
    }
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, McpError> {
    let bytes = fs::read(path).map_err(|error| McpError::Io(error.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|error| McpError::InvalidPlugin(error.to_string()))
}

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("plugin id is invalid: {0}")]
    InvalidPluginId(String),
    #[error("plugin not found: {0}")]
    PluginNotFound(String),
    #[error("plugin manifest missing: {0}")]
    PluginManifestMissing(String),
    #[error("plugin manifest/runtime is invalid: {0}")]
    InvalidPlugin(String),
    #[error("MCP server not found: {0}")]
    ServerNotFound(String),
    #[error("unsafe MCP transport: {0}")]
    UnsafeTransport(String),
    #[error("invalid MCP request: {0}")]
    InvalidRequest(String),
    #[error("MCP transport failed: {0}")]
    Transport(String),
    #[error("MCP protocol failed: {0}")]
    Protocol(String),
    #[error("MCP remote error {code}: {message}")]
    Remote { code: i64, message: String },
    #[error("MCP I/O failed: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_plugin_traversal_and_insecure_remote_http() {
        assert!(validate_plugin_id("../evil").is_err());
        let root = std::env::temp_dir().join(format!("mahayana-mcp-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create root");
        let result = parse_transport(
            &root,
            json!({"type":"http","url":"http://example.com/mcp"}),
            None,
        );
        assert!(matches!(result, Err(McpError::UnsafeTransport(_))));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parses_sse_json_payloads() {
        let value = parse_sse_json(
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"ok\":true}}\n\n",
        )
        .expect("parse SSE");
        assert_eq!(value["result"]["ok"], true);
    }
}
