use super::*;

const REMOTE_PAIRING_SECONDS: i64 = 10 * 60;
const REMOTE_CONTROL_SESSION_SECONDS: i64 = 2 * 60 * 60;
const REMOTE_SIGNAL_SECONDS: i64 = 5 * 60;
const REMOTE_SIGNAL_MAX_BYTES: usize = 256 * 1024;
const REMOTE_SIGNAL_MAX_ROWS_PER_SESSION: i64 = 512;
const REMOTE_SESSION_MAX_PER_CLIENT: i64 = 4;
const REMOTE_SESSION_MAX_PER_DEVICE: i64 = 32;
const REMOTE_CLIENT_MAX_PER_DEVICE: i64 = 64;
const REMOTE_COMPUTER_MAX_PER_ACCOUNT: i64 = 64;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerRegisterRequest {
    device_id: String,
    label: String,
    device_secret: String,
    #[serde(default = "default_remote_provider")]
    provider: String,
    #[serde(default = "default_remote_platform")]
    platform: String,
    #[serde(default = "default_remote_app_version")]
    app_version: String,
    #[serde(default)]
    capabilities: Vec<String>,
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
    client_token: String,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteComputerTransportRequest {
    role: String,
    #[serde(default)]
    device_secret: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    mobile_token: Option<String>,
    #[serde(default)]
    direct_available: bool,
    #[serde(default)]
    relay_region: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerRow {
    device_id: String,
    label: String,
    provider: String,
    platform: String,
    app_version: String,
    capabilities_json: String,
    active_session_count: i64,
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
struct RemoteComputerClientAuthRow {
    client_token_hash: String,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerSessionRow {
    client_id: String,
    mobile_token_hash: String,
    state: String,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct RemoteComputerTransportRow {
    client_id: String,
    mobile_token_hash: String,
    state: String,
    expires_at: i64,
    provider: String,
    route_policy: String,
    selected_route: Option<String>,
    relay_region: Option<String>,
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
    raw[..12].to_ascii_uppercase()
}

fn new_remote_mobile_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn new_remote_client_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn remote_label(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.chars().count() <= 80).then(|| value.to_string())
}

fn default_remote_provider() -> String {
    "fabushi-webrtc".to_string()
}

fn default_remote_platform() -> String {
    "unknown".to_string()
}

fn default_remote_app_version() -> String {
    "unknown".to_string()
}

fn remote_provider(value: &str) -> Option<String> {
    let value = value.trim();
    matches!(value, "fabushi-webrtc" | "rustdesk-sidecar").then(|| value.to_string())
}

fn remote_platform(value: &str) -> Option<String> {
    let value = value.trim();
    matches!(
        value,
        "windows" | "macos" | "linux" | "android" | "ios" | "web" | "unknown"
    )
    .then(|| value.to_string())
}

fn remote_app_version(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".+_-".contains(character)))
    .then(|| value.to_string())
}

fn remote_capabilities(values: &[String]) -> Option<Vec<String>> {
    if values.len() > 32 {
        return None;
    }
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if !matches!(
            value,
            "remote-desktop"
                | "input"
                | "clipboard"
                | "file-transfer"
                | "display"
                | "audio"
                | "session-management"
        ) {
            return None;
        }
        if !normalized.iter().any(|existing| existing == value) {
            normalized.push(value.to_string());
        }
    }
    normalized.sort();
    Some(normalized)
}

fn remote_role(value: &str) -> bool {
    matches!(value, "desktop" | "mobile")
}

fn remote_signal_kind(value: &str) -> bool {
    matches!(value, "offer" | "answer" | "ice" | "ready" | "close")
}

fn remote_signal_kind_allowed(role: &str, kind: &str) -> bool {
    match role {
        "mobile" => matches!(kind, "offer" | "ice" | "ready" | "close"),
        "desktop" => matches!(kind, "answer" | "ice" | "ready" | "close"),
        _ => false,
    }
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
        "SELECT computer.device_id, computer.label, computer.provider,
                computer.platform, computer.app_version, computer.capabilities_json,
                (SELECT COUNT(*) FROM remote_computer_sessions session
                 WHERE session.user_id = computer.user_id
                   AND session.device_id = computer.device_id
                   AND session.state = 'active'
                   AND session.expires_at > ?2) AS active_session_count,
                computer.last_seen_at, computer.created_at
         FROM remote_computers computer
         WHERE computer.user_id = ?1 AND computer.revoked_at IS NULL
         ORDER BY computer.last_seen_at DESC LIMIT 64",
        &account.user_id,
        now_seconds()
    )?
    .all()
    .await?
    .results::<RemoteComputerRow>()?;
    let now = now_seconds();
    let computers = rows
        .into_iter()
        .map(|row| {
            let capabilities = serde_json::from_str::<Vec<String>>(&row.capabilities_json)
                .ok()
                .and_then(|values| remote_capabilities(&values))
                .unwrap_or_default();
            json!({
                "deviceId": row.device_id,
                "label": row.label,
                "provider": row.provider,
                "platform": row.platform,
                "appVersion": row.app_version,
                "capabilities": capabilities,
                "activeSessionCount": row.active_session_count,
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
    let Some(provider) = remote_provider(&input.provider) else {
        return error_response(400, "invalid_provider", "Computer provider is invalid.");
    };
    let Some(platform) = remote_platform(&input.platform) else {
        return error_response(400, "invalid_platform", "Computer platform is invalid.");
    };
    let Some(app_version) = remote_app_version(&input.app_version) else {
        return error_response(
            400,
            "invalid_app_version",
            "Computer appVersion is invalid.",
        );
    };
    let Some(capabilities) = remote_capabilities(&input.capabilities) else {
        return error_response(
            400,
            "invalid_capabilities",
            "Computer capabilities are invalid.",
        );
    };
    let capabilities_json = serde_json::to_string(&capabilities)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let device_secret_hash = remote_secret_hash(&input.device_secret);
    let code = new_remote_pairing_code();
    let code_hash = remote_secret_hash(&code);
    let now = now_seconds();
    let expires_at = now + REMOTE_PAIRING_SECONDS;
    let database = context.env.d1(DATABASE_BINDING)?;
    let result = worker::query!(
        &database,
        "INSERT INTO remote_computers
         (device_id, user_id, label, device_secret_hash, pairing_code_hash, pairing_expires_at,
          created_at, last_seen_at, revoked_at, provider, platform, app_version, capabilities_json)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, NULL, ?8, ?9, ?10, ?11
         WHERE (SELECT COUNT(*) FROM remote_computers existing
                WHERE existing.user_id = ?2 AND existing.revoked_at IS NULL
                  AND existing.device_id <> ?1) < ?12
         ON CONFLICT(device_id) DO UPDATE SET
           label = excluded.label,
           pairing_code_hash = excluded.pairing_code_hash,
           pairing_expires_at = excluded.pairing_expires_at,
           last_seen_at = excluded.last_seen_at,
           provider = excluded.provider,
           platform = excluded.platform,
           app_version = excluded.app_version,
           capabilities_json = excluded.capabilities_json,
           revoked_at = NULL
         WHERE remote_computers.user_id = excluded.user_id
           AND remote_computers.device_secret_hash = excluded.device_secret_hash",
        &input.device_id,
        &account.user_id,
        &label,
        &device_secret_hash,
        &code_hash,
        expires_at,
        now,
        &provider,
        &platform,
        &app_version,
        &capabilities_json,
        REMOTE_COMPUTER_MAX_PER_ACCOUNT
    )?
    .run()
    .await?;
    if result.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        let owned = worker::query!(
            &database,
            "SELECT 1 AS ok FROM remote_computers
             WHERE device_id = ?1 AND user_id = ?2 AND device_secret_hash = ?3 LIMIT 1",
            &input.device_id,
            &account.user_id,
            &device_secret_hash
        )?
        .first::<RemoteComputerExistsRow>(None)
        .await?
        .is_some();
        if owned {
            return error_response(
                429,
                "computer_limit",
                "This account has too many registered computers; remove one before reactivating this device.",
            );
        }
        return error_response(
            403,
            "device_secret_mismatch",
            "This computer registration is owned by another device secret.",
        );
    }
    Ok(Response::from_json(&json!({
        "deviceId": input.device_id,
        "label": label,
        "provider": provider,
        "platform": platform,
        "appVersion": app_version,
        "capabilities": capabilities,
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
    if code.len() != 12 || !code.bytes().all(|byte| byte.is_ascii_hexdigit()) {
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
    let client_token = new_remote_client_token();
    let client_token_hash = remote_secret_hash(&client_token);
    let pairing_claim_hash = remote_secret_hash(&format!("pair-claim:{client_id}:{client_token}"));
    let results = database
        .batch(vec![
            worker::query!(
                &database,
                "UPDATE remote_computers SET pairing_code_hash = ?1
                 WHERE device_id = ?2 AND user_id = ?3 AND pairing_code_hash = ?4
                   AND pairing_expires_at > ?5 AND revoked_at IS NULL
                   AND (SELECT COUNT(*) FROM remote_computer_clients c
                        WHERE c.device_id = remote_computers.device_id
                          AND c.user_id = remote_computers.user_id
                          AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL) < ?6",
                &pairing_claim_hash,
                &computer.device_id,
                &account.user_id,
                &code_hash,
                now,
                REMOTE_CLIENT_MAX_PER_DEVICE
            )?,
            worker::query!(
                &database,
                "INSERT INTO remote_computer_clients
                 (client_id, device_id, user_id, label, paired_at, last_seen_at, revoked_at, client_token_hash)
                 SELECT ?1, d.device_id, d.user_id, ?2, ?3, ?3, NULL, ?4
                 FROM remote_computers d
                 WHERE d.device_id = ?5 AND d.user_id = ?6 AND d.pairing_code_hash = ?7
                   AND d.revoked_at IS NULL
                   AND (SELECT COUNT(*) FROM remote_computer_clients c
                        WHERE c.device_id = d.device_id AND c.user_id = d.user_id
                          AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL) < ?8",
                &client_id,
                &label,
                now,
                &client_token_hash,
                &computer.device_id,
                &account.user_id,
                &pairing_claim_hash,
                REMOTE_CLIENT_MAX_PER_DEVICE
            )?,
            worker::query!(
                &database,
                "UPDATE remote_computers
                 SET pairing_code_hash = CASE
                       WHEN EXISTS (SELECT 1 FROM remote_computer_clients c
                                    WHERE c.client_id = ?4 AND c.device_id = ?1
                                      AND c.user_id = ?2 AND c.revoked_at IS NULL
                                      AND c.client_token_hash IS NOT NULL)
                       THEN NULL ELSE ?5 END,
                     pairing_expires_at = CASE
                       WHEN EXISTS (SELECT 1 FROM remote_computer_clients c
                                    WHERE c.client_id = ?4 AND c.device_id = ?1
                                      AND c.user_id = ?2 AND c.revoked_at IS NULL
                                      AND c.client_token_hash IS NOT NULL)
                       THEN NULL ELSE pairing_expires_at END
                 WHERE device_id = ?1 AND user_id = ?2 AND pairing_code_hash = ?3",
                &computer.device_id,
                &account.user_id,
                &pairing_claim_hash,
                &client_id,
                &code_hash
            )?,
        ])
        .await?;
    let claim_changes = d1_changes(results.first());
    let insert_changes = d1_changes(results.get(1));
    if claim_changes == 0 {
        let limit_reached = worker::query!(
            &database,
            "SELECT 1 AS ok FROM remote_computers d
             WHERE d.device_id = ?1 AND d.user_id = ?2 AND d.pairing_code_hash = ?3
               AND d.pairing_expires_at > ?4 AND d.revoked_at IS NULL
               AND (SELECT COUNT(*) FROM remote_computer_clients c
                    WHERE c.device_id = d.device_id AND c.user_id = d.user_id
                      AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL) >= ?5
             LIMIT 1",
            &computer.device_id,
            &account.user_id,
            &code_hash,
            now,
            REMOTE_CLIENT_MAX_PER_DEVICE
        )?
        .first::<RemoteComputerExistsRow>(None)
        .await?
        .is_some();
        if limit_reached {
            return error_response(
                429,
                "paired_client_limit",
                "This computer has too many paired clients; revoke one before pairing again.",
            );
        }
        return error_response(
            404,
            "pairing_code_not_found",
            "Pairing code was already consumed or expired.",
        );
    }
    if insert_changes == 0 {
        return error_response(
            429,
            "paired_client_limit",
            "This computer has too many paired clients; revoke one before pairing again.",
        );
    }
    Ok(Response::from_json(&json!({
        "deviceId": computer.device_id,
        "computerLabel": computer.label,
        "clientId": client_id,
        "clientToken": client_token,
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
           AND c.client_token_hash IS NOT NULL
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
         JOIN remote_computer_clients c
           ON c.client_id = s.client_id AND c.device_id = s.device_id AND c.user_id = s.user_id
         JOIN remote_computers d ON d.device_id = s.device_id AND d.user_id = s.user_id
         WHERE s.user_id = ?1 AND s.device_id = ?2 AND s.state <> 'closed'
           AND s.expires_at > ?3 AND c.revoked_at IS NULL
           AND c.client_token_hash IS NOT NULL AND d.revoked_at IS NULL
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
                "permissions": {"display": true, "input": true, "clipboard": false, "fileTransfer": false, "audio": false},
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
    if input.client_token.len() < 48 || input.client_token.len() > 256 {
        return error_response(
            400,
            "invalid_client_token",
            "Paired client credential is invalid.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let client = worker::query!(
        &database,
        "SELECT client_token_hash
         FROM remote_computer_clients
         WHERE client_id = ?1 AND device_id = ?2 AND user_id = ?3
           AND revoked_at IS NULL AND client_token_hash IS NOT NULL
         LIMIT 1",
        &input.client_id,
        &device_id,
        &account.user_id
    )?
    .first::<RemoteComputerClientAuthRow>(None)
    .await?;
    let candidate_hash = remote_secret_hash(&input.client_token);
    if !client.is_some_and(|client| {
        constant_time_eq(
            candidate_hash.as_bytes(),
            client.client_token_hash.as_bytes(),
        )
    }) {
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
        "UPDATE remote_computer_sessions SET state = 'closed', closed_at = ?1
         WHERE user_id = ?2 AND device_id = ?3 AND state <> 'closed' AND expires_at <= ?1",
        now,
        &account.user_id,
        &device_id
    )?
    .run()
    .await?;
    let inserted = worker::query!(
        &database,
        "INSERT INTO remote_computer_sessions
         (session_id, device_id, client_id, user_id, mobile_token_hash, state, created_at, expires_at, closed_at)
         SELECT ?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, NULL
         WHERE EXISTS (
                 SELECT 1 FROM remote_computer_clients c
                 JOIN remote_computers d
                   ON d.device_id = c.device_id AND d.user_id = c.user_id
                 WHERE c.client_id = ?3 AND c.device_id = ?2 AND c.user_id = ?4
                   AND c.client_token_hash = ?10 AND c.revoked_at IS NULL
                   AND d.revoked_at IS NULL
               )
           AND (SELECT COUNT(*) FROM remote_computer_sessions
                WHERE user_id = ?4 AND device_id = ?2 AND client_id = ?3
                  AND state <> 'closed' AND expires_at > ?6) < ?8
           AND (SELECT COUNT(*) FROM remote_computer_sessions
                WHERE user_id = ?4 AND device_id = ?2
                  AND state <> 'closed' AND expires_at > ?6) < ?9",
        &session_id,
        &device_id,
        &input.client_id,
        &account.user_id,
        &mobile_token_hash,
        now,
        expires_at,
        REMOTE_SESSION_MAX_PER_CLIENT,
        REMOTE_SESSION_MAX_PER_DEVICE,
        &candidate_hash
    )?
    .run()
    .await?;
    if inserted.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        let still_paired = worker::query!(
            &database,
            "SELECT 1 AS ok FROM remote_computer_clients c
             JOIN remote_computers d
               ON d.device_id = c.device_id AND d.user_id = c.user_id
             WHERE c.client_id = ?1 AND c.device_id = ?2 AND c.user_id = ?3
               AND c.client_token_hash = ?4 AND c.revoked_at IS NULL
               AND d.revoked_at IS NULL LIMIT 1",
            &input.client_id,
            &device_id,
            &account.user_id,
            &candidate_hash
        )?
        .first::<RemoteComputerExistsRow>(None)
        .await?
        .is_some();
        if !still_paired {
            return error_response(
                403,
                "client_not_paired",
                "This phone is no longer paired with the computer.",
            );
        }
        return error_response(
            429,
            "control_session_limit",
            "Too many pending control sessions exist for this paired client.",
        );
    }
    Ok(Response::from_json(&json!({
        "sessionId": session_id,
        "deviceId": device_id,
        "clientId": input.client_id,
        "mobileToken": mobile_token,
        "createdAt": now,
        "expiresAt": expires_at,
        "state": "pending",
        "permissions": {"display": true, "input": true, "clipboard": false, "fileTransfer": false, "audio": false},
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
           AND EXISTS (SELECT 1 FROM remote_computer_clients c
                       WHERE c.client_id = remote_computer_sessions.client_id
                         AND c.device_id = ?2 AND c.user_id = ?3
                         AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL)
           AND NOT EXISTS (
               SELECT 1 FROM remote_computer_sessions active
               WHERE active.device_id = ?2 AND active.user_id = ?3
                 AND active.session_id <> ?1 AND active.state = 'active'
                 AND active.expires_at > ?4
                 AND EXISTS (SELECT 1 FROM remote_computer_clients active_client
                             WHERE active_client.client_id = active.client_id
                               AND active_client.device_id = active.device_id
                               AND active_client.user_id = active.user_id
                               AND active_client.revoked_at IS NULL
                               AND active_client.client_token_hash IS NOT NULL)
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
        "SELECT s.client_id, s.mobile_token_hash, s.state, s.expires_at
         FROM remote_computer_sessions s
         JOIN remote_computer_clients c
           ON c.client_id = s.client_id AND c.device_id = s.device_id AND c.user_id = s.user_id
         JOIN remote_computers d ON d.device_id = s.device_id AND d.user_id = s.user_id
         WHERE s.session_id = ?1 AND s.device_id = ?2 AND s.user_id = ?3
           AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL
           AND d.revoked_at IS NULL LIMIT 1",
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
    // Pending sessions are only consent requests. Neither controller nor target may
    // negotiate transport or exchange signaling until the target device explicitly
    // activates the session with its device secret.
    if session.state != "active" || session.expires_at <= now {
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

fn remote_relay_available(env: &Env, provider: &str) -> bool {
    match provider {
        "fabushi-webrtc" => env
            .var("REMOTE_TURN_URL")
            .ok()
            .is_some_and(|value| !value.to_string().trim().is_empty()),
        "rustdesk-sidecar" => env
            .var("RUSTDESK_RELAY_URL")
            .ok()
            .is_some_and(|value| !value.to_string().trim().is_empty()),
        _ => false,
    }
}

fn remote_relay_region(value: Option<&str>) -> Option<Option<String>> {
    let Some(value) = value else {
        return Some(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Some(None);
    }
    (value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then(|| Some(value.to_ascii_lowercase()))
}

pub(super) async fn remote_computer_session_transport(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?.to_string();
    let session_id = route_identifier(&context, "session_id")?.to_string();
    let input: RemoteComputerTransportRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_transport_request",
                "Transport negotiation request must be valid JSON.",
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
    let Some(relay_region) = remote_relay_region(input.relay_region.as_deref()) else {
        return error_response(400, "invalid_relay_region", "relayRegion is invalid.");
    };
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(session) = worker::query!(
        &database,
        "SELECT s.client_id, s.mobile_token_hash, s.state, s.expires_at,
                s.provider, s.route_policy, s.selected_route, s.relay_region
         FROM remote_computer_sessions s
         JOIN remote_computer_clients c
           ON c.client_id = s.client_id AND c.device_id = s.device_id AND c.user_id = s.user_id
         JOIN remote_computers d ON d.device_id = s.device_id AND d.user_id = s.user_id
         WHERE s.session_id = ?1 AND s.device_id = ?2 AND s.user_id = ?3
           AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL
           AND d.revoked_at IS NULL LIMIT 1",
        &session_id,
        &device_id,
        &account.user_id
    )?
    .first::<RemoteComputerTransportRow>(None)
    .await?
    else {
        return error_response(
            404,
            "control_session_not_found",
            "Control session was not found.",
        );
    };
    let now = now_seconds();
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
    let actor_allowed = remote_session_actor_allowed(
        &RemoteComputerSessionRow {
            client_id: session.client_id.clone(),
            mobile_token_hash: session.mobile_token_hash.clone(),
            state: session.state.clone(),
            expires_at: session.expires_at,
        },
        &input.role,
        input.client_id.as_deref(),
        input.mobile_token.as_deref(),
        desktop_authorized,
        now,
    );
    if !actor_allowed {
        return error_response(
            403,
            "control_session_forbidden",
            "This actor cannot negotiate transport for the control session.",
        );
    }

    let relay_available = remote_relay_available(&context.env, &session.provider);
    let selected_route = if session.route_policy == "relay-only" {
        if !relay_available {
            return error_response(
                503,
                "relay_unavailable",
                "The session requires relay transport, but no authenticated relay is configured.",
            );
        }
        "relay"
    } else if input.direct_available {
        "direct"
    } else if relay_available {
        "relay"
    } else {
        return error_response(
            503,
            "transport_unavailable",
            "Direct transport is unavailable and no authenticated relay is configured.",
        );
    };
    if session.selected_route.as_deref() == Some("relay") && selected_route == "direct" {
        return error_response(
            409,
            "transport_route_locked",
            "A session that fell back to relay cannot be upgraded back to direct transport.",
        );
    }
    let stored_region = if selected_route == "relay" {
        relay_region.or(session.relay_region)
    } else {
        None
    };
    let updated = worker::query!(
        &database,
        "UPDATE remote_computer_sessions
         SET selected_route = ?1, relay_region = ?2, transport_updated_at = ?3
         WHERE session_id = ?4 AND device_id = ?5 AND user_id = ?6
           AND state <> 'closed' AND expires_at > ?3
           AND (selected_route IS NULL OR selected_route = ?1
                OR (selected_route = 'direct' AND ?1 = 'relay'))",
        selected_route,
        stored_region.as_deref(),
        now,
        &session_id,
        &device_id,
        &account.user_id
    )?
    .run()
    .await?;
    if updated.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        return error_response(
            409,
            "transport_route_conflict",
            "Transport route changed concurrently; refresh the session before retrying.",
        );
    }
    Ok(Response::from_json(&json!({
        "sessionId": session_id,
        "provider": session.provider,
        "routePolicy": session.route_policy,
        "selectedRoute": selected_route,
        "relayRegion": stored_region,
        "transportUpdatedAt": now,
    }))?
    .with_headers(auth_headers()))
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
         WHERE session_id = ?2 AND user_id = ?3 AND device_id = ?4",
        now,
        &session_id,
        &account.user_id,
        &device_id
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
        || !remote_signal_kind_allowed(&input.sender_role, &input.kind)
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
    worker::query!(
        &database,
        "DELETE FROM remote_computer_signals WHERE expires_at <= ?1",
        now
    )?
    .run()
    .await?;
    let inserted = worker::query!(
        &database,
        "INSERT INTO remote_computer_signals
         (session_id, user_id, sender_role, kind, payload_json, created_at, expires_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7
         WHERE EXISTS (
                 SELECT 1 FROM remote_computer_sessions s
                 JOIN remote_computer_clients c
                   ON c.client_id = s.client_id AND c.device_id = s.device_id
                  AND c.user_id = s.user_id
                 JOIN remote_computers d
                   ON d.device_id = s.device_id AND d.user_id = s.user_id
                 WHERE s.session_id = ?1 AND s.user_id = ?2 AND s.device_id = ?9
                   AND s.state <> 'closed' AND s.expires_at > ?6
                   AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL
                   AND d.revoked_at IS NULL
               )
           AND (SELECT COUNT(*) FROM remote_computer_signals
                WHERE session_id = ?1 AND user_id = ?2 AND expires_at > ?6) < ?8",
        &input.session_id,
        &account.user_id,
        &input.sender_role,
        &input.kind,
        &payload_json,
        now,
        expires_at,
        REMOTE_SIGNAL_MAX_ROWS_PER_SESSION,
        &device_id
    )?
    .run()
    .await?;
    if inserted.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        let current_session =
            load_remote_control_session(&database, &account.user_id, &device_id, &input.session_id)
                .await?;
        if !current_session.as_ref().is_some_and(|current| {
            remote_session_actor_allowed(
                current,
                &input.sender_role,
                input.client_id.as_deref(),
                input.mobile_token.as_deref(),
                desktop_authorized,
                now,
            )
        }) {
            return error_response(
                409,
                "control_session_unavailable",
                "The control session was closed, expired, or revoked.",
            );
        }
        return error_response(
            429,
            "signal_queue_full",
            "The remote-control signaling queue is full; reconnect before retrying.",
        );
    }
    if input.sender_role == "mobile" {
        worker::query!(
            &database,
            "UPDATE remote_computer_clients SET last_seen_at = ?1
             WHERE client_id = ?2 AND user_id = ?3 AND device_id = ?4
               AND revoked_at IS NULL",
            now,
            &session.client_id,
            &account.user_id,
            &device_id
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
         FROM remote_computer_signals signal
         WHERE signal.user_id = ?1 AND signal.session_id = ?2 AND signal.sender_role <> ?3
           AND signal.signal_id > ?4 AND signal.expires_at > ?5
           AND EXISTS (
               SELECT 1 FROM remote_computer_sessions s
               JOIN remote_computer_clients c
                 ON c.client_id = s.client_id AND c.device_id = s.device_id
                AND c.user_id = s.user_id
               JOIN remote_computers d
                 ON d.device_id = s.device_id AND d.user_id = s.user_id
               WHERE s.session_id = signal.session_id AND s.user_id = signal.user_id
                 AND s.device_id = ?6 AND s.state <> 'closed' AND s.expires_at > ?5
                 AND c.revoked_at IS NULL AND c.client_token_hash IS NOT NULL
                 AND d.revoked_at IS NULL
           )
         ORDER BY signal.signal_id ASC LIMIT 128",
        &account.user_id,
        &input.session_id,
        &input.receiver_role,
        after,
        now,
        &device_id
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
             WHERE client_id = ?2 AND user_id = ?3 AND device_id = ?4
               AND revoked_at IS NULL",
            now,
            &session.client_id,
            &account.user_id,
            &device_id
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
