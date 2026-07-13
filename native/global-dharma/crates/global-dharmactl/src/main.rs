use global_dharma_core::Config;
use std::{
    env,
    process::{Command, ExitCode},
};
fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let config = env::var("GLOBAL_DHARMA_CONFIG")
        .unwrap_or_else(|_| "/etc/global-dharma/global-dharma.toml".into());
    match args.get(1).map(String::as_str) {
        Some("validate-config") => match Config::load(config) {
            Ok(c) => {
                println!("valid authorized-node config ({} nodes)", c.nodes.len());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("invalid config: {e}");
                ExitCode::from(2)
            }
        },
        Some("systemd-unit") => {
            print!("{}", include_str!("../../../deploy/global-dharma.service"));
            ExitCode::SUCCESS
        }
        Some("compose") => {
            print!("{}", include_str!("../../../deploy/compose.yaml"));
            ExitCode::SUCCESS
        }
        Some("start") | Some("stop") => service(args[1].as_str()),
        Some("install-systemd") => install_systemd(),
        Some("status") | Some("logs") => proxy(args[1].as_str(), ""),
        Some("send") => {
            let task = args.get(2).cloned().unwrap_or_else(|| "manual".into());
            let content = args.get(3).cloned().unwrap_or_default();
            proxy(
                "send",
                &serde_json::json!({"task_id":task,"content":content}).to_string(),
            )
        }
        _ => {
            eprintln!("usage: global-dharmactl validate-config|status|logs|send <task-id> <content>|start|stop|install-systemd|systemd-unit|compose");
            ExitCode::from(64)
        }
    }
}
fn service(action: &str) -> ExitCode {
    match Command::new("systemctl")
        .arg(action)
        .arg("global-dharma.service")
        .status()
    {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => {
            eprintln!("systemctl {action} failed: {error}");
            ExitCode::from(1)
        }
    }
}
fn install_systemd() -> ExitCode {
    let executable = env::current_exe().ok();
    let Some(executable) = executable else {
        eprintln!("cannot determine executable path");
        return ExitCode::from(1);
    };
    let root = executable
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| std::path::Path::new("/usr/local"));
    let script = root.join("share/global-dharma/install-systemd.sh");
    match Command::new(script).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => {
            eprintln!("systemd install failed: {error}");
            ExitCode::from(1)
        }
    }
}
fn proxy(path: &str, body: &str) -> ExitCode {
    let base =
        env::var("GLOBAL_DHARMA_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:18888".into());
    let result = if body.is_empty() {
        ureq::get(&format!("{base}/{path}")).call()
    } else {
        ureq::post(&format!("{base}/{path}"))
            .set("Content-Type", "application/json")
            .send_string(body)
    };
    match result {
        Ok(r) => {
            println!("{}", r.into_string().unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("daemon request failed: {e}");
            ExitCode::from(1)
        }
    }
}
