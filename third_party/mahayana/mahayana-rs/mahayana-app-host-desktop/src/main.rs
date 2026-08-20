use mahayana_app_host::{AppHost, default_app_data_dir, dispatch_json};
use std::io::{self, BufRead, Write};

fn main() {
    let host = match AppHost::new(default_app_data_dir()) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("failed to initialize Mahayana app host: {error}");
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
