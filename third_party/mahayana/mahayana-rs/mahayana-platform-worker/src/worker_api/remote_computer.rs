use super::*;

const REMOTE_PAIRING_SECONDS: i64 = 10 * 60;
const REMOTE_CONTROL_SESSION_SECONDS: i64 = 2 * 60 * 60;
const REMOTE_SIGNAL_SECONDS: i64 = 5 * 60;
const REMOTE_SIGNAL_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerRegisterRequest {
    device_id: String,
    label: String,
    device_secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerHeartbeatRequest {
    device_id: String,
    device_secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerPairRequest {
    pairing_code: String,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerSessionCreateRequest {
    client_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerDesktopAuthRequest {
    device_secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerSignalRequest {
    session_id: String,
    sender_role: String,
    #[serde(default)]
    device_secret: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    mobile_token: Option<String>,
    kind: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerSignalDrainRequest {
    session_id: String,
    receiver_role: String,
    #[serde(default)]
    device_secret: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    mobile_token: Option<String>,
    #[serde(default)]
    after_signal_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerSessionCloseRequest {
    role: String,
    #[serde(default)]
    device_secret: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    mobile_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerRow {
    device_id: String,
    label: String,
    last_seen_at: i64,
    created_at: i64,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerPairRow {
    device_id: String,
    label: String,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerExistsRow {
    ok: i64,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerSecretRow {
    device_secret_hash: String,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerClientRow {
    client_id: String,
    label: String,
    paired_at: i64,
    last_seen_at: i64,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerSessionRow {
    client_id: String,
    mobile_token_hash: String,
    state: String,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerSessionListRow {
    session_id: String,
    client_id: String,
    client_label: String,
    state: String,
    created_at: i64,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerSignalRow {
    signal_id: i64,
    sender_role: String,
    kind: String,
    payload_json: String,
    created_at: i64,
}

fn remote_secret_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"fabushi-remote-computer-v1\0");
    hasher.update(value.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

async fn remote_desktop_secret_matches(
    database: &worker::D1Database,
    user_id: &str,
    device_id: &str,
    device_secret: &str,
) -> Result<bool> {
    if device_secret.len() < 48 || device_secret.len() > 256 {
        return Ok(false);
    }
    let row = worker::query!(
        database,
        "SELECT device_secret_hash FROM remote_computers
         WHERE device_id = ?1 AND user_id = ?2 AND revoked_at IS NULL LIMIT 1",
        device_id,
        user_id
    )?
    .first::<RemoteComputerSecretRow>(None)
    .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let candidate = remote_secret_hash(device_secret);
    Ok(constant_time_eq(
        candidate.as_bytes(),
        row.device_secret_hash.as_bytes(),
    ))
}

fn new_remote_pairing_code() -> String {
    let raw = Uuid::new_v4().simple().to_string();
    raw[..8].to_ascii_uppercase()
}

fn new_remote_mobile_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn remote_label(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.chars().count() <= 80).then(|| value.to_string())
}

fn remote_role(value: &str) -> bool {
    matches!(value, "desktop" | "mobile")
}

fn remote_signal_kind(value: &str) -> bool {
    matches!(value, "offer" | "answer" | "ice" | "ready" | "close")
}

fn remote_ice_servers(env: &Env) -> Vec<Value> {
    let mut servers =
        vec![json!({"urls": ["stun:stun.l.google.com:19302", "stun:stun1.l.google.com:19302"]})];
    if let Ok(turn_url) = env.var("REMOTE_TURN_URL") {
        let urls = turn_url
            .to_string()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !urls.is_empty() {
            let username = env
                .secret("REMOTE_TURN_USERNAME")
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let credential = env
                .secret("REMOTE_TURN_CREDENTIAL")
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default();
            if username.is_empty() || credential.is_empty() {
                servers.push(json!({"urls": urls}));
            } else {
                servers.push(json!({
                    "urls": urls,
                    "username": username,
                    "credential": credential,
                }));
            }
        }
    }
    servers
}

pub(super) async fn remote_computer_list(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to list computers.",
            );
        }
    };
    let database = context.env.d1(DATABASE_BINDING)?;
    let rows = worker::query!(
        &database,
        "SELECT device_id, label, last_seen_at, created_at
         FROM remote_computers
         WHERE user_id = ?1 AND revoked_at IS NULL
         ORDER BY last_seen_at DESC LIMIT 64",
        &account.user_id
    )?
    .all()
    .await?
    .results::<RemoteComputerRow>()?;
    let now = now_seconds();
    let computers = rows
        .into_iter()
        .map(|row| {
            json!({
                "deviceId": row.device_id,
                "label": row.label,
                "lastSeenAt": row.last_seen_at,
                "createdAt": row.created_at,
                "online": now.saturating_sub(row.last_seen_at) <= 45,
            })
        })
        .collect::<Vec<_>>();
    Ok(Response::from_json(&json!({"computers": computers}))?.with_headers(auth_headers()))
}

pub(super) async fn remote_computer_register(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to register a computer.",
            );
        }
    };
    let input: RemoteComputerRegisterRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_remote_computer",
                "Computer registration must be valid JSON.",
            );
        }
    };
    if !valid_relay_identifier(&input.device_id, 128) {
        return error_response(400, "invalid_device_id", "deviceId is invalid.");
    }
    let Some(label) = remote_label(&input.label) else {
        return error_response(400, "invalid_device_label", "Computer label is invalid.");
    };
    if input.device_secret.len() < 48 || input.device_secret.len() > 256 {
        return error_response(
            400,
            "invalid_device_secret",
            "Computer device secret is invalid.",
        );
    }
    let device_secret_hash = remote_secret_hash(&input.device_secret);
    let code = new_remote_pairing_code();
    let code_hash = remote_secret_hash(&code);
    let now = now_seconds();
    let expires_at = now + REMOTE_PAIRING_SECONDS;
    let database = context.env.d1(DATABASE_BINDING)?;
    let result = worker::query!(
        &database,
        "INSERT INTO remote_computers
         (device_id, user_id, label, device_secret_hash, pairing_code_hash, pairing_expires_at, created_at, last_seen_at, revoked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, NULL)
         ON CONFLICT(device_id) DO UPDATE SET
           label = excluded.label,
           pairing_code_hash = excluded.pairing_code_hash,
           pairing_expires_at = excluded.pairing_expires_at,
           last_seen_at = excluded.last_seen_at,
           revoked_at = NULL
         WHERE remote_computers.user_id = excluded.user_id
           AND remote_computers.device_secret_hash = excluded.device_secret_hash",
        &input.device_id,
        &account.user_id,
        &label,
        &device_secret_hash,
        &code_hash,
        expires_at,
        now
    )?
    .run()
    .await?;
    if result.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        return error_response(
            403,
            "device_secret_mismatch",
            "This computer registration is owned by another device secret.",
        );
    }
    Ok(Response::from_json(&json!({
        "deviceId": input.device_id,
        "label": label,
        "pairingCode": code,
        "pairingExpiresAt": expires_at,
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn remote_computer_heartbeat(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(401, "unauthorized", "A valid account token is required.");
        }
    };
    let input: RemoteComputerHeartbeatRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => return error_response(400, "invalid_heartbeat", "Heartbeat must be valid JSON."),
    };
    if !valid_relay_identifier(&input.device_id, 128) {
        return error_response(400, "invalid_device_id", "deviceId is invalid.");
    }
    if input.device_secret.len() < 48 || input.device_secret.len() > 256 {
        return error_response(
            400,
            "invalid_device_secret",
            "Computer device secret is invalid.",
        );
    }
    let device_secret_hash = remote_secret_hash(&input.device_secret);
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let result = worker::query!(
        &database,
        "UPDATE remote_computers SET last_seen_at = ?1
         WHERE device_id = ?2 AND user_id = ?3 AND device_secret_hash = ?4 AND revoked_at IS NULL",
        now,
        &input.device_id,
        &account.user_id,
        &device_secret_hash
    )?
    .run()
    .await?;
    if result.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        return error_response(404, "computer_not_found", "Computer is not registered.");
    }
    Ok(Response::from_json(&json!({"ok": true, "lastSeenAt": now}))?.with_headers(auth_headers()))
}

pub(super) async fn remote_computer_pair(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(401, "unauthorized", "Sign in before pairing a computer.");
        }
    };
    let input: RemoteComputerPairRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_pairing",
                "Pairing request must be valid JSON.",
            );
        }
    };
    let code = input.pairing_code.trim().to_ascii_uppercase();
    if code.len() != 8 || !code.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return error_response(400, "invalid_pairing_code", "Pairing code is invalid.");
    }
    let Some(label) = remote_label(&input.label) else {
        return error_response(400, "invalid_client_label", "Client label is invalid.");
    };
    let code_hash = remote_secret_hash(&code);
    let now = now_seconds();
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(computer) = worker::query!(
        &database,
        "SELECT device_id, label FROM remote_computers
         WHERE user_id = ?1 AND pairing_code_hash = ?2
           AND pairing_expires_at > ?3 AND revoked_at IS NULL
         LIMIT 1",
        &account.user_id,
        &code_hash,
        now
    )?
    .first::<RemoteComputerPairRow>(None)
    .await?
    else {
        return error_response(
            404,
            "pairing_code_not_found",
            "Pairing code is expired or does not belong to this account.",
        );
    };
    let client_id = format!("remote-client-{}", Uuid::new_v4());
    database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT INTO remote_computer_clients
                 (client_id, device_id, user_id, label, paired_at, last_seen_at, revoked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, NULL)",
                &client_id,
                &computer.device_id,
                &account.user_id,
                &label,
                now
            )?,
            worker::query!(
                &database,
                "UPDATE remote_computers
                 SET pairing_code_hash = NULL, pairing_expires_at = NULL
                 WHERE device_id = ?1 AND user_id = ?2",
                &computer.device_id,
                &account.user_id
            )?,
        ])
        .await?;
    Ok(Response::from_json(&json!({
        "deviceId": computer.device_id,
        "computerLabel": computer.label,
        "clientId": client_id,
        "clientLabel": label,
        "pairedAt": now,
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn remote_computer_clients(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let rows = worker::query!(
        &database,
        "SELECT c.client_id, c.label, c.paired_at, c.last_seen_at
         FROM remote_computer_clients c
         JOIN remote_computers d ON d.device_id = c.device_id
         WHERE c.user_id = ?1 AND c.device_id = ?2 AND c.revoked_at IS NULL
           AND d.user_id = ?1 AND d.revoked_at IS NULL
         ORDER BY c.paired_at DESC LIMIT 64",
        &account.user_id,
        device_id
    )?
    .all()
    .await?
    .results::<RemoteComputerClientRow>()?;
    let clients = rows
        .into_iter()
        .map(|row| {
            json!({
                "clientId": row.client_id,
                "label": row.label,
                "pairedAt": row.paired_at,
                "lastSeenAt": row.last_seen_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(
        Response::from_json(&json!({"deviceId": device_id, "clients": clients}))?
            .with_headers(auth_headers()),
    )
}

pub(super) async fn remote_computer_client_revoke(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?;
    let client_id = route_identifier(&context, "client_id")?;
    let now = now_seconds();
    let database = context.env.d1(DATABASE_BINDING)?;
    database
        .batch(vec![
            worker::query!(
                &database,
                "UPDATE remote_computer_clients SET revoked_at = ?1
                 WHERE client_id = ?2 AND device_id = ?3 AND user_id = ?4 AND revoked_at IS NULL",
                now,
                client_id,
                device_id,
                &account.user_id
            )?,
            worker::query!(
                &database,
                "UPDATE remote_computer_sessions SET state = 'closed', closed_at = ?1
                 WHERE client_id = ?2 AND device_id = ?3 AND user_id = ?4 AND state <> 'closed'",
                now,
                client_id,
                device_id,
                &account.user_id
            )?,
        ])
        .await?;
    Ok(
        Response::from_json(&json!({"revoked": true, "clientId": client_id}))?
            .with_headers(auth_headers()),
    )
}

pub(super) async fn remote_computer_sessions(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?;
    let input: RemoteComputerDesktopAuthRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_device_auth",
                "Desktop session list requires a device secret.",
            );
        }
    };
    let now = now_seconds();
    let database = context.env.d1(DATABASE_BINDING)?;
    if !remote_desktop_secret_matches(&database, &account.user_id, device_id, &input.device_secret)
        .await?
    {
        return error_response(
            403,
            "device_secret_mismatch",
            "Desktop device authentication failed.",
        );
    };
    worker::query!(
        &database,
        "UPDATE remote_computer_sessions SET state = 'closed', closed_at = ?1
         WHERE user_id = ?2 AND device_id = ?3 AND state <> 'closed' AND expires_at <= ?1",
        now,
        &account.user_id,
        device_id
    )?
    .run()
    .await?;
    let rows = worker::query!(
        &database,
        "SELECT s.session_id, s.client_id, c.label AS client_label,
                s.state, s.created_at, s.expires_at
         FROM remote_computer_sessions s
         JOIN remote_computer_clients c ON c.client_id = s.client_id
         JOIN remote_computers d ON d.device_id = s.device_id
         WHERE s.user_id = ?1 AND s.device_id = ?2 AND s.state <> 'closed'
           AND s.expires_at > ?3 AND c.revoked_at IS NULL AND d.revoked_at IS NULL
         ORDER BY s.created_at DESC LIMIT 32",
        &account.user_id,
        device_id,
        now
    )?
    .all()
    .await?
    .results::<RemoteComputerSessionListRow>()?;
    let sessions = rows
        .into_iter()
        .map(|row| {
            json!({
                "sessionId": row.session_id,
                "clientId": row.client_id,
                "clientLabel": row.client_label,
                "state": row.state,
                "createdAt": row.created_at,
                "expiresAt": row.expires_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(Response::from_json(&json!({
        "deviceId": device_id,
        "sessions": sessions,
        "iceServers": remote_ice_servers(&context.env),
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn remote_computer_session_create(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?.to_string();
    let input: RemoteComputerSessionCreateRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_control_session",
                "Session request must be valid JSON.",
            );
        }
    };
    if !valid_relay_identifier(&input.client_id, 160) {
        return error_response(400, "invalid_client_id", "clientId is invalid.");
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let client = worker::query!(
        &database,
        "SELECT client_id, label, paired_at, last_seen_at
         FROM remote_computer_clients
         WHERE client_id = ?1 AND device_id = ?2 AND user_id = ?3 AND revoked_at IS NULL
         LIMIT 1",
        &input.client_id,
        &device_id,
        &account.user_id
    )?
    .first::<RemoteComputerClientRow>(None)
    .await?;
    if client.is_none() {
        return error_response(
            403,
            "client_not_paired",
            "This phone is not paired with the computer.",
        );
    }
    let computer = worker::query!(
        &database,
        "SELECT device_id, label FROM remote_computers
         WHERE device_id = ?1 AND user_id = ?2 AND revoked_at IS NULL LIMIT 1",
        &device_id,
        &account.user_id
    )?
    .first::<RemoteComputerPairRow>(None)
    .await?;
    if computer.is_none() {
        return error_response(404, "computer_not_found", "Computer is not available.");
    }
    let session_id = format!("remote-session-{}", Uuid::new_v4());
    let mobile_token = new_remote_mobile_token();
    let mobile_token_hash = remote_secret_hash(&mobile_token);
    let now = now_seconds();
    let expires_at = now + REMOTE_CONTROL_SESSION_SECONDS;
    worker::query!(
        &database,
        "INSERT INTO remote_computer_sessions
         (session_id, device_id, client_id, user_id, mobile_token_hash, state, created_at, expires_at, closed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, NULL)",
        &session_id,
        &device_id,
        &input.client_id,
        &account.user_id,
        &mobile_token_hash,
        now,
        expires_at
    )?
    .run()
    .await?;
    Ok(Response::from_json(&json!({
        "sessionId": session_id,
        "deviceId": device_id,
        "clientId": input.client_id,
        "mobileToken": mobile_token,
        "createdAt": now,
        "expiresAt": expires_at,
        "state": "pending",
        "iceServers": remote_ice_servers(&context.env),
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn remote_computer_session_activate(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?;
    let session_id = route_identifier(&context, "session_id")?;
    let input: RemoteComputerDesktopAuthRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_device_auth",
                "Desktop session activation requires a device secret.",
            );
        }
    };
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let device_secret_hash = remote_secret_hash(&input.device_secret);
    let result = worker::query!(
        &database,
        "UPDATE remote_computer_sessions SET state = 'active'
         WHERE session_id = ?1 AND device_id = ?2 AND user_id = ?3
           AND state = 'pending' AND expires_at > ?4
           AND EXISTS (SELECT 1 FROM remote_computers d
                       WHERE d.device_id = ?2 AND d.user_id = ?3
                         AND d.device_secret_hash = ?5 AND d.revoked_at IS NULL)
           AND NOT EXISTS (
               SELECT 1 FROM remote_computer_sessions active
               WHERE active.device_id = ?2 AND active.user_id = ?3
                 AND active.session_id <> ?1 AND active.state = 'active'
                 AND active.expires_at > ?4
           )",
        session_id,
        device_id,
        &account.user_id,
        now,
        &device_secret_hash
    )?
    .run()
    .await?;
    if result.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        return error_response(
            404,
            "control_session_not_found",
            "Pending control session was not found.",
        );
    }
    let Some(session) =
        load_remote_control_session(&database, &account.user_id, device_id, session_id).await?
    else {
        return error_response(
            404,
            "control_session_not_found",
            "Control session disappeared after activation.",
        );
    };
    Ok(Response::from_json(&json!({
        "sessionId": session_id,
        "clientId": session.client_id,
        "expiresAt": session.expires_at,
        "state": "active",
        "iceServers": remote_ice_servers(&context.env),
    }))?
    .with_headers(auth_headers()))
}

async fn load_remote_control_session(
    database: &worker::D1Database,
    user_id: &str,
    device_id: &str,
    session_id: &str,
) -> Result<Option<RemoteComputerSessionRow>> {
    worker::query!(
        database,
        "SELECT client_id, mobile_token_hash, state, expires_at
         FROM remote_computer_sessions
         WHERE session_id = ?1 AND device_id = ?2 AND user_id = ?3 LIMIT 1",
        session_id,
        device_id,
        user_id
    )?
    .first::<RemoteComputerSessionRow>(None)
    .await
}

fn remote_session_actor_allowed(
    session: &RemoteComputerSessionRow,
    role: &str,
    client_id: Option<&str>,
    mobile_token: Option<&str>,
    desktop_authorized: bool,
    now: i64,
) -> bool {
    if session.state == "closed" || session.expires_at <= now {
        return false;
    }
    match role {
        "desktop" => desktop_authorized,
        "mobile" => {
            client_id == Some(session.client_id.as_str())
                && mobile_token.map(remote_secret_hash).is_some_and(|hash| {
                    constant_time_eq(hash.as_bytes(), session.mobile_token_hash.as_bytes())
                })
        }
        _ => false,
    }
}

pub(super) async fn remote_computer_session_close(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?.to_string();
    let session_id = route_identifier(&context, "session_id")?.to_string();
    let input: RemoteComputerSessionCloseRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_control_session",
                "Close request must be valid JSON.",
            );
        }
    };
    if !remote_role(&input.role) {
        return error_response(
            400,
            "invalid_control_role",
            "Control role must be desktop or mobile.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let Some(session) =
        load_remote_control_session(&database, &account.user_id, &device_id, &session_id).await?
    else {
        return error_response(
            404,
            "control_session_not_found",
            "Control session was not found.",
        );
    };
    let desktop_authorized = if input.role == "desktop" {
        match input.device_secret.as_deref() {
            Some(secret) => {
                remote_desktop_secret_matches(&database, &account.user_id, &device_id, secret)
                    .await?
            }
            None => false,
        }
    } else {
        false
    };
    if !remote_session_actor_allowed(
        &session,
        &input.role,
        input.client_id.as_deref(),
        input.mobile_token.as_deref(),
        desktop_authorized,
        now,
    ) {
        return error_response(
            403,
            "control_session_forbidden",
            "This client cannot close the control session.",
        );
    }
    worker::query!(
        &database,
        "UPDATE remote_computer_sessions SET state = 'closed', closed_at = ?1
         WHERE session_id = ?2 AND user_id = ?3",
        now,
        &session_id,
        &account.user_id
    )?
    .run()
    .await?;
    Ok(
        Response::from_json(&json!({"sessionId": session_id, "state": "closed"}))?
            .with_headers(auth_headers()),
    )
}

pub(super) async fn remote_computer_signal(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?.to_string();
    let input: RemoteComputerSignalRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(400, "invalid_signal", "Signal request must be valid JSON.");
        }
    };
    if !valid_relay_identifier(&input.session_id, 160)
        || !remote_role(&input.sender_role)
        || !remote_signal_kind(&input.kind)
    {
        return error_response(400, "invalid_signal", "Signal identifiers are invalid.");
    }
    let payload_json = serde_json::to_string(&input.payload)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    if payload_json.len() > REMOTE_SIGNAL_MAX_BYTES {
        return error_response(
            413,
            "signal_too_large",
            "WebRTC signal payload exceeds 256 KiB.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let Some(session) =
        load_remote_control_session(&database, &account.user_id, &device_id, &input.session_id)
            .await?
    else {
        return error_response(
            404,
            "control_session_not_found",
            "Control session was not found.",
        );
    };
    let desktop_authorized = if input.sender_role == "desktop" {
        match input.device_secret.as_deref() {
            Some(secret) => {
                remote_desktop_secret_matches(&database, &account.user_id, &device_id, secret)
                    .await?
            }
            None => false,
        }
    } else {
        false
    };
    if !remote_session_actor_allowed(
        &session,
        &input.sender_role,
        input.client_id.as_deref(),
        input.mobile_token.as_deref(),
        desktop_authorized,
        now,
    ) {
        return error_response(
            403,
            "control_session_forbidden",
            "This client cannot signal for the control session.",
        );
    }
    let expires_at = now + REMOTE_SIGNAL_SECONDS;
    database
        .batch(vec![
            worker::query!(
                &database,
                "DELETE FROM remote_computer_signals WHERE expires_at <= ?1",
                now
            )?,
            worker::query!(
                &database,
                "INSERT INTO remote_computer_signals
                 (session_id, user_id, sender_role, kind, payload_json, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                &input.session_id,
                &account.user_id,
                &input.sender_role,
                &input.kind,
                &payload_json,
                now,
                expires_at
            )?,
        ])
        .await?;
    if input.sender_role == "mobile" {
        worker::query!(
            &database,
            "UPDATE remote_computer_clients SET last_seen_at = ?1
             WHERE client_id = ?2 AND user_id = ?3",
            now,
            &session.client_id,
            &account.user_id
        )?
        .run()
        .await?;
    }
    Ok(
        Response::from_json(&json!({"accepted": true, "expiresAt": expires_at}))?
            .with_headers(auth_headers()),
    )
}

pub(super) async fn remote_computer_signal_drain(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?.to_string();
    let input: RemoteComputerSignalDrainRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_signal_drain",
                "Signal drain must be valid JSON.",
            );
        }
    };
    if !valid_relay_identifier(&input.session_id, 160) || !remote_role(&input.receiver_role) {
        return error_response(
            400,
            "invalid_signal_drain",
            "Signal drain identifiers are invalid.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let Some(session) =
        load_remote_control_session(&database, &account.user_id, &device_id, &input.session_id)
            .await?
    else {
        return error_response(
            404,
            "control_session_not_found",
            "Control session was not found.",
        );
    };
    let desktop_authorized = if input.receiver_role == "desktop" {
        match input.device_secret.as_deref() {
            Some(secret) => {
                remote_desktop_secret_matches(&database, &account.user_id, &device_id, secret)
                    .await?
            }
            None => false,
        }
    } else {
        false
    };
    if !remote_session_actor_allowed(
        &session,
        &input.receiver_role,
        input.client_id.as_deref(),
        input.mobile_token.as_deref(),
        desktop_authorized,
        now,
    ) {
        return error_response(
            403,
            "control_session_forbidden",
            "This client cannot read control signals.",
        );
    }
    let after = input.after_signal_id.unwrap_or(0).max(0);
    let rows = worker::query!(
        &database,
        "SELECT signal_id, sender_role, kind, payload_json, created_at
         FROM remote_computer_signals
         WHERE user_id = ?1 AND session_id = ?2 AND sender_role <> ?3
           AND signal_id > ?4 AND expires_at > ?5
         ORDER BY signal_id ASC LIMIT 128",
        &account.user_id,
        &input.session_id,
        &input.receiver_role,
        after,
        now
    )?
    .all()
    .await?
    .results::<RemoteComputerSignalRow>()?;
    let mut last_signal_id = after;
    let signals = rows
        .into_iter()
        .map(|row| {
            last_signal_id = last_signal_id.max(row.signal_id);
            json!({
                "signalId": row.signal_id,
                "senderRole": row.sender_role,
                "kind": row.kind,
                "payload": serde_json::from_str::<Value>(&row.payload_json).unwrap_or(Value::Null),
                "createdAt": row.created_at,
            })
        })
        .collect::<Vec<_>>();
    if input.receiver_role == "mobile" {
        worker::query!(
            &database,
            "UPDATE remote_computer_clients SET last_seen_at = ?1
             WHERE client_id = ?2 AND user_id = ?3",
            now,
            &session.client_id,
            &account.user_id
        )?
        .run()
        .await?;
    }
    Ok(Response::from_json(&json!({
        "sessionId": input.session_id,
        "signals": signals,
        "lastSignalId": last_signal_id,
    }))?
    .with_headers(auth_headers()))
}
