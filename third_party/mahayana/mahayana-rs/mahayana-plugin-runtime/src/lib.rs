//! Cross-platform Mahayana plugin host primitives.
//!
//! The marketplace is metadata-only: release bytes stay on GitHub, npm, or an
//! arbitrary immutable HTTPS origin. This crate resolves, verifies, stages,
//! installs, and activates those artifacts locally. It also owns the runtime
//! lifecycle/service graph so desktop and mobile shells share one source of
//! truth.

use base64::Engine as _;
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tar::Archive;
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

const DEFAULT_MAX_ARTIFACT_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactFormat {
    TarGz,
    Zip,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ArtifactSource {
    /// Immutable or versioned HTTPS artifact URL.
    Https { url: String },
    /// Public GitHub release asset. No marketplace binary proxy is required.
    GithubRelease {
        repository: String,
        tag: String,
        asset: String,
    },
    /// npm registry package tarball. The resolver reads registry metadata and
    /// downloads dist.tarball directly from the registry/CDN.
    Npm {
        package: String,
        version: String,
        #[serde(default = "default_npm_registry")]
        registry: String,
    },
}

fn default_npm_registry() -> String {
    "https://registry.npmjs.org".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseArtifact {
    pub id: String,
    pub runtime: String,
    pub platforms: Vec<String>,
    pub source: ArtifactSource,
    pub sha256: String,
    pub size: u64,
    pub format: ArtifactFormat,
    #[serde(default)]
    pub entry: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalReleaseManifest {
    pub schema_version: u32,
    pub protocol: String,
    pub plugin_id: String,
    pub version: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub artifacts: Vec<ReleaseArtifact>,
}

impl ExternalReleaseManifest {
    pub fn select_artifact(
        &self,
        platform: &str,
        preferred_runtimes: &[&str],
    ) -> Result<&ReleaseArtifact, RuntimeError> {
        let candidates = self
            .artifacts
            .iter()
            .filter(|artifact| artifact.platforms.iter().any(|p| p == platform))
            .collect::<Vec<_>>();
        for runtime in preferred_runtimes {
            if let Some(artifact) = candidates
                .iter()
                .copied()
                .find(|artifact| artifact.runtime == *runtime)
            {
                return Ok(artifact);
            }
        }
        candidates
            .first()
            .copied()
            .ok_or_else(|| RuntimeError::NoArtifactForPlatform(platform.to_string()))
    }

    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.schema_version != 1 || self.protocol != "mahayana.external-release.v1" {
            return Err(RuntimeError::UnsupportedReleaseProtocol);
        }
        validate_identifier(&self.plugin_id, "pluginId")?;
        if self.version.trim().is_empty() || self.artifacts.is_empty() {
            return Err(RuntimeError::InvalidRelease(
                "version/artifacts missing".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for artifact in &self.artifacts {
            if !ids.insert(&artifact.id) {
                return Err(RuntimeError::InvalidRelease(format!(
                    "duplicate artifact id {}",
                    artifact.id
                )));
            }
            if artifact.size == 0 || artifact.sha256.len() != 64 {
                return Err(RuntimeError::InvalidRelease(format!(
                    "artifact {} has invalid digest/size",
                    artifact.id
                )));
            }
            if !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(RuntimeError::InvalidRelease(format!(
                    "artifact {} has invalid sha256",
                    artifact.id
                )));
            }
            if artifact.platforms.is_empty() || artifact.runtime.trim().is_empty() {
                return Err(RuntimeError::InvalidRelease(format!(
                    "artifact {} has no platform/runtime",
                    artifact.id
                )));
            }
            validate_source(&artifact.source)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedArtifact {
    pub url: Url,
    pub expected_sha256: String,
    pub expected_size: u64,
    pub format: ArtifactFormat,
}

#[derive(Clone)]
pub struct ArtifactResolver {
    client: Client,
    max_bytes: usize,
}

impl ArtifactResolver {
    pub fn new() -> Result<Self, RuntimeError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::limited(8))
            .user_agent("Mahayana-Rust-Host/1")
            .build()
            .map_err(|error| RuntimeError::Transport(error.to_string()))?;
        Ok(Self {
            client,
            max_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
        })
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn resolve(&self, artifact: &ReleaseArtifact) -> Result<ResolvedArtifact, RuntimeError> {
        let url = match &artifact.source {
            ArtifactSource::Https { url } => parse_public_https(url)?,
            ArtifactSource::GithubRelease {
                repository,
                tag,
                asset,
            } => {
                validate_github_repository(repository)?;
                validate_path_segment(tag, "GitHub release tag")?;
                validate_path_segment(asset, "GitHub release asset")?;
                parse_public_https(&format!(
                    "https://github.com/{repository}/releases/download/{tag}/{asset}"
                ))?
            }
            ArtifactSource::Npm {
                package,
                version,
                registry,
            } => self.resolve_npm_tarball(package, version, registry)?,
        };
        Ok(ResolvedArtifact {
            url,
            expected_sha256: artifact.sha256.to_ascii_lowercase(),
            expected_size: artifact.size,
            format: artifact.format.clone(),
        })
    }

    fn resolve_npm_tarball(
        &self,
        package: &str,
        version: &str,
        registry: &str,
    ) -> Result<Url, RuntimeError> {
        validate_npm_package(package)?;
        validate_path_segment(version, "npm version")?;
        let base = parse_public_https(registry)?;
        let encoded_package = package.replace('/', "%2f");
        let metadata_url = base
            .join(&format!("{encoded_package}/{version}"))
            .map_err(|error| RuntimeError::InvalidSource(error.to_string()))?;
        let response = self
            .client
            .get(metadata_url)
            .send()
            .map_err(|error| RuntimeError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(RuntimeError::HttpStatus(response.status().as_u16()));
        }
        let metadata: serde_json::Value = response
            .json()
            .map_err(|error| RuntimeError::InvalidSource(error.to_string()))?;
        let tarball = metadata
            .pointer("/dist/tarball")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| RuntimeError::InvalidSource("npm dist.tarball missing".into()))?;
        parse_public_https(tarball)
    }

    pub fn download_verified(&self, artifact: &ReleaseArtifact) -> Result<Vec<u8>, RuntimeError> {
        let resolved = self.resolve(artifact)?;
        let expected_size =
            usize::try_from(resolved.expected_size).map_err(|_| RuntimeError::ArtifactTooLarge)?;
        if expected_size == 0 || expected_size > self.max_bytes {
            return Err(RuntimeError::ArtifactTooLarge);
        }
        let response = self
            .client
            .get(resolved.url)
            .send()
            .map_err(|error| RuntimeError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(RuntimeError::HttpStatus(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length != resolved.expected_size)
        {
            return Err(RuntimeError::SizeMismatch);
        }
        let mut bytes = Vec::with_capacity(expected_size.min(4 * 1024 * 1024));
        response
            .take((self.max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(RuntimeError::Io)?;
        if bytes.len() > self.max_bytes {
            return Err(RuntimeError::ArtifactTooLarge);
        }
        verify_bytes(&bytes, &resolved.expected_sha256, resolved.expected_size)?;
        Ok(bytes)
    }

    pub fn install_npm_dependencies(
        &self,
        package_root: &Path,
        registry: &str,
    ) -> Result<NpmInstallSummary, RuntimeError> {
        parse_public_https(registry)?;
        let mut state = NpmInstallState::default();
        self.install_npm_dependencies_recursive(package_root, registry, 0, &mut state)?;
        Ok(NpmInstallSummary {
            package_count: state.package_count,
            downloaded_bytes: state.downloaded_bytes,
        })
    }

    fn install_npm_dependencies_recursive(
        &self,
        package_root: &Path,
        registry: &str,
        depth: usize,
        state: &mut NpmInstallState,
    ) -> Result<(), RuntimeError> {
        if depth > 20 {
            return Err(RuntimeError::NpmDependencyLimit(
                "dependency depth exceeds 20".into(),
            ));
        }
        let manifest_path = package_root.join("package.json");
        if !manifest_path.is_file() {
            return Ok(());
        }
        let source = fs::read_to_string(&manifest_path)?;
        let manifest: NpmInstalledPackageJson = serde_json::from_str(&source).map_err(|error| {
            RuntimeError::InvalidNpmPackage {
                path: manifest_path.clone(),
                message: error.to_string(),
            }
        })?;
        let optional = manifest
            .optional_dependencies
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut dependencies = manifest.dependencies;
        dependencies.extend(manifest.optional_dependencies);
        for (requested_name, requirement) in dependencies {
            if npm_builtin_is_host_provided(&requested_name) {
                continue;
            }
            let result = self.install_one_npm_dependency(
                package_root,
                registry,
                &requested_name,
                &requirement,
                depth,
                state,
            );
            if optional.contains(&requested_name) && result.is_err() {
                continue;
            }
            result?;
        }
        Ok(())
    }

    fn install_one_npm_dependency(
        &self,
        package_root: &Path,
        registry: &str,
        requested_name: &str,
        requirement: &str,
        depth: usize,
        state: &mut NpmInstallState,
    ) -> Result<(), RuntimeError> {
        if state.package_count >= 512 || state.downloaded_bytes > 250 * 1024 * 1024 {
            return Err(RuntimeError::NpmDependencyLimit(
                "dependency graph exceeds local limits".into(),
            ));
        }
        let (actual_name, actual_requirement) = npm_dependency_target(requested_name, requirement)?;
        let resolved = self.resolve_npm_dependency(registry, &actual_name, &actual_requirement)?;
        let destination = npm_dependency_destination(package_root, requested_name)?;
        if destination.join("package.json").is_file() {
            let existing: NpmInstalledPackageJson = serde_json::from_str(&fs::read_to_string(
                destination.join("package.json"),
            )?)
            .map_err(|error| RuntimeError::InvalidNpmPackage {
                path: destination.join("package.json"),
                message: error.to_string(),
            })?;
            if existing.version.as_deref() == Some(resolved.version.as_str()) {
                return self.install_npm_dependencies_recursive(
                    &destination,
                    registry,
                    depth + 1,
                    state,
                );
            }
        }
        if destination.exists() {
            fs::remove_dir_all(&destination)?;
        }
        fs::create_dir_all(&destination)?;
        let bytes = self.download_bounded(&resolved.tarball, 50 * 1024 * 1024)?;
        verify_npm_integrity(&bytes, resolved.integrity.as_deref(), resolved.shasum.as_deref())?;
        state.package_count += 1;
        state.downloaded_bytes = state.downloaded_bytes.saturating_add(bytes.len() as u64);
        extract_artifact(&bytes, &ArtifactFormat::TarGz, &destination)?;
        normalize_npm_package_root(&destination)?;
        self.install_npm_dependencies_recursive(&destination, registry, depth + 1, state)
    }

    fn resolve_npm_dependency(
        &self,
        registry: &str,
        package: &str,
        requirement: &str,
    ) -> Result<ResolvedNpmDependency, RuntimeError> {
        validate_npm_package(package)?;
        let registry = parse_public_https(registry)?;
        let encoded = package.replace('/', "%2f");
        let url = registry
            .join(&encoded)
            .map_err(|error| RuntimeError::InvalidSource(error.to_string()))?;
        let response = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .send()
            .map_err(|error| RuntimeError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(RuntimeError::HttpStatus(response.status().as_u16()));
        }
        let metadata: NpmFullPackageMetadata = response
            .json()
            .map_err(|error| RuntimeError::InvalidSource(error.to_string()))?;
        let version = select_npm_version(&metadata, requirement).ok_or_else(|| {
            RuntimeError::NpmVersionNotFound {
                package: package.to_string(),
                requirement: requirement.to_string(),
            }
        })?;
        let version_metadata = metadata.versions.get(&version).ok_or_else(|| {
            RuntimeError::NpmVersionNotFound {
                package: package.to_string(),
                requirement: requirement.to_string(),
            }
        })?;
        parse_public_https(&version_metadata.dist.tarball)?;
        Ok(ResolvedNpmDependency {
            version,
            tarball: version_metadata.dist.tarball.clone(),
            integrity: version_metadata.dist.integrity.clone(),
            shasum: version_metadata.dist.shasum.clone(),
        })
    }

    fn download_bounded(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, RuntimeError> {
        let url = parse_public_https(url)?;
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|error| RuntimeError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(RuntimeError::HttpStatus(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(RuntimeError::ArtifactTooLarge);
        }
        let mut bytes = Vec::new();
        response
            .take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(RuntimeError::ArtifactTooLarge);
        }
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmInstallSummary {
    pub package_count: usize,
    pub downloaded_bytes: u64,
}

#[derive(Default)]
struct NpmInstallState {
    package_count: usize,
    downloaded_bytes: u64,
}

#[derive(Debug)]
struct ResolvedNpmDependency {
    version: String,
    tarball: String,
    integrity: Option<String>,
    shasum: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NpmInstalledPackageJson {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    optional_dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct NpmFullPackageMetadata {
    #[serde(default, rename = "dist-tags")]
    dist_tags: BTreeMap<String, String>,
    #[serde(default)]
    versions: BTreeMap<String, NpmFullVersionMetadata>,
}

#[derive(Debug, Deserialize)]
struct NpmFullVersionMetadata {
    dist: NpmFullDistMetadata,
}

#[derive(Debug, Deserialize)]
struct NpmFullDistMetadata {
    tarball: String,
    #[serde(default)]
    integrity: Option<String>,
    #[serde(default)]
    shasum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginPointer {
    pub plugin_id: String,
    pub version: String,
    pub artifact_id: String,
    pub artifact_sha256: String,
    pub runtime: String,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub requested_permissions: Vec<String>,
    pub installed_path: String,
}

pub struct PluginInstaller {
    root: PathBuf,
    resolver: ArtifactResolver,
}

impl PluginInstaller {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        Ok(Self {
            root: root.into(),
            resolver: ArtifactResolver::new()?,
        })
    }

    pub fn with_resolver(root: impl Into<PathBuf>, resolver: ArtifactResolver) -> Self {
        Self {
            root: root.into(),
            resolver,
        }
    }

    pub fn install(
        &self,
        release: &ExternalReleaseManifest,
        platform: &str,
        preferred_runtimes: &[&str],
    ) -> Result<InstalledPluginPointer, RuntimeError> {
        release.validate()?;
        let artifact = release.select_artifact(platform, preferred_runtimes)?;
        let bytes = self.resolver.download_verified(artifact)?;
        self.install_verified_bytes(release, artifact, &bytes)
    }

    pub fn install_verified_bytes(
        &self,
        release: &ExternalReleaseManifest,
        artifact: &ReleaseArtifact,
        bytes: &[u8],
    ) -> Result<InstalledPluginPointer, RuntimeError> {
        release.validate()?;
        if !release
            .artifacts
            .iter()
            .any(|candidate| candidate.id == artifact.id)
        {
            return Err(RuntimeError::InvalidRelease(
                "artifact does not belong to release".into(),
            ));
        }
        verify_bytes(bytes, &artifact.sha256, artifact.size)?;
        let plugin_root = self.root.join(&release.plugin_id);
        let versions_root = plugin_root.join("versions");
        fs::create_dir_all(&versions_root).map_err(RuntimeError::Io)?;
        let final_dir = versions_root.join(&release.version).join(&artifact.id);
        if final_dir.exists() {
            let pointer = InstalledPluginPointer {
                plugin_id: release.plugin_id.clone(),
                version: release.version.clone(),
                artifact_id: artifact.id.clone(),
                artifact_sha256: artifact.sha256.to_ascii_lowercase(),
                runtime: artifact.runtime.clone(),
                entry: artifact.entry.clone(),
                requested_permissions: release.permissions.clone(),
                installed_path: final_dir.to_string_lossy().into_owned(),
            };
            self.activate(&plugin_root, &pointer)?;
            return Ok(pointer);
        }

        let staging = plugin_root
            .join(".staging")
            .join(Uuid::new_v4().to_string());
        fs::create_dir_all(&staging).map_err(RuntimeError::Io)?;
        let extraction = extract_artifact(bytes, &artifact.format, &staging);
        if let Err(error) = extraction {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        if let ArtifactSource::Npm { registry, .. } = &artifact.source {
            if let Err(error) = normalize_npm_package_root(&staging).and_then(|_| {
                ArtifactResolver::new()?.install_npm_dependencies(&staging, registry)?;
                Ok(())
            }) {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        }
        if let Some(entry) = &artifact.entry {
            let entry = safe_relative_path(entry)?;
            if !staging.join(entry).exists() {
                let _ = fs::remove_dir_all(&staging);
                return Err(RuntimeError::MissingEntry(artifact.entry.clone().unwrap()));
            }
        }
        if let Some(parent) = final_dir.parent() {
            fs::create_dir_all(parent).map_err(RuntimeError::Io)?;
        }
        fs::rename(&staging, &final_dir).map_err(RuntimeError::Io)?;
        let pointer = InstalledPluginPointer {
            plugin_id: release.plugin_id.clone(),
            version: release.version.clone(),
            artifact_id: artifact.id.clone(),
            artifact_sha256: artifact.sha256.to_ascii_lowercase(),
            runtime: artifact.runtime.clone(),
            entry: artifact.entry.clone(),
            requested_permissions: release.permissions.clone(),
            installed_path: final_dir.to_string_lossy().into_owned(),
        };
        self.activate(&plugin_root, &pointer)?;
        Ok(pointer)
    }

    fn activate(
        &self,
        plugin_root: &Path,
        pointer: &InstalledPluginPointer,
    ) -> Result<(), RuntimeError> {
        fs::create_dir_all(plugin_root).map_err(RuntimeError::Io)?;
        let bytes = serde_json::to_vec_pretty(pointer)
            .map_err(|error| RuntimeError::InvalidRelease(error.to_string()))?;
        let temp = plugin_root.join(format!("active.{}.json", Uuid::new_v4()));
        fs::write(&temp, bytes).map_err(RuntimeError::Io)?;
        let active = plugin_root.join("active.json");
        #[cfg(windows)]
        if active.exists() {
            fs::remove_file(&active).map_err(RuntimeError::Io)?;
        }
        fs::rename(&temp, &active).map_err(RuntimeError::Io)?;
        Ok(())
    }

    pub fn active(&self, plugin_id: &str) -> Result<Option<InstalledPluginPointer>, RuntimeError> {
        validate_identifier(plugin_id, "pluginId")?;
        let path = self.root.join(plugin_id).join("active.json");
        let source = match fs::read(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(RuntimeError::Io(error)),
        };
        serde_json::from_slice(&source)
            .map(Some)
            .map_err(|error| RuntimeError::InvalidRelease(error.to_string()))
    }
}

/// Owns cleanup callbacks for one plugin instance. Every runtime adapter
/// (JS/WASM/MCP/native) registers timers, service registrations, workers,
/// subscriptions, child handles, and other resources here so unload is
/// deterministic and idempotent.
#[derive(Default)]
pub struct EffectScope {
    disposers: Vec<Box<dyn FnOnce() + Send + 'static>>,
    disposed: bool,
}

impl EffectScope {
    pub fn register(
        &mut self,
        disposer: impl FnOnce() + Send + 'static,
    ) -> Result<(), RuntimeError> {
        if self.disposed {
            return Err(RuntimeError::EffectScopeDisposed);
        }
        self.disposers.push(Box::new(disposer));
        Ok(())
    }

    pub fn dispose(&mut self) {
        if self.disposed {
            return;
        }
        self.disposed = true;
        while let Some(disposer) = self.disposers.pop() {
            disposer();
        }
    }

    pub fn is_disposed(&self) -> bool {
        self.disposed
    }
}

impl Drop for EffectScope {
    fn drop(&mut self) {
        self.dispose();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginState {
    Installed,
    Pending,
    Loading,
    Active,
    Failed,
    Unloading,
    Disposed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRequirement {
    pub service: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceProvider {
    pub plugin_id: String,
    pub generation: u64,
}

#[derive(Debug, Default)]
pub struct ServiceRegistry {
    providers: HashMap<String, ServiceProvider>,
    next_generation: u64,
}

impl ServiceRegistry {
    pub fn provide(&mut self, service: impl Into<String>, plugin_id: impl Into<String>) -> u64 {
        self.next_generation = self.next_generation.saturating_add(1).max(1);
        let generation = self.next_generation;
        self.providers.insert(
            service.into(),
            ServiceProvider {
                plugin_id: plugin_id.into(),
                generation,
            },
        );
        generation
    }

    pub fn remove_provider(&mut self, plugin_id: &str) -> Vec<String> {
        let removed = self
            .providers
            .iter()
            .filter(|(_, provider)| provider.plugin_id == plugin_id)
            .map(|(service, _)| service.clone())
            .collect::<Vec<_>>();
        for service in &removed {
            self.providers.remove(service);
        }
        removed
    }

    pub fn remove_service(&mut self, service: &str, plugin_id: &str) -> bool {
        if self
            .providers
            .get(service)
            .is_some_and(|provider| provider.plugin_id == plugin_id)
        {
            self.providers.remove(service);
            true
        } else {
            false
        }
    }

    pub fn get(&self, service: &str) -> Option<&ServiceProvider> {
        self.providers.get(service)
    }
}

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub plugin_id: String,
    pub enabled: bool,
    pub state: PluginState,
    pub provides: Vec<String>,
    pub injects: Vec<ServiceRequirement>,
    observed_generations: BTreeMap<String, u64>,
}

impl PluginEntry {
    pub fn new(
        plugin_id: impl Into<String>,
        provides: Vec<String>,
        injects: Vec<ServiceRequirement>,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            enabled: true,
            state: PluginState::Installed,
            provides,
            injects,
            observed_generations: BTreeMap::new(),
        }
    }

    fn dependencies_ready(&self, services: &ServiceRegistry) -> bool {
        self.injects
            .iter()
            .filter(|requirement| requirement.required)
            .all(|requirement| services.get(&requirement.service).is_some())
    }

    fn dependency_epoch_changed(&self, services: &ServiceRegistry) -> bool {
        self.injects.iter().any(|requirement| {
            let current = services
                .get(&requirement.service)
                .map(|provider| provider.generation)
                .unwrap_or(0);
            self.observed_generations
                .get(&requirement.service)
                .copied()
                .unwrap_or(0)
                != current
        })
    }

    fn snapshot_dependencies(&mut self, services: &ServiceRegistry) {
        self.observed_generations = self
            .injects
            .iter()
            .map(|requirement| {
                (
                    requirement.service.clone(),
                    services
                        .get(&requirement.service)
                        .map(|provider| provider.generation)
                        .unwrap_or(0),
                )
            })
            .collect();
    }
}

#[derive(Debug, Default)]
pub struct PluginSupervisor {
    pub services: ServiceRegistry,
    pub entries: HashMap<String, PluginEntry>,
}

impl PluginSupervisor {
    pub fn insert(&mut self, entry: PluginEntry) {
        self.entries.insert(entry.plugin_id.clone(), entry);
    }

    /// Reconciles the graph to a stable state. Runtime-specific load/unload
    /// hooks intentionally live above this primitive; this method owns the
    /// deterministic state/service semantics shared by JS, WASM, MCP, and
    /// native runtimes.
    pub fn reconcile(&mut self) {
        // First unload entries whose provider epoch changed or disappeared.
        let to_unload = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.state == PluginState::Active
                    && (!entry.enabled
                        || !entry.dependencies_ready(&self.services)
                        || entry.dependency_epoch_changed(&self.services))
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in to_unload {
            self.services.remove_provider(&id);
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.state = if entry.enabled {
                    PluginState::Pending
                } else {
                    PluginState::Disposed
                };
            }
        }

        // Repeatedly activate entries made ready by providers from this round.
        loop {
            let ready = self
                .entries
                .iter()
                .filter(|(_, entry)| {
                    entry.enabled
                        && matches!(entry.state, PluginState::Installed | PluginState::Pending)
                        && entry.dependencies_ready(&self.services)
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            if ready.is_empty() {
                break;
            }
            let mut changed = false;
            for id in ready {
                let provides = self
                    .entries
                    .get(&id)
                    .map(|entry| entry.provides.clone())
                    .unwrap_or_default();
                for service in provides {
                    self.services.provide(service, id.clone());
                }
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.snapshot_dependencies(&self.services);
                    entry.state = PluginState::Active;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        for entry in self.entries.values_mut() {
            if entry.enabled
                && !entry.dependencies_ready(&self.services)
                && entry.state != PluginState::Active
            {
                entry.state = PluginState::Pending;
            }
        }
    }

    pub fn replace_provider_generation(&mut self, service: &str, plugin_id: &str) {
        self.services
            .provide(service.to_string(), plugin_id.to_string());
        self.reconcile();
    }
}

fn npm_builtin_is_host_provided(package: &str) -> bool {
    matches!(
        package,
        "@deepseek-ai/cordis" | "@deepseek-ai/dsh-tools" | "@deepseek-ai/dsh-llm"
    )
}

fn npm_dependency_target(
    requested_name: &str,
    requirement: &str,
) -> Result<(String, String), RuntimeError> {
    validate_npm_package(requested_name)?;
    let requirement = requirement.trim();
    if let Some(alias) = requirement.strip_prefix("npm:") {
        let split = if alias.starts_with('@') {
            let slash = alias
                .find('/')
                .ok_or_else(|| RuntimeError::InvalidSource("invalid npm alias".into()))?;
            alias[slash + 1..]
                .rfind('@')
                .map(|index| slash + 1 + index)
        } else {
            alias.rfind('@')
        };
        let (package, version) = match split {
            Some(index) if index > 0 => (&alias[..index], &alias[index + 1..]),
            _ => (alias, "latest"),
        };
        validate_npm_package(package)?;
        if version.trim().is_empty() {
            return Err(RuntimeError::InvalidSource("npm alias has empty version".into()));
        }
        return Ok((package.to_string(), version.to_string()));
    }
    if requirement.starts_with("file:")
        || requirement.starts_with("git:")
        || requirement.starts_with("git+")
        || requirement.starts_with("workspace:")
        || requirement.starts_with("http:")
        || requirement.starts_with("https:")
    {
        return Err(RuntimeError::InvalidSource(format!(
            "npm dependency source is not portable: {requirement}"
        )));
    }
    Ok((requested_name.to_string(), requirement.to_string()))
}

fn npm_dependency_destination(root: &Path, package: &str) -> Result<PathBuf, RuntimeError> {
    validate_npm_package(package)?;
    let mut destination = root.join("node_modules");
    for segment in package.split('/') {
        if segment.is_empty() || matches!(segment, "." | "..") {
            return Err(RuntimeError::InvalidSource(format!(
                "invalid npm package path: {package}"
            )));
        }
        destination.push(segment);
    }
    Ok(destination)
}

fn select_npm_version(metadata: &NpmFullPackageMetadata, requirement: &str) -> Option<String> {
    let requirement = requirement.trim();
    if requirement.is_empty() || requirement == "*" || requirement == "latest" {
        if let Some(version) = metadata.dist_tags.get("latest") {
            return Some(version.clone());
        }
    }
    if let Some(version) = metadata.dist_tags.get(requirement) {
        return Some(version.clone());
    }
    if metadata.versions.contains_key(requirement) {
        return Some(requirement.to_string());
    }
    let normalized = requirement
        .split("||")
        .map(str::trim)
        .map(|branch| branch.split_whitespace().collect::<Vec<_>>().join(", "))
        .collect::<Vec<_>>();
    let requirements = normalized
        .iter()
        .filter_map(|branch| VersionReq::parse(branch).ok())
        .collect::<Vec<_>>();
    metadata
        .versions
        .keys()
        .filter_map(|raw| Version::parse(raw).ok().map(|version| (raw, version)))
        .filter(|(_, version)| requirements.iter().any(|req| req.matches(version)))
        .max_by(|(_, left), (_, right)| left.cmp(right))
        .map(|(raw, _)| raw.clone())
}

fn verify_npm_integrity(
    bytes: &[u8],
    integrity: Option<&str>,
    shasum: Option<&str>,
) -> Result<(), RuntimeError> {
    if let Some(integrity) = integrity {
        let mut supported = false;
        for token in integrity.split_whitespace() {
            if let Some(value) = token.strip_prefix("sha512-") {
                supported = true;
                let expected = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .map_err(|error| RuntimeError::InvalidSource(error.to_string()))?;
                if expected.as_slice() == Sha512::digest(bytes).as_slice() {
                    return Ok(());
                }
            } else if let Some(value) = token.strip_prefix("sha256-") {
                supported = true;
                let expected = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .map_err(|error| RuntimeError::InvalidSource(error.to_string()))?;
                if expected.as_slice() == Sha256::digest(bytes).as_slice() {
                    return Ok(());
                }
            }
        }
        if supported {
            return Err(RuntimeError::NpmIntegrityMismatch);
        }
    }
    if let Some(shasum) = shasum {
        let actual = format!("{:x}", Sha1::digest(bytes));
        if actual.eq_ignore_ascii_case(shasum.trim()) {
            return Ok(());
        }
        return Err(RuntimeError::NpmIntegrityMismatch);
    }
    Err(RuntimeError::InvalidSource(
        "npm package metadata has no supported integrity digest".into(),
    ))
}

fn normalize_npm_package_root(root: &Path) -> Result<(), RuntimeError> {
    let package = root.join("package");
    if !package.is_dir() || root.join("package.json").is_file() {
        return Ok(());
    }
    let entries = fs::read_dir(&package)?.collect::<Result<Vec<_>, _>>()?;
    for entry in entries {
        let target = root.join(entry.file_name());
        if target.exists() {
            return Err(RuntimeError::Archive(format!(
                "npm package root collision: {}",
                target.display()
            )));
        }
        fs::rename(entry.path(), target)?;
    }
    fs::remove_dir(&package)?;
    Ok(())
}

fn extract_artifact(
    bytes: &[u8],
    format: &ArtifactFormat,
    destination: &Path,
) -> Result<(), RuntimeError> {
    match format {
        ArtifactFormat::TarGz => {
            let decoder = GzDecoder::new(Cursor::new(bytes));
            let mut archive = Archive::new(decoder);
            for entry in archive.entries().map_err(RuntimeError::Io)? {
                let mut entry = entry.map_err(RuntimeError::Io)?;
                let entry_type = entry.header().entry_type();
                if !entry_type.is_file() && !entry_type.is_dir() {
                    return Err(RuntimeError::Archive(
                        "archive links and special files are not allowed".into(),
                    ));
                }
                let path = entry.path().map_err(RuntimeError::Io)?.into_owned();
                let safe = safe_archive_path(&path)?;
                let target = destination.join(safe);
                if entry_type.is_dir() {
                    fs::create_dir_all(&target).map_err(RuntimeError::Io)?;
                } else {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(RuntimeError::Io)?;
                    }
                    entry.unpack(&target).map_err(RuntimeError::Io)?;
                }
            }
        }
        ArtifactFormat::Zip => {
            let mut archive = ZipArchive::new(Cursor::new(bytes))
                .map_err(|error| RuntimeError::Archive(error.to_string()))?;
            for index in 0..archive.len() {
                let mut entry = archive
                    .by_index(index)
                    .map_err(|error| RuntimeError::Archive(error.to_string()))?;
                let path = entry
                    .enclosed_name()
                    .ok_or_else(|| RuntimeError::Archive("unsafe zip path".into()))?
                    .to_path_buf();
                let target = destination.join(path);
                if entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    return Err(RuntimeError::Archive(
                        "zip symlinks are not allowed".into(),
                    ));
                }
                if entry.is_dir() {
                    fs::create_dir_all(&target).map_err(RuntimeError::Io)?;
                } else {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(RuntimeError::Io)?;
                    }
                    let mut output = File::create(&target).map_err(RuntimeError::Io)?;
                    io::copy(&mut entry, &mut output).map_err(RuntimeError::Io)?;
                }
            }
        }
        ArtifactFormat::Directory => {
            return Err(RuntimeError::Archive(
                "directory artifacts cannot be installed from network bytes".into(),
            ));
        }
    }
    Ok(())
}

fn verify_bytes(
    bytes: &[u8],
    expected_sha256: &str,
    expected_size: u64,
) -> Result<(), RuntimeError> {
    if bytes.len() as u64 != expected_size {
        return Err(RuntimeError::SizeMismatch);
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(RuntimeError::DigestMismatch);
    }
    Ok(())
}

fn validate_source(source: &ArtifactSource) -> Result<(), RuntimeError> {
    match source {
        ArtifactSource::Https { url } => {
            parse_public_https(url)?;
        }
        ArtifactSource::GithubRelease {
            repository,
            tag,
            asset,
        } => {
            validate_github_repository(repository)?;
            validate_path_segment(tag, "GitHub release tag")?;
            validate_path_segment(asset, "GitHub release asset")?;
        }
        ArtifactSource::Npm {
            package,
            version,
            registry,
        } => {
            validate_npm_package(package)?;
            validate_path_segment(version, "npm version")?;
            parse_public_https(registry)?;
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, kind: &'static str) -> Result<(), RuntimeError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(RuntimeError::InvalidIdentifier {
            kind,
            value: value.to_string(),
        })
    }
}

fn validate_npm_package(value: &str) -> Result<(), RuntimeError> {
    if value.is_empty()
        || value.len() > 214
        || value.contains(char::is_whitespace)
        || value.contains("..")
        || value.contains('\\')
    {
        return Err(RuntimeError::InvalidSource(
            "invalid npm package name".into(),
        ));
    }
    if value.starts_with('@') {
        let Some((scope, name)) = value.split_once('/') else {
            return Err(RuntimeError::InvalidSource(
                "invalid scoped npm package".into(),
            ));
        };
        if scope.len() < 2 || name.is_empty() || name.contains('/') {
            return Err(RuntimeError::InvalidSource(
                "invalid scoped npm package".into(),
            ));
        }
    } else if value.contains('/') {
        return Err(RuntimeError::InvalidSource(
            "invalid npm package name".into(),
        ));
    }
    Ok(())
}

fn validate_github_repository(value: &str) -> Result<(), RuntimeError> {
    let Some((owner, repo)) = value.split_once('/') else {
        return Err(RuntimeError::InvalidSource(
            "GitHub repository must be owner/repo".into(),
        ));
    };
    if owner.is_empty()
        || repo.is_empty()
        || repo.contains('/')
        || !owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(RuntimeError::InvalidSource(
            "invalid GitHub repository".into(),
        ));
    }
    Ok(())
}

fn validate_path_segment(value: &str, kind: &'static str) -> Result<(), RuntimeError> {
    if value.is_empty()
        || value.len() > 255
        || value.contains(['/', '\\', '\r', '\n'])
        || value == "."
        || value == ".."
    {
        Err(RuntimeError::InvalidSource(format!("invalid {kind}")))
    } else {
        Ok(())
    }
}

fn parse_public_https(value: &str) -> Result<Url, RuntimeError> {
    let url = Url::parse(value).map_err(|error| RuntimeError::InvalidSource(error.to_string()))?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() {
        return Err(RuntimeError::InvalidSource(
            "artifact URLs must be public HTTPS URLs".into(),
        ));
    }
    Ok(url)
}

fn safe_relative_path(value: &str) -> Result<PathBuf, RuntimeError> {
    let path = Path::new(value.trim_start_matches("./"));
    safe_archive_path(path)
}

fn safe_archive_path(path: &Path) -> Result<PathBuf, RuntimeError> {
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(RuntimeError::Archive("unsafe archive path".into()));
    }
    Ok(path.to_path_buf())
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionStore {
    #[serde(default)]
    grants: BTreeMap<String, BTreeSet<String>>,
}

/// Persistent least-privilege grants for installed plugins.
///
/// Granting is allowed only for permissions declared by the plugin release;
/// callers can therefore expose this API to platform shells without allowing
/// those shells to invent new plugin privileges.
pub struct PermissionManager {
    path: PathBuf,
    store: PermissionStore,
}

impl PermissionManager {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        let path = path.into();
        let store = if path.is_file() {
            let source = fs::read_to_string(&path)?;
            serde_json::from_str(&source).map_err(|error| {
                RuntimeError::Io(io::Error::new(io::ErrorKind::InvalidData, error))
            })?
        } else {
            PermissionStore::default()
        };
        Ok(Self { path, store })
    }

    pub fn grants_for(&self, plugin_id: &str) -> Vec<String> {
        self.store
            .grants
            .get(plugin_id)
            .map(|grants| grants.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn is_granted(&self, plugin_id: &str, permission: &str) -> bool {
        self.store
            .grants
            .get(plugin_id)
            .is_some_and(|grants| grants.contains(permission))
    }

    pub fn grant(
        &mut self,
        plugin_id: &str,
        requested: &[String],
        permission: &str,
    ) -> Result<(), RuntimeError> {
        if !requested.iter().any(|candidate| candidate == permission) {
            return Err(RuntimeError::PermissionNotRequested {
                plugin_id: plugin_id.to_string(),
                permission: permission.to_string(),
            });
        }
        self.store
            .grants
            .entry(plugin_id.to_string())
            .or_default()
            .insert(permission.to_string());
        self.persist()
    }

    pub fn revoke(&mut self, plugin_id: &str, permission: &str) -> Result<(), RuntimeError> {
        let remove_plugin = if let Some(grants) = self.store.grants.get_mut(plugin_id) {
            grants.remove(permission);
            grants.is_empty()
        } else {
            false
        };
        if remove_plugin {
            self.store.grants.remove(plugin_id);
        }
        self.persist()
    }

    pub fn retain_requested(
        &mut self,
        plugin_id: &str,
        requested: &[String],
    ) -> Result<(), RuntimeError> {
        let requested = requested.iter().cloned().collect::<BTreeSet<_>>();
        let remove_plugin = if let Some(grants) = self.store.grants.get_mut(plugin_id) {
            grants.retain(|permission| requested.contains(permission));
            grants.is_empty()
        } else {
            false
        };
        if remove_plugin {
            self.store.grants.remove(plugin_id);
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), RuntimeError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&self.store).map_err(|error| {
            RuntimeError::Io(io::Error::new(io::ErrorKind::InvalidData, error))
        })?;
        let temporary = self.path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        fs::write(&temporary, bytes)?;
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(temporary, &self.path)?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("unsupported marketplace release protocol")]
    UnsupportedReleaseProtocol,
    #[error("invalid release: {0}")]
    InvalidRelease(String),
    #[error("invalid {kind}: {value}")]
    InvalidIdentifier { kind: &'static str, value: String },
    #[error("invalid artifact source: {0}")]
    InvalidSource(String),
    #[error("no compatible artifact for platform {0}")]
    NoArtifactForPlatform(String),
    #[error("artifact exceeds configured size limit")]
    ArtifactTooLarge,
    #[error("artifact size mismatch")]
    SizeMismatch,
    #[error("artifact SHA-256 mismatch")]
    DigestMismatch,
    #[error("npm package integrity mismatch")]
    NpmIntegrityMismatch,
    #[error("npm dependency limit exceeded: {0}")]
    NpmDependencyLimit(String),
    #[error("invalid npm package manifest at {path}: {message}")]
    InvalidNpmPackage { path: PathBuf, message: String },
    #[error("no npm version satisfies {package}@{requirement}")]
    NpmVersionNotFound { package: String, requirement: String },
    #[error("plugin {plugin_id} did not request permission {permission}")]
    PermissionNotRequested {
        plugin_id: String,
        permission: String,
    },
    #[error("artifact entry is missing: {0}")]
    MissingEntry(String),
    #[error("plugin effect scope is already disposed")]
    EffectScopeDisposed,
    #[error("HTTP status {0}")]
    HttpStatus(u16),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("archive error: {0}")]
    Archive(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *contents).unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn service_dependency_waits_then_activates() {
        let mut supervisor = PluginSupervisor::default();
        supervisor.insert(PluginEntry::new(
            "consumer",
            vec![],
            vec![ServiceRequirement {
                service: "storage".into(),
                required: true,
            }],
        ));
        supervisor.reconcile();
        assert_eq!(supervisor.entries["consumer"].state, PluginState::Pending);

        supervisor.insert(PluginEntry::new("provider", vec!["storage".into()], vec![]));
        supervisor.reconcile();
        assert_eq!(supervisor.entries["provider"].state, PluginState::Active);
        assert_eq!(supervisor.entries["consumer"].state, PluginState::Active);
    }

    #[test]
    fn provider_generation_change_reloads_dependency_epoch() {
        let mut supervisor = PluginSupervisor::default();
        supervisor.insert(PluginEntry::new("provider", vec!["storage".into()], vec![]));
        supervisor.insert(PluginEntry::new(
            "consumer",
            vec![],
            vec![ServiceRequirement {
                service: "storage".into(),
                required: true,
            }],
        ));
        supervisor.reconcile();
        let before = supervisor.entries["consumer"].observed_generations["storage"];
        supervisor.replace_provider_generation("storage", "provider");
        let after = supervisor.entries["consumer"].observed_generations["storage"];
        assert!(after > before);
        assert_eq!(supervisor.entries["consumer"].state, PluginState::Active);
    }

    #[test]
    fn installs_verified_archive_and_switches_active_pointer() {
        let archive = tar_gz(&[("plugin.json", br#"{"ok":true}"#)]);
        let digest = format!("{:x}", Sha256::digest(&archive));
        let artifact = ReleaseArtifact {
            id: "mobile-web".into(),
            runtime: "local-web".into(),
            platforms: vec!["ios".into(), "android".into()],
            source: ArtifactSource::Https {
                url: "https://example.com/plugin.tar.gz".into(),
            },
            sha256: digest.clone(),
            size: archive.len() as u64,
            format: ArtifactFormat::TarGz,
            entry: Some("./plugin.json".into()),
        };
        let release = ExternalReleaseManifest {
            schema_version: 1,
            protocol: "mahayana.external-release.v1".into(),
            plugin_id: "global-dharma".into(),
            version: "1.0.0".into(),
            permissions: vec!["network".into()],
            artifacts: vec![artifact.clone()],
        };
        let temp = tempfile::tempdir().unwrap();
        let installer = PluginInstaller::new(temp.path()).unwrap();
        let pointer = installer
            .install_verified_bytes(&release, &artifact, &archive)
            .unwrap();
        assert_eq!(pointer.runtime, "local-web");
        assert!(
            Path::new(&pointer.installed_path)
                .join("plugin.json")
                .is_file()
        );
        assert_eq!(installer.active("global-dharma").unwrap(), Some(pointer));
    }

    #[test]
    fn permission_manager_persists_only_declared_grants() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("permissions.json");
        let requested = vec!["storage.local".to_string(), "network.request".to_string()];
        let mut manager = PermissionManager::load(&path).unwrap();
        manager
            .grant("sample-plugin", &requested, "network.request")
            .unwrap();
        assert!(manager.is_granted("sample-plugin", "network.request"));
        assert!(matches!(
            manager.grant("sample-plugin", &requested, "process.spawn"),
            Err(RuntimeError::PermissionNotRequested { .. })
        ));
        drop(manager);

        let mut manager = PermissionManager::load(&path).unwrap();
        assert_eq!(
            manager.grants_for("sample-plugin"),
            vec!["network.request".to_string()]
        );
        manager
            .retain_requested("sample-plugin", &["storage.local".to_string()])
            .unwrap();
        assert!(manager.grants_for("sample-plugin").is_empty());
    }

    #[test]
    fn npm_semver_alias_and_integrity_helpers_are_deterministic() {
        let metadata = NpmFullPackageMetadata {
            dist_tags: BTreeMap::from([("latest".into(), "2.0.0".into())]),
            versions: BTreeMap::from([
                (
                    "1.2.0".into(),
                    NpmFullVersionMetadata {
                        dist: NpmFullDistMetadata {
                            tarball: "https://registry.npmjs.org/a/-/a-1.2.0.tgz".into(),
                            integrity: None,
                            shasum: None,
                        },
                    },
                ),
                (
                    "1.9.0".into(),
                    NpmFullVersionMetadata {
                        dist: NpmFullDistMetadata {
                            tarball: "https://registry.npmjs.org/a/-/a-1.9.0.tgz".into(),
                            integrity: None,
                            shasum: None,
                        },
                    },
                ),
                (
                    "2.0.0".into(),
                    NpmFullVersionMetadata {
                        dist: NpmFullDistMetadata {
                            tarball: "https://registry.npmjs.org/a/-/a-2.0.0.tgz".into(),
                            integrity: None,
                            shasum: None,
                        },
                    },
                ),
            ]),
        };
        assert_eq!(select_npm_version(&metadata, "^1.2.0").as_deref(), Some("1.9.0"));
        assert_eq!(select_npm_version(&metadata, "latest").as_deref(), Some("2.0.0"));
        assert_eq!(
            npm_dependency_target("alias", "npm:@scope/real@^1.0.0").unwrap(),
            ("@scope/real".into(), "^1.0.0".into())
        );

        let payload = b"verified npm tarball";
        let integrity = format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(payload))
        );
        verify_npm_integrity(payload, Some(&integrity), None).unwrap();
        assert!(matches!(
            verify_npm_integrity(b"tampered", Some(&integrity), None),
            Err(RuntimeError::NpmIntegrityMismatch)
        ));
    }

    #[test]
    fn npm_package_root_is_flattened_without_lifecycle_execution() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("package/lib")).unwrap();
        fs::write(
            root.path().join("package/package.json"),
            r#"{"name":"sample","version":"1.0.0","scripts":{"postinstall":"exit 99"}}"#,
        )
        .unwrap();
        fs::write(root.path().join("package/lib/index.js"), "export default 1").unwrap();
        normalize_npm_package_root(root.path()).unwrap();
        assert!(root.path().join("package.json").is_file());
        assert!(root.path().join("lib/index.js").is_file());
        assert!(!root.path().join("package").exists());
    }

    #[test]
    fn rejects_tar_symlinks_even_when_archive_path_is_relative() {
        let encoder = flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_path("safe-link").unwrap();
        header.set_link_name("/etc/passwd").unwrap();
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
        let encoder = builder.into_inner().unwrap();
        let archive_bytes = encoder.finish().unwrap();
        let destination = tempfile::tempdir().unwrap();
        assert!(matches!(
            extract_artifact(&archive_bytes, &ArtifactFormat::TarGz, destination.path()),
            Err(RuntimeError::Archive(_))
        ));
    }

    #[test]
    fn rejects_archive_traversal() {
        let path = Path::new("../escape");
        assert!(safe_archive_path(path).is_err());
    }

    #[test]
    fn effect_scope_disposes_once_in_reverse_registration_order() {
        let count = Arc::new(AtomicUsize::new(0));
        let first = count.clone();
        let second = count.clone();
        let mut scope = EffectScope::default();
        scope
            .register(move || {
                assert_eq!(first.fetch_add(1, Ordering::SeqCst), 1);
            })
            .unwrap();
        scope
            .register(move || {
                assert_eq!(second.fetch_add(1, Ordering::SeqCst), 0);
            })
            .unwrap();
        scope.dispose();
        scope.dispose();
        assert!(scope.is_disposed());
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert!(scope.register(|| {}).is_err());
    }
}
