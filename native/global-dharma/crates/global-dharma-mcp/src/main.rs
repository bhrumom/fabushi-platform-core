use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    process::Command,
};

const PROTOCOL_VERSION: &str = "2025-06-18";
const UI_URI: &str = "ui://fabushi/global-dharma/home-v1.html";
const APP_MIME: &str = "text/html;profile=mcp-app";

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_error(
                    &mut stdout,
                    Value::Null,
                    -32700,
                    &format!("parse error: {error}"),
                );
                continue;
            }
        };
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            write_error(
                &mut stdout,
                request.get("id").cloned().unwrap_or(Value::Null),
                -32600,
                "invalid request",
            );
            continue;
        };
        if request.get("id").is_none() {
            // Standard MCP notifications do not receive responses. Cancellation
            // is observed between process calls; a future async runner can also
            // terminate an in-flight child without changing this wire contract.
            if matches!(
                method,
                "notifications/initialized" | "notifications/cancelled"
            ) {
                continue;
            }
            continue;
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let response = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {"listChanged": true},
                    "resources": {"subscribe": false, "listChanged": false}
                },
                "serverInfo": {"name": "global-dharma", "version": env!("CARGO_PKG_VERSION")}
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => call_tool(&request, &mut stdout),
            "resources/list" => Ok(json!({"resources": [{
                "uri": UI_URI,
                "name": "全球法布施首页",
                "description": "全球法布施版本化 MCP App UI",
                "mimeType": APP_MIME
            }]})),
            "resources/read" => read_resource(&request),
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        match response {
            Ok(result) => write_json(
                &mut stdout,
                &json!({"jsonrpc": "2.0", "id": id, "result": result}),
            ),
            Err((code, message)) => write_error(&mut stdout, id, code, &message),
        }
    }
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "home",
            "加载全球法布施首页",
            json!({}),
            true,
            false,
            false,
            true,
        ),
        tool(
            "start",
            "启动全球法布施服务",
            json!({}),
            false,
            false,
            true,
            false,
        ),
        tool(
            "stop",
            "停止全球法布施服务",
            json!({}),
            false,
            true,
            false,
            false,
        ),
        tool(
            "loop",
            "执行一次调度循环",
            json!({}),
            false,
            false,
            true,
            false,
        ),
        tool(
            "status",
            "读取服务状态",
            json!({}),
            true,
            false,
            false,
            false,
        ),
        tool(
            "send",
            "向管理员授权节点发送内容",
            json!({
                "content": {"type": "string", "description": "要发送的内容"},
                "task_id": {"type": "string", "default": "mahayana"}
            }),
            false,
            false,
            true,
            false,
        ),
        tool(
            "logs",
            "读取最近日志",
            json!({"limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}}),
            true,
            false,
            false,
            false,
        ),
        tool(
            "validate_config",
            "验证当前配置",
            json!({}),
            true,
            false,
            false,
            false,
        ),
        tool(
            "deploy_latest",
            "部署最新已验证版本",
            json!({}),
            false,
            false,
            true,
            false,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn tool(
    name: &str,
    description: &str,
    properties: Value,
    read_only: bool,
    destructive: bool,
    open_world: bool,
    with_ui: bool,
) -> Value {
    let mut value = json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "additionalProperties": false},
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "openWorldHint": open_world
        }
    });
    if with_ui {
        value["_meta"] = json!({"ui/resourceUri": UI_URI});
    }
    value
}

fn call_tool(request: &Value, stdout: &mut impl Write) -> Result<Value, (i64, String)> {
    let name = request
        .pointer("/params/name")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "tools/call requires params.name".to_string()))?;
    let args = request.pointer("/params/arguments");
    if name == "home" {
        return Ok(json!({
            "content": [{"type": "text", "text": "全球法布施已就绪。"}],
            "structuredContent": {"ready": true},
            "_meta": {"ui/resourceUri": UI_URI}
        }));
    }
    if let Some(token) = request.pointer("/params/_meta/progressToken") {
        write_json(
            stdout,
            &json!({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {"progressToken": token, "progress": 0, "total": 1, "message": format!("正在执行 {name}")}
            }),
        );
    }
    match run_ctl(name, args) {
        Ok(output) => Ok(json!({
            "content": [{"type": "text", "text": output}],
            "structuredContent": {"tool": name, "completed": true}
        })),
        Err(error) => Ok(json!({
            "content": [{"type": "text", "text": error}],
            "isError": true
        })),
    }
}

fn read_resource(request: &Value) -> Result<Value, (i64, String)> {
    let uri = request
        .pointer("/params/uri")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "resources/read requires params.uri".to_string()))?;
    if uri != UI_URI {
        return Err((-32002, format!("resource not found: {uri}")));
    }
    Ok(json!({"contents": [{"uri": UI_URI, "mimeType": APP_MIME, "text": home_html()}]}))
}

fn home_html() -> &'static str {
    r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'"><style>body{font:15px system-ui;background:#101722;color:#eef4ff;padding:20px}button{margin:4px;padding:9px 12px;border-radius:9px;border:1px solid #476080;background:#1a2a40;color:inherit}pre{white-space:pre-wrap}</style></head><body><h1>全球法布施</h1><p>命令与数据全部通过 MCP。</p><div id="tools"><button data-tool="status">/status</button><button data-tool="start">/start</button><button data-tool="stop">/stop</button><button data-tool="loop">/loop</button><button data-tool="logs">/logs</button></div><pre id="out">已连接</pre><script>(()=>{let id=0;const pending=new Map();const out=document.querySelector('#out');addEventListener('message',event=>{const m=event.data;if(!m||m.jsonrpc!=='2.0')return;if(m.id!==undefined&&pending.has(m.id)){pending.get(m.id)(m);pending.delete(m.id)}if(m.method==='ui/notifications/tool-result')out.textContent=JSON.stringify(m.params,null,2)});function call(name){const requestId=++id;return new Promise(resolve=>{pending.set(requestId,resolve);parent.postMessage({jsonrpc:'2.0',id:requestId,method:'tools/call',params:{name,arguments:{}}},'*')})}document.querySelectorAll('[data-tool]').forEach(button=>button.onclick=async()=>{out.textContent='执行中…';const response=await call(button.dataset.tool);out.textContent=JSON.stringify(response.result??response.error,null,2)})})()</script></body></html>"#
}

fn run_ctl(action: &str, args: Option<&Value>) -> Result<String, String> {
    let command = std::env::var("GLOBAL_DHARMA_CTL").unwrap_or_else(|_| "global-dharmactl".into());
    let mut child = Command::new(command);
    match action {
        "deploy_latest" => child.arg("install-systemd"),
        "validate_config" => child.arg("validate-config"),
        "status" | "logs" | "start" | "stop" | "loop" => child.arg(action),
        "send" => {
            let task = args
                .and_then(|value| value.get("task_id"))
                .and_then(Value::as_str)
                .unwrap_or("mahayana");
            let content = args
                .and_then(|value| value.get("content"))
                .and_then(Value::as_str)
                .ok_or_else(|| "send requires arguments.content".to_string())?;
            child.arg("send").arg(task).arg(content)
        }
        _ => return Err(format!("unsupported Global Dharma tool: {action}")),
    };
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

fn write_error(stdout: &mut impl Write, id: Value, code: i64, message: &str) {
    write_json(
        stdout,
        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    );
}

fn write_json(stdout: &mut impl Write, value: &Value) {
    let _ = writeln!(stdout, "{value}");
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_exact_unprefixed_tool_contract() {
        let names = tools()
            .into_iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "home",
                "start",
                "stop",
                "loop",
                "status",
                "send",
                "logs",
                "validate_config",
                "deploy_latest"
            ]
        );
    }

    #[test]
    fn home_uses_mcp_app_resource_mime() {
        assert!(home_html().contains("tools/call"));
        let legacy_bridge = ["Fabushi", "MiniApp"].concat();
        assert!(!home_html().contains(&legacy_bridge));
        assert_eq!(APP_MIME, "text/html;profile=mcp-app");
    }
}
