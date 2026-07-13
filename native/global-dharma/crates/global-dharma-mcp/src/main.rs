use std::{
    io::{self, BufRead},
    process::Command,
};
fn main() {
    for line in io::stdin().lock().lines().flatten() {
        let request: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let result = match method {
            "tools/list" => {
                serde_json::json!({"tools":[tool("deploy_latest",false),tool("status",true),tool("logs",true),tool("start",false),tool("stop",false),tool("send",false),tool("validate_config",true)]})
            }
            "tools/call" => {
                let name = request
                    .pointer("/params/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let action = name.rsplit('.').next().unwrap_or(name);
                let readonly = matches!(action, "status" | "logs" | "validate_config");
                let confirmed = request
                    .pointer("/params/arguments/confirmed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !readonly && !confirmed {
                    serde_json::json!({"content":[{"type":"text","text":format!("global_dharma.{action}: confirmation required before execution")}],"requiresConfirmation":true})
                } else {
                    match run_ctl(action, request.pointer("/params/arguments")) {
                        Ok(output) => {
                            serde_json::json!({"content":[{"type":"text","text":output}],"requiresConfirmation":false})
                        }
                        Err(error) => {
                            serde_json::json!({"content":[{"type":"text","text":error}],"isError":true})
                        }
                    }
                }
            }
            _ => serde_json::json!({"error":"unsupported_method"}),
        };
        println!(
            "{}",
            serde_json::json!({"jsonrpc":"2.0","id":request.get("id").cloned().unwrap_or(serde_json::Value::Null),"result":result})
        );
    }
}
fn tool(name: &str, readonly: bool) -> serde_json::Value {
    let mut properties = serde_json::json!({});
    if name == "send" {
        properties["content"] = serde_json::json!({"type":"string","description":"Content to send to administrator-authorized nodes."});
        properties["task_id"] = serde_json::json!({"type":"string"});
    }
    if !readonly {
        properties["confirmed"] = serde_json::json!({"type":"boolean","description":"Must be true after the user explicitly approves this state-changing action."});
    }
    serde_json::json!({"name":format!("global_dharma.{name}"),"description":format!("Global Dharma {name}"),"inputSchema":{"type":"object","properties":properties},"annotations":{"readOnlyHint":readonly}})
}

fn run_ctl(action: &str, args: Option<&serde_json::Value>) -> Result<String, String> {
    let command = std::env::var("GLOBAL_DHARMA_CTL").unwrap_or_else(|_| "global-dharmactl".into());
    let mut child = Command::new(command);
    match action {
        "deploy_latest" => {
            child.arg("install-systemd");
        }
        "validate_config" => {
            child.arg("validate-config");
        }
        "status" | "logs" | "start" | "stop" => {
            child.arg(action);
        }
        "send" => {
            let task = args
                .and_then(|v| v.get("task_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("mahayana");
            let content = args
                .and_then(|v| v.get("content"))
                .and_then(|v| v.as_str())
                .ok_or("send requires arguments.content")?;
            child.arg("send").arg(task).arg(content);
        }
        _ => return Err(format!("unsupported Global Dharma tool: {action}")),
    }
    let output = child
        .output()
        .map_err(|error| format!("global-dharmactl launch failed: {error}"))?;
    let text = String::from_utf8_lossy(if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    })
    .trim()
    .to_string();
    if output.status.success() {
        Ok(if text.is_empty() {
            "completed".into()
        } else {
            text
        })
    } else {
        Err(if text.is_empty() {
            format!("global-dharmactl exited {}", output.status)
        } else {
            text
        })
    }
}
