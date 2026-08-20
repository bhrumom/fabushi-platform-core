use base64::Engine;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::Validation;
use jsonwebtoken::decode;
use jsonwebtoken::decode_header;
use jsonwebtoken::jwk::JwkSet;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use url::Url;
use worker::Env;
use worker::Fetch;
use worker::Headers;
use worker::Method;
use worker::Request;
use worker::RequestInit;
use worker::Result;

pub(crate) const PROVIDER_ORDER: [&str; 6] = [
    "apple",
    "alipay",
    "google",
    "microsoft",
    "github",
    "cloudflare",
];

const APPLE_ISSUER: &str = "https://appleid.apple.com";
const APPLE_AUTHORIZE_URL: &str = "https://appleid.apple.com/auth/authorize";
const APPLE_TOKEN_URL: &str = "https://appleid.apple.com/auth/token";
const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";
const CLOUDFLARE_API_USER_URL: &str = "https://api.cloudflare.com/client/v4/user";
const BRIDGE_HEADER: &str = "X-Fabushi-Auth-Bridge";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderKind {
    Oidc,
    Github,
    Cloudflare,
    Apple,
    AlipayBridge,
}

#[derive(Debug, Clone)]
pub(crate) struct IdentityProviderConfig {
    pub id: &'static str,
    pub display_name: &'static str,
    pub issuer: &'static str,
    pub kind: ProviderKind,
    pub authorization_endpoint: &'static str,
    pub token_endpoint: &'static str,
    pub userinfo_endpoint: &'static str,
    pub scopes: &'static str,
    pub client_id: String,
    pub client_secret: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderIdentityProfile {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub legacy_subject: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeCapabilities {
    #[serde(default)]
    alipay: bool,
    #[serde(default)]
    email: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlipayBridgeIdentity {
    subject: String,
    #[serde(default)]
    legacy_subject: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlipayBridgeResponse {
    ok: bool,
    #[serde(default)]
    identity: Option<AlipayBridgeIdentity>,
}

fn env_text(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .ok()
        .map(|value| value.to_string())
        .or_else(|| env.var(name).ok().map(|value| value.to_string()))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required_secret(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .ok()
        .map(|value| value.to_string())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn configured_provider(env: &Env, provider: &str) -> Option<IdentityProviderConfig> {
    match provider {
        "google" => Some(IdentityProviderConfig {
            id: "google",
            display_name: "Google",
            issuer: "https://accounts.google.com",
            kind: ProviderKind::Oidc,
            authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth",
            token_endpoint: "https://oauth2.googleapis.com/token",
            userinfo_endpoint: "https://openidconnect.googleapis.com/v1/userinfo",
            scopes: "openid email profile",
            client_id: env_text(env, "OAUTH_GOOGLE_CLIENT_ID")?,
            client_secret: Some(required_secret(env, "OAUTH_GOOGLE_CLIENT_SECRET")?),
        }),
        "microsoft" => Some(IdentityProviderConfig {
            id: "microsoft",
            display_name: "Microsoft",
            issuer: "https://login.microsoftonline.com/common/v2.0",
            kind: ProviderKind::Oidc,
            authorization_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            userinfo_endpoint: "https://graph.microsoft.com/oidc/userinfo",
            scopes: "openid email profile",
            client_id: env_text(env, "OAUTH_MICROSOFT_CLIENT_ID")?,
            client_secret: Some(required_secret(env, "OAUTH_MICROSOFT_CLIENT_SECRET")?),
        }),
        "github" => Some(IdentityProviderConfig {
            id: "github",
            display_name: "GitHub",
            issuer: "https://github.com",
            kind: ProviderKind::Github,
            authorization_endpoint: "https://github.com/login/oauth/authorize",
            token_endpoint: "https://github.com/login/oauth/access_token",
            userinfo_endpoint: "https://api.github.com/user",
            scopes: "read:user user:email",
            client_id: env_text(env, "OAUTH_GITHUB_CLIENT_ID")?,
            client_secret: Some(required_secret(env, "OAUTH_GITHUB_CLIENT_SECRET")?),
        }),
        "cloudflare" => Some(IdentityProviderConfig {
            id: "cloudflare",
            display_name: "Cloudflare",
            issuer: "https://dash.cloudflare.com",
            kind: ProviderKind::Cloudflare,
            authorization_endpoint: "https://dash.cloudflare.com/oauth2/auth",
            token_endpoint: "https://dash.cloudflare.com/oauth2/token",
            userinfo_endpoint: CLOUDFLARE_API_USER_URL,
            scopes: "offline_access user-details.read",
            client_id: env_text(env, "OAUTH_CLOUDFLARE_CLIENT_ID")?,
            client_secret: Some(required_secret(env, "OAUTH_CLOUDFLARE_CLIENT_SECRET")?),
        }),
        "apple" => Some(IdentityProviderConfig {
            id: "apple",
            display_name: "Apple",
            issuer: APPLE_ISSUER,
            kind: ProviderKind::Apple,
            authorization_endpoint: APPLE_AUTHORIZE_URL,
            token_endpoint: APPLE_TOKEN_URL,
            userinfo_endpoint: APPLE_JWKS_URL,
            scopes: "name email",
            client_id: env_text(env, "OAUTH_APPLE_CLIENT_ID")?,
            client_secret: None,
        }),
        "alipay" => {
            let _bridge_url = env_text(env, "AUTH_PROVIDER_BRIDGE_URL")?;
            let _bridge_secret = required_secret(env, "AUTH_PROVIDER_BRIDGE_SECRET")?;
            Some(IdentityProviderConfig {
                id: "alipay",
                display_name: "支付宝",
                issuer: "https://openauth.alipay.com",
                kind: ProviderKind::AlipayBridge,
                authorization_endpoint: "",
                token_endpoint: "",
                userinfo_endpoint: "",
                scopes: "auth_user",
                client_id: String::new(),
                client_secret: None,
            })
        }
        _ => None,
    }
}

pub(crate) async fn provider_available(env: &Env, provider: &str) -> bool {
    let Some(config) = configured_provider(env, provider) else {
        return false;
    };
    if config.kind != ProviderKind::AlipayBridge {
        return true;
    }
    bridge_capabilities(env)
        .await
        .map(|capabilities| capabilities.alipay)
        .unwrap_or(false)
}

pub(crate) async fn registration_email_available(env: &Env) -> bool {
    let Ok(base) = registration_bridge_url(env) else {
        return false;
    };
    bridge_capabilities_at(env, &base)
        .await
        .map(|capabilities| capabilities.email)
        .unwrap_or(false)
}

pub(crate) async fn build_authorization_url(
    env: &Env,
    provider: &IdentityProviderConfig,
    state: &str,
    callback: &str,
    verifier: &str,
) -> Result<Url> {
    if provider.kind == ProviderKind::AlipayBridge {
        let bridge = bridge_url(env)?;
        let mut url = Url::parse(&format!(
            "{bridge}/api/internal/auth-provider/alipay/authorize"
        ))
        .map_err(rust_error)?;
        url.query_pairs_mut().append_pair("state", state);
        return Ok(url);
    }

    let mut url = Url::parse(provider.authorization_endpoint).map_err(rust_error)?;
    let mut query = url.query_pairs_mut();
    query
        .append_pair("client_id", &provider.client_id)
        .append_pair("redirect_uri", callback)
        .append_pair("scope", provider.scopes)
        .append_pair("state", state);

    match provider.kind {
        ProviderKind::Apple => {
            query
                .append_pair("response_type", "code id_token")
                .append_pair("response_mode", "form_post")
                .append_pair("nonce", verifier);
        }
        ProviderKind::Oidc | ProviderKind::Github | ProviderKind::Cloudflare => {
            query.append_pair("response_type", "code");
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));
            query
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
            if provider.kind == ProviderKind::Oidc && provider.id != "github" {
                query.append_pair("prompt", "select_account");
            }
        }
        ProviderKind::AlipayBridge => unreachable!(),
    }
    drop(query);
    Ok(url)
}

pub(crate) async fn complete_provider(
    env: &Env,
    provider: &IdentityProviderConfig,
    code: &str,
    callback: &str,
    verifier: &str,
    apple_id_token: Option<&str>,
    apple_user_json: Option<&str>,
) -> Result<ProviderIdentityProfile> {
    match provider.kind {
        ProviderKind::AlipayBridge => alipay_bridge_exchange(env, code).await,
        ProviderKind::Apple => {
            let id_token = apple_id_token
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| worker::Error::RustError("Apple identity token missing".into()))?;
            apple_validate_identity(provider, id_token, verifier, apple_user_json).await
        }
        ProviderKind::Cloudflare => {
            let access_token =
                oauth_exchange_cloudflare(provider, code, callback, verifier).await?;
            cloudflare_profile(provider, &access_token).await
        }
        ProviderKind::Github => {
            let access_token = oauth_exchange_standard(provider, code, callback, verifier).await?;
            github_profile(provider, &access_token).await
        }
        ProviderKind::Oidc => {
            let access_token = oauth_exchange_standard(provider, code, callback, verifier).await?;
            oidc_profile(provider, &access_token).await
        }
    }
}

pub(crate) async fn send_registration_code(env: &Env, email: &str, code: &str) -> Result<()> {
    let payload = json!({"email": email, "code": code});
    let base = registration_bridge_url(env)?;
    let mut response = bridge_fetch_at(
        env,
        &base,
        "/api/internal/auth-provider/email/send-registration-code",
        Method::Post,
        Some(payload.to_string()),
    )
    .await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "registration email provider unavailable".into(),
        ));
    }
    let value: Value = response.json().await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(worker::Error::RustError(
            "registration email delivery failed".into(),
        ));
    }
    Ok(())
}

async fn bridge_capabilities(env: &Env) -> Result<BridgeCapabilities> {
    let base = bridge_url(env)?;
    bridge_capabilities_at(env, &base).await
}

async fn bridge_capabilities_at(env: &Env, base: &str) -> Result<BridgeCapabilities> {
    let mut response = bridge_fetch_at(
        env,
        base,
        "/api/internal/auth-provider/capabilities",
        Method::Get,
        None,
    )
    .await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "identity bridge capabilities unavailable".into(),
        ));
    }
    response.json().await
}

async fn alipay_bridge_exchange(env: &Env, code: &str) -> Result<ProviderIdentityProfile> {
    let payload = json!({"authCode": code});
    let base = bridge_url(env)?;
    let mut response = bridge_fetch_at(
        env,
        &base,
        "/api/internal/auth-provider/alipay/exchange",
        Method::Post,
        Some(payload.to_string()),
    )
    .await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "Alipay identity exchange failed".into(),
        ));
    }
    let result: AlipayBridgeResponse = response.json().await?;
    let identity = result
        .ok
        .then_some(result.identity)
        .flatten()
        .filter(|identity| !identity.subject.trim().is_empty())
        .ok_or_else(|| worker::Error::RustError("Alipay identity missing".into()))?;
    Ok(ProviderIdentityProfile {
        issuer: "https://openauth.alipay.com".into(),
        subject: identity.subject,
        email: None,
        email_verified: false,
        display_name: identity.display_name,
        avatar_url: identity.avatar_url,
        legacy_subject: identity.legacy_subject,
    })
}

async fn bridge_fetch_at(
    env: &Env,
    base: &str,
    path: &str,
    method: Method,
    body: Option<String>,
) -> Result<worker::Response> {
    let secret = required_secret(env, "AUTH_PROVIDER_BRIDGE_SECRET")
        .ok_or_else(|| worker::Error::RustError("identity bridge secret unavailable".into()))?;
    let headers = Headers::new();
    headers.set(BRIDGE_HEADER, &secret)?;
    headers.set("Accept", "application/json")?;
    if body.is_some() {
        headers.set("Content-Type", "application/json")?;
    }
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(headers);
    if let Some(body) = body {
        init.with_body(Some(wasm_bindgen::JsValue::from_str(&body)));
    }
    let request = Request::new_with_init(&format!("{base}{path}"), &init)?;
    Fetch::Request(request).send().await
}

fn bridge_url(env: &Env) -> Result<String> {
    env_text(env, "AUTH_PROVIDER_BRIDGE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| worker::Error::RustError("identity bridge URL unavailable".into()))
}

fn registration_bridge_url(env: &Env) -> Result<String> {
    env_text(env, "AUTH_REGISTRATION_BRIDGE_URL")
        .or_else(|| env_text(env, "AUTH_PROVIDER_BRIDGE_URL"))
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| worker::Error::RustError("registration bridge URL unavailable".into()))
}

async fn oauth_exchange_standard(
    provider: &IdentityProviderConfig,
    code: &str,
    callback: &str,
    verifier: &str,
) -> Result<String> {
    let client_secret = provider
        .client_secret
        .as_deref()
        .ok_or_else(|| worker::Error::RustError("OAuth client secret unavailable".into()))?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &provider.client_id)
        .append_pair("client_secret", client_secret)
        .append_pair("code", code)
        .append_pair("redirect_uri", callback)
        .append_pair("grant_type", "authorization_code")
        .append_pair("code_verifier", verifier)
        .finish();
    token_request(provider.token_endpoint, None, body)
        .await?
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| worker::Error::RustError("OAuth access token missing".into()))
}

async fn oauth_exchange_cloudflare(
    provider: &IdentityProviderConfig,
    code: &str,
    callback: &str,
    verifier: &str,
) -> Result<String> {
    let client_secret = provider
        .client_secret
        .as_deref()
        .ok_or_else(|| worker::Error::RustError("Cloudflare client secret unavailable".into()))?;
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{client_secret}", provider.client_id));
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("redirect_uri", callback)
        .append_pair("code_verifier", verifier)
        .finish();
    token_request(
        provider.token_endpoint,
        Some(format!("Basic {basic}")),
        body,
    )
    .await?
    .get("access_token")
    .and_then(Value::as_str)
    .map(str::to_string)
    .ok_or_else(|| worker::Error::RustError("Cloudflare access token missing".into()))
}

async fn token_request(url: &str, authorization: Option<String>, body: String) -> Result<Value> {
    let headers = Headers::new();
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;
    headers.set("Accept", "application/json")?;
    if let Some(authorization) = authorization {
        headers.set("Authorization", &authorization)?;
    }
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(wasm_bindgen::JsValue::from_str(&body)));
    let request = Request::new_with_init(url, &init)?;
    let mut response = Fetch::Request(request).send().await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "OAuth token exchange failed".into(),
        ));
    }
    response.json().await
}

async fn oauth_fetch_json(url: &str, access_token: &str) -> Result<Value> {
    let mut request = Request::new(url, Method::Get)?;
    request
        .headers_mut()?
        .set("Authorization", &format!("Bearer {access_token}"))?;
    request.headers_mut()?.set("Accept", "application/json")?;
    request
        .headers_mut()?
        .set("User-Agent", "Fabushi-Identity-Broker")?;
    let mut response = Fetch::Request(request).send().await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "OAuth identity request failed".into(),
        ));
    }
    response.json().await
}

async fn oidc_profile(
    provider: &IdentityProviderConfig,
    access_token: &str,
) -> Result<ProviderIdentityProfile> {
    let profile = oauth_fetch_json(provider.userinfo_endpoint, access_token).await?;
    let subject = profile
        .get("sub")
        .or_else(|| profile.get("id"))
        .and_then(value_as_identifier)
        .ok_or_else(|| worker::Error::RustError("OAuth subject missing".into()))?;
    let email = profile
        .get("email")
        .or_else(|| profile.get("mail"))
        .or_else(|| profile.get("userPrincipalName"))
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    // Google publishes email_verified. Microsoft OIDC userinfo does not provide an equivalent
    // verified-email claim, so Microsoft identities are subject-keyed and are never auto-linked by
    // email unless a future provider-specific verified signal is added.
    let email_verified = profile
        .get("email_verified")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ProviderIdentityProfile {
        issuer: provider.issuer.into(),
        subject,
        email,
        email_verified,
        display_name: profile
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        avatar_url: profile
            .get("picture")
            .and_then(Value::as_str)
            .map(str::to_string),
        legacy_subject: None,
    })
}

async fn github_profile(
    provider: &IdentityProviderConfig,
    access_token: &str,
) -> Result<ProviderIdentityProfile> {
    let profile = oauth_fetch_json(provider.userinfo_endpoint, access_token).await?;
    let subject = profile
        .get("id")
        .and_then(value_as_identifier)
        .ok_or_else(|| worker::Error::RustError("GitHub subject missing".into()))?;
    let emails = oauth_fetch_json("https://api.github.com/user/emails", access_token).await?;
    let verified_email = emails.as_array().and_then(|items| {
        items.iter().find(|item| {
            item.get("primary").and_then(Value::as_bool) == Some(true)
                && item.get("verified").and_then(Value::as_bool) == Some(true)
        })
    });
    let email = verified_email
        .and_then(|item| item.get("email"))
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    Ok(ProviderIdentityProfile {
        issuer: provider.issuer.into(),
        subject,
        email_verified: email.is_some(),
        email,
        display_name: profile
            .get("name")
            .or_else(|| profile.get("login"))
            .and_then(Value::as_str)
            .map(str::to_string),
        avatar_url: profile
            .get("avatar_url")
            .and_then(Value::as_str)
            .map(str::to_string),
        legacy_subject: None,
    })
}

async fn cloudflare_profile(
    provider: &IdentityProviderConfig,
    access_token: &str,
) -> Result<ProviderIdentityProfile> {
    let envelope = oauth_fetch_json(CLOUDFLARE_API_USER_URL, access_token).await?;
    if envelope.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(worker::Error::RustError(
            "Cloudflare identity request failed".into(),
        ));
    }
    let profile = envelope
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| worker::Error::RustError("Cloudflare identity missing".into()))?;
    let subject = profile
        .get("id")
        .and_then(value_as_identifier)
        .ok_or_else(|| worker::Error::RustError("Cloudflare subject missing".into()))?;
    let email = profile
        .get("email")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| worker::Error::RustError("Cloudflare verified email missing".into()))?;
    let first = profile
        .get("first_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let last = profile
        .get("last_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let display_name = format!("{first} {last}").trim().to_string();
    Ok(ProviderIdentityProfile {
        issuer: provider.issuer.into(),
        subject,
        email: Some(email.clone()),
        email_verified: true,
        display_name: (!display_name.is_empty())
            .then_some(display_name)
            .or_else(|| email.split('@').next().map(str::to_string)),
        avatar_url: None,
        legacy_subject: None,
    })
}

async fn apple_validate_identity(
    provider: &IdentityProviderConfig,
    id_token: &str,
    expected_nonce: &str,
    apple_user_json: Option<&str>,
) -> Result<ProviderIdentityProfile> {
    let header = decode_header(id_token).map_err(jwt_error)?;
    if header.alg != Algorithm::RS256 {
        return Err(worker::Error::RustError(
            "Apple identity token algorithm rejected".into(),
        ));
    }
    let kid = header
        .kid
        .as_deref()
        .ok_or_else(|| worker::Error::RustError("Apple identity key id missing".into()))?;
    let mut request = Request::new(APPLE_JWKS_URL, Method::Get)?;
    request.headers_mut()?.set("Accept", "application/json")?;
    let mut response = Fetch::Request(request).send().await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "Apple public keys unavailable".into(),
        ));
    }
    let jwks: JwkSet = response.json().await?;
    let jwk = jwks
        .find(kid)
        .ok_or_else(|| worker::Error::RustError("Apple identity key not found".into()))?;
    let key = DecodingKey::from_jwk(jwk).map_err(jwt_error)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[APPLE_ISSUER]);
    validation.set_audience(&[provider.client_id.as_str()]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = decode::<Value>(id_token, &key, &validation)
        .map_err(jwt_error)?
        .claims;
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| worker::Error::RustError("Apple identity subject missing".into()))?;
    let nonce = claims
        .get("nonce")
        .and_then(Value::as_str)
        .ok_or_else(|| worker::Error::RustError("Apple identity nonce missing".into()))?;
    if !constant_time_text_eq(nonce, expected_nonce) {
        return Err(worker::Error::RustError(
            "Apple identity nonce mismatch".into(),
        ));
    }
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let email_verified = claims
        .get("email_verified")
        .map(claim_truthy)
        .unwrap_or(false);
    if email.is_some() && !email_verified {
        return Err(worker::Error::RustError(
            "Apple did not verify the returned email".into(),
        ));
    }
    Ok(ProviderIdentityProfile {
        issuer: APPLE_ISSUER.into(),
        subject,
        email,
        email_verified,
        display_name: apple_display_name(apple_user_json),
        avatar_url: None,
        legacy_subject: None,
    })
}

fn apple_display_name(user_json: Option<&str>) -> Option<String> {
    let value: Value = serde_json::from_str(user_json?).ok()?;
    let name = value.get("name")?;
    let first = name
        .get("firstName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let last = name
        .get("lastName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let display = format!("{first} {last}").trim().to_string();
    (!display.is_empty()).then_some(display)
}

fn value_as_identifier(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
        .or_else(|| value.as_u64().map(|id| id.to_string()))
}

fn claim_truthy(value: &Value) -> bool {
    value.as_bool() == Some(true)
        || value
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn constant_time_text_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn jwt_error(error: jsonwebtoken::errors::Error) -> worker::Error {
    worker::Error::RustError(error.to_string())
}

fn rust_error(error: url::ParseError) -> worker::Error {
    worker::Error::RustError(error.to_string())
}
