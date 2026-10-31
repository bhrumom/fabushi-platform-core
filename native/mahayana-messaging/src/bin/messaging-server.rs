use fabushi_messaging_core::{MessagingServerConfig, MessagingTcpServer};
use std::env;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("Fabushi messaging server failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let bind = env::var("FABUSHI_MESSAGING_BIND").unwrap_or_else(|_| "127.0.0.1:9400".into());
    let legacy_snapshot_env = env::var_os("FABUSHI_MESSAGING_SNAPSHOT").map(PathBuf::from);
    let database = env::var_os("FABUSHI_MESSAGING_DATABASE")
        .map(PathBuf::from)
        .or_else(|| {
            legacy_snapshot_env
                .as_ref()
                .map(|path| path.with_extension("sqlite3"))
        })
        .unwrap_or_else(|| PathBuf::from("fabushi-messaging.sqlite3"));
    let legacy_snapshot =
        legacy_snapshot_env.unwrap_or_else(|| PathBuf::from("fabushi-messaging-snapshot.json"));
    let access_registry = env::var_os("FABUSHI_MESSAGING_ACCESS_REGISTRY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fabushi-messaging-access.json"));
    let config = MessagingServerConfig::new(bind, database, access_registry)
        .with_legacy_snapshot(legacy_snapshot);
    let server = MessagingTcpServer::load(config)?;
    server.serve()?;
    Ok(())
}
