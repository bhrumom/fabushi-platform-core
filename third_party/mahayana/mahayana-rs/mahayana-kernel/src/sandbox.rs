//! Mahayana-owned sandbox contracts.
//!
//! The product kernel owns the policy vocabulary. Native executors and
//! compatibility adapters translate these decisions into platform-specific
//! sandboxes instead of leaking an upstream agent's sandbox types.

use crate::{Capability, ExecutionPolicy, RiskLevel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Component, Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    Read,
    Write,
    Create,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    Resolve,
    Connect,
    Listen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SandboxRequest {
    Filesystem {
        access: FilesystemAccess,
        path: String,
    },
    Process {
        program: String,
        args: Vec<String>,
    },
    Network {
        access: NetworkAccess,
        host: String,
        port: Option<u16>,
    },
    Environment {
        name: String,
        write: bool,
    },
    ComputerUse {
        operation: String,
    },
    Capability {
        capability: Capability,
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxVerdict {
    pub decision: SandboxDecision,
    pub risk: RiskLevel,
    pub reason: String,
}

impl SandboxVerdict {
    pub fn allowed(risk: RiskLevel, reason: impl Into<String>) -> Self {
        Self {
            decision: SandboxDecision::Allow,
            risk,
            reason: reason.into(),
        }
    }

    pub fn ask(risk: RiskLevel, reason: impl Into<String>) -> Self {
        Self {
            decision: SandboxDecision::Ask,
            risk,
            reason: reason.into(),
        }
    }

    pub fn denied(risk: RiskLevel, reason: impl Into<String>) -> Self {
        Self {
            decision: SandboxDecision::Deny,
            risk,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRule {
    /// Exact hostname or suffix rule prefixed by `*.`. `*` matches any host.
    pub host: String,
    /// Empty means any port for the matched host.
    pub ports: BTreeSet<u16>,
}

impl NetworkRule {
    pub fn new(
        host: impl Into<String>,
        ports: impl IntoIterator<Item = u16>,
    ) -> Result<Self, SandboxError> {
        let host = normalize_host(&host.into())?;
        Ok(Self {
            host,
            ports: ports.into_iter().collect(),
        })
    }

    fn matches(&self, host: &str, port: Option<u16>) -> bool {
        let host = host.to_ascii_lowercase();
        let host_matches = self.host == "*"
            || self.host == host
            || self
                .host
                .strip_prefix("*.")
                .is_some_and(|suffix| host != suffix && host.ends_with(&format!(".{suffix}")));
        host_matches
            && (self.ports.is_empty() || port.is_some_and(|port| self.ports.contains(&port)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Lexical workspace roots. Adapters should additionally canonicalize on
    /// the real filesystem before execution to defend against symlink escape.
    pub workspace_roots: Vec<String>,
    pub allow_workspace_reads: bool,
    pub allow_workspace_writes: bool,
    pub allow_process: bool,
    pub allowed_programs: BTreeSet<String>,
    pub allow_network: bool,
    pub network_rules: Vec<NetworkRule>,
    pub allow_listen: bool,
    pub readable_environment: BTreeSet<String>,
    pub writable_environment: BTreeSet<String>,
    pub allow_computer_use: bool,
}

impl SandboxPolicy {
    pub fn from_execution_policy(policy: &ExecutionPolicy) -> Self {
        Self {
            workspace_roots: Vec::new(),
            allow_workspace_reads: true,
            allow_workspace_writes: policy.allow_workspace_writes,
            allow_process: policy.allow_process,
            allowed_programs: BTreeSet::new(),
            allow_network: policy.allow_network,
            network_rules: Vec::new(),
            allow_listen: false,
            readable_environment: BTreeSet::new(),
            writable_environment: BTreeSet::new(),
            allow_computer_use: false,
        }
    }

    pub fn desktop_default() -> Self {
        Self::from_execution_policy(&ExecutionPolicy::interactive_default())
    }

    pub fn mobile_default() -> Self {
        Self::from_execution_policy(&ExecutionPolicy::mobile_default())
    }

    pub fn with_workspace_root(mut self, root: impl Into<String>) -> Result<Self, SandboxError> {
        let root = normalize_absolute_path(&root.into())?;
        if !self.workspace_roots.contains(&root) {
            self.workspace_roots.push(root);
            self.workspace_roots.sort();
        }
        Ok(self)
    }

    pub fn allow_program(mut self, program: impl Into<String>) -> Result<Self, SandboxError> {
        let program = normalize_program(&program.into())?;
        self.allowed_programs.insert(program);
        Ok(self)
    }

    pub fn allow_network_rule(mut self, rule: NetworkRule) -> Self {
        self.network_rules.push(rule);
        self
    }

    pub fn allow_environment_read(mut self, name: impl Into<String>) -> Result<Self, SandboxError> {
        self.readable_environment
            .insert(normalize_env_name(&name.into())?);
        Ok(self)
    }

    pub fn allow_environment_write(
        mut self,
        name: impl Into<String>,
    ) -> Result<Self, SandboxError> {
        self.writable_environment
            .insert(normalize_env_name(&name.into())?);
        Ok(self)
    }

    pub fn evaluate(&self, request: &SandboxRequest) -> SandboxVerdict {
        match request {
            SandboxRequest::Filesystem { access, path } => self.evaluate_filesystem(*access, path),
            SandboxRequest::Process { program, .. } => self.evaluate_process(program),
            SandboxRequest::Network { access, host, port } => {
                self.evaluate_network(*access, host, *port)
            }
            SandboxRequest::Environment { name, write } => self.evaluate_environment(name, *write),
            SandboxRequest::ComputerUse { operation } => {
                if self.allow_computer_use {
                    SandboxVerdict::ask(
                        RiskLevel::ExternalSideEffect,
                        format!("computer-use operation requires explicit approval: {operation}"),
                    )
                } else {
                    SandboxVerdict::denied(
                        RiskLevel::ExternalSideEffect,
                        "computer use is disabled by Mahayana sandbox policy",
                    )
                }
            }
            SandboxRequest::Capability { capability, target } => {
                self.evaluate_capability(*capability, target)
            }
        }
    }

    fn evaluate_filesystem(&self, access: FilesystemAccess, path: &str) -> SandboxVerdict {
        let Some(normalized) = normalize_absolute_path_for_evaluation(path) else {
            return SandboxVerdict::denied(
                filesystem_risk(access),
                "filesystem path is not absolute or contains traversal",
            );
        };
        if self.workspace_roots.is_empty()
            || !self
                .workspace_roots
                .iter()
                .any(|root| path_within(&normalized, root))
        {
            return SandboxVerdict::denied(
                filesystem_risk(access),
                "filesystem path is outside configured workspace roots",
            );
        }
        match access {
            FilesystemAccess::Read if self.allow_workspace_reads => SandboxVerdict::allowed(
                RiskLevel::ReadOnly,
                "read is inside an approved workspace root",
            ),
            FilesystemAccess::Write | FilesystemAccess::Create | FilesystemAccess::Delete
                if self.allow_workspace_writes =>
            {
                SandboxVerdict::ask(
                    RiskLevel::WorkspaceWrite,
                    "workspace mutation is sandboxed and requires policy approval",
                )
            }
            FilesystemAccess::Read => SandboxVerdict::denied(
                RiskLevel::ReadOnly,
                "workspace reads are disabled by Mahayana sandbox policy",
            ),
            _ => SandboxVerdict::denied(
                RiskLevel::WorkspaceWrite,
                "workspace writes are disabled by Mahayana sandbox policy",
            ),
        }
    }

    fn evaluate_process(&self, program: &str) -> SandboxVerdict {
        if !self.allow_process {
            return SandboxVerdict::denied(
                RiskLevel::SystemWrite,
                "process execution is disabled by Mahayana sandbox policy",
            );
        }
        let Ok(program) = normalize_program(program) else {
            return SandboxVerdict::denied(RiskLevel::SystemWrite, "process program is invalid");
        };
        if !self.allowed_programs.is_empty() && !self.allowed_programs.contains(&program) {
            return SandboxVerdict::denied(
                RiskLevel::SystemWrite,
                format!("program is outside the sandbox allowlist: {program}"),
            );
        }
        SandboxVerdict::ask(
            RiskLevel::SystemWrite,
            format!("process execution requires explicit approval: {program}"),
        )
    }

    fn evaluate_network(
        &self,
        access: NetworkAccess,
        host: &str,
        port: Option<u16>,
    ) -> SandboxVerdict {
        if !self.allow_network {
            return SandboxVerdict::denied(
                RiskLevel::ExternalSideEffect,
                "network access is disabled by Mahayana sandbox policy",
            );
        }
        if access == NetworkAccess::Listen && !self.allow_listen {
            return SandboxVerdict::denied(
                RiskLevel::ExternalSideEffect,
                "listening sockets are disabled by Mahayana sandbox policy",
            );
        }
        let Ok(host) = normalize_host(host) else {
            return SandboxVerdict::denied(
                RiskLevel::ExternalSideEffect,
                "network host is invalid",
            );
        };
        if !self.network_rules.is_empty()
            && !self
                .network_rules
                .iter()
                .any(|rule| rule.matches(&host, port))
        {
            return SandboxVerdict::denied(
                RiskLevel::ExternalSideEffect,
                format!("network target is outside the sandbox allowlist: {host}"),
            );
        }
        let risk = if access == NetworkAccess::Resolve {
            RiskLevel::ReadOnly
        } else {
            RiskLevel::ExternalSideEffect
        };
        if access == NetworkAccess::Resolve {
            SandboxVerdict::allowed(risk, "DNS resolution allowed by sandbox policy")
        } else {
            SandboxVerdict::ask(
                risk,
                format!("network operation requires explicit approval: {host}"),
            )
        }
    }

    fn evaluate_environment(&self, name: &str, write: bool) -> SandboxVerdict {
        let Ok(name) = normalize_env_name(name) else {
            return SandboxVerdict::denied(RiskLevel::ReadOnly, "environment name is invalid");
        };
        if write {
            if self.writable_environment.contains(&name) {
                SandboxVerdict::ask(
                    RiskLevel::SystemWrite,
                    format!("environment mutation requires approval: {name}"),
                )
            } else {
                SandboxVerdict::denied(
                    RiskLevel::SystemWrite,
                    format!("environment variable is not writable: {name}"),
                )
            }
        } else if self.readable_environment.contains(&name) {
            SandboxVerdict::allowed(RiskLevel::ReadOnly, "environment read is allowlisted")
        } else {
            SandboxVerdict::denied(
                RiskLevel::ReadOnly,
                format!("environment variable is not readable: {name}"),
            )
        }
    }

    fn evaluate_capability(&self, capability: Capability, target: &str) -> SandboxVerdict {
        match capability {
            Capability::Network | Capability::WebSearch => {
                self.evaluate_network(NetworkAccess::Connect, target, None)
            }
            Capability::Process | Capability::Git => self.evaluate_process(target),
            Capability::ComputerUse => self.evaluate(&SandboxRequest::ComputerUse {
                operation: target.to_owned(),
            }),
            Capability::FilesystemWrite | Capability::Workspace => {
                self.evaluate_filesystem(FilesystemAccess::Write, target)
            }
            Capability::FilesystemRead => self.evaluate_filesystem(FilesystemAccess::Read, target),
            _ => SandboxVerdict::ask(
                RiskLevel::ReadOnly,
                format!("capability is delegated to its Mahayana adapter: {capability:?}"),
            ),
        }
    }
}

fn filesystem_risk(access: FilesystemAccess) -> RiskLevel {
    match access {
        FilesystemAccess::Read => RiskLevel::ReadOnly,
        FilesystemAccess::Write | FilesystemAccess::Create | FilesystemAccess::Delete => {
            RiskLevel::WorkspaceWrite
        }
    }
}

fn normalize_program(program: &str) -> Result<String, SandboxError> {
    let value = program.trim();
    if value.is_empty()
        || value.contains('\0')
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_whitespace)
    {
        return Err(SandboxError::InvalidProgram);
    }
    Ok(value.to_string())
}

fn normalize_env_name(name: &str) -> Result<String, SandboxError> {
    let value = name.trim();
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        || value.as_bytes()[0].is_ascii_digit()
    {
        return Err(SandboxError::InvalidEnvironmentName);
    }
    Ok(value.to_string())
}

fn normalize_host(host: &str) -> Result<String, SandboxError> {
    let value = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let bare = value.strip_prefix("*.").unwrap_or(&value);
    if value == "*" {
        return Ok(value);
    }
    if bare.is_empty()
        || bare.contains('/')
        || bare.contains(':')
        || bare.chars().any(char::is_whitespace)
        || bare.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        })
    {
        return Err(SandboxError::InvalidHost);
    }
    Ok(value)
}

fn normalize_absolute_path(path: &str) -> Result<String, SandboxError> {
    normalize_absolute_path_for_evaluation(path).ok_or(SandboxError::InvalidPath)
}

fn normalize_absolute_path_for_evaluation(path: &str) -> Option<String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return None;
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    Some(format!("/{}", parts.join("/")))
}

fn path_within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SandboxError {
    #[error("sandbox path must be an absolute traversal-free path")]
    InvalidPath,
    #[error("sandbox process program is invalid")]
    InvalidProgram,
    #[error("sandbox network host is invalid")]
    InvalidHost,
    #[error("sandbox environment variable name is invalid")]
    InvalidEnvironmentName,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_scope_rejects_escape_and_prefix_confusion() {
        let policy = SandboxPolicy::desktop_default()
            .with_workspace_root("/repo/fabushi")
            .unwrap();
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Filesystem {
                    access: FilesystemAccess::Read,
                    path: "/repo/fabushi/src/lib.rs".into(),
                })
                .decision,
            SandboxDecision::Allow
        );
        for path in ["/repo/fabushi/../secret", "/repo/fabushi-evil/file"] {
            assert_eq!(
                policy
                    .evaluate(&SandboxRequest::Filesystem {
                        access: FilesystemAccess::Read,
                        path: path.into(),
                    })
                    .decision,
                SandboxDecision::Deny
            );
        }
    }

    #[test]
    fn writes_are_distinct_from_reads_and_require_approval() {
        let policy = SandboxPolicy::desktop_default()
            .with_workspace_root("/repo")
            .unwrap();
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Filesystem {
                    access: FilesystemAccess::Write,
                    path: "/repo/a.txt".into(),
                })
                .decision,
            SandboxDecision::Ask
        );
        let mut read_only = policy;
        read_only.allow_workspace_writes = false;
        assert_eq!(
            read_only
                .evaluate(&SandboxRequest::Filesystem {
                    access: FilesystemAccess::Write,
                    path: "/repo/a.txt".into(),
                })
                .decision,
            SandboxDecision::Deny
        );
    }

    #[test]
    fn process_allowlist_is_exact_and_never_shell_parsed() {
        let policy = SandboxPolicy::desktop_default()
            .allow_program("cargo")
            .unwrap();
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Process {
                    program: "cargo".into(),
                    args: vec!["test".into()],
                })
                .decision,
            SandboxDecision::Ask
        );
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Process {
                    program: "cargo;rm".into(),
                    args: Vec::new(),
                })
                .decision,
            SandboxDecision::Deny
        );
    }

    #[test]
    fn host_rules_do_not_match_suffix_confusion() {
        let policy = SandboxPolicy::desktop_default()
            .allow_network_rule(NetworkRule::new("*.example.com", [443]).unwrap());
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Network {
                    access: NetworkAccess::Connect,
                    host: "api.example.com".into(),
                    port: Some(443),
                })
                .decision,
            SandboxDecision::Ask
        );
        for host in ["example.com", "evil-example.com"] {
            assert_eq!(
                policy
                    .evaluate(&SandboxRequest::Network {
                        access: NetworkAccess::Connect,
                        host: host.into(),
                        port: Some(443),
                    })
                    .decision,
                SandboxDecision::Deny
            );
        }
    }

    #[test]
    fn listening_and_environment_access_are_fail_closed_by_default() {
        let policy = SandboxPolicy::desktop_default();
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Network {
                    access: NetworkAccess::Listen,
                    host: "localhost".into(),
                    port: Some(8080),
                })
                .decision,
            SandboxDecision::Deny
        );
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Environment {
                    name: "HOME".into(),
                    write: false,
                })
                .decision,
            SandboxDecision::Deny
        );
    }

    #[test]
    fn mobile_policy_disallows_processes() {
        let policy = SandboxPolicy::mobile_default();
        assert_eq!(
            policy
                .evaluate(&SandboxRequest::Process {
                    program: "git".into(),
                    args: vec!["status".into()],
                })
                .decision,
            SandboxDecision::Deny
        );
    }
}
