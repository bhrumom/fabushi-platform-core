use crate::{Capability, CapabilitySet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Product-owned extension kinds exposed by Mahayana public contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Mcp,
    Skill,
    Plugin,
    Connector,
    MiniApp,
}

impl ExtensionKind {
    pub fn capability(self) -> Capability {
        match self {
            Self::Mcp => Capability::Mcp,
            Self::Skill => Capability::Skills,
            Self::Plugin => Capability::Plugins,
            Self::Connector => Capability::Connectors,
            Self::MiniApp => Capability::MiniApps,
        }
    }
}

/// Provider-neutral descriptor shared by MCP, Skills, Plugins, Connectors and MiniApps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionDescriptor {
    pub id: String,
    pub kind: ExtensionKind,
    pub capabilities: CapabilitySet,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl ExtensionDescriptor {
    pub fn new(
        id: impl Into<String>,
        kind: ExtensionKind,
        capabilities: impl IntoIterator<Item = Capability>,
    ) -> Result<Self, String> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err("extension id must not be empty".into());
        }
        let mut values = capabilities.into_iter().collect::<Vec<_>>();
        if !values.contains(&kind.capability()) {
            values.push(kind.capability());
        }
        Ok(Self {
            id,
            kind,
            capabilities: CapabilitySet::new(values),
            metadata: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_extension_kind_maps_to_a_mahayana_capability() {
        for kind in [
            ExtensionKind::Mcp,
            ExtensionKind::Skill,
            ExtensionKind::Plugin,
            ExtensionKind::Connector,
            ExtensionKind::MiniApp,
        ] {
            let descriptor = ExtensionDescriptor::new("example", kind, []).expect("descriptor");
            assert!(descriptor.capabilities.contains(kind.capability()));
        }
    }

    #[test]
    fn descriptor_rejects_empty_identity() {
        assert!(ExtensionDescriptor::new(" ", ExtensionKind::Plugin, []).is_err());
    }
}
