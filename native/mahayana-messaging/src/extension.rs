use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_COMPONENT_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionKind {
    McpConnector,
    Skill,
    WebMiniapp,
}

impl ExtensionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::McpConnector => "mcp-connector",
            Self::Skill => "skill",
            Self::WebMiniapp => "web-miniapp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionOrigin {
    Public,
    Team,
    PrivateCreated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionTrustDomain {
    CatalogAuthorized,
    TeamAuthorized,
    PrivateLocalAuthor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionReleaseChannel {
    Stable,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionPlatform {
    Desktop,
    Ios,
    Android,
    Web,
    Cli,
    Mobile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionIdentity {
    pub namespace: String,
    pub name: String,
    pub kind: ExtensionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionIdentityError {
    InvalidNamespace,
    InvalidName,
}

impl fmt::Display for ExtensionIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNamespace => write!(f, "invalid extension namespace"),
            Self::InvalidName => write!(f, "invalid extension name"),
        }
    }
}

impl std::error::Error for ExtensionIdentityError {}

impl ExtensionIdentity {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        kind: ExtensionKind,
    ) -> Result<Self, ExtensionIdentityError> {
        let namespace = namespace.into();
        let name = name.into();
        if !valid_component(&namespace) {
            return Err(ExtensionIdentityError::InvalidNamespace);
        }
        if !valid_component(&name) {
            return Err(ExtensionIdentityError::InvalidName);
        }
        Ok(Self {
            namespace,
            name,
            kind,
        })
    }

    pub fn stable_id(&self) -> String {
        format!("{}/{}:{}", self.namespace, self.name, self.kind.as_str())
    }
}

fn valid_component(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_COMPONENT_BYTES || !value.is_ascii() {
        return false;
    }
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    bytes.all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'.' | b'_' | b'-')
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCompatibility {
    pub platforms: Vec<ExtensionPlatform>,
    pub min_host_api: Option<u32>,
}

impl ExtensionCompatibility {
    pub fn supports(&self, platform: ExtensionPlatform, host_api: u32) -> bool {
        self.platforms.contains(&platform)
            && self.min_host_api.map_or(true, |minimum| host_api >= minimum)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionRelease {
    pub version: String,
    pub channel: ExtensionReleaseChannel,
    pub compatibility: ExtensionCompatibility,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionDescriptor {
    pub schema_version: u32,
    pub identity: ExtensionIdentity,
    pub origin: ExtensionOrigin,
    pub trust_domain: ExtensionTrustDomain,
    pub release: ExtensionRelease,
}

impl ExtensionDescriptor {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn is_installable_on(&self, platform: ExtensionPlatform, host_api: u32) -> bool {
        !self.release.revoked && self.release.compatibility.supports(platform, host_api)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_identity_matches_registry_wire_format() {
        let id = ExtensionIdentity::new("fabushi", "github", ExtensionKind::McpConnector).unwrap();
        assert_eq!(id.stable_id(), "fabushi/github:mcp-connector");
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            r#"{"namespace":"fabushi","name":"github","kind":"mcp-connector"}"#
        );
    }

    #[test]
    fn identity_rejects_unsafe_or_implicitly_normalized_components() {
        assert_eq!(
            ExtensionIdentity::new("Fabushi", "github", ExtensionKind::McpConnector),
            Err(ExtensionIdentityError::InvalidNamespace)
        );
        assert_eq!(
            ExtensionIdentity::new("fabushi", "../github", ExtensionKind::McpConnector),
            Err(ExtensionIdentityError::InvalidName)
        );
        assert_eq!(
            ExtensionIdentity::new("fabushi", "github connector", ExtensionKind::McpConnector),
            Err(ExtensionIdentityError::InvalidName)
        );
    }

    #[test]
    fn unknown_security_enum_fails_closed() {
        let raw = r#"{
          "schemaVersion":1,
          "identity":{"namespace":"fabushi","name":"github","kind":"native-plugin"},
          "origin":"public",
          "trustDomain":"catalog-authorized",
          "release":{
            "version":"1.0.0",
            "channel":"stable",
            "compatibility":{"platforms":["desktop"],"minHostApi":1},
            "requestedCapabilities":[],
            "revoked":false
          }
        }"#;
        assert!(serde_json::from_str::<ExtensionDescriptor>(raw).is_err());
    }

    #[test]
    fn trust_does_not_override_platform_or_host_compatibility() {
        let descriptor = ExtensionDescriptor {
            schema_version: ExtensionDescriptor::SCHEMA_VERSION,
            identity: ExtensionIdentity::new(
                "fabushi",
                "github",
                ExtensionKind::McpConnector,
            )
            .unwrap(),
            origin: ExtensionOrigin::Public,
            trust_domain: ExtensionTrustDomain::CatalogAuthorized,
            release: ExtensionRelease {
                version: "1.0.0".into(),
                channel: ExtensionReleaseChannel::Stable,
                compatibility: ExtensionCompatibility {
                    platforms: vec![ExtensionPlatform::Desktop],
                    min_host_api: Some(3),
                },
                requested_capabilities: vec!["mcp.tools.call".into()],
                revoked: false,
            },
        };
        assert!(!descriptor.is_installable_on(ExtensionPlatform::Ios, 99));
        assert!(!descriptor.is_installable_on(ExtensionPlatform::Desktop, 2));
        assert!(descriptor.is_installable_on(ExtensionPlatform::Desktop, 3));
    }

    #[test]
    fn revoked_release_is_never_installable() {
        let descriptor = ExtensionDescriptor {
            schema_version: ExtensionDescriptor::SCHEMA_VERSION,
            identity: ExtensionIdentity::new(
                "private-local",
                "notes",
                ExtensionKind::WebMiniapp,
            )
            .unwrap(),
            origin: ExtensionOrigin::PrivateCreated,
            trust_domain: ExtensionTrustDomain::PrivateLocalAuthor,
            release: ExtensionRelease {
                version: "1.0.0".into(),
                channel: ExtensionReleaseChannel::Stable,
                compatibility: ExtensionCompatibility {
                    platforms: vec![ExtensionPlatform::Ios, ExtensionPlatform::Android],
                    min_host_api: None,
                },
                requested_capabilities: vec![],
                revoked: true,
            },
        };
        assert!(!descriptor.is_installable_on(ExtensionPlatform::Ios, 1));
    }
}
