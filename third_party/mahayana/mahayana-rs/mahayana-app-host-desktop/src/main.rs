use mahayana_unified_app_host::{UnifiedAppHost, default_unified_app_data_dir, dispatch_json};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

fn ensure_managed_runtime_layout(app_data_dir: &Path) -> io::Result<()> {
    // The desktop product owns this fallback workspace.  It must exist before
    // the native engine canonicalizes the path while opening the first Agent
    // session.  User-selected workspace paths are validated elsewhere and are
    // never created implicitly.
    fs::create_dir_all(app_data_dir.join("feature-host/runtime/workspace"))
}

fn main() {
    let app_data_dir = default_unified_app_data_dir();
    if let Err(error) = ensure_managed_runtime_layout(&app_data_dir) {
        eprintln!(
            "failed to initialize managed Mahayana runtime layout at {}: {error}",
            app_data_dir.display()
        );
        std::process::exit(1);
    }

    let host = match UnifiedAppHost::new(app_data_dir) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("failed to initialize unified Mahayana app host: {error}");
            std::process::exit(1);
        }
    };
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("failed to read host request: {error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = dispatch_json(&host, &line);
        if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_managed_runtime_layout;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn creates_product_owned_fallback_workspace_before_host_startup() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "fabushi-mahayana-desktop-layout-{}-{suffix}",
            std::process::id()
        ));

        ensure_managed_runtime_layout(&root).expect("initialize managed runtime layout");
        assert!(root.join("feature-host/runtime/workspace").is_dir());

        fs::remove_dir_all(root).expect("remove test layout");
    }
}
