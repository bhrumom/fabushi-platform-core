#![cfg(unix)]

use mahayana_workspace_engine::{WorkspaceEngine, WorkspaceError};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_root(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()))
}

#[test]
fn restore_rejects_live_symlink_escape_before_mutating_external_files() {
    let root = unique_root("mahayana-workspace-symlink-root");
    let external = unique_root("mahayana-workspace-symlink-victim");
    fs::create_dir_all(root.join("src")).expect("create workspace");
    fs::create_dir_all(&external).expect("create external directory");
    fs::write(root.join("src/lib.rs"), "pub fn safe() {}\n").expect("write workspace file");
    let victim = external.join("victim.txt");
    fs::write(&victim, "untouched").expect("write victim");

    let engine = WorkspaceEngine::open(&root).expect("open workspace");
    let checkpoint = engine.create_checkpoint(None).expect("create checkpoint");

    fs::remove_dir_all(root.join("src")).expect("remove live src");
    symlink(&external, root.join("src")).expect("replace src with external symlink");

    let result = engine.restore_checkpoint(&checkpoint.id);
    assert!(matches!(result, Err(WorkspaceError::UnsafeRelativePath(_))));
    assert_eq!(
        fs::read_to_string(&victim).expect("read victim"),
        "untouched"
    );
    assert!(!external.join("lib.rs").exists());

    fs::remove_file(root.join("src")).expect("remove symlink");
    fs::remove_dir_all(root).expect("cleanup workspace");
    fs::remove_dir_all(external).expect("cleanup external directory");
}
