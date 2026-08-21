use fabushi_messaging_core::{CallSignalingServerConfig, CallSignalingTcpServer};
use std::env;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("Fabushi call signaling server failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let bind = env::var("FABUSHI_CALL_SIGNAL_BIND").unwrap_or_else(|_| "127.0.0.1:9410".into());
    let access_registry = env::var_os("FABUSHI_MESSAGING_ACCESS_REGISTRY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fabushi-messaging-access.json"));
    let config = CallSignalingServerConfig::new(bind, access_registry);
    let server = CallSignalingTcpServer::new(config)?;
    server.serve()?;
    Ok(())
}
