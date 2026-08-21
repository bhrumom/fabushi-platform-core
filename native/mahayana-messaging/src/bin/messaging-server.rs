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
    let snapshot = env::var_os("FABUSHI_MESSAGING_SNAPSHOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fabushi-messaging-snapshot.json"));
    let access_registry = env::var_os("FABUSHI_MESSAGING_ACCESS_REGISTRY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fabushi-messaging-access.json"));
    let config = MessagingServerConfig::new(bind, snapshot, access_registry);
    let server = MessagingTcpServer::load(config)?;
    server.serve()?;
    Ok(())
}
