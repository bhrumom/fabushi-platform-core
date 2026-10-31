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
                "description": "全球法布施图形化 MCP App UI",
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
    r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; img-src data:"><style>
:root{color-scheme:dark;font-family:Inter,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}*{box-sizing:border-box}body{margin:0;min-height:100vh;background:linear-gradient(145deg,#071512,#0b151d 58%,#101824);color:#eef8f5}.shell{max-width:920px;margin:auto;padding:26px}.hero{display:flex;align-items:center;gap:14px;margin-bottom:20px}.logo{width:50px;height:50px;border-radius:17px;background:linear-gradient(135deg,#62e4cf,#1ba9a0);display:grid;place-items:center;font-size:24px;box-shadow:0 10px 28px rgba(36,196,176,.22)}h1{font-size:24px;margin:0 0 4px}.muted{color:#9db3af;font-size:13px}.grid{display:grid;grid-template-columns:minmax(0,1.55fr) minmax(250px,.8fr);gap:16px}.card{background:rgba(17,29,35,.9);border:1px solid rgba(151,221,208,.13);border-radius:18px;padding:18px;box-shadow:0 14px 34px rgba(0,0,0,.18)}.card h2{font-size:16px;margin:0 0 6px}.card p{margin:0 0 14px;color:#a9b9b7;font-size:13px;line-height:1.55}.drop{border:1px dashed #3f716b;border-radius:14px;padding:14px;background:#0c191d;display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:12px}.filemeta{min-width:0}.filename{font-weight:650;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.filesub{font-size:12px;color:#8ea4a1;margin-top:3px}.btn{appearance:none;border:1px solid #375c57;background:#15272a;color:#eef8f5;border-radius:11px;padding:9px 12px;font-weight:650;cursor:pointer}.btn:hover{background:#1c3235}.btn.primary{border-color:#40c8b7;background:#39c5b3;color:#06211d}.btn.primary:hover{background:#55d5c5}.btn.danger{border-color:#74444b;background:#2a1b20;color:#ffdce1}textarea{width:100%;min-height:190px;resize:vertical;border:1px solid #294642;border-radius:13px;background:#091417;color:#edf8f6;padding:13px;font:14px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace;outline:none}textarea:focus{border-color:#45c9b8;box-shadow:0 0 0 3px rgba(69,201,184,.1)}.sendrow{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-top:12px}.counter{font-size:12px;color:#819995}.quick{display:grid;grid-template-columns:1fr 1fr;gap:9px}.quick .btn{min-height:46px;text-align:left}.status{margin-top:14px;border-radius:13px;padding:12px;background:#0b171a;border:1px solid #203a37}.statusline{display:flex;align-items:center;gap:8px;font-weight:650}.dot{width:9px;height:9px;border-radius:99px;background:#42d4b7;box-shadow:0 0 0 4px rgba(66,212,183,.08)}.output{margin-top:8px;color:#b8c9c6;font-size:13px;line-height:1.55;white-space:pre-wrap;word-break:break-word;max-height:220px;overflow:auto}.advanced{margin-top:16px}.advanced summary{cursor:pointer;color:#a9b9b7;font-size:13px}.advanced .row{display:flex;flex-wrap:wrap;gap:8px;margin-top:10px}@media(max-width:720px){.shell{padding:16px}.grid{grid-template-columns:1fr}.sendrow{align-items:flex-start;flex-direction:column}.sendrow .btn.primary{width:100%}}
</style></head><body><main class="shell"><div class="hero"><div class="logo">☸</div><div><h1>全球法布施</h1><div class="muted">AI 与你使用同一套 WebMCP 工具 · 操作过程会同步显示</div></div></div><section class="grid"><div class="card"><h2>发送法布施内容</h2><p>直接输入内容，或选择一个文本文件。发送前宿主仍会按权限规则确认。</p><div class="drop"><div class="filemeta"><div class="filename" id="file-name">未选择文件</div><div class="filesub" id="file-sub">支持 TXT、Markdown、CSV、JSON、HTML、XML、LOG，最大 2 MB</div></div><button class="btn" id="pick-file" data-testid="global-dharma-pick-file">选择文件</button><input id="file" data-testid="global-dharma-file-picker" type="file" hidden accept=".txt,.md,.markdown,.csv,.json,.html,.htm,.xml,.log,text/plain,text/markdown,text/csv,application/json"></div><textarea id="content" data-testid="global-dharma-content" maxlength="20000" placeholder="例如：愿以此《金刚经》内容……"></textarea><div class="sendrow"><span class="counter" id="counter">0 / 20000 字</span><button class="btn primary" id="send" data-testid="global-dharma-send">发送到全球法布施</button></div></div><aside class="card"><h2>运行控制</h2><p>不用记命令，直接点击即可。AI 在聊天里也会通过相同 WebMCP 工具执行。</p><div class="quick"><button class="btn" data-tool="status">查看状态</button><button class="btn" data-tool="start">启动服务</button><button class="btn" data-tool="loop">运行一次</button><button class="btn danger" data-tool="stop">停止服务</button></div><div class="status" data-testid="global-dharma-status"><div class="statusline"><span class="dot"></span><span id="status-title">WebMCP 已连接</span></div><div class="output" id="out">可以发送内容或查看运行状态。</div></div><details class="advanced"><summary>高级工具</summary><div class="row"><button class="btn" data-tool="logs">查看日志</button><button class="btn" data-tool="validate_config">检查配置</button><button class="btn" data-tool="deploy_latest">部署最新版</button></div></details></aside></section></main><script>(()=>{let id=0;const pending=new Map();const out=document.querySelector('#out');const title=document.querySelector('#status-title');const content=document.querySelector('#content');const counter=document.querySelector('#counter');const input=document.querySelector('#file');const fileName=document.querySelector('#file-name');const fileSub=document.querySelector('#file-sub');function human(result){if(!result)return'操作完成';if(result.error)return result.error.message||String(result.error);const value=result.result??result;const text=value?.content?.find?.(item=>item&&item.type==='text')?.text;if(text)return text;const structured=value?.structuredContent;if(structured&&typeof structured==='object'){if(structured.completed===true)return'操作已完成';return JSON.stringify(structured,null,2)}return typeof value==='string'?value:'操作已完成'}function setState(label,message,error=false){title.textContent=label;out.textContent=message;const dot=document.querySelector('.dot');dot.style.background=error?'#ef6f7d':'#42d4b7'}addEventListener('message',event=>{const m=event.data;if(!m||m.jsonrpc!=='2.0')return;if(m.id!==undefined&&pending.has(m.id)){pending.get(m.id)(m);pending.delete(m.id)}if(m.method==='ui/notifications/tool-result')setState('操作已更新',human(m.params))});function call(name,args={}){const requestId=++id;setState('正在执行…','正在调用 '+name+'，请稍候。');return new Promise(resolve=>{pending.set(requestId,resolve);parent.postMessage({jsonrpc:'2.0',id:requestId,method:'tools/call',params:{name,arguments:args}},'*')})}async function run(name,args={}){try{const response=await call(name,args);const message=human(response);setState(response.error?'操作失败':'操作完成',message,Boolean(response.error));return response}catch(error){setState('操作失败',error?.message||String(error),true);return null}}document.querySelectorAll('[data-tool]').forEach(button=>button.onclick=()=>void run(button.dataset.tool));content.addEventListener('input',()=>counter.textContent=content.value.length+' / 20000 字');document.querySelector('#pick-file').onclick=()=>input.click();input.addEventListener('change',async()=>{const file=input.files?.[0];if(!file)return;if(file.size>2*1024*1024){fileName.textContent=file.name;fileSub.textContent='文件超过 2 MB，请选择更小的文本文件';setState('文件过大','为避免一次发送过多内容，单个文件最多 2 MB。',true);return}try{const text=await file.text();content.value=text.slice(0,20000);counter.textContent=content.value.length+' / 20000 字';fileName.textContent=file.name;fileSub.textContent=Math.max(1,Math.round(file.size/1024))+' KB · 已读取到输入框';setState('文件已就绪','请检查内容，然后点击“发送到全球法布施”。')}catch(error){setState('读取文件失败',error?.message||String(error),true)}});document.querySelector('#send').onclick=async()=>{const text=content.value.trim();if(!text){setState('还没有内容','请输入内容或先选择文本文件。',true);content.focus();return}const response=await run('send',{content:text,task_id:'mahayana'});if(response&&!response.error){content.value='';counter.textContent='0 / 20000 字'}}})()</script></body></html>"#
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
    fn home_uses_graphical_mcp_app_resource_with_file_picker() {
        let html = home_html();
        assert!(html.contains("tools/call"));
        assert!(html.contains("global-dharma-file-picker"));
        assert!(html.contains("发送到全球法布施"));
        assert!(html.contains("file.text()"));
        assert!(html.contains("data-tool=\"status\""));
        let legacy_bridge = ["Fabushi", "MiniApp"].concat();
        assert!(!html.contains(&legacy_bridge));
        assert_eq!(APP_MIME, "text/html;profile=mcp-app");
    }
}
