use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::IpAddr,
    path::Path,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub nodes: Vec<AuthorizedNode>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_max_bytes")]
    pub max_content_bytes: usize,
    #[serde(default = "default_rate")]
    pub max_requests_per_minute: u32,
}
fn default_bind() -> String {
    "127.0.0.1:18888".into()
}
fn default_max_bytes() -> usize {
    1024 * 1024
}
fn default_rate() -> u32 {
    30
}
impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            max_content_bytes: default_max_bytes(),
            max_requests_per_minute: default_rate(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthorizedNode {
    pub id: String,
    pub endpoint: Url,
    /// SHA-256 public identity advertised by the HTTPS node in X-Global-Dharma-Node-Key.
    pub public_key_sha256: String,
    #[serde(default)]
    pub regions: Vec<String>,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path)
                .map_err(|e| format!("stat config: {e}"))?
                .permissions()
                .mode();
            if mode & 0o022 != 0 {
                return Err("config must not be group/world writable".into());
            }
        }
        let raw = fs::read_to_string(path).map_err(|e| format!("read config: {e}"))?;
        let config: Self = toml::from_str(&raw).map_err(|e| format!("parse config: {e}"))?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.daemon.bind.starts_with("127.0.0.1:") && !self.daemon.bind.starts_with("[::1]:") {
            return Err("daemon.bind must be loopback only".into());
        }
        if self.daemon.max_content_bytes == 0 || self.daemon.max_content_bytes > 16 * 1024 * 1024 {
            return Err("daemon.max_content_bytes must be 1..=16777216".into());
        }
        if self.daemon.max_requests_per_minute == 0 || self.daemon.max_requests_per_minute > 600 {
            return Err("daemon.max_requests_per_minute must be 1..=600".into());
        }
        for node in &self.nodes {
            validate_node(node)?;
        }
        Ok(())
    }
}

pub fn validate_node(node: &AuthorizedNode) -> Result<(), String> {
    if node.id.trim().is_empty() {
        return Err("node id is required".into());
    }
    if node.endpoint.scheme() != "https" {
        return Err(format!("node {} must use https", node.id));
    }
    if !node.endpoint.username().is_empty() || node.endpoint.password().is_some() {
        return Err(format!("node {} must not embed credentials", node.id));
    }
    let host = node
        .endpoint
        .host_str()
        .ok_or_else(|| format!("node {} has no host", node.id))?;
    if host.eq_ignore_ascii_case("localhost") {
        return Err(format!("node {} cannot target localhost", node.id));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_or_local(ip) {
            return Err(format!(
                "node {} cannot target private/local address",
                node.id
            ));
        }
    }
    if node.public_key_sha256.len() != 64
        || !node
            .public_key_sha256
            .bytes()
            .all(|c| c.is_ascii_hexdigit())
    {
        return Err(format!("node {} has invalid public_key_sha256", node.id));
    }
    Ok(())
}
fn is_private_or_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_unspecified()
        }
        IpAddr::V6(v) => {
            v.is_loopback()
                || v.is_unspecified()
                || v.is_unique_local()
                || v.is_unicast_link_local()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeliveryRequest {
    pub task_id: String,
    pub content: String,
    #[serde(default)]
    pub region: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Receipt {
    pub task_id: String,
    pub node_id: String,
    pub content_sha256: String,
    pub delivered_at_unix_ms: u128,
    pub status: String,
}
pub fn content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}
pub fn validate_request(config: &Config, request: &DeliveryRequest) -> Result<(), String> {
    if request.task_id.trim().is_empty() || request.task_id.len() > 128 {
        return Err("task_id must be 1..=128 characters".into());
    }
    if request.content.trim().is_empty() {
        return Err("content is required".into());
    }
    if request.content.len() > config.daemon.max_content_bytes {
        return Err("content exceeds configured maximum".into());
    }
    Ok(())
}

pub struct RateLimiter {
    limit: u32,
    window: Duration,
    started: Instant,
    used: u32,
}
impl RateLimiter {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            window: Duration::from_secs(60),
            started: Instant::now(),
            used: 0,
        }
    }
    pub fn acquire(&mut self) -> Result<(), String> {
        if self.started.elapsed() >= self.window {
            self.started = Instant::now();
            self.used = 0;
        }
        if self.used >= self.limit {
            return Err("rate limit reached".into());
        }
        self.used += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_nodes() {
        let node = AuthorizedNode {
            id: "x".into(),
            endpoint: Url::parse("http://127.0.0.1/a").unwrap(),
            public_key_sha256: "a".repeat(64),
            regions: vec![],
        };
        assert!(validate_node(&node).is_err());
    }
    #[test]
    fn limiter_is_bounded() {
        let mut limiter = RateLimiter::new(1);
        assert!(limiter.acquire().is_ok());
        assert!(limiter.acquire().is_err());
    }
}
