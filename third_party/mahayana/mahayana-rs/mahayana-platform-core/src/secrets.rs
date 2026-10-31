use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretNamespace {
    Account,
    ManagedRequest,
    Plugin,
    McpOAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum SecretScope {
    Global,
    Environment(String),
}

impl SecretScope {
    pub fn environment(id: impl Into<String>) -> Result<Self, SecretStoreError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(SecretStoreError::InvalidScope);
        }
        Ok(Self::Environment(id))
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretName(String);

impl SecretName {
    pub fn new(raw: impl Into<String>) -> Result<Self, SecretStoreError> {
        let raw = raw.into();
        let value = raw.trim();
        if value.is_empty()
            || !value
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        {
            return Err(SecretStoreError::InvalidName);
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("SecretName").field(&self.0).finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue(<redacted>)")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SecretKey {
    pub namespace: SecretNamespace,
    pub scope: SecretScope,
    pub name: SecretName,
}

impl SecretKey {
    pub fn new(namespace: SecretNamespace, scope: SecretScope, name: SecretName) -> Self {
        Self {
            namespace,
            scope,
            name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretListEntry {
    pub key: SecretKey,
}

/// Provider-neutral secure-storage contract used by Mahayana product code.
///
/// Backends may use the OS keyring, encrypted local files, hardware-backed
/// stores, or remote secure storage. Secret values intentionally do not
/// implement `Serialize`, and their Debug/Display output is always redacted.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &SecretKey, value: &SecretValue) -> Result<(), SecretStoreError>;
    fn get(&self, key: &SecretKey) -> Result<Option<SecretValue>, SecretStoreError>;
    fn delete(&self, key: &SecretKey) -> Result<bool, SecretStoreError>;
    fn list(
        &self,
        namespace: SecretNamespace,
        scope: Option<&SecretScope>,
    ) -> Result<Vec<SecretListEntry>, SecretStoreError>;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretStoreError {
    #[error("secret name must contain only A-Z, 0-9, or _")]
    InvalidName,
    #[error("secret scope is invalid")]
    InvalidScope,
    #[error("secret store is unavailable: {0}")]
    Unavailable(String),
    #[error("secret store operation failed: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_are_stable_and_environment_safe() {
        let name = SecretName::new("MAHAYANA_ACCOUNT_SESSION").unwrap();
        assert_eq!(name.as_str(), "MAHAYANA_ACCOUNT_SESSION");
        assert_eq!(
            SecretName::new("not-safe"),
            Err(SecretStoreError::InvalidName)
        );
    }

    #[test]
    fn secret_values_never_render_plaintext() {
        let value = SecretValue::new("very-sensitive-token");
        assert_eq!(format!("{value:?}"), "SecretValue(<redacted>)");
        assert_eq!(value.to_string(), "<redacted>");
        assert_eq!(value.expose(), "very-sensitive-token");
    }

    #[test]
    fn scopes_and_namespaces_are_provider_neutral() {
        let key = SecretKey::new(
            SecretNamespace::ManagedRequest,
            SecretScope::environment("fabushi").unwrap(),
            SecretName::new("MAHAYANA_REQUESTED_SECRET_123").unwrap(),
        );
        let encoded = serde_json::to_string(&key).unwrap().to_ascii_lowercase();
        assert!(!encoded.contains("codex"));
        assert!(!encoded.contains("grok"));
    }
}
