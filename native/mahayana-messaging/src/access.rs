use crate::actor::ActorId;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

const TOKEN_BYTES: usize = 32;
const TOKEN_PREFIX: &str = "fmsg_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessScope {
    Messaging,
    Calls,
    BlobsRead,
    BlobsWrite,
    Payments,
    MiniApps,
    Administration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessGrant {
    pub id: String,
    pub actor_id: ActorId,
    pub device_id: String,
    pub session_id: String,
    pub scopes: BTreeSet<AccessScope>,
    pub issued_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

impl AccessGrant {
    pub fn active_at(&self, now_ms: i64) -> bool {
        self.revoked_at_ms.is_none()
            && self
                .expires_at_ms
                .is_none_or(|expires_at_ms| expires_at_ms > now_ms)
    }

    pub fn allows(&self, scope: AccessScope) -> bool {
        self.scopes.contains(&scope) || self.scopes.contains(&AccessScope::Administration)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedAccess {
    pub grant_id: String,
    pub actor_id: ActorId,
    pub device_id: String,
    pub session_id: String,
    pub scopes: BTreeSet<AccessScope>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct IssuedAccessToken {
    pub token: String,
    pub grant: AccessGrant,
}

impl std::fmt::Debug for IssuedAccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedAccessToken")
            .field("token", &"[REDACTED]")
            .field("grant", &self.grant)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretGrant {
    token_sha256: [u8; 32],
    grant: AccessGrant,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessTokenRegistry {
    grants: Vec<SecretGrant>,
}

impl AccessTokenRegistry {
    pub fn issue(
        &mut self,
        token: impl AsRef<[u8]>,
        grant: AccessGrant,
    ) -> Result<(), AccessError> {
        let token = token.as_ref();
        validate_token(token)?;
        validate_grant(&grant)?;
        let token_sha256 = token_hash(token);
        if self.grants.iter().any(|existing| {
            existing.grant.id == grant.id || constant_time_eq(&existing.token_sha256, &token_sha256)
        }) {
            return Err(AccessError::DuplicateGrant);
        }
        self.grants.push(SecretGrant {
            token_sha256,
            grant,
        });
        Ok(())
    }

    pub fn issue_random(&mut self, grant: AccessGrant) -> Result<IssuedAccessToken, AccessError> {
        let token = random_token()?;
        self.issue(token.as_bytes(), grant.clone())?;
        Ok(IssuedAccessToken { token, grant })
    }

    pub fn authorize(
        &self,
        token: impl AsRef<[u8]>,
        actor_id: &ActorId,
        device_id: &str,
        session_id: &str,
        scope: AccessScope,
        now_ms: i64,
    ) -> Result<AuthorizedAccess, AccessError> {
        let token = token.as_ref();
        validate_token(token)?;
        let token_sha256 = token_hash(token);
        let matching = self
            .grants
            .iter()
            .find(|entry| constant_time_eq(&entry.token_sha256, &token_sha256))
            .ok_or(AccessError::Unauthorized)?;
        if &matching.grant.actor_id != actor_id
            || matching.grant.device_id != device_id
            || matching.grant.session_id != session_id
        {
            return Err(AccessError::IdentityMismatch);
        }
        if !matching.grant.active_at(now_ms) {
            return Err(AccessError::ExpiredOrRevoked);
        }
        if !matching.grant.allows(scope) {
            return Err(AccessError::ScopeDenied(scope));
        }
        Ok(AuthorizedAccess {
            grant_id: matching.grant.id.clone(),
            actor_id: matching.grant.actor_id.clone(),
            device_id: matching.grant.device_id.clone(),
            session_id: matching.grant.session_id.clone(),
            scopes: matching.grant.scopes.clone(),
        })
    }

    pub fn revoke(&mut self, grant_id: &str, now_ms: i64) -> Result<(), AccessError> {
        let entry = self
            .grants
            .iter_mut()
            .find(|entry| entry.grant.id == grant_id)
            .ok_or_else(|| AccessError::GrantNotFound(grant_id.to_string()))?;
        entry.grant.revoked_at_ms = Some(now_ms);
        Ok(())
    }

    pub fn rotate_random(
        &mut self,
        grant_id: &str,
        old_token: impl AsRef<[u8]>,
    ) -> Result<IssuedAccessToken, AccessError> {
        validate_token(old_token.as_ref())?;
        let old_hash = token_hash(old_token.as_ref());
        let entry = self
            .grants
            .iter_mut()
            .find(|entry| entry.grant.id == grant_id)
            .ok_or_else(|| AccessError::GrantNotFound(grant_id.to_string()))?;
        if !constant_time_eq(&entry.token_sha256, &old_hash) {
            return Err(AccessError::Unauthorized);
        }
        let token = random_token()?;
        entry.token_sha256 = token_hash(token.as_bytes());
        Ok(IssuedAccessToken {
            token,
            grant: entry.grant.clone(),
        })
    }

    pub fn grant(&self, grant_id: &str) -> Option<&AccessGrant> {
        self.grants
            .iter()
            .find(|entry| entry.grant.id == grant_id)
            .map(|entry| &entry.grant)
    }

    pub fn active_grants_for_actor<'a>(
        &'a self,
        actor_id: &'a ActorId,
        now_ms: i64,
    ) -> impl Iterator<Item = &'a AccessGrant> + 'a {
        self.grants
            .iter()
            .map(|entry| &entry.grant)
            .filter(move |grant| &grant.actor_id == actor_id && grant.active_at(now_ms))
    }
}

#[derive(Debug, Clone)]
pub struct FileAccessTokenStore {
    path: PathBuf,
}

impl FileAccessTokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AccessTokenRegistry, AccessError> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(AccessTokenRegistry::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, registry: &AccessTokenRegistry) -> Result<(), AccessError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file_name = self
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("access.json");
        let temporary = self
            .path
            .with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
        let payload = serde_json::to_vec_pretty(registry)?;
        {
            let mut file = File::create(&temporary)?;
            file.write_all(&payload)?;
            file.sync_all()?;
        }
        fs::rename(&temporary, &self.path)?;
        sync_directory(self.path.parent());
        Ok(())
    }

    pub fn issue_random(&self, grant: AccessGrant) -> Result<IssuedAccessToken, AccessError> {
        let mut registry = self.load()?;
        let issued = registry.issue_random(grant)?;
        self.save(&registry)?;
        Ok(issued)
    }

    pub fn authorize(
        &self,
        token: impl AsRef<[u8]>,
        actor_id: &ActorId,
        device_id: &str,
        session_id: &str,
        scope: AccessScope,
        now_ms: i64,
    ) -> Result<AuthorizedAccess, AccessError> {
        self.load()?
            .authorize(token, actor_id, device_id, session_id, scope, now_ms)
    }

    pub fn revoke(&self, grant_id: &str, now_ms: i64) -> Result<(), AccessError> {
        let mut registry = self.load()?;
        registry.revoke(grant_id, now_ms)?;
        self.save(&registry)
    }
}

fn validate_grant(grant: &AccessGrant) -> Result<(), AccessError> {
    if grant.id.trim().is_empty()
        || grant.device_id.trim().is_empty()
        || grant.session_id.trim().is_empty()
        || !grant.actor_id.is_valid()
    {
        return Err(AccessError::InvalidGrant);
    }
    if grant.scopes.is_empty() {
        return Err(AccessError::EmptyScopes);
    }
    if grant
        .expires_at_ms
        .is_some_and(|expires_at_ms| expires_at_ms <= grant.issued_at_ms)
    {
        return Err(AccessError::InvalidGrant);
    }
    Ok(())
}

fn validate_token(token: &[u8]) -> Result<(), AccessError> {
    if token.len() < TOKEN_BYTES {
        return Err(AccessError::WeakToken);
    }
    Ok(())
}

fn random_token() -> Result<String, AccessError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes).map_err(|error| AccessError::Random(error.to_string()))?;
    let token = format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes));
    bytes.fill(0);
    Ok(token)
}

fn token_hash(token: &[u8]) -> [u8; 32] {
    Sha256::digest(token).into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

fn sync_directory(directory: Option<&Path>) {
    let Some(directory) = directory else {
        return;
    };
    if let Ok(file) = File::open(directory) {
        let _ = file.sync_all();
    }
}

#[derive(Debug, Error)]
pub enum AccessError {
    #[error("messaging access token must contain at least 32 bytes")]
    WeakToken,
    #[error("messaging access grant is invalid")]
    InvalidGrant,
    #[error("messaging access grant has no scopes")]
    EmptyScopes,
    #[error("messaging access grant or token already exists")]
    DuplicateGrant,
    #[error("messaging access token is unauthorized")]
    Unauthorized,
    #[error("messaging access token actor, device, or session does not match request context")]
    IdentityMismatch,
    #[error("messaging access token is expired or revoked")]
    ExpiredOrRevoked,
    #[error("messaging access token is missing required scope {0:?}")]
    ScopeDenied(AccessScope),
    #[error("messaging access grant {0} was not found")]
    GrantNotFound(String),
    #[error("messaging access random source failed: {0}")]
    Random(String),
    #[error("messaging access persistence failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("messaging access serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(fill: u8) -> Vec<u8> {
        vec![fill; 48]
    }

    fn grant(actor_id: &ActorId) -> AccessGrant {
        AccessGrant {
            id: "grant:1".into(),
            actor_id: actor_id.clone(),
            device_id: "desktop:1".into(),
            session_id: "account-session:1".into(),
            scopes: BTreeSet::from([AccessScope::Messaging, AccessScope::Calls]),
            issued_at_ms: 1,
            expires_at_ms: Some(100),
            revoked_at_ms: None,
        }
    }

    #[test]
    fn token_is_bound_to_actor_device_session_scope_and_revocation() {
        let actor_id = ActorId::new("human:1");
        let mut registry = AccessTokenRegistry::default();
        registry.issue(token(7), grant(&actor_id)).unwrap();

        assert!(registry
            .authorize(
                token(7),
                &actor_id,
                "desktop:1",
                "account-session:1",
                AccessScope::Messaging,
                2,
            )
            .is_ok());
        assert!(matches!(
            registry.authorize(
                token(7),
                &actor_id,
                "desktop:1",
                "wrong-session",
                AccessScope::Messaging,
                2,
            ),
            Err(AccessError::IdentityMismatch)
        ));
        assert!(matches!(
            registry.authorize(
                token(7),
                &actor_id,
                "desktop:1",
                "account-session:1",
                AccessScope::Payments,
                2,
            ),
            Err(AccessError::ScopeDenied(AccessScope::Payments))
        ));
        registry.revoke("grant:1", 3).unwrap();
        assert!(matches!(
            registry.authorize(
                token(7),
                &actor_id,
                "desktop:1",
                "account-session:1",
                AccessScope::Messaging,
                4,
            ),
            Err(AccessError::ExpiredOrRevoked)
        ));
    }

    #[test]
    fn persisted_registry_contains_only_hashes_and_random_token_is_one_time_output() {
        let root = std::env::temp_dir().join(format!(
            "fabushi-access-contract-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("access.json");
        let store = FileAccessTokenStore::new(&path);
        let actor_id = ActorId::new("human:1");
        let issued = store.issue_random(grant(&actor_id)).unwrap();
        assert!(issued.token.starts_with(TOKEN_PREFIX));
        let persisted = fs::read_to_string(&path).unwrap();
        assert!(!persisted.contains(&issued.token));
        assert!(store
            .authorize(
                issued.token.as_bytes(),
                &actor_id,
                "desktop:1",
                "account-session:1",
                AccessScope::Messaging,
                2,
            )
            .is_ok());
        let _ = fs::remove_dir_all(root);
    }
}
