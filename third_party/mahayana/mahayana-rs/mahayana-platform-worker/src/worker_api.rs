use crate::auth::hash_password_argon2id;
use crate::auth::hash_refresh_token;
use crate::auth::new_password_salt;
use crate::auth::new_refresh_token;
use crate::auth::verify_argon2id;
use crate::auth::verify_pbkdf2_sha256;
use crate::identity_auth::IdentityProviderConfig as OAuthProviderConfig;
use crate::identity_auth::PROVIDER_ORDER;
use crate::identity_auth::ProviderIdentityProfile as OAuthIdentityProfile;
use crate::identity_auth::build_authorization_url;
use crate::identity_auth::complete_provider;
use crate::identity_auth::configured_provider;
use crate::identity_auth::provider_available;
use crate::identity_auth::registration_email_available;
use crate::identity_auth::send_registration_code;
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
use worker::Method;
use worker::Request;
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
const UNLIMITED_AI_TOKEN_LIMIT: i64 = 9_007_199_254_740_991;
const BUILTIN_SUPER_ADMIN_ACCOUNT_IDS: &[&str] = &["22"];
const BUILTIN_UNLIMITED_ACCOUNT_IDS: &[&str] = &["197915874789377"];
const BUILTIN_UNLIMITED_ACCOUNT_USERNAMES: &[&str] = &["fabushi_mcp_ci_test"];
const MARKETPLACE_DEPLOYMENT_VERIFY_ATTEMPTS: usize = 6;
const MARKETPLACE_DEPLOYMENT_VERIFY_DELAY_SECONDS: u64 = 3;

fn is_builtin_super_admin_account_id(user_id: &str) -> bool {
    BUILTIN_SUPER_ADMIN_ACCOUNT_IDS.contains(&user_id.trim())
}

fn is_builtin_unlimited_account_id(user_id: &str) -> bool {
    BUILTIN_SUPER_ADMIN_ACCOUNT_IDS.contains(&user_id.trim())
        || BUILTIN_UNLIMITED_ACCOUNT_IDS.contains(&user_id.trim())
}

fn is_builtin_unlimited_account_username(username: &str) -> bool {
    BUILTIN_UNLIMITED_ACCOUNT_USERNAMES.contains(&username.trim())
}

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
struct MarketplaceInstalledPluginRow {
    plugin_id: String,
    display_name: String,
    description: String,
    latest_version: Option<String>,
    projection_json: Option<String>,
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
struct BrowserLoginProofRequest {
    poll_secret: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserLoginStartRequest {
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    platform: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserAttemptStatusRow {
    status: String,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct BrowserAttemptRow {
    attempt_id: String,
    provider: String,
    device_id: String,
    code_verifier: String,
    state_hash: String,
    status: String,
    expires_at: i64,
}

#[derive(Debug, Clone, Copy)]
struct PasswordAuthRejection {
    status: u16,
    code: &'static str,
    message: &'static str,
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

#[derive(Debug, Deserialize)]
struct OAuthIdentityRow {
    user_id: String,
}

#[derive(Debug, Deserialize)]
struct MaxUserIdRow {
    max_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RegistrationChallengeRow {
    attempt_id: String,
    code_hash: String,
    sent_at: i64,
    expires_at: i64,
    failed_attempts: i64,
    consumed_at: Option<i64>,
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

mod account;
mod ai_usage;
mod ci_runner;
mod commerce;
mod developer_commerce_proxy;
mod listener_relay;
mod marketplace;
mod remote_computer;
mod security;
mod user_payment_proxy;

use account::*;
use ai_usage::*;
use ci_runner::*;
use commerce::*;
use developer_commerce_proxy::*;
use listener_relay::*;
use marketplace::*;
use remote_computer::*;
use security::constant_time_eq;
use user_payment_proxy::*;

#[event(fetch, respond_with_errors)]
pub async fn main(request: Request, env: Env, _context: Context) -> Result<Response> {
    Router::new()
        .get("/health", |_, _| Response::from_json(&json!({"ok": true})))
        .post_async("/api/auth/login", password_login)
        .post_async("/api/auth/browser/start", browser_login_start)
        .get_async("/api/auth/browser/portal", browser_login_portal)
        .get_async("/api/auth/browser/authorize", browser_login_authorize)
        .post_async("/api/auth/browser/password", browser_login_password)
        .post_async("/api/auth/browser/register/code", browser_registration_code)
        .post_async("/api/auth/browser/register", browser_registration_complete)
        .post_async("/api/auth/browser/attempts/:attempt_id", browser_login_poll)
        .post_async(
            "/api/auth/browser/attempts/:attempt_id/cancel",
            browser_login_cancel,
        )
        .post_async(
            "/api/auth/browser/attempts/:attempt_id/reopen",
            browser_login_reopen,
        )
        .get_async("/api/auth/oauth/providers", oauth_providers)
        .post_async("/api/auth/oauth/start", oauth_start)
        .get_async("/api/auth/oauth/callback", oauth_callback)
        .post_async("/api/auth/oauth/callback", oauth_callback)
        .get_async("/api/auth/oauth/attempts/:attempt_id", oauth_poll)
        .post_async("/api/auth/refresh", refresh_access_token)
        .get_async("/api/auth/user-info", account_user_info)
        .post_async("/api/auth/logout", account_logout)
        .get("/v1/auth/jwks.json", |_, context| {
            let jwks = context.env.secret("ACCESS_TOKEN_JWKS")?.to_string();
            Ok(Response::ok(jwks)?.with_headers(json_headers()))
        })
        .post_async("/v1/auth/plugin-token", delegated_plugin_token)
        .post_async(
            "/v1/auth/plugin-token/introspect",
            delegated_plugin_token_introspect,
        )
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
        .post_async("/v1/ci/runner-session", ci_runner_session_create)
        .get_async("/v1/computers", remote_computer_list)
        .post_async("/v1/computers/register", remote_computer_register)
        .post_async("/v1/computers/heartbeat", remote_computer_heartbeat)
        .post_async("/v1/computers/pair", remote_computer_pair)
        .get_async("/v1/computers/:device_id/clients", remote_computer_clients)
        .post_async(
            "/v1/computers/:device_id/clients/:client_id/revoke",
            remote_computer_client_revoke,
        )
        .post_async(
            "/v1/computers/:device_id/sessions/list",
            remote_computer_sessions,
        )
        .post_async(
            "/v1/computers/:device_id/sessions",
            remote_computer_session_create,
        )
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
        .get_async("/v1/marketplace/added", marketplace_added)
        .post_async(
            "/v1/marketplace/plugins/:plugin_id/add",
            marketplace_plugin_add,
        )
        .post_async(
            "/v1/marketplace/plugins/:plugin_id/route",
            marketplace_plugin_route,
        )
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
        .get_async("/v1/developer/commerce/profile", developer_commerce_proxy)
        .post_async("/v1/developer/commerce/profile", developer_commerce_proxy)
        .get_async("/v1/developer/commerce/payout", developer_commerce_proxy)
        .post_async(
            "/v1/developer/commerce/payout/profile",
            developer_commerce_proxy,
        )
        .post_async(
            "/v1/developer/commerce/payout/request",
            developer_commerce_proxy,
        )
        .get_async("/v1/developer/commerce/miniapps", developer_commerce_proxy)
        .post_async(
            "/v1/developer/commerce/miniapps/:mini_app_id",
            developer_commerce_proxy,
        )
        .get_async(
            "/v1/developer/commerce/miniapps/:mini_app_id/products",
            developer_commerce_proxy,
        )
        .post_async(
            "/v1/developer/commerce/miniapps/:mini_app_id/products",
            developer_commerce_proxy,
        )
        .post_async(
            "/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id",
            developer_commerce_proxy,
        )
        .post_async(
            "/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id/google/sync",
            developer_commerce_proxy,
        )
        .post_async(
            "/v1/pay/intents/:payment_id/apple/advanced-commerce",
            developer_commerce_proxy,
        )
        .post_async("/v1/miniapps/:mini_app_id/pay/intents", user_payment_proxy)
        .get_async("/v1/pay/intents/:payment_id", user_payment_proxy)
        .post_async("/v1/pay/intents/:payment_id/checkout", user_payment_proxy)
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
        || manifest.get("protocol").and_then(Value::as_str) != Some("mahayana.external-release.v1")
        || manifest.get("pluginId").and_then(Value::as_str) != Some(plugin_id)
        || manifest.get("version").and_then(Value::as_str) != Some(version)
    {
        return Err(
            "external release manifest identity/protocol does not match the request".into(),
        );
    }
    let artifacts = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .filter(|artifacts| !artifacts.is_empty())
        .ok_or_else(|| {
            "external release manifest must contain at least one artifact".to_string()
        })?;
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
            return Err(
                "external artifact sha256 must be a 64-character hexadecimal digest".into(),
            );
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
