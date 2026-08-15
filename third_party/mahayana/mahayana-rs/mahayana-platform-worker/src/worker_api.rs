use crate::auth::hash_password_argon2id;
use crate::auth::hash_refresh_token;
use crate::auth::new_password_salt;
use crate::auth::new_refresh_token;
use crate::auth::verify_argon2id;
use crate::auth::verify_pbkdf2_sha256;
use base64::Engine;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use jsonwebtoken::Validation;
use jsonwebtoken::decode;
use jsonwebtoken::encode;
use mahayana_platform_core::AccountAccessTokenClaims;
use mahayana_platform_core::AccountUsageStatus;
use mahayana_platform_core::Currency;
use mahayana_platform_core::DelegatedTokenRequest;
use mahayana_platform_core::Entitlement;
use mahayana_platform_core::EntitlementStatus;
use mahayana_platform_core::PluginAccessTokenClaims;
use mahayana_platform_core::PurchaseRequest;
use mahayana_platform_core::Quote;
use mahayana_platform_core::UsageCaptureRequest;
use mahayana_platform_core::UsageReservation;
use mahayana_platform_core::UsageReservationRequest;
use mahayana_platform_core::canonical_json_bytes;
use mahayana_platform_core::canonical_json_sha256;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use std::time::Duration;
use url::Url;
use uuid::Uuid;
use worker::Context;
use worker::Date;
use worker::Delay;
use worker::Env;
use worker::Fetch;
use worker::FormEntry;
use worker::Headers;
use worker::Method;
use worker::Request;
use worker::RequestInit;
use worker::Response;
use worker::Result;
use worker::RouteContext;
use worker::Router;
use worker::event;

const DATABASE_BINDING: &str = "PLATFORM_DB";
const ACCOUNT_DATABASE_BINDING: &str = "ACCOUNT_DB";
const ACCESS_TOKEN_ISSUER: &str = "https://api.ombhrum.com";
const ACCESS_TOKEN_AUDIENCE: &str = "mahayana-platform";
const ACCESS_TOKEN_SECONDS: i64 = 15 * 60;
const REFRESH_TOKEN_SECONDS: i64 = 30 * 24 * 60 * 60;
const LOGIN_FAILURE_WINDOW_SECONDS: i64 = 15 * 60;
const MAX_ACCOUNT_LOGIN_FAILURES: i64 = 10;
const OAUTH_ATTEMPT_SECONDS: i64 = 10 * 60;
const USAGE_WINDOW_SECONDS: i64 = 30 * 24 * 60 * 60;
const USAGE_RESERVATION_SECONDS: i64 = 10 * 60;
const MAX_TOKENS_PER_RESERVATION: i64 = 2_000_000;
const MARKETPLACE_DEPLOYMENT_VERIFY_ATTEMPTS: usize = 6;
const MARKETPLACE_DEPLOYMENT_VERIFY_DELAY_SECONDS: u64 = 3;

#[derive(Debug, Deserialize, Serialize)]
struct MarketplacePluginRow {
    plugin_id: String,
    display_name: String,
    description: String,
    latest_version: Option<String>,
    package_sha256: Option<String>,
    package_size: Option<f64>,
    platforms_json: String,
    deployment_url: Option<String>,
    published_at: Option<f64>,
    source_json: String,
    release_manifest_json: String,
    release_manifest_sha256: String,
    release_status: String,
}

#[derive(Debug, Deserialize)]
struct MarketplacePluginOwnerRow {
    publisher_user_id: String,
}

#[derive(Debug, Deserialize)]
struct MarketplaceExistingReleaseRow {
    package_sha256: String,
}

#[derive(Debug, Deserialize)]
struct MarketplaceReleaseStatusRow {
    release_status: String,
    revoked_at: Option<f64>,
    revocation_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MarketplaceReleaseMetadataRow {
    plugin_id: String,
    version: String,
    package_sha256: String,
    package_size: f64,
    deployment_url: String,
    published_at: f64,
    platforms_json: String,
    source_json: String,
    release_manifest_json: String,
    release_manifest_sha256: String,
    release_status: String,
    revoked_at: Option<f64>,
    revocation_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MarketplaceReleaseDownloadRow {
    deployment_url: String,
    package_key: String,
    package_sha256: String,
    package_size: f64,
    release_status: String,
    revoked_at: Option<f64>,
    revocation_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketplaceSiteManifest {
    schema_version: u32,
    plugin_id: String,
    version: String,
    package_path: String,
    package_sha256: String,
    package_size: usize,
    runtime: String,
    source: Value,
    release_manifest_path: String,
    release_manifest_sha256: String,
}

#[derive(Debug, Deserialize)]
struct BalanceRow {
    currency: String,
    available: i64,
    reserved: i64,
}

#[derive(Debug, Deserialize)]
struct PriceRow {
    product_id: String,
    price_id: String,
    plugin_id: String,
    sku: String,
    capability: String,
    currency: String,
    amount: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderRow {
    order_id: String,
    plugin_id: String,
    sku: String,
    currency: String,
    amount: i64,
    status: String,
    created_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteRequest {
    sku: String,
}

#[derive(Debug, Deserialize)]
struct UsageBudgetRow {
    window_start: i64,
    window_end: i64,
    token_limit: i64,
    used_tokens: i64,
    reserved_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct UsageReservationRow {
    reservation_id: String,
    request_id: String,
    reserved_tokens: i64,
    expires_at: i64,
    state: String,
}

#[derive(Debug, Deserialize)]
struct UsageEventRow {
    reservation_id: String,
}

#[derive(Debug, Deserialize)]
struct AccountUserRow {
    id: i64,
    user_no: Option<i64>,
    username: String,
    username_changed_at: Option<String>,
    email: Option<String>,
    nickname: Option<String>,
    avatar: Option<String>,
    phone_number: Option<String>,
    firebase_uid: Option<String>,
    alipay_user_id: Option<String>,
    alipay_nickname: Option<String>,
    alipay_avatar: Option<String>,
    wechat_headimgurl: Option<String>,
    password_hash: Option<String>,
    salt: Option<String>,
    iterations: Option<i64>,
    algo: Option<String>,
    upgraded_password_phc: Option<String>,
    main_practice_title: Option<String>,
    main_practice_file_path: Option<String>,
    main_practice_selected_at: Option<String>,
    created_at: String,
    email_verified: Option<i64>,
    membership_type: Option<String>,
    membership_expires_at: Option<String>,
    free_trial_end_date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PasswordLoginRequest {
    username: String,
    password: String,
    #[serde(default)]
    device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OAuthStartRequest {
    provider: String,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    platform: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthAttemptRow {
    attempt_id: String,
    provider: String,
    device_id: String,
    code_verifier: String,
    status: String,
    session_json: Option<String>,
    expires_at: i64,
    delivered_at: Option<i64>,
}

struct OAuthProviderConfig {
    id: &'static str,
    display_name: &'static str,
    issuer: &'static str,
    authorization_endpoint: &'static str,
    token_endpoint: &'static str,
    userinfo_endpoint: &'static str,
    scopes: &'static str,
    client_id: String,
    client_secret: String,
}

#[derive(Debug)]
struct OAuthIdentityProfile {
    issuer: String,
    subject: String,
    email: Option<String>,
    email_verified: bool,
    display_name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthIdentityRow {
    user_id: String,
}

#[derive(Debug, Deserialize)]
struct MaxUserIdRow {
    max_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefreshAccessRequest {
    refresh_token: String,
    #[serde(default)]
    device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenRow {
    token_hash: String,
    session_id: String,
    generation: i64,
    state: String,
    user_id: String,
    device_id: String,
    session_expires_at: i64,
    revoked_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LoginFailureCountRow {
    failure_count: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListenerRegistrationInput {
    platform: String,
    subscriptions: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListenerRegistrationRequest {
    registrations: Vec<ListenerRegistrationInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListenerDrainRequest {
    #[serde(default)]
    ack_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListenerIngressRequest {
    event_id: String,
    user_id: String,
    platform: String,
    event: Value,
}

#[derive(Debug, Deserialize)]
struct ListenerEventRow {
    event_id: String,
    platform: String,
    event_json: String,
    created_at: f64,
}

#[derive(Debug)]
struct AuthenticatedAccount {
    user_id: String,
    session_id: Option<String>,
    scopes: Vec<String>,
    is_test_account: bool,
}

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

#[event(fetch, respond_with_errors)]
pub async fn main(request: Request, env: Env, _context: Context) -> Result<Response> {
    Router::new()
        .get("/health", |_, _| Response::from_json(&json!({"ok": true})))
        .post_async("/api/auth/login", password_login)
        .get_async("/api/auth/oauth/providers", oauth_providers)
        .post_async("/api/auth/oauth/start", oauth_start)
        .get_async("/api/auth/oauth/callback", oauth_callback)
        .get_async("/api/auth/oauth/attempts/:attempt_id", oauth_poll)
        .post_async("/api/auth/refresh", refresh_access_token)
        .get_async("/api/auth/user-info", account_user_info)
        .post_async("/api/auth/logout", account_logout)
        .get("/v1/auth/jwks.json", |_, context| {
            let jwks = context.env.secret("ACCESS_TOKEN_JWKS")?.to_string();
            Ok(Response::ok(jwks)?.with_headers(json_headers()))
        })
        .post_async("/v1/auth/plugin-token", delegated_plugin_token)
        .get_async("/v1/ai/usage", ai_usage_status)
        .post_async("/v1/ai/usage/reservations", ai_usage_reserve)
        .post_async(
            "/v1/ai/usage/reservations/:reservation_id/capture",
            ai_usage_capture,
        )
        .post_async(
            "/v1/ai/usage/reservations/:reservation_id/release",
            ai_usage_release,
        )
        .post_async("/v1/listeners/register", listener_register)
        .post_async("/v1/listeners/drain", listener_drain)
        .post_async("/v1/listeners/ingest", listener_ingest)
        .get_async("/v1/computers", remote_computer_list)
        .post_async("/v1/computers/register", remote_computer_register)
        .post_async("/v1/computers/heartbeat", remote_computer_heartbeat)
        .post_async("/v1/computers/pair", remote_computer_pair)
        .get_async("/v1/computers/:device_id/clients", remote_computer_clients)
        .post_async(
            "/v1/computers/:device_id/clients/:client_id/revoke",
            remote_computer_client_revoke,
        )
        .post_async("/v1/computers/:device_id/sessions/list", remote_computer_sessions)
        .post_async("/v1/computers/:device_id/sessions", remote_computer_session_create)
        .post_async(
            "/v1/computers/:device_id/sessions/:session_id/activate",
            remote_computer_session_activate,
        )
        .post_async(
            "/v1/computers/:device_id/sessions/:session_id/close",
            remote_computer_session_close,
        )
        .post_async("/v1/computers/:device_id/signals", remote_computer_signal)
        .post_async(
            "/v1/computers/:device_id/signals/drain",
            remote_computer_signal_drain,
        )
        .get_async("/v1/marketplace/plugins", marketplace_plugins)
        .post_async("/v1/marketplace/releases", marketplace_release_publish)
        .post_async(
            "/v1/marketplace/external-releases",
            marketplace_external_release_publish,
        )
        .get_async(
            "/v1/marketplace/plugins/:plugin_id/releases/:version",
            marketplace_release_metadata,
        )
        .get_async(
            "/v1/marketplace/plugins/:plugin_id/releases/:version/download",
            marketplace_plugin_download,
        )
        .post_async(
            "/v1/marketplace/plugins/:plugin_id/releases/:version/revoke",
            marketplace_release_revoke,
        )
        .get_async("/v1/wallet/balance", wallet_balance)
        .get_async("/v1/wallet/history", wallet_history)
        .post_async("/v1/plugins/:plugin_id/commerce/quote", commerce_quote)
        .post_async(
            "/v1/plugins/:plugin_id/commerce/purchase",
            commerce_purchase,
        )
        .get_async(
            "/v1/plugins/:plugin_id/entitlements/:capability",
            commerce_entitlement,
        )
        .get_async("/v1/purchases", purchases)
        .post_async("/v1/purchases/restore", purchases_restore)
        .run(request, env)
        .await
}

fn is_listener_platform(value: &str) -> bool {
    matches!(
        value,
        "slack" | "github" | "git" | "teams" | "linear" | "sentry" | "pagerduty"
    )
}

fn valid_relay_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
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
    format!(
        "{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
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
    let mut servers = vec![
        json!({"urls": ["stun:stun.l.google.com:19302", "stun:stun1.l.google.com:19302"]}),
    ];
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

async fn remote_computer_list(request: Request, context: RouteContext<()>) -> Result<Response> {
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

async fn remote_computer_register(
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
        return error_response(400, "invalid_device_secret", "Computer device secret is invalid.");
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
        return error_response(403, "device_secret_mismatch", "This computer registration is owned by another device secret.");
    }
    Ok(Response::from_json(&json!({
        "deviceId": input.device_id,
        "label": label,
        "pairingCode": code,
        "pairingExpiresAt": expires_at,
    }))?
    .with_headers(auth_headers()))
}

async fn remote_computer_heartbeat(
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
        return error_response(400, "invalid_device_secret", "Computer device secret is invalid.");
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

async fn remote_computer_pair(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(401, "unauthorized", "Sign in before pairing a computer.");
        }
    };
    let input: RemoteComputerPairRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => return error_response(400, "invalid_pairing", "Pairing request must be valid JSON."),
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
        return error_response(404, "pairing_code_not_found", "Pairing code is expired or does not belong to this account.");
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

async fn remote_computer_clients(request: Request, context: RouteContext<()>) -> Result<Response> {
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
        .map(|row| json!({
            "clientId": row.client_id,
            "label": row.label,
            "pairedAt": row.paired_at,
            "lastSeenAt": row.last_seen_at,
        }))
        .collect::<Vec<_>>();
    Ok(Response::from_json(&json!({"deviceId": device_id, "clients": clients}))?.with_headers(auth_headers()))
}

async fn remote_computer_client_revoke(
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
    Ok(Response::from_json(&json!({"revoked": true, "clientId": client_id}))?.with_headers(auth_headers()))
}

async fn remote_computer_sessions(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?;
    let input: RemoteComputerDesktopAuthRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => return error_response(400, "invalid_device_auth", "Desktop session list requires a device secret."),
    };
    let now = now_seconds();
    let database = context.env.d1(DATABASE_BINDING)?;
    if !remote_desktop_secret_matches(&database, &account.user_id, device_id, &input.device_secret).await? {
        return error_response(403, "device_secret_mismatch", "Desktop device authentication failed.");
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
        .map(|row| json!({
            "sessionId": row.session_id,
            "clientId": row.client_id,
            "clientLabel": row.client_label,
            "state": row.state,
            "createdAt": row.created_at,
            "expiresAt": row.expires_at,
        }))
        .collect::<Vec<_>>();
    Ok(Response::from_json(&json!({
        "deviceId": device_id,
        "sessions": sessions,
        "iceServers": remote_ice_servers(&context.env),
    }))?
    .with_headers(auth_headers()))
}

async fn remote_computer_session_create(
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
        Err(_) => return error_response(400, "invalid_control_session", "Session request must be valid JSON."),
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
        return error_response(403, "client_not_paired", "This phone is not paired with the computer.");
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

async fn remote_computer_session_activate(
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
        Err(_) => return error_response(400, "invalid_device_auth", "Desktop session activation requires a device secret."),
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
        return error_response(404, "control_session_not_found", "Pending control session was not found.");
    }
    let Some(session) = load_remote_control_session(&database, &account.user_id, device_id, session_id).await? else {
        return error_response(404, "control_session_not_found", "Control session disappeared after activation.");
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
                && mobile_token
                    .map(remote_secret_hash)
                    .is_some_and(|hash| constant_time_eq(hash.as_bytes(), session.mobile_token_hash.as_bytes()))
        }
        _ => false,
    }
}

async fn remote_computer_session_close(
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
        Err(_) => return error_response(400, "invalid_control_session", "Close request must be valid JSON."),
    };
    if !remote_role(&input.role) {
        return error_response(400, "invalid_control_role", "Control role must be desktop or mobile.");
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let Some(session) = load_remote_control_session(&database, &account.user_id, &device_id, &session_id).await? else {
        return error_response(404, "control_session_not_found", "Control session was not found.");
    };
    let desktop_authorized = if input.role == "desktop" {
        match input.device_secret.as_deref() {
            Some(secret) => remote_desktop_secret_matches(
                &database,
                &account.user_id,
                &device_id,
                secret,
            )
            .await?,
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
        return error_response(403, "control_session_forbidden", "This client cannot close the control session.");
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
    Ok(Response::from_json(&json!({"sessionId": session_id, "state": "closed"}))?.with_headers(auth_headers()))
}

async fn remote_computer_signal(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "A valid account token is required."),
    };
    let device_id = route_identifier(&context, "device_id")?.to_string();
    let input: RemoteComputerSignalRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => return error_response(400, "invalid_signal", "Signal request must be valid JSON."),
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
        return error_response(413, "signal_too_large", "WebRTC signal payload exceeds 256 KiB.");
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let Some(session) = load_remote_control_session(&database, &account.user_id, &device_id, &input.session_id).await? else {
        return error_response(404, "control_session_not_found", "Control session was not found.");
    };
    let desktop_authorized = if input.sender_role == "desktop" {
        match input.device_secret.as_deref() {
            Some(secret) => remote_desktop_secret_matches(
                &database,
                &account.user_id,
                &device_id,
                secret,
            )
            .await?,
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
        return error_response(403, "control_session_forbidden", "This client cannot signal for the control session.");
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
    Ok(Response::from_json(&json!({"accepted": true, "expiresAt": expires_at}))?.with_headers(auth_headers()))
}

async fn remote_computer_signal_drain(
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
        Err(_) => return error_response(400, "invalid_signal_drain", "Signal drain must be valid JSON."),
    };
    if !valid_relay_identifier(&input.session_id, 160) || !remote_role(&input.receiver_role) {
        return error_response(400, "invalid_signal_drain", "Signal drain identifiers are invalid.");
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let Some(session) = load_remote_control_session(&database, &account.user_id, &device_id, &input.session_id).await? else {
        return error_response(404, "control_session_not_found", "Control session was not found.");
    };
    let desktop_authorized = if input.receiver_role == "desktop" {
        match input.device_secret.as_deref() {
            Some(secret) => remote_desktop_secret_matches(
                &database,
                &account.user_id,
                &device_id,
                secret,
            )
            .await?,
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
        return error_response(403, "control_session_forbidden", "This client cannot read control signals.");
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

async fn listener_register(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to register listeners.",
            );
        }
    };
    let input: ListenerRegistrationRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_listener_registration",
                "Listener registrations must be valid JSON.",
            );
        }
    };
    if input.registrations.len() > 32 {
        return error_response(
            400,
            "too_many_listener_registrations",
            "At most 32 listener platform registrations are allowed.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let mut statements = vec![worker::query!(
        &database,
        "DELETE FROM listener_registrations WHERE user_id = ?1",
        &account.user_id
    )?];
    let mut seen = std::collections::BTreeSet::new();
    for registration in input.registrations {
        if !is_listener_platform(&registration.platform)
            || !seen.insert(registration.platform.clone())
        {
            return error_response(
                400,
                "invalid_listener_platform",
                "Listener platforms must be supported and unique.",
            );
        }
        let subscriptions_json = serde_json::to_string(&registration.subscriptions)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        if subscriptions_json.len() > 64 * 1024 {
            return error_response(
                400,
                "listener_registration_too_large",
                "Listener subscriptions must be at most 64 KiB per platform.",
            );
        }
        statements.push(worker::query!(
            &database,
            "INSERT INTO listener_registrations
             (user_id, platform, subscriptions_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            &account.user_id,
            &registration.platform,
            &subscriptions_json,
            now
        )?);
    }
    database.batch(statements).await?;
    Ok(Response::from_json(&json!({"registered": seen.len()}))?.with_headers(auth_headers()))
}

async fn listener_drain(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to drain listener events.",
            );
        }
    };
    let input: ListenerDrainRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_listener_drain",
                "Listener drain requests must be valid JSON.",
            );
        }
    };
    if input.ack_ids.len() > 100
        || input
            .ack_ids
            .iter()
            .any(|id| !valid_relay_identifier(id, 256))
    {
        return error_response(
            400,
            "invalid_listener_ack",
            "At most 100 normalized event IDs may be acknowledged at once.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    if !input.ack_ids.is_empty() {
        let now = now_seconds();
        let mut statements = Vec::with_capacity(input.ack_ids.len());
        for event_id in &input.ack_ids {
            statements.push(worker::query!(
                &database,
                "UPDATE listener_events
                 SET acknowledged_at = COALESCE(acknowledged_at, ?1)
                 WHERE event_id = ?2 AND user_id = ?3",
                now,
                event_id,
                &account.user_id
            )?);
        }
        database.batch(statements).await?;
    }
    let rows = worker::query!(
        &database,
        "SELECT event_id, platform, event_json, created_at
         FROM listener_events
         WHERE user_id = ?1 AND acknowledged_at IS NULL
         ORDER BY created_at ASC, event_id ASC
         LIMIT 100",
        &account.user_id
    )?
    .all()
    .await?
    .results::<ListenerEventRow>()?;
    let events = rows
        .into_iter()
        .filter_map(|row| {
            let event = serde_json::from_str::<Value>(&row.event_json).ok()?;
            Some(json!({
                "id": row.event_id,
                "platform": row.platform,
                "createdAt": exact_nonnegative_i64(row.created_at).unwrap_or_default(),
                "event": event,
            }))
        })
        .collect::<Vec<_>>();
    Ok(Response::from_json(&json!({"events": events}))?.with_headers(auth_headers()))
}

async fn listener_ingest(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let authorization = request.headers().get("Authorization")?.unwrap_or_default();
    let supplied = authorization.strip_prefix("Bearer ").unwrap_or_default();
    let expected = match context.env.secret("LISTENER_RELAY_INGEST_TOKEN") {
        Ok(secret) => secret.to_string(),
        Err(_) => {
            return error_response(
                503,
                "listener_ingest_not_configured",
                "Listener relay ingress is not configured.",
            );
        }
    };
    if supplied.is_empty() || !constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
        return error_response(401, "unauthorized", "Invalid listener relay ingress token.");
    }
    let input: ListenerIngressRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_listener_event",
                "Listener ingress requests must be valid JSON.",
            );
        }
    };
    if !valid_relay_identifier(&input.event_id, 256)
        || !valid_relay_identifier(&input.user_id, 256)
        || !is_listener_platform(&input.platform)
    {
        return error_response(
            400,
            "invalid_listener_event",
            "Listener event identifiers or platform are invalid.",
        );
    }
    let event_json = serde_json::to_string(&input.event)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    if event_json.len() > 128 * 1024 {
        return error_response(
            413,
            "listener_event_too_large",
            "Listener events must be at most 128 KiB.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let registration = worker::query!(
        &database,
        "SELECT platform FROM listener_registrations
         WHERE user_id = ?1 AND platform = ?2",
        &input.user_id,
        &input.platform
    )?
    .first::<Value>(None)
    .await?;
    if registration.is_none() {
        return error_response(
            409,
            "listener_not_registered",
            "The target account has not registered this listener platform.",
        );
    }
    let result = worker::query!(
        &database,
        "INSERT INTO listener_events
         (event_id, user_id, platform, event_json, created_at, acknowledged_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL)
         ON CONFLICT(event_id) DO NOTHING",
        &input.event_id,
        &input.user_id,
        &input.platform,
        &event_json,
        now_seconds()
    )?
    .run()
    .await?;
    Ok(Response::from_json(&json!({
        "accepted": d1_changes(Some(&result)) > 0,
        "eventId": input.event_id,
    }))?)
}

async fn password_login(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let login: PasswordLoginRequest = match request.json().await {
        Ok(login) => login,
        Err(_) => {
            return error_response(
                400,
                "invalid_login_request",
                "用户名或邮箱、手机号和密码不能为空",
            );
        }
    };
    let identifier = login.username.trim();
    if identifier.is_empty() || login.password.is_empty() || login.password.len() > 1024 {
        return error_response(
            400,
            "invalid_login_request",
            "用户名或邮箱、手机号和密码不能为空",
        );
    }
    let device_id = match normalize_device_id(login.device_id.as_deref()) {
        Ok(device_id) => device_id,
        Err(_) => return error_response(400, "invalid_device_id", "invalid device id"),
    };
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let user = lookup_login_user(&database, identifier).await?;
    let Some(user) = user else {
        return error_response(401, "invalid_credentials", "账号或密码错误");
    };
    let now = now_seconds();
    if account_login_is_rate_limited(&database, &user.id.to_string(), now).await? {
        return error_response(429, "login_rate_limited", "登录尝试过多，请稍后再试");
    }

    let password_valid = if let Some(upgraded) = user.upgraded_password_phc.as_deref() {
        verify_argon2id(&login.password, upgraded)
    } else {
        let Some(password_hash) = user.password_hash.as_deref() else {
            return error_response(401, "password_not_configured", "当前账号尚未设置密码");
        };
        let Some(salt) = user.salt.as_deref() else {
            return error_response(401, "password_not_configured", "当前账号尚未设置密码");
        };
        verify_pbkdf2_sha256(
            &login.password,
            salt,
            password_hash,
            user.iterations,
            user.algo.as_deref(),
        )
        .unwrap_or(false)
    };
    if !password_valid {
        record_auth_event(
            &database,
            Some(&user.id.to_string()),
            None,
            "login_failed",
            now_seconds(),
        )
        .await?;
        return error_response(401, "invalid_credentials", "账号或密码错误");
    }

    if user.upgraded_password_phc.is_none() {
        let upgraded = hash_password_argon2id(&login.password, &new_password_salt())
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO account_password_credentials
             (user_id, password_phc, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            user.id.to_string(),
            upgraded,
            now
        )?
        .run()
        .await?;
    }

    let session_id = Uuid::new_v4().to_string();
    let family_id = Uuid::new_v4().to_string();
    let refresh_token = new_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token);
    let refresh_expires_at = now + REFRESH_TOKEN_SECONDS;
    let (access_token, access_expires_at, access_jti) = issue_account_access_token(
        &context.env,
        &user.id.to_string(),
        &device_id,
        &session_id,
        now,
    )?;
    let statements = vec![
        worker::query!(
            &database,
            "INSERT INTO account_sessions
             (session_id, refresh_family_id, user_id, device_id, current_refresh_token_hash,
              created_at, last_used_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
            &session_id,
            &family_id,
            user.id.to_string(),
            &device_id,
            &refresh_hash,
            now,
            refresh_expires_at
        )?,
        worker::query!(
            &database,
            "INSERT INTO account_refresh_tokens
             (token_hash, session_id, generation, state, issued_at, expires_at)
             VALUES (?1, ?2, 0, 'active', ?3, ?4)",
            &refresh_hash,
            &session_id,
            now,
            refresh_expires_at
        )?,
        worker::query!(
            &database,
            "INSERT INTO account_auth_events
             (event_id, user_id, session_id, event_type, occurred_at, details_json)
             VALUES (?1, ?2, ?3, 'login_succeeded', ?4, ?5)",
            Uuid::new_v4().to_string(),
            user.id.to_string(),
            &session_id,
            now,
            json!({"accessJti": access_jti}).to_string()
        )?,
    ];
    database.batch(statements).await?;

    account_session_response(
        &user,
        &access_token,
        &refresh_token,
        access_expires_at,
        refresh_expires_at,
        &session_id,
        &device_id,
    )
}

fn oauth_provider(env: &Env, provider: &str) -> Option<OAuthProviderConfig> {
    let (
        id,
        display_name,
        issuer,
        authorization_endpoint,
        token_endpoint,
        userinfo_endpoint,
        scopes,
    ) = match provider {
        "google" => (
            "google",
            "Google",
            "https://accounts.google.com",
            "https://accounts.google.com/o/oauth2/v2/auth",
            "https://oauth2.googleapis.com/token",
            "https://openidconnect.googleapis.com/v1/userinfo",
            "openid email profile",
        ),
        "microsoft" => (
            "microsoft",
            "Microsoft",
            "https://login.microsoftonline.com/common/v2.0",
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            "https://graph.microsoft.com/oidc/userinfo",
            "openid email profile",
        ),
        "github" => (
            "github",
            "GitHub",
            "https://github.com",
            "https://github.com/login/oauth/authorize",
            "https://github.com/login/oauth/access_token",
            "https://api.github.com/user",
            "read:user user:email",
        ),
        _ => return None,
    };
    let prefix = provider.to_ascii_uppercase();
    let client_id = env
        .secret(&format!("OAUTH_{prefix}_CLIENT_ID"))
        .ok()?
        .to_string();
    let client_secret = env
        .secret(&format!("OAUTH_{prefix}_CLIENT_SECRET"))
        .ok()?
        .to_string();
    Some(OAuthProviderConfig {
        id,
        display_name,
        issuer,
        authorization_endpoint,
        token_endpoint,
        userinfo_endpoint,
        scopes,
        client_id,
        client_secret,
    })
}

async fn oauth_providers(_request: Request, context: RouteContext<()>) -> Result<Response> {
    let providers = ["google", "microsoft", "github"]
        .into_iter()
        .filter_map(|id| oauth_provider(&context.env, id))
        .map(|provider| {
            json!({
                "id": provider.id,
                "displayName": provider.display_name,
                "enabled": true,
            })
        })
        .collect::<Vec<_>>();
    Ok(Response::from_json(&json!({"providers": providers}))?.with_headers(auth_headers()))
}

async fn oauth_start(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let start: OAuthStartRequest = match request.json().await {
        Ok(start) => start,
        Err(_) => return error_response(400, "invalid_oauth_request", "provider is required"),
    };
    let Some(provider) = oauth_provider(&context.env, start.provider.trim()) else {
        return error_response(400, "oauth_provider_unavailable", "该登录方式尚未配置");
    };
    if let Some(platform) = start.platform.as_deref()
        && !matches!(
            platform,
            "desktop" | "macos" | "windows" | "linux" | "web" | "mobile"
        )
    {
        return error_response(400, "invalid_oauth_platform", "unsupported OAuth platform");
    }
    let device_id = normalize_device_id(start.device_id.as_deref())?;
    let attempt_id = Uuid::new_v4().to_string();
    let state = format!("mos_{}", Uuid::new_v4().simple());
    let state_hash = format!("{:x}", Sha256::digest(state.as_bytes()));
    let code_verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(code_verifier.as_bytes()));
    let now = now_seconds();
    let expires_at = now + OAUTH_ATTEMPT_SECONDS;
    let callback = format!(
        "{}/api/auth/oauth/callback",
        context
            .env
            .var("AUTH_PUBLIC_BASE_URL")?
            .to_string()
            .trim_end_matches('/')
    );
    let mut authorization_url = Url::parse(provider.authorization_endpoint)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    authorization_url
        .query_pairs_mut()
        .append_pair("client_id", &provider.client_id)
        .append_pair("redirect_uri", &callback)
        .append_pair("response_type", "code")
        .append_pair("scope", provider.scopes)
        .append_pair("state", &state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");
    if provider.id != "github" {
        authorization_url
            .query_pairs_mut()
            .append_pair("prompt", "select_account");
    }
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(
        &database,
        "INSERT INTO account_oauth_attempts
         (attempt_id, state_hash, code_verifier, provider, device_id, status, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7)",
        &attempt_id,
        &state_hash,
        &code_verifier,
        provider.id,
        &device_id,
        now,
        expires_at
    )?
    .run()
    .await?;
    Ok(Response::from_json(&json!({
        "attemptId": attempt_id,
        "provider": provider.id,
        "authorizationUrl": authorization_url.as_str(),
        "expiresAt": expires_at,
    }))?
    .with_headers(auth_headers()))
}

async fn oauth_poll(_request: Request, context: RouteContext<()>) -> Result<Response> {
    let attempt_id = route_identifier(&context, "attempt_id")?;
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT attempt_id, provider, device_id, code_verifier, status, session_json, expires_at, delivered_at
         FROM account_oauth_attempts WHERE attempt_id = ?1 LIMIT 1",
        attempt_id
    )?
    .first::<OAuthAttemptRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(404, "oauth_attempt_missing", "登录链接不存在或已失效");
    };
    let now = now_seconds();
    if row.expires_at <= now && row.status == "pending" {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'expired' WHERE attempt_id = ?1",
            &row.attempt_id
        )?
        .run()
        .await?;
        return Response::from_json(&json!({"status": "expired", "provider": row.provider}));
    }
    if row.status != "completed" {
        return Response::from_json(&json!({"status": row.status, "provider": row.provider}));
    }
    if row.delivered_at.is_some() {
        return error_response(410, "oauth_session_delivered", "登录结果已经领取");
    }
    let Some(session_json) = row.session_json else {
        return error_response(410, "oauth_session_missing", "登录结果已经失效");
    };
    let session: Value = serde_json::from_str(&session_json)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    worker::query!(
        &database,
        "UPDATE account_oauth_attempts SET session_json = NULL, delivered_at = ?1
         WHERE attempt_id = ?2 AND delivered_at IS NULL",
        now,
        &row.attempt_id
    )?
    .run()
    .await?;
    Response::from_json(&json!({
        "status": "completed",
        "provider": row.provider,
        "session": session,
    }))
}

async fn oauth_callback(request: Request, context: RouteContext<()>) -> Result<Response> {
    let url = request.url()?;
    let query = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let Some(state) = query.get("state").map(|value| value.as_ref()) else {
        return oauth_result_page(false, "登录状态缺失，请返回应用重试");
    };
    let state_hash = format!("{:x}", Sha256::digest(state.as_bytes()));
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let attempt = worker::query!(
        &database,
        "SELECT attempt_id, provider, device_id, code_verifier, status, session_json, expires_at, delivered_at
         FROM account_oauth_attempts WHERE state_hash = ?1 LIMIT 1",
        &state_hash
    )?
    .first::<OAuthAttemptRow>(None)
    .await?;
    let Some(attempt) = attempt else {
        return oauth_result_page(false, "登录状态无效，请返回应用重试");
    };
    if attempt.status != "pending" || attempt.expires_at <= now_seconds() {
        return oauth_result_page(false, "登录链接已失效，请返回应用重试");
    }
    if query.contains_key("error") {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'cancelled' WHERE attempt_id = ?1",
            &attempt.attempt_id
        )?
        .run()
        .await?;
        return oauth_result_page(false, "登录已取消，可以关闭此窗口");
    }
    let Some(code) = query.get("code").map(|value| value.as_ref()) else {
        return oauth_result_page(false, "授权码缺失，请返回应用重试");
    };
    let Some(provider) = oauth_provider(&context.env, &attempt.provider) else {
        return oauth_result_page(false, "该登录方式当前不可用");
    };
    let callback = format!(
        "{}/api/auth/oauth/callback",
        context
            .env
            .var("AUTH_PUBLIC_BASE_URL")?
            .to_string()
            .trim_end_matches('/')
    );
    let access_token =
        oauth_exchange_code(&provider, code, &callback, &attempt.code_verifier).await?;
    let profile = oauth_fetch_profile(&provider, &access_token).await?;
    let user = oauth_resolve_user(&database, &provider, &profile).await?;
    let session = create_account_session_value(
        &database,
        &context.env,
        &user,
        &attempt.device_id,
        &format!("oauth_{}", provider.id),
    )
    .await?;
    let now = now_seconds();
    worker::query!(
        &database,
        "UPDATE account_oauth_attempts
         SET status = 'completed', session_json = ?1, completed_at = ?2
         WHERE attempt_id = ?3 AND status = 'pending'",
        session.to_string(),
        now,
        &attempt.attempt_id
    )?
    .run()
    .await?;
    oauth_result_page(true, "登录成功，可以关闭此窗口并返回大乘")
}

async fn oauth_exchange_code(
    provider: &OAuthProviderConfig,
    code: &str,
    callback: &str,
    code_verifier: &str,
) -> Result<String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &provider.client_id)
        .append_pair("client_secret", &provider.client_secret)
        .append_pair("code", code)
        .append_pair("redirect_uri", callback)
        .append_pair("grant_type", "authorization_code")
        .append_pair("code_verifier", code_verifier)
        .finish();
    let headers = Headers::new();
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;
    headers.set("Accept", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(wasm_bindgen::JsValue::from_str(&body)));
    let outbound = Request::new_with_init(provider.token_endpoint, &init)?;
    let mut response = Fetch::Request(outbound).send().await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "OAuth token exchange failed".into(),
        ));
    }
    let token: Value = response.json().await?;
    token
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| worker::Error::RustError("OAuth access token missing".into()))
}

async fn oauth_fetch_json(url: &str, access_token: &str) -> Result<Value> {
    let mut request = Request::new(url, Method::Get)?;
    request
        .headers_mut()?
        .set("Authorization", &format!("Bearer {access_token}"))?;
    request.headers_mut()?.set("Accept", "application/json")?;
    request
        .headers_mut()?
        .set("User-Agent", "Fabushi-Auth-Broker")?;
    let mut response = Fetch::Request(request).send().await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "OAuth profile request failed".into(),
        ));
    }
    response.json().await
}

async fn oauth_fetch_profile(
    provider: &OAuthProviderConfig,
    access_token: &str,
) -> Result<OAuthIdentityProfile> {
    let profile = oauth_fetch_json(provider.userinfo_endpoint, access_token).await?;
    let subject = profile
        .get("sub")
        .or_else(|| profile.get("id"))
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_string)
                .or_else(|| value.as_i64().map(|id| id.to_string()))
        })
        .ok_or_else(|| worker::Error::RustError("OAuth subject missing".into()))?;
    let mut email = profile
        .get("email")
        .or_else(|| profile.get("mail"))
        .or_else(|| profile.get("userPrincipalName"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut email_verified = profile
        .get("email_verified")
        .and_then(Value::as_bool)
        .unwrap_or(provider.id == "microsoft");
    if provider.id == "github" {
        let emails = oauth_fetch_json("https://api.github.com/user/emails", access_token).await?;
        if let Some(primary) = emails.as_array().and_then(|items| {
            items.iter().find(|item| {
                item.get("primary").and_then(Value::as_bool) == Some(true)
                    && item.get("verified").and_then(Value::as_bool) == Some(true)
            })
        }) {
            email = primary
                .get("email")
                .and_then(Value::as_str)
                .map(str::to_string);
            email_verified = email.is_some();
        }
    }
    Ok(OAuthIdentityProfile {
        issuer: provider.issuer.to_string(),
        subject,
        email,
        email_verified,
        display_name: profile
            .get("name")
            .or_else(|| profile.get("login"))
            .and_then(Value::as_str)
            .map(str::to_string),
        avatar_url: profile
            .get("picture")
            .or_else(|| profile.get("avatar_url"))
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

async fn oauth_resolve_user(
    database: &worker::D1Database,
    provider: &OAuthProviderConfig,
    profile: &OAuthIdentityProfile,
) -> Result<AccountUserRow> {
    let identity = worker::query!(
        database,
        "SELECT user_id FROM account_identities WHERE issuer = ?1 AND subject = ?2 LIMIT 1",
        &profile.issuer,
        &profile.subject
    )?
    .first::<OAuthIdentityRow>(None)
    .await?;
    let now = now_seconds();
    let user_id = if let Some(identity) = identity {
        worker::query!(
            database,
            "UPDATE account_identities SET email = ?1, email_verified = ?2,
             display_name = ?3, avatar_url = ?4, last_login_at = ?5
             WHERE issuer = ?6 AND subject = ?7",
            &profile.email,
            i64::from(profile.email_verified),
            &profile.display_name,
            &profile.avatar_url,
            now,
            &profile.issuer,
            &profile.subject
        )?
        .run()
        .await?;
        identity.user_id
    } else {
        if !profile.email_verified {
            return Err(worker::Error::RustError(
                "OAuth provider did not return a verified email".into(),
            ));
        }
        let email = profile
            .email
            .as_deref()
            .ok_or_else(|| worker::Error::RustError("OAuth email missing".into()))?;
        let existing = lookup_login_user(database, email).await?;
        let user_id = if let Some(user) = existing {
            user.id.to_string()
        } else {
            let max = worker::query!(database, "SELECT MAX(id) AS max_id FROM users")
                .first::<MaxUserIdRow>(None)
                .await?
                .and_then(|row| row.max_id)
                .unwrap_or(10_000);
            let id = max + 1;
            let subject_slug = profile
                .subject
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .take(12)
                .collect::<String>();
            let username = format!(
                "{}_{}_{}",
                provider.id,
                subject_slug,
                &Uuid::new_v4().simple().to_string()[..6]
            );
            let created_at = Date::now().to_string();
            worker::query!(
                database,
                "INSERT INTO users
                 (id, user_no, username, email, nickname, avatar, password_hash, salt,
                  iterations, algo, email_verified, membership_type, created_at)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, '', '', 0, '', 1, 'trial', ?6)",
                id,
                &username,
                email,
                &profile.display_name,
                &profile.avatar_url,
                &created_at
            )?
            .run()
            .await?;
            worker::query!(
                database,
                "INSERT OR IGNORE INTO email_username_mapping (email, username, user_id)
                 VALUES (?1, ?2, ?3)",
                email,
                &username,
                id
            )?
            .run()
            .await?;
            id.to_string()
        };
        worker::query!(
            database,
            "INSERT INTO account_identities
             (identity_id, user_id, provider, issuer, subject, email, email_verified,
              display_name, avatar_url, created_at, last_login_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, ?9)",
            Uuid::new_v4().to_string(),
            &user_id,
            provider.id,
            &profile.issuer,
            &profile.subject,
            &profile.email,
            &profile.display_name,
            &profile.avatar_url,
            now
        )?
        .run()
        .await?;
        user_id
    };
    lookup_account_user_by_id(database, &user_id)
        .await?
        .ok_or_else(|| worker::Error::RustError("OAuth account missing".into()))
}

async fn create_account_session_value(
    database: &worker::D1Database,
    env: &Env,
    user: &AccountUserRow,
    device_id: &str,
    event_type: &str,
) -> Result<Value> {
    let now = now_seconds();
    let session_id = Uuid::new_v4().to_string();
    let family_id = Uuid::new_v4().to_string();
    let refresh_token = new_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token);
    let refresh_expires_at = now + REFRESH_TOKEN_SECONDS;
    let (access_token, access_expires_at, access_jti) =
        issue_account_access_token(env, &user.id.to_string(), device_id, &session_id, now)?;
    database
        .batch(vec![
            worker::query!(
                database,
                "INSERT INTO account_sessions
                 (session_id, refresh_family_id, user_id, device_id, current_refresh_token_hash,
                  created_at, last_used_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
                &session_id,
                &family_id,
                user.id.to_string(),
                device_id,
                &refresh_hash,
                now,
                refresh_expires_at
            )?,
            worker::query!(
                database,
                "INSERT INTO account_refresh_tokens
                 (token_hash, session_id, generation, state, issued_at, expires_at)
                 VALUES (?1, ?2, 0, 'active', ?3, ?4)",
                &refresh_hash,
                &session_id,
                now,
                refresh_expires_at
            )?,
            worker::query!(
                database,
                "INSERT INTO account_auth_events
                 (event_id, user_id, session_id, event_type, occurred_at, details_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                Uuid::new_v4().to_string(),
                user.id.to_string(),
                &session_id,
                event_type,
                now,
                json!({"accessJti": access_jti}).to_string()
            )?,
        ])
        .await?;
    Ok(json!({
        "accessToken": access_token,
        "refreshToken": refresh_token,
        "tokenType": "Bearer",
        "expiresIn": ACCESS_TOKEN_SECONDS,
        "accessTokenExpiresAt": access_expires_at,
        "refreshTokenExpiresAt": refresh_expires_at,
        "sessionId": session_id,
        "deviceId": device_id,
        "username": user.username,
        "userId": user.id,
        "userNo": user.user_no.unwrap_or(user.id),
        "user": serialize_account_user(user),
    }))
}

fn oauth_result_page(success: bool, message: &str) -> Result<Response> {
    let color = if success { "#63dfb0" } else { "#ff8f99" };
    Response::from_html(format!(
        "<!doctype html><meta charset=utf-8><title>大乘登录</title><style>body{{margin:0;display:grid;place-items:center;min-height:100vh;background:#11141b;color:#f7f8fb;font:15px system-ui}}main{{width:min(360px,80vw);padding:36px;border:1px solid #303643;border-radius:20px;background:#1b1f28;text-align:center}}b{{display:grid;place-items:center;width:52px;height:52px;margin:auto;border-radius:15px;background:#6559e8;font-size:24px}}h1{{font-size:20px}}p{{color:{color};line-height:1.6}}</style><main><b>乘</b><h1>大乘账号</h1><p>{message}</p></main><script>setTimeout(()=>window.close(),1800)</script>"
    ))
}

async fn refresh_access_token(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let refresh: RefreshAccessRequest = match request.json().await {
        Ok(refresh) => refresh,
        Err(_) => {
            return error_response(400, "invalid_refresh_request", "refresh token is required");
        }
    };
    if !refresh.refresh_token.starts_with("mrt_") || refresh.refresh_token.len() != 68 {
        return error_response(401, "invalid_refresh_token", "登录会话已失效");
    }
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let token_hash = hash_refresh_token(&refresh.refresh_token);
    let row = worker::query!(
        &database,
        "SELECT rt.token_hash, rt.session_id, rt.generation, rt.state,
                s.user_id, s.device_id, s.expires_at AS session_expires_at, s.revoked_at
         FROM account_refresh_tokens rt
         JOIN account_sessions s ON s.session_id = rt.session_id
         WHERE rt.token_hash = ?1
         LIMIT 1",
        &token_hash
    )?
    .first::<RefreshTokenRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(401, "invalid_refresh_token", "登录会话已失效");
    };
    let now = now_seconds();
    if row.state != "active" {
        revoke_account_session(&database, &row.session_id, "refresh_token_reuse", now).await?;
        return error_response(401, "refresh_token_reused", "登录会话已撤销，请重新登录");
    }
    if row.revoked_at.is_some() || row.session_expires_at <= now {
        return error_response(401, "refresh_token_expired", "登录会话已过期，请重新登录");
    }
    if let Some(device_id) = refresh.device_id.as_deref()
        && device_id != row.device_id
    {
        return error_response(401, "device_mismatch", "登录设备不匹配，请重新登录");
    }
    let user = lookup_account_user_by_id(&database, &row.user_id).await?;
    let Some(user) = user else {
        revoke_account_session(&database, &row.session_id, "account_missing", now).await?;
        return error_response(401, "account_missing", "账号不存在");
    };

    let next_refresh = new_refresh_token();
    let next_hash = hash_refresh_token(&next_refresh);
    let next_generation = row.generation + 1;
    let (access_token, access_expires_at, access_jti) = issue_account_access_token(
        &context.env,
        &row.user_id,
        &row.device_id,
        &row.session_id,
        now,
    )?;
    let statements = vec![
        worker::query!(
            &database,
            "UPDATE account_refresh_tokens
             SET state = 'used', used_at = ?1, replaced_by_hash = ?2
             WHERE token_hash = ?3 AND state = 'active'",
            now,
            &next_hash,
            &row.token_hash
        )?,
        worker::query!(
            &database,
            "INSERT INTO account_refresh_tokens
             (token_hash, session_id, generation, state, issued_at, expires_at)
             VALUES (?1, ?2, ?3, 'active', ?4, ?5)",
            &next_hash,
            &row.session_id,
            next_generation,
            now,
            row.session_expires_at
        )?,
        worker::query!(
            &database,
            "UPDATE account_sessions
             SET current_refresh_token_hash = ?1, last_used_at = ?2
             WHERE session_id = ?3 AND revoked_at IS NULL",
            &next_hash,
            now,
            &row.session_id
        )?,
        worker::query!(
            &database,
            "INSERT INTO account_auth_events
             (event_id, user_id, session_id, event_type, occurred_at, details_json)
             VALUES (?1, ?2, ?3, 'refresh_rotated', ?4, ?5)",
            Uuid::new_v4().to_string(),
            &row.user_id,
            &row.session_id,
            now,
            json!({"generation": next_generation, "accessJti": access_jti}).to_string()
        )?,
    ];
    if database.batch(statements).await.is_err() {
        return error_response(
            409,
            "refresh_conflict",
            "登录会话正在轮换，请使用最新凭据重试",
        );
    }

    account_session_response(
        &user,
        &access_token,
        &next_refresh,
        access_expires_at,
        row.session_expires_at,
        &row.session_id,
        &row.device_id,
    )
}

async fn account_user_info(request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "登录已过期，请重新登录"),
    };
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let Some(user) = lookup_account_user_by_id(&database, &account.user_id).await? else {
        return error_response(404, "account_missing", "账号不存在");
    };
    Ok(Response::from_json(&serialize_account_user(&user))?.with_headers(auth_headers()))
}

async fn account_logout(request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "登录已过期，请重新登录"),
    };
    if let Some(session_id) = account.session_id {
        let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
        revoke_account_session(&database, &session_id, "logout", now_seconds()).await?;
    }
    Ok(
        Response::from_json(&json!({"success": true, "loggedIn": false}))?
            .with_headers(auth_headers()),
    )
}

async fn ai_usage_status(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let status = current_usage_status(&database, &context.env, &user_id, now_seconds()).await?;
    Response::from_json(&status)
}

async fn ai_usage_reserve(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    require_model_gateway(&request, &context.env)?;
    let user_id = authenticated_user(&request, &context.env)?;
    let reservation: UsageReservationRequest = request.json().await?;
    if !is_opaque_id(&reservation.request_id)
        || reservation.input_token_budget < 0
        || reservation.output_token_budget < 0
    {
        return error_response(
            400,
            "invalid_usage_reservation",
            "invalid usage reservation",
        );
    }
    let reserved_tokens = reservation
        .input_token_budget
        .checked_add(reservation.output_token_budget)
        .filter(|tokens| *tokens > 0 && *tokens <= MAX_TOKENS_PER_RESERVATION)
        .ok_or_else(|| worker::Error::RustError("invalid token reservation size".into()))?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    expire_usage_reservations(&database, &user_id, now).await?;
    if let Some(existing) =
        usage_reservation_by_request(&database, &user_id, &reservation.request_id).await?
    {
        return Response::from_json(&UsageReservation {
            reservation_id: existing.reservation_id,
            request_id: existing.request_id,
            reserved_tokens: existing.reserved_tokens,
            expires_at: existing.expires_at,
        });
    }

    let window_start = usage_window_start(now);
    let window_end = window_start + USAGE_WINDOW_SECONDS;
    let default_limit = default_usage_limit(&context.env)?;
    worker::query!(
        &database,
        "INSERT OR IGNORE INTO ai_usage_budgets
         (user_id, window_start, window_end, token_limit, used_tokens, reserved_tokens, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, 0, ?5)",
        &user_id,
        window_start,
        window_end,
        default_limit,
        now
    )?
    .run()
    .await?;

    let reservation_id = Uuid::new_v4().to_string();
    let expires_at = now + USAGE_RESERVATION_SECONDS;
    let results = database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT OR IGNORE INTO ai_usage_reservations
                 (reservation_id, user_id, window_start, request_id, input_token_budget,
                  output_token_budget, reserved_tokens, state, expires_at, created_at, updated_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'reserved', ?8, ?9, ?9
                 FROM ai_usage_budgets b
                 WHERE b.user_id = ?2 AND b.window_start = ?3
                   AND b.token_limit - b.used_tokens - b.reserved_tokens >= ?7",
                &reservation_id,
                &user_id,
                window_start,
                &reservation.request_id,
                reservation.input_token_budget,
                reservation.output_token_budget,
                reserved_tokens,
                expires_at,
                now
            )?,
            worker::query!(
                &database,
                "UPDATE ai_usage_budgets
                 SET reserved_tokens = reserved_tokens + ?1, updated_at = ?2
                 WHERE user_id = ?3 AND window_start = ?4
                   AND EXISTS (
                       SELECT 1 FROM ai_usage_reservations r
                       WHERE r.reservation_id = ?5 AND r.user_id = ?3
                         AND r.window_start = ?4 AND r.state = 'reserved'
                   )",
                reserved_tokens,
                now,
                &user_id,
                window_start,
                &reservation_id
            )?,
        ])
        .await?;
    if d1_changes(results.first()) == 0 {
        if let Some(existing) =
            usage_reservation_by_request(&database, &user_id, &reservation.request_id).await?
        {
            return Response::from_json(&UsageReservation {
                reservation_id: existing.reservation_id,
                request_id: existing.request_id,
                reserved_tokens: existing.reserved_tokens,
                expires_at: existing.expires_at,
            });
        }
        let status = current_usage_status(&database, &context.env, &user_id, now).await?;
        return usage_limit_response(&status);
    }
    Response::from_json(&UsageReservation {
        reservation_id,
        request_id: reservation.request_id,
        reserved_tokens,
        expires_at,
    })
}

async fn ai_usage_capture(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    require_model_gateway(&request, &context.env)?;
    let user_id = authenticated_user(&request, &context.env)?;
    let reservation_id = route_identifier(&context, "reservation_id")?;
    let capture: UsageCaptureRequest = request.json().await?;
    if !is_opaque_id(&capture.provider_response_id)
        || [
            capture.input_tokens,
            capture.cached_input_tokens,
            capture.output_tokens,
            capture.reasoning_output_tokens,
            capture.total_tokens,
        ]
        .into_iter()
        .any(|tokens| tokens < 0)
        || capture.cached_input_tokens > capture.input_tokens
        || capture.reasoning_output_tokens > capture.output_tokens
        || capture.total_tokens != capture.input_tokens.saturating_add(capture.output_tokens)
    {
        return error_response(
            400,
            "invalid_usage_capture",
            "invalid provider usage breakdown",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    if let Some(existing) =
        usage_event_by_response(&database, &capture.provider_response_id).await?
    {
        if existing.reservation_id != reservation_id {
            return error_response(
                409,
                "usage_response_conflict",
                "provider response was already captured",
            );
        }
        let status = current_usage_status(&database, &context.env, &user_id, now_seconds()).await?;
        return Response::from_json(&status);
    }
    let Some(reservation) = usage_reservation_by_id(&database, &user_id, reservation_id).await?
    else {
        return error_response(
            404,
            "usage_reservation_not_found",
            "usage reservation was not found",
        );
    };
    if reservation.state != "reserved" {
        return error_response(
            409,
            "usage_reservation_terminal",
            "usage reservation is already terminal",
        );
    }
    if capture.total_tokens > reservation.reserved_tokens {
        return error_response(
            409,
            "usage_capture_exceeds_reservation",
            "provider usage exceeds reservation",
        );
    }
    let now = now_seconds();
    let event_id = Uuid::new_v4().to_string();
    let results = database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT INTO ai_usage_events
                 (event_id, reservation_id, provider_response_id, input_tokens,
                  cached_input_tokens, output_tokens, reasoning_output_tokens, total_tokens, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
                 FROM ai_usage_reservations r
                 WHERE r.reservation_id = ?2 AND r.user_id = ?10 AND r.state = 'reserved'
                   AND ?8 <= r.reserved_tokens
                   AND EXISTS (
                       SELECT 1 FROM ai_usage_budgets b
                       WHERE b.user_id = r.user_id AND b.window_start = r.window_start
                   )",
                &event_id,
                reservation_id,
                &capture.provider_response_id,
                capture.input_tokens,
                capture.cached_input_tokens,
                capture.output_tokens,
                capture.reasoning_output_tokens,
                capture.total_tokens,
                now,
                &user_id
            )?,
            worker::query!(
                &database,
                "UPDATE ai_usage_budgets
                 SET reserved_tokens = reserved_tokens - (
                         SELECT r.reserved_tokens FROM ai_usage_reservations r
                         WHERE r.reservation_id = ?1 AND r.user_id = ?2
                     ),
                     used_tokens = used_tokens + ?3,
                     updated_at = ?4
                 WHERE user_id = ?2
                   AND window_start = (
                       SELECT r.window_start FROM ai_usage_reservations r
                       WHERE r.reservation_id = ?1 AND r.user_id = ?2
                   )
                   AND EXISTS (
                       SELECT 1 FROM ai_usage_events e WHERE e.event_id = ?5
                   )",
                reservation_id,
                &user_id,
                capture.total_tokens,
                now,
                &event_id
            )?,
            worker::query!(
                &database,
                "UPDATE ai_usage_reservations
                 SET actual_input_tokens = ?1, actual_cached_input_tokens = ?2,
                     actual_output_tokens = ?3, actual_reasoning_output_tokens = ?4,
                     actual_total_tokens = ?5, state = 'captured', updated_at = ?6
                 WHERE reservation_id = ?7 AND user_id = ?8 AND state = 'reserved'
                   AND EXISTS (
                       SELECT 1 FROM ai_usage_events e WHERE e.event_id = ?9
                   )",
                capture.input_tokens,
                capture.cached_input_tokens,
                capture.output_tokens,
                capture.reasoning_output_tokens,
                capture.total_tokens,
                now,
                reservation_id,
                &user_id,
                &event_id
            )?,
        ])
        .await?;
    if d1_changes(results.first()) == 0 {
        if let Some(existing) =
            usage_event_by_response(&database, &capture.provider_response_id).await?
        {
            if existing.reservation_id != reservation_id {
                return error_response(
                    409,
                    "usage_response_conflict",
                    "provider response was already captured",
                );
            }
            let status = current_usage_status(&database, &context.env, &user_id, now).await?;
            return Response::from_json(&status);
        }
        return error_response(
            409,
            "usage_reservation_terminal",
            "usage reservation is already terminal",
        );
    }
    let status = current_usage_status(&database, &context.env, &user_id, now).await?;
    Response::from_json(&status)
}

async fn ai_usage_release(request: Request, context: RouteContext<()>) -> Result<Response> {
    require_model_gateway(&request, &context.env)?;
    let user_id = authenticated_user(&request, &context.env)?;
    let reservation_id = route_identifier(&context, "reservation_id")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    database
        .batch(vec![
            worker::query!(
                &database,
                "UPDATE ai_usage_budgets
                 SET reserved_tokens = reserved_tokens - (
                         SELECT r.reserved_tokens FROM ai_usage_reservations r
                         WHERE r.reservation_id = ?1 AND r.user_id = ?2 AND r.state = 'reserved'
                     ),
                     updated_at = ?3
                 WHERE user_id = ?2
                   AND window_start = (
                       SELECT r.window_start FROM ai_usage_reservations r
                       WHERE r.reservation_id = ?1 AND r.user_id = ?2 AND r.state = 'reserved'
                   )",
                reservation_id,
                &user_id,
                now
            )?,
            worker::query!(
                &database,
                "UPDATE ai_usage_reservations SET state = 'released', updated_at = ?1
                 WHERE reservation_id = ?2 AND user_id = ?3 AND state = 'reserved'
                   AND EXISTS (
                       SELECT 1 FROM ai_usage_budgets b
                       WHERE b.user_id = ?3 AND b.window_start = ai_usage_reservations.window_start
                   )",
                now,
                reservation_id,
                &user_id
            )?,
        ])
        .await?;
    let status = current_usage_status(&database, &context.env, &user_id, now).await?;
    Response::from_json(&status)
}

async fn marketplace_plugins(request: Request, context: RouteContext<()>) -> Result<Response> {
    let mut query = "%".to_string();
    let mut platform = None;
    for (key, value) in request.url()?.query_pairs() {
        if key == "q" && !value.trim().is_empty() {
            query = format!("%{}%", value.trim());
        } else if key == "platform" && !value.trim().is_empty() {
            let value = value.trim().to_string();
            if !matches!(value.as_str(), "cli" | "desktop" | "mobile" | "web") {
                return error_response(
                    400,
                    "invalid_marketplace_platform",
                    "platform must be cli, desktop, mobile, or web.",
                );
            }
            platform = Some(value);
        }
    }
    let platform_pattern = platform
        .as_deref()
        .map(|value| format!("%\"{value}\"%"))
        .unwrap_or_else(|| "%".to_string());
    let database = context.env.d1(DATABASE_BINDING)?;
    let rows = worker::query!(
        &database,
        "SELECT mp.plugin_id, mp.display_name, mp.description, mp.latest_version,
                pr.package_sha256, pr.package_size, mp.platforms_json,
                pr.deployment_url, pr.published_at, pr.source_json,
                pr.release_manifest_json, pr.release_manifest_sha256, pr.release_status
         FROM marketplace_plugins mp
         JOIN plugin_releases pr
           ON pr.plugin_id = mp.plugin_id AND pr.version = mp.latest_version
         WHERE mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.release_status = 'approved'
           AND pr.deployment_url <> ''
           AND (mp.display_name LIKE ?1 OR mp.description LIKE ?1 OR mp.plugin_id LIKE ?1)
           AND mp.platforms_json LIKE ?2
         ORDER BY mp.updated_at DESC LIMIT 100",
        &query,
        &platform_pattern
    )?
    .all()
    .await?
    .results::<MarketplacePluginRow>()?;
    let plugins = rows
        .into_iter()
        .map(|row| {
            let platforms =
                serde_json::from_str::<Vec<String>>(&row.platforms_json).unwrap_or_default();
            let source = serde_json::from_str::<Value>(&row.source_json).unwrap_or(Value::Null);
            let release_manifest =
                serde_json::from_str::<Value>(&row.release_manifest_json).unwrap_or(Value::Null);
            json!({
                "pluginId": row.plugin_id,
                "displayName": row.display_name,
                "description": row.description,
                "latestVersion": row.latest_version,
                "packageSha256": row.package_sha256,
                "packageSize": row.package_size.and_then(exact_nonnegative_i64),
                "platforms": platforms,
                "deploymentUrl": row.deployment_url,
                "publishedAt": row.published_at.and_then(exact_nonnegative_i64),
                "source": source,
                "releaseManifest": release_manifest,
                "releaseManifestSha256": row.release_manifest_sha256,
                "releaseStatus": row.release_status,
            })
        })
        .collect::<Vec<_>>();
    Response::from_json(&json!({"plugins": plugins}))
}

async fn marketplace_release_publish(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    const MAX_PACKAGE_BYTES: usize = 50 * 1024 * 1024;

    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to publish marketplace releases.",
            );
        }
    };
    if !account.is_test_account
        && !account
            .scopes
            .iter()
            .any(|scope| scope == "marketplace.publish")
    {
        return error_response(
            403,
            "marketplace_publish_forbidden",
            "The account token does not permit marketplace publishing.",
        );
    }

    let form = request.form_data().await?;
    let field = |name: &str| -> Result<String> {
        match form.get(name) {
            Some(FormEntry::Field(value)) if !value.trim().is_empty() => Ok(value),
            _ => Err(worker::Error::RustError(format!(
                "marketplace release field {name} is required"
            ))),
        }
    };
    let plugin_id = field("pluginId")?;
    let version = field("version")?;
    if !is_identifier(&plugin_id) || !is_version_identifier(&version) {
        return error_response(
            400,
            "invalid_marketplace_release_identifier",
            "pluginId and version must be normalized identifiers.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let existing_plugin = worker::query!(
        &database,
        "SELECT publisher_user_id FROM marketplace_plugins WHERE plugin_id = ?1",
        &plugin_id
    )?
    .first::<MarketplacePluginOwnerRow>(None)
    .await?;
    if let Some(existing_plugin) = existing_plugin {
        if existing_plugin.publisher_user_id != account.user_id {
            return error_response(
                403,
                "marketplace_plugin_owner_mismatch",
                "The authenticated publisher does not own this plugin ID.",
            );
        }
    }
    let existing_release = worker::query!(
        &database,
        "SELECT package_sha256 FROM plugin_releases WHERE plugin_id = ?1 AND version = ?2",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceExistingReleaseRow>(None)
    .await?;
    if let Some(existing_release) = existing_release {
        return error_response(
            409,
            "version_already_exists",
            &format!(
                "Release {plugin_id}@{version} is immutable and already exists with package SHA-256 {}.",
                existing_release.package_sha256
            ),
        );
    }
    let deployment_url = field("deploymentUrl")?;
    if !is_public_https_url(&deployment_url) {
        return error_response(
            400,
            "invalid_marketplace_deployment_url",
            "deploymentUrl must be a public HTTPS URL.",
        );
    }
    let expected_sha256 = field("packageSha256")?.to_ascii_lowercase();
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return error_response(
            400,
            "invalid_marketplace_package_sha256",
            "packageSha256 must be a 64-character hexadecimal digest.",
        );
    }
    let expected_size = field("packageSize")?
        .parse::<usize>()
        .map_err(|_| worker::Error::RustError("invalid packageSize".into()))?;
    if expected_size == 0 || expected_size > MAX_PACKAGE_BYTES {
        return error_response(
            400,
            "invalid_marketplace_package_size",
            "packageSize must be between 1 byte and 50 MiB.",
        );
    }
    let platforms = serde_json::from_str::<Vec<String>>(&field("platforms")?)
        .map_err(|_| worker::Error::RustError("invalid marketplace platforms".into()))?;
    if platforms.is_empty()
        || platforms
            .iter()
            .any(|platform| !matches!(platform.as_str(), "cli" | "desktop" | "mobile" | "web"))
    {
        return error_response(
            400,
            "invalid_marketplace_platforms",
            "platforms must contain supported Mahayana targets.",
        );
    }
    let source = serde_json::from_str::<Value>(&field("source")?)
        .map_err(|_| worker::Error::RustError("invalid marketplace source".into()))?;
    if let Err(message) = validate_github_source_identity(&source) {
        return error_response(400, "invalid_marketplace_source", &message);
    }
    let release_manifest = serde_json::from_str::<Value>(&field("releaseManifest")?)
        .map_err(|_| worker::Error::RustError("invalid marketplace release manifest".into()))?;
    if let Err(message) = validate_multi_artifact_release_manifest(
        &release_manifest,
        &plugin_id,
        &version,
        &expected_sha256,
        expected_size,
        &platforms,
        &source,
    ) {
        return error_response(400, "invalid_marketplace_release_manifest", &message);
    }
    let source_json = serde_json::to_string(&source)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_bytes = canonical_json_bytes(&release_manifest)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_json = String::from_utf8(release_manifest_bytes)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_sha256 = canonical_json_sha256(&release_manifest)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let package = match form.get("package") {
        Some(FormEntry::File(file)) => file.bytes().await?,
        _ => {
            return error_response(
                400,
                "marketplace_package_missing",
                "The release package file is required.",
            );
        }
    };
    if package.len() != expected_size {
        return error_response(
            400,
            "marketplace_package_size_mismatch",
            "The uploaded package size does not match release metadata.",
        );
    }
    let actual_sha256 = format!("{:x}", Sha256::digest(&package));
    if actual_sha256 != expected_sha256 {
        return error_response(
            400,
            "marketplace_package_sha256_mismatch",
            "The uploaded package digest does not match release metadata.",
        );
    }

    let remote_package = match verified_marketplace_site_package_with_retry(
        &deployment_url,
        &plugin_id,
        &version,
        &actual_sha256,
        expected_size,
        &source,
        &release_manifest,
        &release_manifest_sha256,
    )
    .await
    {
        Ok(package) => package,
        Err(message) => {
            return error_response(400, "marketplace_deployment_verification_failed", &message);
        }
    };
    if remote_package != package {
        return error_response(
            400,
            "marketplace_deployment_package_mismatch",
            "The package served by the Cloudflare plugin site differs from the uploaded release package.",
        );
    }
    let package_key = marketplace_asset_url(&deployment_url, "/mahayana/plugin.tar.gz")?;

    let now = now_seconds();
    let package_size = i64::try_from(expected_size)
        .map_err(|_| worker::Error::RustError("packageSize exceeds D1 integer range".into()))?;
    let platforms_json = serde_json::to_string(&platforms)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_status = if account.is_test_account {
        "approved"
    } else {
        "pending"
    };
    database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT INTO marketplace_plugins
                 (plugin_id, display_name, description, publisher_user_id, latest_version,
                  visibility, review_state, created_at, updated_at, platforms_json)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)
                 ON CONFLICT(plugin_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   description = excluded.description,
                   latest_version = excluded.latest_version,
                   visibility = excluded.visibility,
                   review_state = excluded.review_state,
                   updated_at = excluded.updated_at,
                   platforms_json = excluded.platforms_json",
                &plugin_id,
                &format!("Published from {deployment_url}"),
                &account.user_id,
                &version,
                if account.is_test_account {
                    "public"
                } else {
                    "unlisted"
                },
                if account.is_test_account {
                    "approved"
                } else {
                    "pending"
                },
                now,
                &platforms_json
            )?,
            worker::query!(
                &database,
                "INSERT INTO plugin_releases
                 (plugin_id, version, package_key, package_sha256, package_size,
                  tuf_target_path, published_at, deployment_url, source_json,
                  release_manifest_json, release_manifest_sha256, release_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                &plugin_id,
                &version,
                &package_key,
                &actual_sha256,
                package_size,
                &format!("plugins/{plugin_id}/{version}.tar.gz"),
                now,
                &deployment_url,
                &source_json,
                &release_manifest_json,
                &release_manifest_sha256,
                release_status
            )?,
        ])
        .await?;

    Response::from_json(&json!({
        "published": true,
        "approved": account.is_test_account,
        "pluginId": plugin_id,
        "version": version,
        "deploymentUrl": deployment_url,
        "packageSha256": actual_sha256,
        "packageSize": package_size,
        "platforms": platforms,
        "source": source,
        "releaseManifest": release_manifest,
        "releaseManifestSha256": release_manifest_sha256,
        "releaseStatus": release_status,
    }))
}

async fn marketplace_external_release_publish(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    const MAX_ARTIFACT_BYTES: usize = 100 * 1024 * 1024;

    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to publish marketplace releases.",
            );
        }
    };
    if !account.is_test_account
        && !account
            .scopes
            .iter()
            .any(|scope| scope == "marketplace.publish")
    {
        return error_response(
            403,
            "marketplace_publish_forbidden",
            "The account token does not permit marketplace publishing.",
        );
    }

    let body: Value = match request.json().await {
        Ok(body) => body,
        Err(_) => {
            return error_response(
                400,
                "invalid_marketplace_release",
                "The external release request must be valid JSON.",
            );
        }
    };
    let plugin_id = match body.get("pluginId").and_then(Value::as_str) {
        Some(value) if is_identifier(value) => value.to_string(),
        _ => {
            return error_response(
                400,
                "invalid_marketplace_release_identifier",
                "pluginId must be a normalized identifier.",
            );
        }
    };
    let version = match body.get("version").and_then(Value::as_str) {
        Some(value) if is_version_identifier(value) => value.to_string(),
        _ => {
            return error_response(
                400,
                "invalid_marketplace_release_identifier",
                "version must be a normalized version identifier.",
            );
        }
    };
    let display_name = body
        .get("displayName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 120)
        .unwrap_or(&plugin_id)
        .to_string();
    let description = body
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.len() <= 2_000)
        .unwrap_or("")
        .to_string();
    let platforms = match body.get("platforms").and_then(Value::as_array) {
        Some(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    if platforms.is_empty()
        || platforms.iter().any(|platform| {
            !matches!(
                platform.as_str(),
                "cli" | "desktop" | "mobile" | "web" | "ios" | "android"
            )
        })
    {
        return error_response(
            400,
            "invalid_marketplace_platforms",
            "platforms must contain supported Mahayana targets.",
        );
    }

    let release_manifest = match body.get("releaseManifest") {
        Some(value) => value.clone(),
        None => {
            return error_response(
                400,
                "invalid_marketplace_release_manifest",
                "releaseManifest is required.",
            );
        }
    };
    if let Err(message) = validate_external_release_manifest(
        &release_manifest,
        &plugin_id,
        &version,
        &platforms,
        MAX_ARTIFACT_BYTES,
    ) {
        return error_response(400, "invalid_marketplace_release_manifest", &message);
    }

    let database = context.env.d1(DATABASE_BINDING)?;
    if let Some(existing) = worker::query!(
        &database,
        "SELECT publisher_user_id FROM marketplace_plugins WHERE plugin_id = ?1",
        &plugin_id
    )?
    .first::<MarketplacePluginOwnerRow>(None)
    .await?
    {
        if existing.publisher_user_id != account.user_id {
            return error_response(
                403,
                "marketplace_plugin_owner_mismatch",
                "The authenticated publisher does not own this plugin ID.",
            );
        }
    }
    if let Some(existing) = worker::query!(
        &database,
        "SELECT package_sha256 FROM plugin_releases WHERE plugin_id = ?1 AND version = ?2",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceExistingReleaseRow>(None)
    .await?
    {
        return error_response(
            409,
            "version_already_exists",
            &format!(
                "Release {plugin_id}@{version} is immutable and already exists with package SHA-256 {}.",
                existing.package_sha256
            ),
        );
    }

    // Admission verifies every external runtime artifact at its publisher URL.
    // The bytes are never persisted by the marketplace.
    let artifacts = release_manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .expect("validated release manifest artifacts");
    let mut resolved_artifacts = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let resolved_url = match resolve_external_artifact_url(artifact).await {
            Ok(url) => url,
            Err(message) => {
                return error_response(400, "marketplace_artifact_resolution_failed", &message);
            }
        };
        let expected_size = artifact
            .get("size")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .expect("validated artifact size");
        let expected_sha256 = artifact
            .get("sha256")
            .and_then(Value::as_str)
            .expect("validated artifact sha256");
        if let Err(message) = verify_external_artifact(
            &resolved_url,
            expected_sha256,
            expected_size,
            MAX_ARTIFACT_BYTES,
        )
        .await
        {
            return error_response(400, "marketplace_artifact_verification_failed", &message);
        }
        resolved_artifacts.push(resolved_url);
    }

    let primary = artifacts.first().expect("validated non-empty artifacts");
    let package_sha256 = primary
        .get("sha256")
        .and_then(Value::as_str)
        .expect("validated primary sha")
        .to_ascii_lowercase();
    let package_size = primary
        .get("size")
        .and_then(Value::as_u64)
        .and_then(|value| i64::try_from(value).ok())
        .expect("validated primary size");
    let primary_url = resolved_artifacts
        .first()
        .expect("validated primary URL")
        .clone();
    let release_manifest_json = serde_json::to_string(&release_manifest)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_sha256 = format!("{:x}", Sha256::digest(release_manifest_json.as_bytes()));
    let source = body
        .get("source")
        .cloned()
        .unwrap_or_else(|| json!({"provider":"external","artifact": primary.get("source").cloned().unwrap_or(Value::Null)}));
    let source_json = serde_json::to_string(&source)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let platforms_json = serde_json::to_string(&platforms)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let now = now_seconds();
    let release_status = if account.is_test_account {
        "approved"
    } else {
        "pending"
    };
    let visibility = if account.is_test_account {
        "public"
    } else {
        "unlisted"
    };
    let review_state = if account.is_test_account {
        "approved"
    } else {
        "pending"
    };

    database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT INTO marketplace_plugins
                 (plugin_id, display_name, description, publisher_user_id, latest_version,
                  visibility, review_state, created_at, updated_at, platforms_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
                 ON CONFLICT(plugin_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   description = excluded.description,
                   latest_version = excluded.latest_version,
                   visibility = excluded.visibility,
                   review_state = excluded.review_state,
                   updated_at = excluded.updated_at,
                   platforms_json = excluded.platforms_json",
                &plugin_id,
                &display_name,
                &description,
                &account.user_id,
                &version,
                visibility,
                review_state,
                now,
                &platforms_json
            )?,
            worker::query!(
                &database,
                "INSERT INTO plugin_releases
                 (plugin_id, version, package_key, package_sha256, package_size,
                  tuf_target_path, published_at, deployment_url, source_json,
                  release_manifest_json, release_manifest_sha256, release_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                &plugin_id,
                &version,
                &primary_url,
                &package_sha256,
                package_size,
                &format!("external/{plugin_id}/{version}"),
                now,
                &primary_url,
                &source_json,
                &release_manifest_json,
                &release_manifest_sha256,
                release_status
            )?,
        ])
        .await?;

    Response::from_json(&json!({
        "published": true,
        "approved": account.is_test_account,
        "storage": "external",
        "pluginId": plugin_id,
        "version": version,
        "platforms": platforms,
        "source": source,
        "releaseManifest": release_manifest,
        "releaseManifestSha256": release_manifest_sha256,
        "resolvedArtifacts": resolved_artifacts,
        "releaseStatus": release_status,
    }))
}

async fn marketplace_release_metadata(
    _request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let version = route_version(&context)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT pr.plugin_id, pr.version, pr.package_sha256, pr.package_size,
                pr.deployment_url, pr.published_at, mp.platforms_json,
                pr.source_json, pr.release_manifest_json, pr.release_manifest_sha256,
                pr.release_status, pr.revoked_at, pr.revocation_reason
         FROM plugin_releases pr
         JOIN marketplace_plugins mp ON mp.plugin_id = pr.plugin_id
         WHERE pr.plugin_id = ?1 AND pr.version = ?2
           AND mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.deployment_url <> ''",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceReleaseMetadataRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    };
    if row.release_status == "revoked" {
        let revoked_at = row.revoked_at.and_then(exact_nonnegative_i64);
        let reason = row
            .revocation_reason
            .as_deref()
            .unwrap_or("publisher_request");
        return error_response(
            410,
            "release_revoked",
            &format!("Release {plugin_id}@{version} was revoked at {revoked_at:?}: {reason}."),
        );
    }
    if row.release_status != "approved" {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    }
    let platforms = serde_json::from_str::<Vec<String>>(&row.platforms_json).unwrap_or_default();
    let source = serde_json::from_str::<Value>(&row.source_json).unwrap_or(Value::Null);
    let release_manifest =
        serde_json::from_str::<Value>(&row.release_manifest_json).unwrap_or(Value::Null);
    let Some(package_size) = exact_nonnegative_i64(row.package_size) else {
        return error_response(
            503,
            "marketplace_package_size_invalid",
            "The approved release has an invalid package size.",
        );
    };
    let Some(published_at) = exact_nonnegative_i64(row.published_at) else {
        return error_response(
            503,
            "marketplace_published_at_invalid",
            "The approved release has an invalid published timestamp.",
        );
    };
    Response::from_json(&json!({
        "pluginId": row.plugin_id,
        "version": row.version,
        "packageSha256": row.package_sha256,
        "packageSize": package_size,
        "deploymentUrl": row.deployment_url,
        "publishedAt": published_at,
        "platforms": platforms,
        "source": source,
        "releaseManifest": release_manifest,
        "releaseManifestSha256": row.release_manifest_sha256,
        "releaseStatus": row.release_status,
    }))
}

async fn marketplace_plugin_download(
    _request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let version = route_version(&context)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let release = worker::query!(
        &database,
        "SELECT pr.deployment_url, pr.package_key, pr.package_sha256, pr.package_size,
                pr.release_status, pr.revoked_at, pr.revocation_reason
         FROM plugin_releases pr
         JOIN marketplace_plugins mp ON mp.plugin_id = pr.plugin_id
         WHERE pr.plugin_id = ?1 AND pr.version = ?2
           AND mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.deployment_url <> ''",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceReleaseDownloadRow>(None)
    .await?;
    let Some(release) = release else {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    };
    if release.release_status == "revoked" {
        let revoked_at = release.revoked_at.and_then(exact_nonnegative_i64);
        let reason = release
            .revocation_reason
            .as_deref()
            .unwrap_or("publisher_request");
        return error_response(
            410,
            "release_revoked",
            &format!("Release {plugin_id}@{version} was revoked at {revoked_at:?}: {reason}."),
        );
    }
    if release.release_status != "approved" {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    }

    if !is_public_https_url(&release.deployment_url) {
        return error_response(
            503,
            "marketplace_deployment_url_invalid",
            "The approved release does not point to a valid Cloudflare Pages/Worker site.",
        );
    }
    let package_size = match exact_nonnegative_i64(release.package_size)
        .and_then(|size| usize::try_from(size).ok())
    {
        Some(size) if size > 0 && size <= 50 * 1024 * 1024 => size,
        _ => {
            return error_response(
                503,
                "marketplace_package_size_invalid",
                "The approved release has an invalid package size.",
            );
        }
    };
    // The marketplace is a metadata/control plane, not a binary CDN. Package
    // bytes stay on the publisher's immutable GitHub/npm/HTTPS origin. Older
    // releases already store their externally served package URL in
    // `package_key`, so redirecting is backward compatible while removing the
    // Worker from the plugin data path. Clients MUST continue validating the
    // catalogued size and SHA-256 after following this redirect.
    let package_url = Url::parse(&release.package_key).map_err(|_| {
        worker::Error::RustError("marketplace release package URL is invalid".into())
    })?;
    if package_url.scheme() != "https" || package_url.host_str().is_none() {
        return error_response(
            503,
            "marketplace_package_url_invalid",
            "The approved release does not point to a public HTTPS artifact.",
        );
    }
    let mut response = Response::redirect_with_status(package_url, 307)?;
    response.headers_mut().set(
        "X-Mahayana-Package-Sha256",
        &release.package_sha256,
    )?;
    response
        .headers_mut()
        .set("X-Mahayana-Package-Size", &package_size.to_string())?;
    response
        .headers_mut()
        .set("Cache-Control", "public, max-age=300")?;
    Ok(response)
}

async fn marketplace_release_revoke(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to revoke marketplace releases.",
            );
        }
    };
    if !account.is_test_account
        && !account
            .scopes
            .iter()
            .any(|scope| scope == "marketplace.publish")
    {
        return error_response(
            403,
            "marketplace_revoke_forbidden",
            "The account token does not permit marketplace release revocation.",
        );
    }

    let plugin_id = route_identifier(&context, "plugin_id")?;
    let version = route_version(&context)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let owner = worker::query!(
        &database,
        "SELECT publisher_user_id FROM marketplace_plugins WHERE plugin_id = ?1",
        &plugin_id
    )?
    .first::<MarketplacePluginOwnerRow>(None)
    .await?;
    let Some(owner) = owner else {
        return error_response(
            404,
            "marketplace_plugin_not_found",
            "The marketplace plugin does not exist.",
        );
    };
    if owner.publisher_user_id != account.user_id {
        return error_response(
            403,
            "marketplace_plugin_owner_mismatch",
            "The authenticated publisher does not own this plugin ID.",
        );
    }

    let release = worker::query!(
        &database,
        "SELECT release_status, revoked_at, revocation_reason
         FROM plugin_releases WHERE plugin_id = ?1 AND version = ?2",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceReleaseStatusRow>(None)
    .await?;
    let Some(release) = release else {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The marketplace release does not exist.",
        );
    };
    if release.release_status == "revoked" {
        return Response::from_json(&json!({
            "revoked": true,
            "pluginId": plugin_id,
            "version": version,
            "releaseStatus": release.release_status,
            "revokedAt": release.revoked_at.and_then(exact_nonnegative_i64),
            "reason": release.revocation_reason,
        }));
    }

    let body = request.json::<Value>().await.unwrap_or_else(|_| json!({}));
    let reason = body
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("publisher_request")
        .trim()
        .to_string();
    if reason.is_empty() || reason.len() > 256 || reason.chars().any(char::is_control) {
        return error_response(
            400,
            "invalid_revocation_reason",
            "Revocation reason must contain 1 to 256 printable characters.",
        );
    }

    let now = now_seconds();
    database
        .batch(vec![
            worker::query!(
                &database,
                "UPDATE plugin_releases
                 SET release_status = 'revoked', revoked_at = ?1, revocation_reason = ?2
                 WHERE plugin_id = ?3 AND version = ?4",
                now,
                &reason,
                &plugin_id,
                &version
            )?,
            worker::query!(
                &database,
                "UPDATE marketplace_plugins
                 SET latest_version = (
                     SELECT version FROM plugin_releases
                     WHERE plugin_id = ?2 AND release_status = 'approved'
                     ORDER BY published_at DESC LIMIT 1
                 ), updated_at = ?1
                 WHERE plugin_id = ?2 AND latest_version = ?3",
                now,
                &plugin_id,
                &version
            )?,
        ])
        .await?;

    Response::from_json(&json!({
        "revoked": true,
        "pluginId": plugin_id,
        "version": version,
        "releaseStatus": "revoked",
        "revokedAt": now,
        "reason": reason,
    }))
}

async fn wallet_balance(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT
             wb.currency AS currency,
             wb.balance - COALESCE((
                 SELECT SUM(cr.amount) FROM consumption_reservations cr
                 WHERE cr.user_id = ?1 AND cr.currency = wb.currency AND cr.state = 'reserved'
             ), 0) AS available,
             COALESCE((
                 SELECT SUM(cr.amount) FROM consumption_reservations cr
                 WHERE cr.user_id = ?1 AND cr.currency = wb.currency AND cr.state = 'reserved'
             ), 0) AS reserved
         FROM wallet_balances wb
         WHERE wb.owner_type = 'user' AND wb.owner_id = ?1 AND wb.currency = 'MBC'",
        &user_id
    )?
    .first::<BalanceRow>(None)
    .await?
    .unwrap_or(BalanceRow {
        currency: "MBC".into(),
        available: 0,
        reserved: 0,
    });
    Response::from_json(&json!({
        "currency": Currency(row.currency),
        "available": row.available,
        "reserved": row.reserved,
    }))
}

async fn wallet_history(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let account_id = format!("user:{user_id}:MBC");
    let rows = worker::query!(
        &database,
        "SELECT je.entry_id, je.reference_type, je.reference_id, je.created_at, jl.amount, jl.currency
         FROM journal_lines jl
         JOIN journal_entries je ON je.entry_id = jl.entry_id
         WHERE jl.account_id = ?1 AND je.state = 'posted'
         ORDER BY je.created_at DESC LIMIT 100",
        &account_id
    )?
    .all()
    .await?
    .results::<serde_json::Value>()?;
    Response::from_json(&json!({"entries": rows, "nextCursor": null}))
}

async fn commerce_quote(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let _user_id = authenticated_user(&request, &context.env)?;
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let quote_request: QuoteRequest = request.json().await?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let price = active_price(&database, plugin_id, quote_request.sku.trim(), now).await?;
    let Some(price) = price else {
        return error_response(404, "product_not_found", "SKU is not available");
    };
    Response::from_json(&Quote {
        quote_id: Uuid::new_v4().to_string(),
        plugin_id: price.plugin_id,
        sku: price.sku,
        amount: price.amount,
        currency: Currency(price.currency),
        expires_at: now + 300,
    })
}

async fn commerce_purchase(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let purchase: PurchaseRequest = request.json().await?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let Some(price) = active_price(&database, plugin_id, purchase.sku.trim(), now).await? else {
        return error_response(404, "product_not_found", "SKU is not available");
    };
    if let Some(existing) =
        order_by_idempotency(&database, &user_id, &purchase.idempotency_key).await?
    {
        if existing.plugin_id != price.plugin_id
            || existing.sku != price.sku
            || existing.currency != price.currency
            || existing.amount != price.amount
        {
            return error_response(
                409,
                "idempotency_conflict",
                "idempotency key was already used for a different product or price",
            );
        }
        if existing.status == "fulfilled" {
            let entitlement = entitlement_for_order(&database, &existing.order_id).await?;
            return Response::from_json(&json!({
                "orderId": existing.order_id,
                "status": existing.status,
                "entitlement": entitlement,
            }));
        }
    }
    let order_id = Uuid::new_v4().to_string();
    let entry_id = Uuid::new_v4().to_string();
    let entitlement_id = Uuid::new_v4().to_string();
    let user_account = format!("user:{user_id}:{}", price.currency);
    let platform_account = format!("platform:content:{}", price.currency);
    let user_line_id = Uuid::new_v4().to_string();
    let platform_line_id = Uuid::new_v4().to_string();
    let audit_id = Uuid::new_v4().to_string();
    let statements = vec![
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO wallet_accounts
             (account_id, owner_type, owner_id, currency, created_at)
             VALUES (?1, 'user', ?2, ?3, ?4)",
            &user_account,
            &user_id,
            &price.currency,
            now
        )?,
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO wallet_accounts
             (account_id, owner_type, owner_id, currency, created_at)
             VALUES (?1, 'platform', 'digital-content', ?2, ?3)",
            &platform_account,
            &price.currency,
            now
        )?,
        worker::query!(
            &database,
            "INSERT INTO orders
             (order_id, buyer_user_id, plugin_id, product_id, price_id, sku, currency, amount,
              status, idempotency_key, created_at, updated_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?10
             WHERE COALESCE((SELECT balance FROM wallet_balances WHERE account_id = ?11), 0) >= ?8
             ON CONFLICT(buyer_user_id, idempotency_key) DO NOTHING",
            &order_id,
            &user_id,
            &price.plugin_id,
            &price.product_id,
            &price.price_id,
            &price.sku,
            &price.currency,
            price.amount,
            &purchase.idempotency_key,
            now,
            &user_account
        )?,
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO journal_entries
             (entry_id, reference_type, reference_id, state, created_at)
             SELECT ?1, 'order', order_id, 'draft', ?2 FROM orders
             WHERE buyer_user_id = ?3 AND idempotency_key = ?4",
            &entry_id,
            now,
            &user_id,
            &purchase.idempotency_key
        )?,
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO journal_lines
             (line_id, entry_id, account_id, currency, amount, created_at)
             SELECT ?1, je.entry_id, ?2, ?3, ?4, ?5 FROM journal_entries je
             JOIN orders o ON o.order_id = je.reference_id
             WHERE je.reference_type = 'order' AND je.state = 'draft'
               AND o.buyer_user_id = ?6 AND o.idempotency_key = ?7",
            &user_line_id,
            &user_account,
            &price.currency,
            -price.amount,
            now,
            &user_id,
            &purchase.idempotency_key
        )?,
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO journal_lines
             (line_id, entry_id, account_id, currency, amount, created_at)
             SELECT ?1, je.entry_id, ?2, ?3, ?4, ?5 FROM journal_entries je
             JOIN orders o ON o.order_id = je.reference_id
             WHERE je.reference_type = 'order' AND je.state = 'draft'
               AND o.buyer_user_id = ?6 AND o.idempotency_key = ?7",
            &platform_line_id,
            &platform_account,
            &price.currency,
            price.amount,
            now,
            &user_id,
            &purchase.idempotency_key
        )?,
        worker::query!(
            &database,
            "UPDATE journal_entries SET state = 'posted', posted_at = ?1
             WHERE entry_id IN (
                 SELECT je.entry_id FROM journal_entries je JOIN orders o ON o.order_id = je.reference_id
                 WHERE je.reference_type = 'order' AND o.buyer_user_id = ?2 AND o.idempotency_key = ?3
             ) AND state = 'draft'
               AND (SELECT COUNT(*) FROM journal_lines jl WHERE jl.entry_id = journal_entries.entry_id) >= 2
               AND NOT EXISTS (
                   SELECT currency FROM journal_lines jl
                   WHERE jl.entry_id = journal_entries.entry_id
                   GROUP BY currency HAVING SUM(amount) <> 0
               )",
            now,
            &user_id,
            &purchase.idempotency_key
        )?,
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO entitlements
             (entitlement_id, user_id, plugin_id, product_id, order_id, capability, status, granted_at)
             SELECT ?1, ?2, o.plugin_id, o.product_id, o.order_id, ?3, 'active', ?4
             FROM orders o JOIN journal_entries je
               ON je.reference_type = 'order' AND je.reference_id = o.order_id AND je.state = 'posted'
             WHERE o.buyer_user_id = ?2 AND o.idempotency_key = ?5",
            &entitlement_id,
            &user_id,
            &price.capability,
            now,
            &purchase.idempotency_key
        )?,
        worker::query!(
            &database,
            "UPDATE orders SET status = 'fulfilled', updated_at = ?1
             WHERE buyer_user_id = ?2 AND idempotency_key = ?3 AND status = 'pending'
               AND EXISTS (
                   SELECT 1 FROM entitlements e
                   WHERE e.order_id = orders.order_id AND e.status = 'active'
               )",
            now,
            &user_id,
            &purchase.idempotency_key
        )?,
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO audit_events
             (event_id, actor_type, actor_id, event_type, subject_type, subject_id, payload_json, created_at)
             SELECT ?1, 'user', ?2, 'commerce.purchase', 'order', o.order_id, '{}', ?3
             FROM orders o
             WHERE o.buyer_user_id = ?2 AND o.idempotency_key = ?4 AND o.status = 'fulfilled'",
            &audit_id,
            &user_id,
            now,
            &purchase.idempotency_key
        )?,
    ];
    database.batch(statements).await?;
    let order = order_by_idempotency(&database, &user_id, &purchase.idempotency_key).await?;
    let Some(order) = order else {
        return error_response(
            402,
            "insufficient_balance",
            "insufficient Mahayana bean balance",
        );
    };
    if order.plugin_id != price.plugin_id || order.sku != price.sku {
        return error_response(
            409,
            "idempotency_conflict",
            "idempotency key was already used for a different product",
        );
    }
    if order.status != "fulfilled" {
        return error_response(
            500,
            "ledger_invariant_violation",
            "order could not be posted as a balanced journal entry",
        );
    }
    let entitlement = entitlement_for_order(&database, &order.order_id).await?;
    Response::from_json(&json!({
        "orderId": order.order_id,
        "status": order.status,
        "entitlement": entitlement,
    }))
}

async fn commerce_entitlement(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let capability = route_identifier(&context, "capability")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    #[derive(Deserialize)]
    struct EntitlementRow {
        entitlement_id: String,
        user_id: String,
        plugin_id: String,
        capability: String,
        expires_at: Option<i64>,
    }
    let row = worker::query!(
        &database,
        "SELECT entitlement_id, user_id, plugin_id, capability, expires_at
         FROM entitlements
         WHERE user_id = ?1 AND plugin_id = ?2 AND capability = ?3 AND status = 'active'
           AND (expires_at IS NULL OR expires_at > ?4)
         ORDER BY granted_at DESC LIMIT 1",
        &user_id,
        plugin_id,
        capability,
        now_seconds()
    )?
    .first::<EntitlementRow>(None)
    .await?;
    let entitlement = row.map(|row| Entitlement {
        entitlement_id: row.entitlement_id,
        user_id: row.user_id,
        plugin_id: row.plugin_id,
        capability: row.capability,
        status: EntitlementStatus::Active,
        expires_at: row.expires_at,
    });
    Response::from_json(&json!({"entitlement": entitlement}))
}

async fn delegated_plugin_token(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let delegated: DelegatedTokenRequest = request.json().await?;
    validate_delegated_request(&delegated)?;
    let now = now_seconds() as usize;
    let expires_at = now + 300;
    let claims = PluginAccessTokenClaims {
        iss: ACCESS_TOKEN_ISSUER.to_string(),
        sub: user_id,
        aud: format!("plugin:{}", delegated.plugin_id),
        scope: delegated.scopes,
        device_id: delegated.device_id,
        jti: Uuid::new_v4().to_string(),
        iat: now,
        exp: expires_at,
        token_use: "plugin".to_string(),
    };
    let private_key = context
        .env
        .secret("PLUGIN_TOKEN_PRIVATE_KEY_PEM")?
        .to_string();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = context
        .env
        .var("PLUGIN_TOKEN_KEY_ID")
        .ok()
        .map(|value| value.to_string());
    let key = EncodingKey::from_rsa_pem(private_key.as_bytes()).map_err(jwt_error)?;
    let token = encode(&header, &claims, &key).map_err(jwt_error)?;
    Response::from_json(&json!({
        "accessToken": token,
        "tokenType": "Bearer",
        "expiresIn": 300,
        "expiresAt": expires_at,
    }))
}

fn validate_delegated_request(request: &DelegatedTokenRequest) -> Result<()> {
    if !is_identifier(&request.plugin_id) {
        return Err(worker::Error::RustError(
            "invalid delegated plugin id".into(),
        ));
    }
    if request.device_id.trim().is_empty() || request.device_id.len() > 128 {
        return Err(worker::Error::RustError(
            "invalid delegated device id".into(),
        ));
    }
    if request.scopes.len() > 32
        || request
            .scopes
            .iter()
            .any(|scope| scope.len() > 96 || !is_scope(scope))
    {
        return Err(worker::Error::RustError(
            "invalid delegated token scopes".into(),
        ));
    }
    Ok(())
}

async fn purchases(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    purchases_response(&context.env, &user_id).await
}

async fn purchases_restore(request: Request, context: RouteContext<()>) -> Result<Response> {
    if request.method() != Method::Post {
        return error_response(405, "method_not_allowed", "POST required");
    }
    let user_id = authenticated_user(&request, &context.env)?;
    purchases_response(&context.env, &user_id).await
}

async fn purchases_response(env: &Env, user_id: &str) -> Result<Response> {
    let database = env.d1(DATABASE_BINDING)?;
    let rows = worker::query!(
        &database,
        "SELECT order_id, plugin_id, sku, currency, amount, status, created_at
         FROM orders WHERE buyer_user_id = ?1 ORDER BY created_at DESC LIMIT 100",
        user_id
    )?
    .all()
    .await?
    .results::<OrderRow>()?;
    Response::from_json(&json!({"purchases": rows, "nextCursor": null}))
}

async fn active_price(
    database: &worker::D1Database,
    plugin_id: &str,
    sku: &str,
    now: i64,
) -> Result<Option<PriceRow>> {
    worker::query!(
        database,
        "SELECT p.product_id, pr.price_id, p.plugin_id, p.sku,
                p.entitlement_capability AS capability, pr.currency, pr.amount
         FROM products p JOIN prices pr ON pr.product_id = p.product_id
         WHERE p.plugin_id = ?1 AND p.sku = ?2 AND p.active = 1 AND pr.active = 1
           AND pr.starts_at <= ?3 AND (pr.ends_at IS NULL OR pr.ends_at > ?3)
         LIMIT 1",
        plugin_id,
        sku,
        now
    )?
    .first::<PriceRow>(None)
    .await
}

async fn order_by_idempotency(
    database: &worker::D1Database,
    user_id: &str,
    idempotency_key: &str,
) -> Result<Option<OrderRow>> {
    worker::query!(
        database,
        "SELECT order_id, plugin_id, sku, currency, amount, status, created_at
         FROM orders WHERE buyer_user_id = ?1 AND idempotency_key = ?2",
        user_id,
        idempotency_key
    )?
    .first::<OrderRow>(None)
    .await
}

async fn entitlement_for_order(
    database: &worker::D1Database,
    order_id: &str,
) -> Result<Option<Entitlement>> {
    #[derive(Deserialize)]
    struct EntitlementRow {
        entitlement_id: String,
        user_id: String,
        plugin_id: String,
        capability: String,
        expires_at: Option<i64>,
    }
    Ok(worker::query!(
        database,
        "SELECT entitlement_id, user_id, plugin_id, capability, expires_at
         FROM entitlements WHERE order_id = ?1 AND status = 'active' LIMIT 1",
        order_id
    )?
    .first::<EntitlementRow>(None)
    .await?
    .map(|row| Entitlement {
        entitlement_id: row.entitlement_id,
        user_id: row.user_id,
        plugin_id: row.plugin_id,
        capability: row.capability,
        status: EntitlementStatus::Active,
        expires_at: row.expires_at,
    }))
}

async fn current_usage_status(
    database: &worker::D1Database,
    env: &Env,
    user_id: &str,
    now: i64,
) -> Result<AccountUsageStatus> {
    let window_start = usage_window_start(now);
    let window_end = window_start + USAGE_WINDOW_SECONDS;
    let row = worker::query!(
        database,
        "SELECT window_start, window_end, token_limit, used_tokens, reserved_tokens
         FROM ai_usage_budgets WHERE user_id = ?1 AND window_start = ?2",
        user_id,
        window_start
    )?
    .first::<UsageBudgetRow>(None)
    .await?;
    let (token_limit, used_tokens, reserved_tokens) = match row {
        Some(row) => {
            debug_assert_eq!(row.window_start, window_start);
            debug_assert_eq!(row.window_end, window_end);
            (row.token_limit, row.used_tokens, row.reserved_tokens)
        }
        None => (default_usage_limit(env)?, 0, 0),
    };
    Ok(AccountUsageStatus {
        window_start,
        window_end,
        token_limit,
        used_tokens,
        reserved_tokens,
        remaining_tokens: token_limit
            .saturating_sub(used_tokens)
            .saturating_sub(reserved_tokens),
    })
}

async fn usage_reservation_by_request(
    database: &worker::D1Database,
    user_id: &str,
    request_id: &str,
) -> Result<Option<UsageReservationRow>> {
    worker::query!(
        database,
        "SELECT reservation_id, request_id, reserved_tokens, expires_at, state
         FROM ai_usage_reservations WHERE user_id = ?1 AND request_id = ?2",
        user_id,
        request_id
    )?
    .first::<UsageReservationRow>(None)
    .await
}

async fn usage_reservation_by_id(
    database: &worker::D1Database,
    user_id: &str,
    reservation_id: &str,
) -> Result<Option<UsageReservationRow>> {
    worker::query!(
        database,
        "SELECT reservation_id, request_id, reserved_tokens, expires_at, state
         FROM ai_usage_reservations WHERE user_id = ?1 AND reservation_id = ?2",
        user_id,
        reservation_id
    )?
    .first::<UsageReservationRow>(None)
    .await
}

async fn usage_event_by_response(
    database: &worker::D1Database,
    provider_response_id: &str,
) -> Result<Option<UsageEventRow>> {
    worker::query!(
        database,
        "SELECT reservation_id FROM ai_usage_events WHERE provider_response_id = ?1",
        provider_response_id
    )?
    .first::<UsageEventRow>(None)
    .await
}

async fn expire_usage_reservations(
    database: &worker::D1Database,
    user_id: &str,
    now: i64,
) -> Result<()> {
    database
        .batch(vec![
            worker::query!(
                database,
                "UPDATE ai_usage_budgets
                 SET reserved_tokens = reserved_tokens - COALESCE((
                         SELECT SUM(r.reserved_tokens) FROM ai_usage_reservations r
                         WHERE r.user_id = ?1 AND r.window_start = ai_usage_budgets.window_start
                           AND r.state = 'reserved' AND r.expires_at <= ?2
                     ), 0),
                     updated_at = ?2
                 WHERE user_id = ?1
                   AND EXISTS (
                       SELECT 1 FROM ai_usage_reservations r
                       WHERE r.user_id = ?1 AND r.window_start = ai_usage_budgets.window_start
                         AND r.state = 'reserved' AND r.expires_at <= ?2
                   )",
                user_id,
                now
            )?,
            worker::query!(
                database,
                "UPDATE ai_usage_reservations SET state = 'expired', updated_at = ?1
                 WHERE user_id = ?2 AND state = 'reserved' AND expires_at <= ?1
                   AND EXISTS (
                       SELECT 1 FROM ai_usage_budgets b
                       WHERE b.user_id = ?2 AND b.window_start = ai_usage_reservations.window_start
                   )",
                now,
                user_id
            )?,
        ])
        .await?;
    Ok(())
}

fn d1_changes(result: Option<&worker::D1Result>) -> usize {
    result
        .and_then(|result| result.meta().ok().flatten())
        .and_then(|meta| meta.changes)
        .unwrap_or_default()
}

fn usage_window_start(now: i64) -> i64 {
    now - now.rem_euclid(USAGE_WINDOW_SECONDS)
}

fn default_usage_limit(env: &Env) -> Result<i64> {
    let value = env.var("DEFAULT_AI_TOKEN_LIMIT")?.to_string();
    value
        .parse::<i64>()
        .ok()
        .filter(|limit| *limit >= 0)
        .ok_or_else(|| worker::Error::RustError("DEFAULT_AI_TOKEN_LIMIT is invalid".into()))
}

fn require_model_gateway(request: &Request, env: &Env) -> Result<()> {
    let supplied = request
        .headers()
        .get("X-Mahayana-Model-Gateway")?
        .ok_or_else(|| worker::Error::RustError("missing model gateway credential".into()))?;
    let expected = env.secret("MODEL_GATEWAY_TOKEN")?.to_string();
    if !constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
        return Err(worker::Error::RustError(
            "invalid model gateway credential".into(),
        ));
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn usage_limit_response(status: &AccountUsageStatus) -> Result<Response> {
    Ok(Response::from_json(&json!({
        "error": {
            "type": "usage_limit_reached",
            "message": "Mahayana model token limit reached",
            "resets_at": status.window_end,
        },
        "usage": status,
    }))?
    .with_status(429))
}

async fn lookup_login_user(
    database: &worker::D1Database,
    identifier: &str,
) -> Result<Option<AccountUserRow>> {
    let select = "SELECT u.id, u.user_no, u.username, u.username_changed_at, u.email,
                         u.nickname, u.avatar, u.phone_number, u.firebase_uid,
                         u.alipay_user_id, u.alipay_nickname, u.alipay_avatar,
                         u.wechat_headimgurl, u.password_hash, u.salt, u.iterations, u.algo,
                         c.password_phc AS upgraded_password_phc,
                         u.main_practice_title, u.main_practice_file_path,
                         u.main_practice_selected_at, u.created_at, u.email_verified,
                         u.membership_type, u.membership_expires_at, u.free_trial_end_date
                  FROM users u
                  LEFT JOIN account_password_credentials c ON c.user_id = CAST(u.id AS TEXT)";
    let (where_clause, normalized) = if identifier.contains('@') {
        ("LOWER(u.email) = ?1", identifier.to_ascii_lowercase())
    } else if looks_like_phone(identifier) {
        ("u.phone_number = ?1", identifier.to_string())
    } else {
        ("u.username = ?1", identifier.to_string())
    };
    let query = format!("{select} WHERE {where_clause} LIMIT 1");
    worker::query!(database, &query, normalized)?
        .first::<AccountUserRow>(None)
        .await
}

async fn lookup_account_user_by_id(
    database: &worker::D1Database,
    user_id: &str,
) -> Result<Option<AccountUserRow>> {
    worker::query!(
        database,
        "SELECT u.id, u.user_no, u.username, u.username_changed_at, u.email,
                u.nickname, u.avatar, u.phone_number, u.firebase_uid,
                u.alipay_user_id, u.alipay_nickname, u.alipay_avatar,
                u.wechat_headimgurl, u.password_hash, u.salt, u.iterations, u.algo,
                c.password_phc AS upgraded_password_phc,
                u.main_practice_title, u.main_practice_file_path,
                u.main_practice_selected_at, u.created_at, u.email_verified,
                u.membership_type, u.membership_expires_at, u.free_trial_end_date
         FROM users u
         LEFT JOIN account_password_credentials c ON c.user_id = CAST(u.id AS TEXT)
         WHERE CAST(u.id AS TEXT) = ?1 OR u.username = ?1
         LIMIT 1",
        user_id
    )?
    .first::<AccountUserRow>(None)
    .await
}

fn looks_like_phone(value: &str) -> bool {
    let value = value.strip_prefix('+').unwrap_or(value);
    (6..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn normalize_device_id(device_id: Option<&str>) -> Result<String> {
    let device_id = device_id.map(str::trim).filter(|value| !value.is_empty());
    let Some(device_id) = device_id else {
        return Ok(format!("device:{}", Uuid::new_v4()));
    };
    if device_id.len() > 128
        || !device_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(worker::Error::RustError("invalid device id".into()));
    }
    Ok(device_id.to_string())
}

fn issue_account_access_token(
    env: &Env,
    user_id: &str,
    device_id: &str,
    session_id: &str,
    now: i64,
) -> Result<(String, i64, String)> {
    let expires_at = now + ACCESS_TOKEN_SECONDS;
    let jti = Uuid::new_v4().to_string();
    let claims = AccountAccessTokenClaims {
        iss: ACCESS_TOKEN_ISSUER.to_string(),
        sub: user_id.to_string(),
        aud: ACCESS_TOKEN_AUDIENCE.to_string(),
        scope: vec![
            "account.read".to_string(),
            "marketplace.read".to_string(),
            "marketplace.publish".to_string(),
            "wallet.read".to_string(),
            "commerce.purchase".to_string(),
            "model.invoke".to_string(),
        ],
        device_id: device_id.to_string(),
        sid: session_id.to_string(),
        jti: jti.clone(),
        iat: usize::try_from(now).unwrap_or_default(),
        exp: usize::try_from(expires_at).unwrap_or(usize::MAX),
        token_use: "access".to_string(),
    };
    let private_key = env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
    let key = EncodingKey::from_rsa_pem(private_key.as_bytes()).map_err(jwt_error)?;
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("JWT".to_string());
    header.kid = Some(env.var("ACCESS_TOKEN_KEY_ID")?.to_string());
    let token = encode(&header, &claims, &key).map_err(jwt_error)?;
    Ok((token, expires_at, jti))
}

fn serialize_account_user(user: &AccountUserRow) -> serde_json::Value {
    let avatar = user
        .avatar
        .as_ref()
        .or(user.alipay_avatar.as_ref())
        .or(user.wechat_headimgurl.as_ref());
    let main_practice = user.main_practice_title.as_ref().map(|title| {
        json!({
            "title": title,
            "filePath": user.main_practice_file_path,
            "selectedAt": user.main_practice_selected_at,
        })
    });
    json!({
        "id": user.id,
        "userId": user.id,
        "userNo": user.user_no.unwrap_or(user.id),
        "username": user.username,
        "usernameChangedAt": user.username_changed_at,
        "email": user.email.as_deref().unwrap_or_default(),
        "nickname": user.nickname.as_deref().unwrap_or(&user.username),
        "avatar": avatar,
        "phoneNumber": user.phone_number,
        "firebaseUid": user.firebase_uid,
        "alipayProviderSubject": user.alipay_user_id,
        "alipayUserId": user.alipay_user_id,
        "alipayNickname": user.alipay_nickname,
        "alipayAvatar": user.alipay_avatar,
        "hasPassword": user.password_hash.is_some() && user.salt.is_some(),
        "mainPractice": main_practice,
        "createdAt": user.created_at,
        "emailVerified": user.email_verified == Some(1),
        "membership": {
            "type": user.membership_type.as_deref().unwrap_or("expired"),
            "expiresAt": user.membership_expires_at.as_ref().or(user.free_trial_end_date.as_ref()),
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn account_session_response(
    user: &AccountUserRow,
    access_token: &str,
    refresh_token: &str,
    access_expires_at: i64,
    refresh_expires_at: i64,
    session_id: &str,
    device_id: &str,
) -> Result<Response> {
    Ok(Response::from_json(&json!({
        "accessToken": access_token,
        "refreshToken": refresh_token,
        "tokenType": "Bearer",
        "expiresIn": ACCESS_TOKEN_SECONDS,
        "accessTokenExpiresAt": access_expires_at,
        "refreshTokenExpiresAt": refresh_expires_at,
        "sessionId": session_id,
        "deviceId": device_id,
        "username": user.username,
        "userId": user.id,
        "userNo": user.user_no.unwrap_or(user.id),
        "user": serialize_account_user(user),
    }))?
    .with_headers(auth_headers()))
}

async fn record_auth_event(
    database: &worker::D1Database,
    user_id: Option<&str>,
    session_id: Option<&str>,
    event_type: &str,
    now: i64,
) -> Result<()> {
    worker::query!(
        database,
        "INSERT INTO account_auth_events
         (event_id, user_id, session_id, event_type, occurred_at, details_json)
         VALUES (?1, ?2, ?3, ?4, ?5, '{}')",
        Uuid::new_v4().to_string(),
        user_id,
        session_id,
        event_type,
        now
    )?
    .run()
    .await?;
    Ok(())
}

async fn account_login_is_rate_limited(
    database: &worker::D1Database,
    user_id: &str,
    now: i64,
) -> Result<bool> {
    let window_start = now - LOGIN_FAILURE_WINDOW_SECONDS;
    let count = worker::query!(
        database,
        "SELECT COUNT(*) AS failure_count
         FROM account_auth_events
         WHERE user_id = ?1 AND event_type = 'login_failed' AND occurred_at >= ?2",
        user_id,
        window_start
    )?
    .first::<LoginFailureCountRow>(None)
    .await?
    .map(|row| row.failure_count)
    .unwrap_or_default();
    Ok(count >= MAX_ACCOUNT_LOGIN_FAILURES)
}

async fn revoke_account_session(
    database: &worker::D1Database,
    session_id: &str,
    reason: &str,
    now: i64,
) -> Result<()> {
    let event_id = Uuid::new_v4().to_string();
    database
        .batch(vec![
            worker::query!(
                database,
                "UPDATE account_sessions
                 SET revoked_at = COALESCE(revoked_at, ?1), revoked_reason = COALESCE(revoked_reason, ?2)
                 WHERE session_id = ?3",
                now,
                reason,
                session_id
            )?,
            worker::query!(
                database,
                "UPDATE account_refresh_tokens SET state = 'revoked'
                 WHERE session_id = ?1 AND state = 'active'",
                session_id
            )?,
            worker::query!(
                database,
                "INSERT INTO account_auth_events
                 (event_id, user_id, session_id, event_type, occurred_at, details_json)
                 SELECT ?1, user_id, session_id, ?2, ?3, '{}'
                 FROM account_sessions WHERE session_id = ?4",
                &event_id,
                reason,
                now,
                session_id
            )?,
        ])
        .await?;
    Ok(())
}

fn authenticated_user(request: &Request, env: &Env) -> Result<String> {
    Ok(authenticated_account(request, env)?.user_id)
}

fn authenticated_account(request: &Request, env: &Env) -> Result<AuthenticatedAccount> {
    let authorization = request
        .headers()
        .get("Authorization")?
        .ok_or_else(|| worker::Error::RustError("missing Authorization header".into()))?;
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| worker::Error::RustError("invalid Authorization scheme".into()))?;
    if let Ok(expected) = env.secret("TEST_ACCOUNT_TOKEN")
        && constant_time_eq(token.as_bytes(), expected.to_string().as_bytes())
    {
        return Ok(AuthenticatedAccount {
            user_id: "user:test_account".to_string(),
            session_id: None,
            scopes: vec![
                "marketplace.read".to_string(),
                "marketplace.publish".to_string(),
                "model.invoke".to_string(),
            ],
            is_test_account: true,
        });
    }
    let public_key = env.secret("ACCESS_TOKEN_PUBLIC_KEY_PEM")?.to_string();
    let key = DecodingKey::from_rsa_pem(public_key.as_bytes()).map_err(jwt_error)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ACCESS_TOKEN_ISSUER]);
    validation.set_audience(&[ACCESS_TOKEN_AUDIENCE]);
    let claims = decode::<AccountAccessTokenClaims>(token, &key, &validation)
        .map_err(jwt_error)?
        .claims;
    if claims.token_use != "access"
        || claims.sub.trim().is_empty()
        || claims.sid.trim().is_empty()
        || claims.device_id.trim().is_empty()
    {
        return Err(worker::Error::RustError(
            "invalid access token claims".into(),
        ));
    }
    Ok(AuthenticatedAccount {
        user_id: claims.sub,
        session_id: Some(claims.sid),
        scopes: claims.scope,
        is_test_account: false,
    })
}

fn marketplace_asset_url(deployment_url: &str, asset_path: &str) -> Result<String> {
    if !is_public_https_url(deployment_url) {
        return Err(worker::Error::RustError(
            "invalid marketplace deployment URL".into(),
        ));
    }
    let mut url = Url::parse(deployment_url)
        .map_err(|_| worker::Error::RustError("invalid marketplace deployment URL".into()))?;
    let mut path = url.path().trim_end_matches('/').to_string();
    path.push_str(asset_path);
    url.set_path(&path);
    Ok(url.to_string())
}

async fn fetch_marketplace_asset(
    deployment_url: &str,
    asset_path: &str,
    max_bytes: usize,
) -> std::result::Result<Vec<u8>, String> {
    let url =
        marketplace_asset_url(deployment_url, asset_path).map_err(|error| error.to_string())?;
    let parsed = url
        .parse()
        .map_err(|error| format!("invalid Cloudflare plugin asset URL: {error}"))?;
    let mut response = Fetch::Url(parsed)
        .send()
        .await
        .map_err(|error| format!("failed to fetch Cloudflare plugin asset: {error}"))?;
    if !(200..300).contains(&response.status_code()) {
        return Err(format!(
            "Cloudflare plugin asset returned HTTP {}.",
            response.status_code()
        ));
    }
    if response
        .headers()
        .get("Content-Length")
        .map_err(|error| error.to_string())?
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > max_bytes)
    {
        return Err("Cloudflare plugin asset exceeds the approved size limit.".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("failed to read Cloudflare plugin asset: {error}"))?;
    if bytes.len() > max_bytes {
        return Err("Cloudflare plugin asset exceeds the approved size limit.".into());
    }
    Ok(bytes)
}

async fn fetch_verified_marketplace_package(
    deployment_url: &str,
    expected_sha256: &str,
    expected_size: usize,
) -> std::result::Result<Vec<u8>, String> {
    let package =
        fetch_marketplace_asset(deployment_url, "/mahayana/plugin.tar.gz", expected_size).await?;
    if package.len() != expected_size {
        return Err("Cloudflare plugin package size does not match approved metadata.".into());
    }
    let actual_sha256 = format!("{:x}", Sha256::digest(&package));
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err("Cloudflare plugin package SHA-256 does not match approved metadata.".into());
    }
    Ok(package)
}

async fn verified_marketplace_site_package_with_retry(
    deployment_url: &str,
    plugin_id: &str,
    version: &str,
    expected_sha256: &str,
    expected_size: usize,
    expected_source: &Value,
    expected_release_manifest: &Value,
    expected_release_manifest_sha256: &str,
) -> std::result::Result<Vec<u8>, String> {
    let mut last_error = "Cloudflare plugin deployment is not available yet.".to_string();
    for attempt in 1..=MARKETPLACE_DEPLOYMENT_VERIFY_ATTEMPTS {
        match verified_marketplace_site_package(
            deployment_url,
            plugin_id,
            version,
            expected_sha256,
            expected_size,
            expected_source,
            expected_release_manifest,
            expected_release_manifest_sha256,
        )
        .await
        {
            Ok(package) => return Ok(package),
            Err(error) => last_error = error,
        }
        if attempt < MARKETPLACE_DEPLOYMENT_VERIFY_ATTEMPTS {
            Delay::from(Duration::from_secs(
                MARKETPLACE_DEPLOYMENT_VERIFY_DELAY_SECONDS,
            ))
            .await;
        }
    }
    Err(format!(
        "Cloudflare plugin deployment verification failed after {MARKETPLACE_DEPLOYMENT_VERIFY_ATTEMPTS} attempts: {last_error}"
    ))
}

async fn verified_marketplace_site_package(
    deployment_url: &str,
    plugin_id: &str,
    version: &str,
    expected_sha256: &str,
    expected_size: usize,
    expected_source: &Value,
    expected_release_manifest: &Value,
    expected_release_manifest_sha256: &str,
) -> std::result::Result<Vec<u8>, String> {
    let manifest_bytes =
        fetch_marketplace_asset(deployment_url, "/mahayana/plugin.json", 64 * 1024).await?;
    let manifest: MarketplaceSiteManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid Cloudflare plugin manifest: {error}"))?;
    let mut mismatched_fields = Vec::new();
    if manifest.schema_version != 2 {
        mismatched_fields.push("schemaVersion");
    }
    if manifest.plugin_id != plugin_id {
        mismatched_fields.push("pluginId");
    }
    if manifest.version != version {
        mismatched_fields.push("version");
    }
    if manifest.package_path != "/mahayana/plugin.tar.gz" {
        mismatched_fields.push("packagePath");
    }
    if !manifest
        .package_sha256
        .eq_ignore_ascii_case(expected_sha256)
    {
        mismatched_fields.push("packageSha256");
    }
    if manifest.package_size != expected_size {
        mismatched_fields.push("packageSize");
    }
    if manifest.runtime != "independent-worker-or-pages" {
        mismatched_fields.push("runtime");
    }
    if manifest.source != *expected_source {
        mismatched_fields.push("source");
    }
    if manifest.release_manifest_path != "/mahayana/release-manifest.json" {
        mismatched_fields.push("releaseManifestPath");
    }
    if !manifest
        .release_manifest_sha256
        .eq_ignore_ascii_case(expected_release_manifest_sha256)
    {
        mismatched_fields.push("releaseManifestSha256");
    }
    if !mismatched_fields.is_empty() {
        return Err(format!(
            "Cloudflare plugin manifest fields do not match the release request: {}.",
            mismatched_fields.join(", ")
        ));
    }
    let release_manifest_bytes = fetch_marketplace_asset(
        deployment_url,
        "/mahayana/release-manifest.json",
        256 * 1024,
    )
    .await?;
    let actual_release_manifest_sha256 = format!("{:x}", Sha256::digest(&release_manifest_bytes));
    if !actual_release_manifest_sha256.eq_ignore_ascii_case(expected_release_manifest_sha256) {
        return Err(
            "Cloudflare release manifest SHA-256 does not match the release request.".into(),
        );
    }
    let deployed_release_manifest = serde_json::from_slice::<Value>(&release_manifest_bytes)
        .map_err(|error| format!("invalid Cloudflare release manifest: {error}"))?;
    if deployed_release_manifest != *expected_release_manifest {
        return Err("Cloudflare release manifest differs from the release request.".into());
    }
    fetch_verified_marketplace_package(deployment_url, expected_sha256, expected_size).await
}

fn json_string<'a>(value: &'a Value, key: &str) -> std::result::Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn is_git_object_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_external_release_manifest(
    manifest: &Value,
    plugin_id: &str,
    version: &str,
    platforms: &[String],
    max_artifact_bytes: usize,
) -> std::result::Result<(), String> {
    if manifest.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || manifest.get("protocol").and_then(Value::as_str)
            != Some("mahayana.external-release.v1")
        || manifest.get("pluginId").and_then(Value::as_str) != Some(plugin_id)
        || manifest.get("version").and_then(Value::as_str) != Some(version)
    {
        return Err("external release manifest identity/protocol does not match the request".into());
    }
    let artifacts = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .filter(|artifacts| !artifacts.is_empty())
        .ok_or_else(|| "external release manifest must contain at least one artifact".to_string())?;
    let mut ids = std::collections::HashSet::new();
    let mut covered = std::collections::HashSet::new();
    for artifact in artifacts {
        let id = json_string(artifact, "id")?;
        if !is_identifier(id) || !ids.insert(id) {
            return Err("external artifact ids must be unique normalized identifiers".into());
        }
        let runtime = json_string(artifact, "runtime")?;
        if runtime.is_empty() || runtime.len() > 64 {
            return Err("external artifact runtime is invalid".into());
        }
        let format = json_string(artifact, "format")?;
        if !matches!(format, "tar-gz" | "zip") {
            return Err("external artifact format must be tar-gz or zip".into());
        }
        let sha256 = json_string(artifact, "sha256")?;
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("external artifact sha256 must be a 64-character hexadecimal digest".into());
        }
        let size = artifact
            .get("size")
            .and_then(Value::as_u64)
            .and_then(|size| usize::try_from(size).ok())
            .filter(|size| *size > 0 && *size <= max_artifact_bytes)
            .ok_or_else(|| "external artifact size is invalid".to_string())?;
        let _ = size;
        let artifact_platforms = artifact
            .get("platforms")
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty())
            .ok_or_else(|| "external artifact platforms are required".to_string())?;
        for platform in artifact_platforms.iter().filter_map(Value::as_str) {
            if !matches!(
                platform,
                "cli" | "desktop" | "mobile" | "web" | "ios" | "android"
            ) {
                return Err(format!("unsupported external artifact platform {platform}"));
            }
            covered.insert(platform.to_string());
        }
        validate_external_artifact_source(
            artifact
                .get("source")
                .ok_or_else(|| "external artifact source is required".to_string())?,
        )?;
        if let Some(entry) = artifact.get("entry").and_then(Value::as_str) {
            if entry.is_empty()
                || entry.starts_with('/')
                || entry.split('/').any(|part| matches!(part, "" | "." | ".."))
            {
                return Err("external artifact entry must be repository-relative".into());
            }
        }
    }
    if platforms.iter().any(|platform| !covered.contains(platform)) {
        return Err("external artifacts do not cover every declared marketplace platform".into());
    }
    Ok(())
}

fn validate_external_artifact_source(source: &Value) -> std::result::Result<(), String> {
    match source.get("type").and_then(Value::as_str) {
        Some("https") => {
            let url = json_string(source, "url")?;
            if !is_external_https_url(url) {
                return Err("https artifact source must be a public HTTPS URL".into());
            }
        }
        Some("github-release") => {
            let repository = json_string(source, "repository")?;
            let parts = repository.split('/').collect::<Vec<_>>();
            if parts.len() != 2
                || parts.iter().any(|part| part.is_empty())
                || repository.bytes().any(|byte| {
                    !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
                })
            {
                return Err("GitHub release repository must be owner/name".into());
            }
            for key in ["tag", "asset"] {
                let value = json_string(source, key)?;
                if value.is_empty()
                    || value.len() > 255
                    || value.contains(['/', '\\', '\r', '\n'])
                    || matches!(value, "." | "..")
                {
                    return Err(format!("GitHub release {key} is invalid"));
                }
            }
        }
        Some("npm") => {
            let package = json_string(source, "package")?;
            let version = json_string(source, "version")?;
            if package.is_empty()
                || package.len() > 214
                || package.contains(char::is_whitespace)
                || package.contains("..")
                || version.is_empty()
                || version.len() > 128
            {
                return Err("npm package/version is invalid".into());
            }
            if let Some(registry) = source.get("registry").and_then(Value::as_str) {
                if !is_external_https_url(registry) {
                    return Err("npm registry must be HTTPS".into());
                }
            }
        }
        _ => return Err("artifact source type must be https, github-release, or npm".into()),
    }
    Ok(())
}

fn is_external_https_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

async fn resolve_external_artifact_url(artifact: &Value) -> std::result::Result<String, String> {
    let source = artifact
        .get("source")
        .ok_or_else(|| "artifact source is required".to_string())?;
    validate_external_artifact_source(source)?;
    match source.get("type").and_then(Value::as_str) {
        Some("https") => Ok(json_string(source, "url")?.to_string()),
        Some("github-release") => Ok(format!(
            "https://github.com/{}/releases/download/{}/{}",
            json_string(source, "repository")?,
            json_string(source, "tag")?,
            json_string(source, "asset")?
        )),
        Some("npm") => {
            let package = json_string(source, "package")?;
            let version = json_string(source, "version")?;
            let registry = source
                .get("registry")
                .and_then(Value::as_str)
                .unwrap_or("https://registry.npmjs.org");
            let metadata_url = format!(
                "{}/{}{}{}",
                registry.trim_end_matches('/'),
                package.replace('/', "%2f"),
                "/",
                version
            );
            let parsed = metadata_url
                .parse()
                .map_err(|error| format!("invalid npm metadata URL: {error}"))?;
            let mut response = Fetch::Url(parsed)
                .send()
                .await
                .map_err(|error| format!("failed to fetch npm package metadata: {error}"))?;
            if !(200..300).contains(&response.status_code()) {
                return Err(format!(
                    "npm registry returned HTTP {}",
                    response.status_code()
                ));
            }
            let metadata: Value = response
                .json()
                .await
                .map_err(|error| format!("invalid npm metadata response: {error}"))?;
            let tarball = metadata
                .pointer("/dist/tarball")
                .and_then(Value::as_str)
                .ok_or_else(|| "npm dist.tarball is missing".to_string())?;
            if !is_external_https_url(tarball) {
                return Err("npm dist.tarball is not a public HTTPS URL".into());
            }
            Ok(tarball.to_string())
        }
        _ => Err("unsupported external artifact source".into()),
    }
}

async fn verify_external_artifact(
    url: &str,
    expected_sha256: &str,
    expected_size: usize,
    max_bytes: usize,
) -> std::result::Result<(), String> {
    if !is_external_https_url(url) || expected_size == 0 || expected_size > max_bytes {
        return Err("external artifact URL or size is invalid".into());
    }
    let parsed = url
        .parse()
        .map_err(|error| format!("invalid external artifact URL: {error}"))?;
    let mut response = Fetch::Url(parsed)
        .send()
        .await
        .map_err(|error| format!("failed to fetch external artifact: {error}"))?;
    if !(200..300).contains(&response.status_code()) {
        return Err(format!(
            "external artifact returned HTTP {}",
            response.status_code()
        ));
    }
    if response
        .headers()
        .get("Content-Length")
        .map_err(|error| error.to_string())?
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length != expected_size || length > max_bytes)
    {
        return Err("external artifact Content-Length does not match approved metadata".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("failed to read external artifact: {error}"))?;
    if bytes.len() != expected_size || bytes.len() > max_bytes {
        return Err("external artifact size does not match approved metadata".into());
    }
    let actual_sha256 = format!("{:x}", Sha256::digest(&bytes));
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err("external artifact SHA-256 does not match approved metadata".into());
    }
    Ok(())
}

fn validate_github_source_identity(source: &Value) -> std::result::Result<(), String> {
    if source.get("provider").and_then(Value::as_str) != Some("github") {
        return Err("source.provider must be github".into());
    }
    let repository = json_string(source, "repository")?;
    let parts = repository.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|part| part.is_empty())
        || repository.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        })
    {
        return Err("source.repository must be a normalized GitHub owner/name".into());
    }
    if source
        .get("repositoryId")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .is_none()
    {
        return Err("source.repositoryId must be a positive GitHub repository ID".into());
    }
    let default_branch = json_string(source, "defaultBranch")?;
    if default_branch
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')))
    {
        return Err("source.defaultBranch is invalid".into());
    }
    let commit = json_string(source, "commit")?;
    let tree_hash = json_string(source, "treeHash")?;
    if !is_git_object_id(commit) || !is_git_object_id(tree_hash) {
        return Err("source commit and treeHash must be 40-character Git object IDs".into());
    }
    let license = json_string(source, "license")?;
    if license
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-')))
    {
        return Err("source.license must be an SPDX identifier".into());
    }
    if source.get("visibility").and_then(Value::as_str) != Some("public") {
        return Err("source.visibility must be public for public marketplace releases".into());
    }
    let subdirectory = json_string(source, "subdirectory")?;
    if subdirectory.starts_with('/')
        || (subdirectory != "."
            && subdirectory
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | "..")))
    {
        return Err("source.subdirectory must be a normalized repository-relative path".into());
    }
    Ok(())
}

fn validate_multi_artifact_release_manifest(
    manifest: &Value,
    plugin_id: &str,
    version: &str,
    package_sha256: &str,
    package_size: usize,
    platforms: &[String],
    source: &Value,
) -> std::result::Result<(), String> {
    if manifest.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || manifest.get("protocol").and_then(Value::as_str)
            != Some("mahayana.multi-artifact-release.v1")
        || manifest.get("pluginId").and_then(Value::as_str) != Some(plugin_id)
        || manifest.get("version").and_then(Value::as_str) != Some(version)
        || manifest.get("source") != Some(source)
    {
        return Err("release manifest identity does not match the release request".into());
    }
    let source_commit = json_string(source, "commit")?;
    let source_tree_hash = json_string(source, "treeHash")?;
    let common = manifest
        .get("common")
        .and_then(Value::as_object)
        .ok_or_else(|| "release manifest common artifact is required".to_string())?;
    if common.get("id").and_then(Value::as_str) != Some("common")
        || common.get("packagePath").and_then(Value::as_str) != Some("/mahayana/plugin.tar.gz")
        || !common
            .get("sha256")
            .and_then(Value::as_str)
            .is_some_and(|digest| digest.eq_ignore_ascii_case(package_sha256))
        || common.get("size").and_then(Value::as_u64) != Some(package_size as u64)
        || common.get("sourceCommit").and_then(Value::as_str) != Some(source_commit)
        || common.get("sourceTreeHash").and_then(Value::as_str) != Some(source_tree_hash)
    {
        return Err("release manifest common artifact is not source-bound".into());
    }
    let artifacts = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .filter(|artifacts| !artifacts.is_empty())
        .ok_or_else(|| "release manifest must contain at least one runtime artifact".to_string())?;
    for artifact in artifacts {
        if json_string(artifact, "id").is_err()
            || json_string(artifact, "runtime").is_err()
            || artifact.get("packagePath").and_then(Value::as_str)
                != Some("/mahayana/plugin.tar.gz")
            || !artifact
                .get("sha256")
                .and_then(Value::as_str)
                .is_some_and(|digest| digest.eq_ignore_ascii_case(package_sha256))
            || artifact.get("size").and_then(Value::as_u64) != Some(package_size as u64)
            || artifact.get("sourceCommit").and_then(Value::as_str) != Some(source_commit)
            || artifact.get("sourceTreeHash").and_then(Value::as_str) != Some(source_tree_hash)
        {
            return Err("release manifest runtime artifact is not source-bound".into());
        }
        let declared_platforms = artifact
            .get("platforms")
            .and_then(Value::as_array)
            .ok_or_else(|| "runtime artifact platforms are required".to_string())?;
        let declared_platforms = declared_platforms
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if platforms
            .iter()
            .any(|platform| !declared_platforms.contains(&platform.as_str()))
        {
            return Err("runtime artifact platforms do not cover the release platforms".into());
        }
    }
    Ok(())
}

fn route_identifier<'a>(context: &'a RouteContext<()>, name: &str) -> Result<&'a str> {
    let value = context
        .param(name)
        .map(String::as_str)
        .filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
        .ok_or_else(|| worker::Error::RustError(format!("invalid route parameter {name}")))?;
    Ok(value)
}

fn route_version(context: &RouteContext<()>) -> Result<&str> {
    context
        .param("version")
        .map(String::as_str)
        .filter(|value| is_version_identifier(value))
        .ok_or_else(|| worker::Error::RustError("invalid route parameter version".into()))
}

fn exact_nonnegative_i64(value: f64) -> Option<i64> {
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= i64::MAX as f64)
        .then_some(value as i64)
}

fn is_version_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
}

fn is_public_https_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    url.host_str()
        .map(str::to_ascii_lowercase)
        .is_some_and(|domain| domain.ends_with(".workers.dev") || domain.ends_with(".pages.dev"))
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_scope(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

fn is_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\'' | b'\\'))
}

fn now_seconds() -> i64 {
    (Date::now().as_millis() / 1_000) as i64
}

fn jwt_error(error: jsonwebtoken::errors::Error) -> worker::Error {
    worker::Error::RustError(format!("access token verification failed: {error}"))
}

fn json_headers() -> worker::Headers {
    let headers = worker::Headers::new();
    let _ = headers.set("Content-Type", "application/json; charset=utf-8");
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
    let _ = headers.set(
        "Access-Control-Allow-Headers",
        "Content-Type, Authorization, X-Mahayana-Model-Gateway",
    );
    headers
}

fn auth_headers() -> worker::Headers {
    let headers = json_headers();
    let _ = headers.set("Cache-Control", "no-store");
    let _ = headers.set("Pragma", "no-cache");
    headers
}

fn error_response(status: u16, code: &str, message: &str) -> Result<Response> {
    Ok(
        Response::from_json(&json!({"error": code, "message": message}))?
            .with_headers(auth_headers())
            .with_status(status),
    )
}
