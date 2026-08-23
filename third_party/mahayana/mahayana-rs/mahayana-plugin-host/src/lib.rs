//! Mahayana-owned plugin manifest, runtime selection and command routing.

use mahayana_platform_core::CommandDeclaration;
use mahayana_platform_core::HostPlatform;
use mahayana_platform_core::MahayanaPluginManifest;
use mahayana_platform_core::ManifestError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPluginManifest {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: Option<String>,
    #[serde(default, rename = "runtimeVariants")]
    pub runtime_variants: Vec<PluginRuntimeVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRuntimeVariant {
    pub id: String,
    pub server: String,
    #[serde(default)]
    pub platforms: Vec<HostPlatform>,
    #[serde(default)]
    pub priority: i64,
}

#[derive(Debug, Clone)]
pub struct LocalPlugin {
    /// Mahayana-owned compatibility DTO for the historical plugin manifest.
    /// The file name may remain `.codex-plugin/plugin.json` for installed
    /// packages, but no Codex runtime or protocol type crosses this boundary.
    pub legacy: LegacyPluginManifest,
    /// Transitional field alias for CLI compatibility call sites. This is the
    /// same Mahayana-owned DTO as `legacy`, not a Codex runtime/protocol type.
    pub codex: LegacyPluginManifest,
    pub mahayana: Option<MahayanaPluginManifest>,
}

impl LocalPlugin {
    pub fn load(plugin_root: &Path) -> Result<Self, PluginHostError> {
        let manifest_path = [
            plugin_root.join(".mahayana-plugin/plugin.json"),
            plugin_root.join(".codex-plugin/plugin.json"),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| PluginHostError::InvalidPluginManifest(plugin_root.to_path_buf()))?;
        let source = fs::read_to_string(&manifest_path).map_err(|source| {
            PluginHostError::ReadPluginManifest {
                path: manifest_path.clone(),
                source,
            }
        })?;
        let legacy = serde_json::from_str::<LegacyPluginManifest>(&source).map_err(|source| {
            PluginHostError::DecodePluginManifest {
                path: manifest_path,
                source,
            }
        })?;
        if legacy.name.trim().is_empty() {
            return Err(PluginHostError::InvalidPluginManifest(
                plugin_root.to_path_buf(),
            ));
        }
        let mahayana = MahayanaPluginManifest::load(plugin_root)?;
        Ok(Self {
            codex: legacy.clone(),
            legacy,
            mahayana,
        })
    }

    pub fn command(&self, name: &str) -> Option<&CommandDeclaration> {
        self.mahayana.as_ref()?.commands.iter().find(|command| {
            command.name == name || command.aliases.iter().any(|alias| alias == name)
        })
    }

    pub fn gate_for_tool(&self, tool: &str) -> Option<&str> {
        let target = format!("tool:{tool}");
        self.mahayana
            .as_ref()?
            .gates
            .iter()
            .find(|gate| gate.target == target)
            .map(|gate| gate.entitlement.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedRuntime {
    pub variant_id: Option<String>,
    pub server: String,
}

pub fn select_runtime(
    platform: HostPlatform,
    mcp_servers: &[String],
    variants: &[PluginRuntimeVariant],
) -> Result<SelectedRuntime, PluginHostError> {
    select_runtime_with_availability(platform, mcp_servers, variants, |_| true)
}

/// Selects the highest-priority runtime whose declared MCP server can be
/// started by the current host. Runtime selection is fully Mahayana-owned;
/// legacy manifest JSON is treated only as an input compatibility format.
pub fn select_runtime_with_availability(
    platform: HostPlatform,
    mcp_servers: &[String],
    variants: &[PluginRuntimeVariant],
    mut is_available: impl FnMut(&str) -> bool,
) -> Result<SelectedRuntime, PluginHostError> {
    let mut candidates = variants
        .iter()
        .filter(|variant| variant.platforms.contains(&platform))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    for selected in candidates {
        if !mcp_servers.contains(&selected.server) {
            return Err(PluginHostError::MissingRuntimeServer(
                selected.server.clone(),
            ));
        }
        if is_available(&selected.server) {
            return Ok(SelectedRuntime {
                variant_id: Some(selected.id.clone()),
                server: selected.server.clone(),
            });
        }
    }
    if !variants
        .iter()
        .any(|variant| variant.platforms.contains(&platform))
    {
        return match mcp_servers {
            [server] if is_available(server) => Ok(SelectedRuntime {
                variant_id: None,
                server: server.clone(),
            }),
            [server] => Err(PluginHostError::RuntimeUnavailable {
                platform,
                servers: vec![server.clone()],
            }),
            [] => Err(PluginHostError::NoRuntimeForPlatform(platform)),
            _ => Err(PluginHostError::AmbiguousLegacyRuntime),
        };
    }
    Err(PluginHostError::RuntimeUnavailable {
        platform,
        servers: variants
            .iter()
            .filter(|variant| variant.platforms.contains(&platform))
            .map(|variant| variant.server.clone())
            .collect(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginCommandInvocation {
    pub plugin_id: String,
    pub command: String,
    pub arguments: Value,
}

impl PluginCommandInvocation {
    pub fn parse_tui(source: &str) -> Result<Self, PluginHostError> {
        let source = source
            .strip_prefix('/')
            .ok_or(PluginHostError::InvalidCommandSyntax)?;
        let (qualified, remainder) = source
            .split_once(char::is_whitespace)
            .map_or((source, ""), |(qualified, remainder)| {
                (qualified, remainder.trim())
            });
        let (plugin_id, command) = qualified
            .split_once(':')
            .ok_or(PluginHostError::InvalidCommandSyntax)?;
        if plugin_id.is_empty() || command.is_empty() {
            return Err(PluginHostError::InvalidCommandSyntax);
        }
        let arguments = if remainder.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(remainder)
                .unwrap_or_else(|_| serde_json::json!({"input": remainder}))
        };
        Ok(Self {
            plugin_id: plugin_id.to_string(),
            command: command.to_string(),
            arguments,
        })
    }
}

pub fn command_index(
    manifests: impl IntoIterator<Item = (String, MahayanaPluginManifest)>,
) -> HashMap<String, (String, String)> {
    let mut index = HashMap::new();
    for (plugin_id, manifest) in manifests {
        for command in manifest.commands {
            index.insert(
                format!("{plugin_id}:{}", command.name),
                (plugin_id.clone(), command.tool.clone()),
            );
            for alias in command.aliases {
                index.insert(
                    format!("{plugin_id}:{alias}"),
                    (plugin_id.clone(), command.tool.clone()),
                );
            }
        }
    }
    index
}

#[derive(Debug, thiserror::Error)]
pub enum PluginHostError {
    #[error("plugin manifest is missing or invalid at {0}")]
    InvalidPluginManifest(PathBuf),
    #[error("failed to read plugin manifest {path}: {source}")]
    ReadPluginManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode plugin manifest {path}: {source}")]
    DecodePluginManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    MahayanaManifest(#[from] ManifestError),
    #[error("selected runtime references undeclared MCP server {0}")]
    MissingRuntimeServer(String),
    #[error("plugin has no MCP runtime for {0:?}")]
    NoRuntimeForPlatform(HostPlatform),
    #[error("plugin MCP runtimes for {platform:?} are unavailable: {servers:?}")]
    RuntimeUnavailable {
        platform: HostPlatform,
        servers: Vec<String>,
    },
    #[error("plugin has multiple MCP servers but no runtimeVariants selection")]
    AmbiguousLegacyRuntime,
    #[error("plugin command must use /<plugin-id>:<command> [JSON]")]
    InvalidCommandSyntax,
}

#[cfg(test)]
#[path = "plugin_host_tests.rs"]
mod tests;
