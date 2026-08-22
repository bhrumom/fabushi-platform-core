//! Product-owned workspace engine for Mahayana.
//!
//! The implementation favors portable filesystem semantics over vendor-specific
//! snapshot mechanisms. Desktop hosts can later add APFS/Btrfs/reflink
//! accelerators behind the same contracts without changing product surfaces.

use mahayana_kernel::{Capability, CapabilitySet};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const STATE_DIRECTORY: &str = ".mahayana/workspace-engine";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn generated_id(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::new_v4())
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceOptions {
    pub excluded_directories: BTreeSet<String>,
    pub max_index_file_bytes: u64,
}

impl Default for WorkspaceOptions {
    fn default() -> Self {
        Self {
            excluded_directories: [
                ".git".to_string(),
                ".mahayana".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                "dist".to_string(),
                "build".to_string(),
            ]
            .into_iter()
            .collect(),
            max_index_file_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointManifest {
    pub id: String,
    pub label: Option<String>,
    pub created_at_ms: i64,
    pub files: Vec<FileSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeDescriptor {
    pub id: String,
    pub path: String,
    pub created_at_ms: i64,
    pub source_checkpoint_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodebaseNode {
    pub path: String,
    pub language: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CodebaseEdge {
    pub from: String,
    pub reference: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodebaseGraph {
    pub generated_at_ms: i64,
    pub nodes: Vec<CodebaseNode>,
    pub edges: Vec<CodebaseEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub path: String,
    pub line: usize,
    pub kind: String,
    pub name: String,
}

pub struct WorkspaceEngine {
    root: PathBuf,
    state_dir: PathBuf,
    options: WorkspaceOptions,
}

impl WorkspaceEngine {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        Self::open_with_options(root, WorkspaceOptions::default())
    }

    pub fn open_with_options(
        root: impl AsRef<Path>,
        options: WorkspaceOptions,
    ) -> Result<Self, WorkspaceError> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        if !root.is_dir() {
            return Err(WorkspaceError::InvalidRoot(root.display().to_string()));
        }
        let state_dir = root.join(STATE_DIRECTORY);
        Ok(Self {
            root,
            state_dir,
            options,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn capabilities() -> CapabilitySet {
        CapabilitySet::new([
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::Workspace,
            Capability::Checkpoint,
            Capability::Worktree,
            Capability::CodebaseGraph,
        ])
    }

    pub fn create_checkpoint(
        &self,
        label: Option<String>,
    ) -> Result<CheckpointManifest, WorkspaceError> {
        let id = generated_id("checkpoint");
        let checkpoint_dir = self.checkpoint_dir(&id);
        let files_dir = checkpoint_dir.join("files");
        fs::create_dir_all(&files_dir).map_err(|error| WorkspaceError::Io(error.to_string()))?;

        let files = self.collect_files()?;
        let mut snapshots = Vec::with_capacity(files.len());
        for source in files {
            let relative = self.relative_path(&source)?;
            let destination = files_dir.join(&relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?;
            }
            fs::copy(&source, &destination)
                .map_err(|error| WorkspaceError::Io(error.to_string()))?;
            let metadata =
                fs::metadata(&source).map_err(|error| WorkspaceError::Io(error.to_string()))?;
            snapshots.push(FileSnapshot {
                path: path_string(&relative),
                size: metadata.len(),
                sha256: sha256_file(&source)?,
            });
        }
        snapshots.sort_by(|left, right| left.path.cmp(&right.path));

        let manifest = CheckpointManifest {
            id: id.clone(),
            label,
            created_at_ms: now_ms(),
            files: snapshots,
        };
        write_json(&checkpoint_dir.join("manifest.json"), &manifest)?;
        Ok(manifest)
    }

    pub fn restore_checkpoint(&self, id: &str) -> Result<CheckpointManifest, WorkspaceError> {
        let manifest = self.read_checkpoint(id)?;
        let files_dir = self.checkpoint_dir(id).join("files");
        for snapshot in &manifest.files {
            let relative = safe_relative(Path::new(&snapshot.path))?;
            let source = files_dir.join(&relative);
            let destination = self.root.join(&relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?;
            }
            fs::copy(&source, &destination)
                .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        }
        Ok(manifest)
    }

    pub fn read_checkpoint(&self, id: &str) -> Result<CheckpointManifest, WorkspaceError> {
        validate_storage_id(id)?;
        read_json(&self.checkpoint_dir(id).join("manifest.json"))
    }

    pub fn list_checkpoints(&self) -> Result<Vec<CheckpointManifest>, WorkspaceError> {
        let root = self.state_dir.join("checkpoints");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut manifests = Vec::new();
        for entry in fs::read_dir(root).map_err(|error| WorkspaceError::Io(error.to_string()))? {
            let entry = entry.map_err(|error| WorkspaceError::Io(error.to_string()))?;
            if !entry
                .file_type()
                .map_err(|error| WorkspaceError::Io(error.to_string()))?
                .is_dir()
            {
                continue;
            }
            let manifest_path = entry.path().join("manifest.json");
            if manifest_path.is_file() {
                manifests.push(read_json(&manifest_path)?);
            }
        }
        manifests.sort_by(|left: &CheckpointManifest, right: &CheckpointManifest| {
            right.created_at_ms.cmp(&left.created_at_ms)
        });
        Ok(manifests)
    }

    pub fn create_worktree(
        &self,
        source_checkpoint_id: Option<&str>,
    ) -> Result<WorktreeDescriptor, WorkspaceError> {
        let id = generated_id("worktree");
        let worktree_root = self.worktree_dir(&id).join("workspace");
        fs::create_dir_all(&worktree_root)
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;

        if let Some(checkpoint_id) = source_checkpoint_id {
            let checkpoint = self.read_checkpoint(checkpoint_id)?;
            let checkpoint_files = self.checkpoint_dir(checkpoint_id).join("files");
            for snapshot in checkpoint.files {
                let relative = safe_relative(Path::new(&snapshot.path))?;
                copy_file(
                    &checkpoint_files.join(&relative),
                    &worktree_root.join(&relative),
                )?;
            }
        } else {
            for source in self.collect_files()? {
                let relative = self.relative_path(&source)?;
                copy_file(&source, &worktree_root.join(relative))?;
            }
        }

        let descriptor = WorktreeDescriptor {
            id: id.clone(),
            path: path_string(&worktree_root),
            created_at_ms: now_ms(),
            source_checkpoint_id: source_checkpoint_id.map(str::to_owned),
        };
        write_json(&self.worktree_dir(&id).join("worktree.json"), &descriptor)?;
        Ok(descriptor)
    }

    pub fn remove_worktree(&self, id: &str) -> Result<(), WorkspaceError> {
        validate_storage_id(id)?;
        let directory = self.worktree_dir(id);
        if directory.exists() {
            fs::remove_dir_all(directory).map_err(|error| WorkspaceError::Io(error.to_string()))?;
        }
        Ok(())
    }

    pub fn list_worktrees(&self) -> Result<Vec<WorktreeDescriptor>, WorkspaceError> {
        let root = self.state_dir.join("worktrees");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut worktrees = Vec::new();
        for entry in fs::read_dir(root).map_err(|error| WorkspaceError::Io(error.to_string()))? {
            let entry = entry.map_err(|error| WorkspaceError::Io(error.to_string()))?;
            let descriptor = entry.path().join("worktree.json");
            if descriptor.is_file() {
                worktrees.push(read_json(&descriptor)?);
            }
        }
        worktrees.sort_by(|left: &WorktreeDescriptor, right: &WorktreeDescriptor| {
            right.created_at_ms.cmp(&left.created_at_ms)
        });
        Ok(worktrees)
    }

    pub fn build_codebase_graph(&self) -> Result<CodebaseGraph, WorkspaceError> {
        let mut nodes = Vec::new();
        let mut edges = BTreeSet::new();
        for file in self.collect_files()? {
            let metadata =
                fs::metadata(&file).map_err(|error| WorkspaceError::Io(error.to_string()))?;
            let relative = self.relative_path(&file)?;
            let language = language_for_path(&relative).to_string();
            nodes.push(CodebaseNode {
                path: path_string(&relative),
                language: language.clone(),
                size: metadata.len(),
            });
            if language == "binary" || metadata.len() > self.options.max_index_file_bytes {
                continue;
            }
            let content = match fs::read_to_string(&file) {
                Ok(content) => content,
                Err(_) => continue,
            };
            for reference in extract_references(&content, &language) {
                edges.insert(CodebaseEdge {
                    from: path_string(&relative),
                    reference,
                    kind: "import".to_string(),
                });
            }
        }
        nodes.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(CodebaseGraph {
            generated_at_ms: now_ms(),
            nodes,
            edges: edges.into_iter().collect(),
        })
    }

    pub fn index_symbols(&self) -> Result<Vec<CodeSymbol>, WorkspaceError> {
        let mut symbols = Vec::new();
        for file in self.collect_files()? {
            let metadata =
                fs::metadata(&file).map_err(|error| WorkspaceError::Io(error.to_string()))?;
            if metadata.len() > self.options.max_index_file_bytes {
                continue;
            }
            let relative = self.relative_path(&file)?;
            let language = language_for_path(&relative);
            if language == "binary" {
                continue;
            }
            let content = match fs::read_to_string(&file) {
                Ok(content) => content,
                Err(_) => continue,
            };
            for (line_index, line) in content.lines().enumerate() {
                if let Some((kind, name)) = extract_symbol(line, language) {
                    symbols.push(CodeSymbol {
                        path: path_string(&relative),
                        line: line_index + 1,
                        kind,
                        name,
                    });
                }
            }
        }
        symbols.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.line.cmp(&right.line))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(symbols)
    }

    fn checkpoint_dir(&self, id: &str) -> PathBuf {
        self.state_dir.join("checkpoints").join(storage_segment(id))
    }

    fn worktree_dir(&self, id: &str) -> PathBuf {
        self.state_dir.join("worktrees").join(storage_segment(id))
    }

    fn collect_files(&self) -> Result<Vec<PathBuf>, WorkspaceError> {
        let mut files = Vec::new();
        self.walk_directory(&self.root, &mut files)?;
        files.sort();
        Ok(files)
    }

    fn walk_directory(
        &self,
        directory: &Path,
        files: &mut Vec<PathBuf>,
    ) -> Result<(), WorkspaceError> {
        for entry in
            fs::read_dir(directory).map_err(|error| WorkspaceError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| WorkspaceError::Io(error.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| WorkspaceError::Io(error.to_string()))?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if self.options.excluded_directories.contains(&name) {
                    continue;
                }
                self.walk_directory(&path, files)?;
            } else if file_type.is_file() {
                files.push(path);
            }
        }
        Ok(())
    }

    fn relative_path(&self, path: &Path) -> Result<PathBuf, WorkspaceError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| WorkspaceError::PathEscapesRoot(path.display().to_string()))?;
        safe_relative(relative)
    }
}

fn safe_relative(path: &Path) -> Result<PathBuf, WorkspaceError> {
    if path.is_absolute() {
        return Err(WorkspaceError::UnsafeRelativePath(
            path.display().to_string(),
        ));
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => safe.push(segment),
            Component::CurDir => {}
            _ => {
                return Err(WorkspaceError::UnsafeRelativePath(
                    path.display().to_string(),
                ));
            }
        }
    }
    Ok(safe)
}

fn validate_storage_id(id: &str) -> Result<(), WorkspaceError> {
    if id.trim().is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains('\0')
    {
        return Err(WorkspaceError::InvalidStorageId(id.to_string()));
    }
    Ok(())
}

fn storage_segment(id: &str) -> String {
    id.replace(':', "_")
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), WorkspaceError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    }
    fs::copy(source, destination).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, WorkspaceError> {
    let mut file = fs::File::open(path).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), WorkspaceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    }
    let json = serde_json::to_vec_pretty(value)
        .map_err(|error| WorkspaceError::Serialization(error.to_string()))?;
    let temporary = path.with_extension("tmp");
    let mut file =
        fs::File::create(&temporary).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    file.write_all(&json)
        .map_err(|error| WorkspaceError::Io(error.to_string()))?;
    file.sync_all()
        .map_err(|error| WorkspaceError::Io(error.to_string()))?;
    fs::rename(&temporary, path).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, WorkspaceError> {
    let bytes = fs::read(path).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|error| WorkspaceError::Serialization(error.to_string()))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn language_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => {
            "javascript"
        }
        Some("py") => "python",
        Some("swift") => "swift",
        Some("kt") | Some("kts") => "kotlin",
        Some("java") => "java",
        Some("go") => "go",
        Some("c") | Some("h") | Some("cc") | Some("cpp") | Some("hpp") => "c_family",
        Some("toml") | Some("json") | Some("yaml") | Some("yml") | Some("md") | Some("txt") => {
            "text"
        }
        _ => "binary",
    }
}

fn extract_references(content: &str, language: &str) -> Vec<String> {
    let mut references = BTreeSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let interesting = match language {
            "rust" => trimmed.starts_with("use ") || trimmed.starts_with("mod "),
            "javascript" => {
                trimmed.starts_with("import ")
                    || trimmed.starts_with("export ")
                    || trimmed.contains("require(")
            }
            "python" => trimmed.starts_with("import ") || trimmed.starts_with("from "),
            "swift" => trimmed.starts_with("import "),
            "kotlin" | "java" => trimmed.starts_with("import "),
            "go" => trimmed.starts_with("import ") || trimmed.starts_with('"'),
            "c_family" => trimmed.starts_with("#include"),
            _ => false,
        };
        if !interesting {
            continue;
        }
        if let Some(reference) = first_quoted(trimmed) {
            references.insert(reference);
            continue;
        }
        let normalized = trimmed
            .trim_start_matches("use ")
            .trim_start_matches("mod ")
            .trim_start_matches("import ")
            .trim_start_matches("from ")
            .trim_end_matches(';')
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(|character: char| {
                matches!(
                    character,
                    '(' | ')' | '{' | '}' | '[' | ']' | ',' | '"' | '\''
                )
            });
        if !normalized.is_empty() {
            references.insert(normalized.to_string());
        }
    }
    references.into_iter().collect()
}

fn first_quoted(line: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let start = line.find(quote)?;
        let remaining = &line[start + quote.len_utf8()..];
        if let Some(end) = remaining.find(quote) {
            let value = &remaining[..end];
            if !value.trim().is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn extract_symbol(line: &str, language: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    let candidates: &[(&str, &str)] = match language {
        "rust" => &[
            ("pub fn ", "function"),
            ("fn ", "function"),
            ("pub struct ", "struct"),
            ("struct ", "struct"),
            ("pub enum ", "enum"),
            ("enum ", "enum"),
            ("pub trait ", "trait"),
            ("trait ", "trait"),
        ],
        "javascript" => &[
            ("export function ", "function"),
            ("function ", "function"),
            ("export class ", "class"),
            ("class ", "class"),
            ("export interface ", "interface"),
            ("interface ", "interface"),
            ("export const ", "constant"),
            ("const ", "constant"),
        ],
        "python" => &[("def ", "function"), ("class ", "class")],
        "swift" => &[
            ("func ", "function"),
            ("struct ", "struct"),
            ("class ", "class"),
        ],
        "kotlin" | "java" => &[("class ", "class"), ("interface ", "interface")],
        "go" => &[("func ", "function"), ("type ", "type")],
        _ => &[],
    };
    for (prefix, kind) in candidates {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = rest
                .split(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '(' | '<' | '{' | ':' | '=' | ';')
                })
                .next()
                .unwrap_or_default()
                .trim();
            if !name.is_empty() {
                return Some(((*kind).to_string(), name.to_string()));
            }
        }
    }
    None
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("workspace root is invalid: {0}")]
    InvalidRoot(String),
    #[error("path escapes workspace root: {0}")]
    PathEscapesRoot(String),
    #[error("unsafe relative path: {0}")]
    UnsafeRelativePath(String),
    #[error("invalid checkpoint/worktree id: {0}")]
    InvalidStorageId(String),
    #[error("workspace I/O failed: {0}")]
    Io(String),
    #[error("workspace serialization failed: {0}")]
    Serialization(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("mahayana-workspace-test-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("src")).expect("create fixture");
        fs::write(
            root.join("src/lib.rs"),
            "use crate::engine::Runner;\npub struct App {}\nfn run() {}\n",
        )
        .expect("write rust source");
        fs::write(
            root.join("src/main.ts"),
            "import { app } from './app';\nfunction main() {}\n",
        )
        .expect("write ts source");
        root
    }

    #[test]
    fn checkpoint_round_trip_restores_file_content() {
        let root = fixture_root();
        let engine = WorkspaceEngine::open(&root).expect("open workspace");
        let checkpoint = engine
            .create_checkpoint(Some("before edit".into()))
            .expect("checkpoint");
        fs::write(root.join("src/lib.rs"), "changed").expect("mutate source");
        engine
            .restore_checkpoint(&checkpoint.id)
            .expect("restore checkpoint");
        let restored = fs::read_to_string(root.join("src/lib.rs")).expect("read restored");
        assert!(restored.contains("pub struct App"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn logical_worktree_is_isolated_from_source_edits() {
        let root = fixture_root();
        let engine = WorkspaceEngine::open(&root).expect("open workspace");
        let worktree = engine.create_worktree(None).expect("create worktree");
        fs::write(root.join("src/lib.rs"), "source changed").expect("mutate source");
        let worktree_content = fs::read_to_string(Path::new(&worktree.path).join("src/lib.rs"))
            .expect("read worktree");
        assert!(worktree_content.contains("pub struct App"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn graph_and_symbol_index_find_cross_language_structure() {
        let root = fixture_root();
        let engine = WorkspaceEngine::open(&root).expect("open workspace");
        let graph = engine.build_codebase_graph().expect("graph");
        assert!(graph.nodes.iter().any(|node| node.path == "src/lib.rs"));
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.reference.contains("crate::engine"))
        );
        let symbols = engine.index_symbols().expect("symbols");
        assert!(symbols.iter().any(|symbol| symbol.name == "App"));
        assert!(symbols.iter().any(|symbol| symbol.name == "main"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(safe_relative(Path::new("../outside")).is_err());
    }
}
