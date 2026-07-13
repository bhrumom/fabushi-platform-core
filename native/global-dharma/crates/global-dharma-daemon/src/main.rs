use global_dharma_core::{
    content_hash, validate_request, Config, DeliveryRequest, RateLimiter, Receipt,
};
use std::{
    env,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

struct State {
    config: Config,
    limiter: RateLimiter,
    receipts: Vec<Receipt>,
    logs: Vec<String>,
}
fn main() {
    let path = env::var("GLOBAL_DHARMA_CONFIG")
        .unwrap_or_else(|_| "/etc/global-dharma/global-dharma.toml".into());
    let config = Config::load(&path).unwrap_or_else(|e| {
        eprintln!("invalid global dharma config: {e}");
        std::process::exit(2)
    });
    let bind = config.daemon.bind.clone();
    let state = Arc::new(Mutex::new(State {
        limiter: RateLimiter::new(config.daemon.max_requests_per_minute),
        config,
        receipts: vec![],
        logs: vec![format!("started {bind}")],
    }));
    let listener = TcpListener::bind(&bind).unwrap_or_else(|e| panic!("bind {bind}: {e}"));
    for stream in listener.incoming().flatten() {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            let _ = handle(stream, state);
        });
    }
}
fn handle(mut stream: TcpStream, state: Arc<Mutex<State>>) -> Result<(), String> {
    let (head, body) = read_request(&mut stream)?;
    let target = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let response = match target {
        "/health" | "/status" => {
            let s = state.lock().unwrap();
            serde_json::json!({"ok":true,"nodes":s.config.nodes.iter().map(|n| &n.id).collect::<Vec<_>>(),"receipts":s.receipts.len()})
        }
        "/logs" => {
            let s = state.lock().unwrap();
            serde_json::json!({"ok":true,"logs":s.logs})
        }
        "/send" => match serde_json::from_str::<DeliveryRequest>(&body)
            .map_err(|e| e.to_string())
            .and_then(|r| send(&state, r))
        {
            Ok(receipts) => serde_json::json!({"ok":true,"receipts":receipts}),
            Err(e) => serde_json::json!({"ok":false,"error":e}),
        },
        _ => serde_json::json!({"ok":false,"error":"not_found"}),
    };
    let json = response.to_string();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", json.len(), json).map_err(|e| e.to_string())?;
    Ok(())
}
fn read_request(stream: &mut TcpStream) -> Result<(String, String), String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("incomplete request".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if bytes.len() > 16 * 1024 {
            return Err("request headers too large".into());
        }
    }
    let end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&bytes[..end]).to_string();
    let length = head
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
                .and_then(|n| n.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    if length > 16 * 1024 * 1024 {
        return Err("request body too large".into());
    }
    while bytes.len() - end < length {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("truncated request body".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok((
        head,
        String::from_utf8_lossy(&bytes[end..end + length]).to_string(),
    ))
}
fn send(state: &Arc<Mutex<State>>, request: DeliveryRequest) -> Result<Vec<Receipt>, String> {
    let (nodes, hash) = {
        let mut s = state.lock().unwrap();
        validate_request(&s.config, &request)?;
        s.limiter.acquire()?;
        (s.config.nodes.clone(), content_hash(&request.content))
    };
    let mut receipts = Vec::new();
    for node in nodes {
        if request
            .region
            .as_ref()
            .is_some_and(|r| !node.regions.is_empty() && !node.regions.iter().any(|n| n == r))
        {
            continue;
        }
        let response = ureq::post(node.endpoint.as_str()).set("Content-Type", "application/json").set("X-Global-Dharma-Task", &request.task_id).send_json(serde_json::json!({"taskId":request.task_id,"content":request.content,"contentSha256":hash})).map_err(|e| format!("node {}: {e}", node.id))?;
        let key = response
            .header("X-Global-Dharma-Node-Key")
            .ok_or_else(|| format!("node {} omitted identity header", node.id))?;
        if !key.eq_ignore_ascii_case(&node.public_key_sha256) {
            return Err(format!("node {} identity mismatch", node.id));
        }
        receipts.push(Receipt {
            task_id: request.task_id.clone(),
            node_id: node.id,
            content_sha256: hash.clone(),
            delivered_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            status: "delivered".into(),
        });
    }
    if receipts.is_empty() {
        return Err("no authorized node matched this request".into());
    }
    let mut s = state.lock().unwrap();
    s.receipts.extend(receipts.clone());
    s.logs.push(format!(
        "delivered task {} to {} node(s)",
        request.task_id,
        receipts.len()
    ));
    let retained_from = s.logs.len().saturating_sub(200);
    s.logs.drain(..retained_from);
    Ok(receipts)
}
