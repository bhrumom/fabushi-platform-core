//! Mahayana-owned encrypted secret storage.
//!
//! The product-facing storage contract lives here instead of in Codex. The
//! on-disk `mahayana_auth.age` and `local.age` formats stay compatible with the
//! previous implementation so existing Fabushi installations can migrate
//! without losing sessions or requested secrets. Managed-secret encryption
//! keys are lazily migrated from the historical `codex` keyring service to the
//! Mahayana-owned service.

use age::decrypt;
use age::encrypt;
use age::scrypt::Identity as ScryptIdentity;
use age::scrypt::Recipient as ScryptRecipient;
use age::secrecy::ExposeSecret;
use age::secrecy::SecretString;
use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rand::TryRngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{Ordering, compiler_fence};
use std::time::{SystemTime, UNIX_EPOCH};

const SECRETS_VERSION: u8 = 1;
const MANAGED_SECRETS_FILENAME: &str = "local.age";
const MAHAYANA_AUTH_SECRETS_FILENAME: &str = "mahayana_auth.age";
const FABUSHI_DESKTOP_AUTH_SECRETS_FILENAME: &str = "fabushi_desktop_auth_v2.age";
const FABUSHI_DESKTOP_MANAGED_SECRETS_FILENAME: &str = "fabushi_desktop_managed_v2.age";
const MAHAYANA_AUTH_KEYRING_SERVICE: &str = "mahayana-cli";
const MAHAYANA_MANAGED_KEYRING_SERVICE: &str = "mahayana-managed-secrets";
const FABUSHI_DESKTOP_AUTH_KEYRING_SERVICE: &str = "com.ombhrum.fabushi.auth.v2";
const FABUSHI_DESKTOP_MANAGED_KEYRING_SERVICE: &str = "com.ombhrum.fabushi.managed-secrets.v2";
const LEGACY_CODEX_KEYRING_SERVICE: &str = "codex";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalSecretsNamespace {
    #[default]
    ManagedSecrets,
    MahayanaAuth,
    /// Fabushi desktop auth storage. This intentionally does not read the
    /// historical `mahayana-cli` keyring ACL.
    FabushiDesktopAuth,
    /// Fabushi desktop managed/requested secrets. This intentionally does not
    /// read or migrate the historical `codex` keyring ACL.
    FabushiDesktopManagedSecrets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SecretsBackendKind {
    #[default]
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretName(String);

impl SecretName {
    pub fn new(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        anyhow::ensure!(!trimmed.is_empty(), "secret name must not be empty");
        anyhow::ensure!(
            trimmed
                .chars()
                .all(|character| character.is_ascii_uppercase()
                    || character.is_ascii_digit()
                    || character == '_'),
            "secret name must contain only A-Z, 0-9, or _"
        );
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for SecretName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretScope {
    Global,
    Environment(String),
}

impl SecretScope {
    pub fn environment(environment_id: impl Into<String>) -> Result<Self> {
        let environment_id = environment_id.into();
        let trimmed = environment_id.trim();
        anyhow::ensure!(!trimmed.is_empty(), "environment id must not be empty");
        Ok(Self::Environment(trimmed.to_owned()))
    }

    pub fn canonical_key(&self, name: &SecretName) -> String {
        match self {
            Self::Global => format!("global/{}", name.as_str()),
            Self::Environment(environment_id) => {
                format!("env/{environment_id}/{}", name.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretListEntry {
    pub scope: SecretScope,
    pub name: SecretName,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct SecretsFile {
    version: u8,
    secrets: BTreeMap<String, String>,
}

impl SecretsFile {
    fn empty() -> Self {
        Self {
            version: SECRETS_VERSION,
            secrets: BTreeMap::new(),
        }
    }
}

trait CredentialStore: fmt::Debug + Send + Sync {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>>;
    fn save(&self, service: &str, account: &str, value: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy)]
struct DefaultCredentialStore;

impl CredentialStore for DefaultCredentialStore {
    fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(service, account)
            .with_context(|| format!("failed to open keyring entry for {service}/{account}"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(anyhow::anyhow!(error)).with_context(|| {
                format!("failed to load secrets key from keyring for {service}/{account}")
            }),
        }
    }

    fn save(&self, service: &str, account: &str, value: &str) -> Result<()> {
        let entry = keyring::Entry::new(service, account)
            .with_context(|| format!("failed to open keyring entry for {service}/{account}"))?;
        entry
            .set_password(value)
            .map_err(anyhow::Error::new)
            .with_context(|| {
                format!("failed to persist secrets key in keyring for {service}/{account}")
            })
    }
}

#[derive(Clone)]
pub struct SecretsManager {
    backend: Arc<LocalSecretsBackend>,
}

impl SecretsManager {
    pub fn new(home: PathBuf, backend_kind: SecretsBackendKind) -> Self {
        Self::new_with_namespace(home, backend_kind, LocalSecretsNamespace::ManagedSecrets)
    }

    pub fn new_with_namespace(
        home: PathBuf,
        backend_kind: SecretsBackendKind,
        namespace: LocalSecretsNamespace,
    ) -> Self {
        match backend_kind {
            SecretsBackendKind::Local => Self {
                backend: Arc::new(LocalSecretsBackend::new_with_store(
                    home,
                    namespace,
                    Arc::new(DefaultCredentialStore),
                )),
            },
        }
    }

    pub fn set(&self, scope: &SecretScope, name: &SecretName, value: &str) -> Result<()> {
        self.backend.set(scope, name, value)
    }

    pub fn get(&self, scope: &SecretScope, name: &SecretName) -> Result<Option<String>> {
        self.backend.get(scope, name)
    }

    pub fn delete(&self, scope: &SecretScope, name: &SecretName) -> Result<bool> {
        self.backend.delete(scope, name)
    }

    pub fn list(&self, scope_filter: Option<&SecretScope>) -> Result<Vec<SecretListEntry>> {
        self.backend.list(scope_filter)
    }
}

#[derive(Debug)]
struct LocalSecretsBackend {
    home: PathBuf,
    namespace: LocalSecretsNamespace,
    credential_store: Arc<dyn CredentialStore>,
}

impl LocalSecretsBackend {
    fn new_with_store(
        home: PathBuf,
        namespace: LocalSecretsNamespace,
        credential_store: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            home,
            namespace,
            credential_store,
        }
    }

    fn set(&self, scope: &SecretScope, name: &SecretName, value: &str) -> Result<()> {
        anyhow::ensure!(!value.is_empty(), "secret value must not be empty");
        let mut file = self.load_file()?;
        file.secrets
            .insert(scope.canonical_key(name), value.to_owned());
        self.save_file(&file)
    }

    fn get(&self, scope: &SecretScope, name: &SecretName) -> Result<Option<String>> {
        Ok(self
            .load_file()?
            .secrets
            .get(&scope.canonical_key(name))
            .cloned())
    }

    fn delete(&self, scope: &SecretScope, name: &SecretName) -> Result<bool> {
        let mut file = self.load_file()?;
        let removed = file.secrets.remove(&scope.canonical_key(name)).is_some();
        if removed {
            self.save_file(&file)?;
        }
        Ok(removed)
    }

    fn list(&self, scope_filter: Option<&SecretScope>) -> Result<Vec<SecretListEntry>> {
        let file = self.load_file()?;
        let mut entries = Vec::new();
        for key in file.secrets.keys() {
            if let Some(entry) = parse_canonical_key(key)
                && scope_filter.is_none_or(|scope| scope == &entry.scope)
            {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    fn secrets_dir(&self) -> PathBuf {
        self.home.join("secrets")
    }

    fn secrets_path(&self) -> PathBuf {
        self.secrets_dir().join(match self.namespace {
            LocalSecretsNamespace::ManagedSecrets => MANAGED_SECRETS_FILENAME,
            LocalSecretsNamespace::MahayanaAuth => MAHAYANA_AUTH_SECRETS_FILENAME,
            LocalSecretsNamespace::FabushiDesktopAuth => FABUSHI_DESKTOP_AUTH_SECRETS_FILENAME,
            LocalSecretsNamespace::FabushiDesktopManagedSecrets => {
                FABUSHI_DESKTOP_MANAGED_SECRETS_FILENAME
            }
        })
    }

    fn keyring_service(&self) -> &'static str {
        match self.namespace {
            LocalSecretsNamespace::ManagedSecrets => MAHAYANA_MANAGED_KEYRING_SERVICE,
            LocalSecretsNamespace::MahayanaAuth => MAHAYANA_AUTH_KEYRING_SERVICE,
            LocalSecretsNamespace::FabushiDesktopAuth => FABUSHI_DESKTOP_AUTH_KEYRING_SERVICE,
            LocalSecretsNamespace::FabushiDesktopManagedSecrets => {
                FABUSHI_DESKTOP_MANAGED_KEYRING_SERVICE
            }
        }
    }

    fn legacy_keyring_service(&self) -> Option<&'static str> {
        match self.namespace {
            LocalSecretsNamespace::ManagedSecrets => Some(LEGACY_CODEX_KEYRING_SERVICE),
            LocalSecretsNamespace::MahayanaAuth
            | LocalSecretsNamespace::FabushiDesktopAuth
            | LocalSecretsNamespace::FabushiDesktopManagedSecrets => None,
        }
    }

    fn load_file(&self) -> Result<SecretsFile> {
        let path = self.secrets_path();
        if !path.exists() {
            return Ok(SecretsFile::empty());
        }
        let ciphertext = fs::read(&path)
            .with_context(|| format!("failed to read secrets file at {}", path.display()))?;
        let passphrase = self.load_or_create_passphrase()?;
        let plaintext = decrypt_with_passphrase(&ciphertext, &passphrase)?;
        let mut file: SecretsFile = serde_json::from_slice(&plaintext)
            .with_context(|| format!("failed to decode secrets file at {}", path.display()))?;
        if file.version == 0 {
            file.version = SECRETS_VERSION;
        }
        anyhow::ensure!(
            file.version <= SECRETS_VERSION,
            "secrets file version {} is newer than supported version {}",
            file.version,
            SECRETS_VERSION
        );
        Ok(file)
    }

    fn save_file(&self, file: &SecretsFile) -> Result<()> {
        let directory = self.secrets_dir();
        fs::create_dir_all(&directory)
            .with_context(|| format!("failed to create secrets dir {}", directory.display()))?;
        harden_directory_permissions(&directory)?;
        let passphrase = self.load_or_create_passphrase()?;
        let plaintext = serde_json::to_vec(file).context("failed to serialize secrets file")?;
        let ciphertext = encrypt_with_passphrase(&plaintext, &passphrase)?;
        write_file_atomically(&self.secrets_path(), &ciphertext)
    }

    fn load_or_create_passphrase(&self) -> Result<SecretString> {
        let account = compute_keyring_account(&self.home);
        if let Some(value) = self
            .credential_store
            .load(self.keyring_service(), &account)?
        {
            return Ok(SecretString::from(value));
        }

        // Existing Fabushi releases used the Codex managed-secrets service for
        // `local.age`. Read it once, then copy the opaque encryption key into a
        // Mahayana-owned keyring namespace. No secret payload is decrypted or
        // exposed during the migration itself.
        if self.secrets_path().exists()
            && let Some(legacy_service) = self.legacy_keyring_service()
            && let Some(value) = self.credential_store.load(legacy_service, &account)?
        {
            self.credential_store
                .save(self.keyring_service(), &account, &value)?;
            return Ok(SecretString::from(value));
        }

        let generated = generate_passphrase()?;
        self.credential_store
            .save(self.keyring_service(), &account, generated.expose_secret())?;
        Ok(generated)
    }
}

pub fn compute_keyring_account(home: &Path) -> String {
    let canonical = home
        .canonicalize()
        .unwrap_or_else(|_| home.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let digest = Sha256::digest(canonical.as_bytes());
    let hex = format!("{digest:x}");
    format!("secrets|{}", hex.get(..16).unwrap_or(hex.as_str()))
}

fn parse_canonical_key(key: &str) -> Option<SecretListEntry> {
    let mut parts = key.split('/');
    match parts.next()? {
        "global" => {
            let name = SecretName::new(parts.next()?).ok()?;
            parts.next().is_none().then_some(SecretListEntry {
                scope: SecretScope::Global,
                name,
            })
        }
        "env" => {
            let environment = parts.next()?.to_owned();
            let name = SecretName::new(parts.next()?).ok()?;
            parts.next().is_none().then_some(SecretListEntry {
                scope: SecretScope::Environment(environment),
                name,
            })
        }
        _ => None,
    }
}

fn generate_passphrase() -> Result<SecretString> {
    let mut bytes = [0_u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .context("failed to generate random secrets key")?;
    let encoded = BASE64_STANDARD.encode(bytes);
    wipe_bytes(&mut bytes);
    Ok(SecretString::from(encoded))
}

fn wipe_bytes(bytes: &mut [u8]) {
    for byte in bytes {
        // SAFETY: every `byte` is a valid mutable reference into the input slice.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

fn encrypt_with_passphrase(plaintext: &[u8], passphrase: &SecretString) -> Result<Vec<u8>> {
    let recipient = ScryptRecipient::new(passphrase.clone());
    encrypt(&recipient, plaintext).context("failed to encrypt secrets file")
}

fn decrypt_with_passphrase(ciphertext: &[u8], passphrase: &SecretString) -> Result<Vec<u8>> {
    let identity = ScryptIdentity::new(passphrase.clone());
    decrypt(&identity, ciphertext).context("failed to decrypt secrets file")
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .with_context(|| format!("missing parent directory for {}", path.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let file_name = path
        .file_name()
        .with_context(|| format!("missing filename for {}", path.display()))?;
    let temporary = directory.join(format!(
        ".{}.tmp-{}-{nonce}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        harden_file_permissions(&temporary)?;
        file.write_all(contents)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temporary.display()))?;
    }

    match fs::rename(&temporary, path) {
        Ok(()) => {
            harden_file_permissions(path)?;
            Ok(())
        }
        Err(initial_error) => {
            #[cfg(target_os = "windows")]
            if path.exists() {
                fs::remove_file(path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
                fs::rename(&temporary, path).with_context(|| {
                    format!(
                        "failed to replace {} from {}",
                        path.display(),
                        temporary.display()
                    )
                })?;
                return Ok(());
            }
            let _ = fs::remove_file(&temporary);
            Err(initial_error).with_context(|| format!("failed to replace {}", path.display()))
        }
    }
}

#[cfg(unix)]
fn harden_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure directory {}", path.display()))
}

#[cfg(not(unix))]
fn harden_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure file {}", path.display()))
}

#[cfg(not(unix))]
fn harden_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, PoisonError};

    #[derive(Debug, Default)]
    struct MockCredentialStore {
        values: Mutex<BTreeMap<(String, String), String>>,
    }

    impl MockCredentialStore {
        fn insert(&self, service: &str, account: &str, value: &str) {
            self.values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert((service.to_owned(), account.to_owned()), value.to_owned());
        }

        fn value(&self, service: &str, account: &str) -> Option<String> {
            self.values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&(service.to_owned(), account.to_owned()))
                .cloned()
        }
    }

    impl CredentialStore for MockCredentialStore {
        fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
            Ok(self.value(service, account))
        }

        fn save(&self, service: &str, account: &str, value: &str) -> Result<()> {
            self.insert(service, account, value);
            Ok(())
        }
    }

    fn backend(
        root: &Path,
        namespace: LocalSecretsNamespace,
        store: Arc<MockCredentialStore>,
    ) -> LocalSecretsBackend {
        LocalSecretsBackend::new_with_store(root.to_path_buf(), namespace, store)
    }

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mahayana-secrets-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn secret_names_are_strict_and_environment_scopes_are_stable() -> Result<()> {
        assert!(SecretName::new("TOKEN_1").is_ok());
        assert!(SecretName::new("token").is_err());
        assert!(SecretName::new("").is_err());
        let name = SecretName::new("TOKEN_1")?;
        let scope = SecretScope::environment("production")?;
        assert_eq!(scope.canonical_key(&name), "env/production/TOKEN_1");
        Ok(())
    }

    #[test]
    fn encrypted_store_round_trips_without_plaintext_at_rest() -> Result<()> {
        let root = temporary_root("roundtrip");
        let store = Arc::new(MockCredentialStore::default());
        let backend = backend(&root, LocalSecretsNamespace::MahayanaAuth, store);
        let name = SecretName::new("MAHAYANA_ACCOUNT_SESSION")?;
        backend.set(&SecretScope::Global, &name, "high-value-secret")?;
        assert_eq!(
            backend.get(&SecretScope::Global, &name)?,
            Some("high-value-secret".to_owned())
        );
        let ciphertext = fs::read(backend.secrets_path())?;
        assert!(!String::from_utf8_lossy(&ciphertext).contains("high-value-secret"));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn managed_store_migrates_legacy_codex_key_without_reencrypting_payload() -> Result<()> {
        let root = temporary_root("legacy");
        let store = Arc::new(MockCredentialStore::default());
        let backend = backend(&root, LocalSecretsNamespace::ManagedSecrets, store.clone());
        fs::create_dir_all(backend.secrets_dir())?;

        let account = compute_keyring_account(&root);
        let legacy_passphrase = SecretString::from("legacy-managed-secret-passphrase".to_owned());
        store.insert(
            LEGACY_CODEX_KEYRING_SERVICE,
            &account,
            legacy_passphrase.expose_secret(),
        );
        let name = SecretName::new("MAHAYANA_REQUESTED_SECRET_123")?;
        let mut secrets = SecretsFile::empty();
        secrets.secrets.insert(
            SecretScope::Global.canonical_key(&name),
            "preserved".to_owned(),
        );
        let plaintext = serde_json::to_vec(&secrets)?;
        fs::write(
            backend.secrets_path(),
            encrypt_with_passphrase(&plaintext, &legacy_passphrase)?,
        )?;

        assert_eq!(
            backend.get(&SecretScope::Global, &name)?,
            Some("preserved".to_owned())
        );
        assert_eq!(
            store.value(MAHAYANA_MANAGED_KEYRING_SERVICE, &account),
            Some(legacy_passphrase.expose_secret().to_owned())
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn namespaces_use_independent_keyring_services() -> Result<()> {
        let root = temporary_root("namespaces");
        let store = Arc::new(MockCredentialStore::default());
        let auth = backend(&root, LocalSecretsNamespace::MahayanaAuth, store.clone());
        let managed = backend(&root, LocalSecretsNamespace::ManagedSecrets, store.clone());
        let desktop_auth = backend(
            &root,
            LocalSecretsNamespace::FabushiDesktopAuth,
            store.clone(),
        );
        let desktop_managed = backend(
            &root,
            LocalSecretsNamespace::FabushiDesktopManagedSecrets,
            store.clone(),
        );
        let auth_name = SecretName::new("MAHAYANA_ACCOUNT_SESSION")?;
        let managed_name = SecretName::new("MANAGED_TOKEN")?;
        auth.set(&SecretScope::Global, &auth_name, "auth-value")?;
        managed.set(&SecretScope::Global, &managed_name, "managed-value")?;
        desktop_auth.set(&SecretScope::Global, &auth_name, "desktop-auth-value")?;
        desktop_managed.set(&SecretScope::Global, &managed_name, "desktop-managed-value")?;
        let account = compute_keyring_account(&root);
        assert!(
            store
                .value(MAHAYANA_AUTH_KEYRING_SERVICE, &account)
                .is_some()
        );
        assert!(
            store
                .value(MAHAYANA_MANAGED_KEYRING_SERVICE, &account)
                .is_some()
        );
        assert_ne!(
            store.value(MAHAYANA_AUTH_KEYRING_SERVICE, &account),
            store.value(MAHAYANA_MANAGED_KEYRING_SERVICE, &account)
        );
        assert!(
            store
                .value(FABUSHI_DESKTOP_AUTH_KEYRING_SERVICE, &account)
                .is_some()
        );
        assert!(
            store
                .value(FABUSHI_DESKTOP_MANAGED_KEYRING_SERVICE, &account)
                .is_some()
        );
        assert_eq!(
            desktop_auth
                .secrets_path()
                .file_name()
                .and_then(|name| name.to_str()),
            Some(FABUSHI_DESKTOP_AUTH_SECRETS_FILENAME)
        );
        assert_eq!(
            desktop_managed
                .secrets_path()
                .file_name()
                .and_then(|name| name.to_str()),
            Some(FABUSHI_DESKTOP_MANAGED_SECRETS_FILENAME)
        );
        assert_eq!(desktop_auth.legacy_keyring_service(), None);
        assert_eq!(desktop_managed.legacy_keyring_service(), None);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
