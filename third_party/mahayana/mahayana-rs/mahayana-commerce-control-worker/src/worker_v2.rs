use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{
    Context, Env, Fetch, Headers, Method, Request, RequestInit, Response, Result, RouteContext,
    Router, ScheduleContext, ScheduledEvent, event,
};

const DB: &str = "PLATFORM_DB";
const ISSUER: &str = "https://api.ombhrum.com";
const AUDIENCE: &str = "mahayana-platform";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, Clone, Deserialize)]
struct AccessClaims {
    sub: String,
    #[serde(default)]
    scope: Vec<String>,
    token_use: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeveloperProfile {
    developer_id: String,
    owner_user_id: String,
    display_name: String,
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MiniAppAccess {
    mini_app_id: String,
    developer_id: String,
    display_name: String,
    status: String,
    role: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileInput {
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MiniAppInput {
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProductRow {
    product_id: String,
    mini_app_id: String,
    developer_id: String,
    sku: String,
    display_name: String,
    description: String,
    product_kind: String,
    entitlement_capability: String,
    tax_code: Option<String>,
    subscription_period_seconds: Option<i64>,
    currency: String,
    amount: i64,
    price_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PayRailConfigRow {
    allowed_rails_json: String,
    provider_product_refs_json: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AppleIntentRow {
    payment_id: String,
    user_id: String,
    mini_app_name: String,
    sku: String,
    partner_name: String,
    partner_id: String,
    display_name: String,
    description: String,
    product_kind: String,
    currency: String,
    amount: i64,
    tax_code: Option<String>,
    generic_product_id: Option<String>,
    sync_state: String,
}

#[derive(Debug, Clone, Serialize)]
struct AppleJwsClaims {
    iss: String,
    iat: i64,
    aud: String,
    bid: String,
    nonce: String,
    request: String,
}

#[derive(Debug, Clone, Serialize)]
struct GoogleServiceClaims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

fn env_text(env: &Env, name: &str) -> Result<String> {
    env.secret(name)
        .map(|v| v.to_string())
        .or_else(|_| env.var(name).map(|v| v.to_string()))
        .map_err(|_| worker::Error::RustError(format!("missing {name}")))
}

fn env_enabled(env: &Env, name: &str) -> bool {
    env.var(name)
        .ok()
        .map(|v| v.to_string().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn bearer_claims(req: &Request, env: &Env) -> Result<AccessClaims> {
    let auth = req
        .headers()
        .get("Authorization")?
        .ok_or_else(|| worker::Error::RustError("missing Authorization".into()))?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(|| worker::Error::RustError("invalid Authorization".into()))?;
    let key = DecodingKey::from_rsa_pem(env_text(env, "ACCESS_TOKEN_PUBLIC_KEY_PEM")?.as_bytes())
        .map_err(|e| worker::Error::RustError(format!("invalid access key: {e}")))?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[AUDIENCE]);
    let claims = decode::<AccessClaims>(token, &key, &validation)
        .map_err(|_| worker::Error::RustError("invalid access token".into()))?
        .claims;
    if claims.token_use != "access" {
        return Err(worker::Error::RustError("wrong token type".into()));
    }
    Ok(claims)
}

fn require_developer(req: &Request, env: &Env) -> Result<String> {
    let c = bearer_claims(req, env)?;
    let dedicated = c.scope.iter().any(|s| s == "commerce.developer.manage");
    let bootstrap = c.scope.iter().any(|s| s == "marketplace.publish")
        && c.scope.iter().any(|s| s == "commerce.purchase");
    if !dedicated && !bootstrap {
        return Err(worker::Error::RustError(
            "developer commerce scope required".into(),
        ));
    }
    Ok(c.sub)
}

fn require_buyer(req: &Request, env: &Env) -> Result<String> {
    let c = bearer_claims(req, env)?;
    if !c.scope.iter().any(|s| s == "commerce.purchase") {
        return Err(worker::Error::RustError(
            "commerce.purchase required".into(),
        ));
    }
    Ok(c.sub)
}

async fn profile(env: &Env, user: &str) -> Result<Option<DeveloperProfile>> {
    worker::query!(&env.d1(DB)?, "SELECT developer_id, owner_user_id, display_name, status FROM developer_commerce_profiles WHERE owner_user_id=?1 LIMIT 1", user)?
        .first::<DeveloperProfile>(None).await
}

async fn app_access(env: &Env, user: &str, app: &str, write: bool) -> Result<MiniAppAccess> {
    let row = worker::query!(&env.d1(DB)?,
        "SELECT o.mini_app_id, o.developer_id, o.display_name, o.status, m.role FROM mini_app_commerce_owners o JOIN mini_app_commerce_members m ON m.mini_app_id=o.mini_app_id WHERE o.mini_app_id=?1 AND m.user_id=?2 AND m.active=1 AND o.status='active' LIMIT 1",
        app, user)?.first::<MiniAppAccess>(None).await?
        .ok_or_else(|| worker::Error::RustError("Mini App commerce access denied".into()))?;
    if write && !matches!(row.role.as_str(), "owner" | "admin" | "catalog_manager") {
        return Err(worker::Error::RustError("read-only commerce role".into()));
    }
    Ok(row)
}

fn config(env: &Env) -> ProviderConfiguration {
    ProviderConfiguration {
        apple_advanced_commerce_enabled: env_enabled(env, "APPLE_ADVANCED_COMMERCE_ENABLED"),
        apple_one_time_generic_product_id: env_text(
            env,
            "APPLE_ADVANCED_COMMERCE_ONETIME_PRODUCT_ID",
        )
        .ok(),
        apple_subscription_generic_product_id: env_text(
            env,
            "APPLE_ADVANCED_COMMERCE_SUBSCRIPTION_PRODUCT_ID",
        )
        .ok(),
        google_catalog_sync_enabled: env_enabled(env, "GOOGLE_PLAY_CATALOG_SYNC_ENABLED"),
    }
}

async fn get_profile(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    Response::from_json(&json!({"profile": profile(&ctx.env, &user).await?}))
}

async fn put_profile(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let input: ProfileInput = req.json().await?;
    let name = input.display_name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.contains('|') {
        return Response::error("invalid displayName", 400);
    }
    let id = profile(&ctx.env, &user)
        .await?
        .map(|p| p.developer_id)
        .unwrap_or_else(|| format!("dev.{}", Uuid::new_v4().simple()));
    let t = now();
    worker::query!(&ctx.env.d1(DB)?, "INSERT INTO developer_commerce_profiles (developer_id,owner_user_id,display_name,status,created_at,updated_at) VALUES (?1,?2,?3,'active',?4,?4) ON CONFLICT(owner_user_id) DO UPDATE SET display_name=excluded.display_name,updated_at=excluded.updated_at", &id,&user,name,t)?.run().await?;
    Response::from_json(&json!({"developerId":id,"displayName":name,"status":"active"}))
}

async fn list_apps(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let rows=worker::query!(&ctx.env.d1(DB)?, "SELECT o.mini_app_id,o.developer_id,o.display_name,o.status,m.role FROM mini_app_commerce_owners o JOIN mini_app_commerce_members m ON m.mini_app_id=o.mini_app_id WHERE m.user_id=?1 AND m.active=1 ORDER BY o.created_at DESC", &user)?.all().await?.results::<Value>()?;
    Response::from_json(&json!({"miniApps":rows}))
}

async fn register_app(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let p = profile(&ctx.env, &user)
        .await?
        .ok_or_else(|| worker::Error::RustError("register developer profile first".into()))?;
    let app = ctx
        .param("mini_app_id")
        .ok_or_else(|| worker::Error::RustError("missing mini app id".into()))?;
    if !is_identifier(app) {
        return Response::error("invalid mini app id", 400);
    }
    let input: MiniAppInput = req.json().await?;
    let name = input.display_name.trim();
    if name.is_empty() || name.chars().count() > 30 {
        return Response::error("invalid displayName", 400);
    }
    let t = now();
    let db = ctx.env.d1(DB)?;
    worker::query!(&db,"INSERT INTO mini_app_commerce_owners (mini_app_id,developer_id,owner_user_id,display_name,status,created_at,updated_at) VALUES (?1,?2,?3,?4,'active',?5,?5) ON CONFLICT(mini_app_id) DO NOTHING",app,&p.developer_id,&user,name,t)?.run().await?;
    let owned:Option<MiniAppAccess>=worker::query!(&db,"SELECT o.mini_app_id,o.developer_id,o.display_name,o.status,'owner' AS role FROM mini_app_commerce_owners o WHERE o.mini_app_id=?1 AND o.owner_user_id=?2",app,&user)?.first(None).await?;
    if owned.is_none() {
        return Response::error("mini app is already owned by another developer", 409);
    }
    worker::query!(&db,"INSERT INTO mini_app_commerce_members (mini_app_id,user_id,role,active,created_at,updated_at) VALUES (?1,?2,'owner',1,?3,?3) ON CONFLICT(mini_app_id,user_id) DO UPDATE SET role='owner',active=1,updated_at=excluded.updated_at",app,&user,t)?.run().await?;
    Response::from_json(&json!({"miniApp":owned}))
}

async fn list_products(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let app = ctx
        .param("mini_app_id")
        .ok_or_else(|| worker::Error::RustError("missing app".into()))?;
    app_access(&ctx.env, &user, app, false).await?;
    let rows=worker::query!(&ctx.env.d1(DB)?,"SELECT c.product_id,c.sku,c.display_name,c.description,c.product_kind,c.entitlement_capability,c.tax_code,c.subscription_period_seconds,c.catalog_status,p.currency,p.amount,p.price_id FROM payment_product_catalog c JOIN prices p ON p.product_id=c.product_id AND p.active=1 WHERE c.mini_app_id=?1 ORDER BY c.created_at DESC",app)?.all().await?.results::<Value>()?;
    Response::from_json(&json!({"products":rows}))
}

async fn product_row(env: &Env, app: &str, product: &str) -> Result<ProductRow> {
    worker::query!(&env.d1(DB)?,"SELECT c.product_id,c.mini_app_id,c.developer_id,c.sku,c.display_name,c.description,c.product_kind,c.entitlement_capability,c.tax_code,c.subscription_period_seconds,p.currency,p.amount,p.price_id FROM payment_product_catalog c JOIN prices p ON p.product_id=c.product_id AND p.active=1 WHERE c.mini_app_id=?1 AND c.product_id=?2 LIMIT 1",app,product)?.first::<ProductRow>(None).await?.ok_or_else(||worker::Error::RustError("product not found".into()))
}

fn pay_rails_json(plans: &[ProviderBindingPlan]) -> Result<(String, String)> {
    let mut rails = Vec::new();
    let mut refs = serde_json::Map::new();
    for p in plans {
        if p.sync_state != "active" {
            continue;
        }
        let rail = match p.provider.as_str() {
            "apple_advanced_commerce" => "apple_in_app_purchase",
            "google_play" => "google_play_billing",
            x => x,
        };
        rails.push(rail);
        if let Some(r) = p.external_product_ref.as_ref() {
            refs.insert(rail.into(), Value::String(r.clone()));
        }
    }
    Ok((
        serde_json::to_string(&rails).map_err(|e| worker::Error::RustError(e.to_string()))?,
        Value::Object(refs).to_string(),
    ))
}

async fn persist_product(
    env: &Env,
    user: &str,
    app: &MiniAppAccess,
    product_id: &str,
    input: &DeveloperProductDraft,
    update: bool,
) -> Result<Value> {
    validate_product_draft(input).map_err(|e| worker::Error::RustError(e.to_string()))?;
    let plans = plan_provider_bindings(&app.mini_app_id, input, &config(env))
        .map_err(|e| worker::Error::RustError(e.to_string()))?;
    let db = env.d1(DB)?;
    let t = now();
    let price_id = format!("price.{}", Uuid::new_v4().simple());
    let (rails, refs) = pay_rails_json(&plans)?;
    if update {
        worker::query!(&db,"UPDATE prices SET active=0,ends_at=COALESCE(ends_at,?1) WHERE product_id=?2 AND active=1",t,product_id)?.run().await?;
    }
    worker::query!(&db,"INSERT INTO payment_product_catalog (product_id,mini_app_id,developer_id,sku,display_name,description,product_kind,entitlement_capability,tax_code,subscription_period_seconds,catalog_status,created_by_user_id,updated_by_user_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'active',?11,?11,?12,?12) ON CONFLICT(product_id) DO UPDATE SET display_name=excluded.display_name,description=excluded.description,entitlement_capability=excluded.entitlement_capability,tax_code=excluded.tax_code,subscription_period_seconds=excluded.subscription_period_seconds,updated_by_user_id=excluded.updated_by_user_id,updated_at=excluded.updated_at",product_id,&app.mini_app_id,&app.developer_id,&input.sku,&input.display_name,&input.description,&input.product_kind,&input.entitlement_capability,input.tax_code.as_deref(),input.subscription_period_seconds,user,t)?.run().await?;
    let mode = if input.product_kind == "digital_consumable" {
        "consumable"
    } else {
        "durable"
    };
    worker::query!(&db,"INSERT INTO products (product_id,plugin_id,sku,seller_user_id,entitlement_capability,consumption_mode,active,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,1,?7,?7) ON CONFLICT(product_id) DO UPDATE SET entitlement_capability=excluded.entitlement_capability,active=1,updated_at=excluded.updated_at",product_id,&app.mini_app_id,&input.sku,&app.developer_id,&input.entitlement_capability,mode,t)?.run().await?;
    worker::query!(&db,"INSERT INTO prices (price_id,product_id,currency,amount,active,starts_at,created_at) VALUES (?1,?2,?3,?4,1,?5,?5)",&price_id,product_id,&input.currency,input.amount,t)?.run().await?;
    let fee = env
        .var("FABUSHI_PAY_DEFAULT_PLATFORM_FEE_BPS")
        .ok()
        .and_then(|v| v.to_string().parse::<i64>().ok())
        .unwrap_or(1000);
    worker::query!(&db,"INSERT INTO payment_product_config (product_id,developer_id,product_kind,platform_fee_bps,allowed_rails_json,provider_product_refs_json,active,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,1,?7,?7) ON CONFLICT(product_id) DO UPDATE SET product_kind=excluded.product_kind,platform_fee_bps=excluded.platform_fee_bps,allowed_rails_json=excluded.allowed_rails_json,provider_product_refs_json=excluded.provider_product_refs_json,active=1,updated_at=excluded.updated_at",product_id,&app.developer_id,&input.product_kind,fee,&rails,&refs,t)?.run().await?;
    worker::query!(&db,"INSERT INTO payment_price_revisions (revision_id,product_id,price_id,currency,amount,actor_user_id,reason,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",&format!("rev.{}",Uuid::new_v4().simple()),product_id,&price_id,&input.currency,input.amount,user,if update{"developer_update"}else{"developer_create"},t)?.run().await?;
    for p in &plans {
        worker::query!(&db,"INSERT INTO payment_provider_bindings (product_id,provider,external_product_ref,generic_product_id,sync_state,metadata_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,'{}',?6,?6) ON CONFLICT(product_id,provider) DO UPDATE SET external_product_ref=excluded.external_product_ref,generic_product_id=excluded.generic_product_id,sync_state=excluded.sync_state,last_error=NULL,updated_at=excluded.updated_at",product_id,&p.provider,p.external_product_ref.as_deref(),p.generic_product_id.as_deref(),&p.sync_state,t)?.run().await?;
    }
    Ok(
        json!({"productId":product_id,"priceId":price_id,"currency":input.currency,"amount":input.amount,"providerBindings":plans,"pricingAuthority":"fabushi-pay"}),
    )
}

async fn create_product(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let app_id = ctx
        .param("mini_app_id")
        .ok_or_else(|| worker::Error::RustError("missing app".into()))?;
    let app = app_access(&ctx.env, &user, app_id, true).await?;
    let input: DeveloperProductDraft = req.json().await?;
    let id = format!("prod.{}", Uuid::new_v4().simple());
    Response::from_json(&persist_product(&ctx.env, &user, &app, &id, &input, false).await?)
}

async fn update_product(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let app_id = ctx
        .param("mini_app_id")
        .ok_or_else(|| worker::Error::RustError("missing app".into()))?;
    let id = ctx
        .param("product_id")
        .ok_or_else(|| worker::Error::RustError("missing product".into()))?;
    let app = app_access(&ctx.env, &user, app_id, true).await?;
    let old = product_row(&ctx.env, app_id, id).await?;
    let input: DeveloperProductDraft = req.json().await?;
    if old.sku != input.sku || old.product_kind != input.product_kind {
        return Response::error("sku and productKind are immutable", 409);
    }
    Response::from_json(&persist_product(&ctx.env, &user, &app, id, &input, true).await?)
}

async fn apple_request(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_buyer(&req, &ctx.env)?;
    if !env_enabled(&ctx.env, "APPLE_ADVANCED_COMMERCE_ENABLED") {
        return Response::error("Apple Advanced Commerce is not enabled", 503);
    }
    let payment = ctx
        .param("payment_id")
        .ok_or_else(|| worker::Error::RustError("missing payment".into()))?;
    let input: AppleRequestInput = req.json().await?;
    let row=worker::query!(&ctx.env.d1(DB)?,"SELECT pi.payment_id,pi.user_id,o.display_name AS mini_app_name,pi.sku,d.display_name AS partner_name,d.developer_id AS partner_id,c.display_name,c.description,pi.product_kind,pi.currency,pi.amount,c.tax_code,b.generic_product_id,b.sync_state FROM payment_intents pi JOIN payment_product_catalog c ON c.product_id=pi.product_id JOIN mini_app_commerce_owners o ON o.mini_app_id=pi.mini_app_id JOIN developer_commerce_profiles d ON d.developer_id=pi.developer_id JOIN payment_provider_bindings b ON b.product_id=pi.product_id AND b.provider='apple_advanced_commerce' WHERE pi.payment_id=?1 LIMIT 1",payment)?.first::<AppleIntentRow>(None).await?.ok_or_else(||worker::Error::RustError("payment not found".into()))?;
    if row.user_id != user {
        return Response::error("payment does not belong to caller", 403);
    }
    if row.sync_state != "active" {
        return Response::error("Apple product is not configured", 409);
    }
    let product = AppleCatalogProduct {
        payment_id: row.payment_id,
        mini_app_name: row.mini_app_name,
        mini_app_sku: row.sku,
        partner_name: row.partner_name,
        partner_id: row.partner_id,
        display_name: row.display_name,
        description: if row.description.is_empty() {
            "Digital purchase".into()
        } else {
            row.description
        },
        product_kind: row.product_kind,
        currency: row.currency,
        amount_minor: row.amount,
        tax_code: row
            .tax_code
            .ok_or_else(|| worker::Error::RustError("missing tax code".into()))?,
        generic_product_id: row
            .generic_product_id
            .ok_or_else(|| worker::Error::RustError("missing generic product id".into()))?,
    };
    let database = ctx.env.d1(DB)?;
    let existing = worker::query!(&database,"SELECT request_reference_id,generic_product_id,request_json FROM apple_advanced_commerce_requests WHERE payment_id=?1 LIMIT 1",&product.payment_id)?.first::<Value>(None).await?;
    let (reference, generic_product_id, request_json, is_new) = if let Some(existing) = existing {
        let reference = existing
            .get("request_reference_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                worker::Error::RustError("invalid stored Apple request reference".into())
            })?
            .to_string();
        let generic = existing
            .get("generic_product_id")
            .and_then(Value::as_str)
            .ok_or_else(|| worker::Error::RustError("invalid stored Apple generic product".into()))?
            .to_string();
        let request_json: Value = serde_json::from_str(
            existing
                .get("request_json")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    worker::Error::RustError("invalid stored Apple request JSON".into())
                })?,
        )
        .map_err(|_| worker::Error::RustError("invalid stored Apple request JSON".into()))?;
        (reference, generic, request_json, false)
    } else {
        let reference = Uuid::new_v4().to_string();
        let envelope = build_advanced_commerce_request(&product, &input, &reference)
            .map_err(|e| worker::Error::RustError(e.to_string()))?;
        (
            reference,
            envelope.generic_product_id,
            envelope.request_json,
            true,
        )
    };
    let request_bytes =
        serde_json::to_vec(&request_json).map_err(|e| worker::Error::RustError(e.to_string()))?;
    let encoded_request = STANDARD.encode(&request_bytes);
    if is_new {
        let dynamic_sku =
            mini_app_partner_sku(&product).map_err(|e| worker::Error::RustError(e.to_string()))?;
        let price_milliunits = minor_units_to_milliunits(&product.currency, product.amount_minor)
            .map_err(|e| worker::Error::RustError(e.to_string()))?;
        let request_json_text = String::from_utf8(request_bytes.clone())
            .map_err(|_| worker::Error::RustError("invalid Apple request encoding".into()))?;
        let t = now();
        worker::query!(&database,"INSERT INTO apple_advanced_commerce_requests (payment_id,request_reference_id,generic_product_id,dynamic_sku,currency,price_milliunits,tax_code,storefront,request_json,request_fingerprint,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",&product.payment_id,&reference,&generic_product_id,&dynamic_sku,&product.currency,price_milliunits,&product.tax_code,&input.storefront,&request_json_text,&encoded_request,t)?.run().await?;
    }
    let claims = AppleJwsClaims {
        iss: env_text(&ctx.env, "APPLE_IAP_ISSUER_ID")?,
        iat: now(),
        aud: "advanced-commerce-api".into(),
        bid: env_text(&ctx.env, "APPLE_BUNDLE_ID")?,
        nonce: Uuid::new_v4().to_string(),
        request: encoded_request,
    };
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(env_text(&ctx.env, "APPLE_IAP_KEY_ID")?);
    let key = EncodingKey::from_ec_pem(
        env_text(&ctx.env, "APPLE_IAP_PRIVATE_KEY_PEM")?
            .replace("\\n", "\n")
            .as_bytes(),
    )
    .map_err(|e| worker::Error::RustError(format!("invalid Apple key: {e}")))?;
    let token = encode(&header, &claims, &key)
        .map_err(|e| worker::Error::RustError(format!("Apple JWS signing failed: {e}")))?;
    Response::from_json(
        &json!({"genericProductId":generic_product_id,"advancedCommerceData":{"signatureInfo":{"token":token}},"requestReferenceId":reference}),
    )
}

async fn google_token(env: &Env) -> Result<String> {
    let t = now();
    let claims = GoogleServiceClaims {
        iss: env_text(env, "GOOGLE_PLAY_SERVICE_ACCOUNT_EMAIL")?,
        scope: "https://www.googleapis.com/auth/androidpublisher".into(),
        aud: GOOGLE_OAUTH_TOKEN_URL.into(),
        iat: t,
        exp: t + 3600,
    };
    let key = EncodingKey::from_rsa_pem(
        env_text(env, "GOOGLE_PLAY_PRIVATE_KEY")?
            .replace("\\n", "\n")
            .as_bytes(),
    )
    .map_err(|e| worker::Error::RustError(format!("invalid Google key: {e}")))?;
    let assertion = encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|e| worker::Error::RustError(e.to_string()))?;
    let form = format!(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={assertion}"
    );
    let headers = Headers::new();
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(&form)));
    let outbound = Request::new_with_init(GOOGLE_OAUTH_TOKEN_URL, &init)?;
    let mut res = Fetch::Request(outbound).send().await?;
    let status = res.status_code();
    let body = res.bytes().await?;
    if status != 200 {
        return Err(worker::Error::RustError(format!(
            "Google OAuth failed HTTP {status}"
        )));
    };
    let token: GoogleTokenResponse = serde_json::from_slice(&body)
        .map_err(|_| worker::Error::RustError("invalid Google OAuth response".into()))?;
    Ok(token.access_token)
}

async fn activate_google_pay_rail(
    env: &Env,
    product_id: &str,
    external_product_ref: &str,
    metadata: &str,
    synced_at: i64,
) -> Result<()> {
    let db = env.d1(DB)?;
    let row = worker::query!(
        &db,
        "SELECT allowed_rails_json, provider_product_refs_json FROM payment_product_config WHERE product_id=?1 LIMIT 1",
        product_id
    )?
    .first::<PayRailConfigRow>(None)
    .await?
    .ok_or_else(|| worker::Error::RustError("payment product config not found".into()))?;
    let mut rails: Vec<String> = serde_json::from_str(&row.allowed_rails_json)
        .map_err(|_| worker::Error::RustError("invalid allowed rails configuration".into()))?;
    let mut refs: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&row.provider_product_refs_json)
            .map_err(|_| worker::Error::RustError("invalid provider product references".into()))?;
    if !rails.iter().any(|rail| rail == "google_play_billing") {
        rails.push("google_play_billing".into());
    }
    refs.insert(
        "google_play_billing".into(),
        external_product_ref.to_string(),
    );
    let rails_json =
        serde_json::to_string(&rails).map_err(|e| worker::Error::RustError(e.to_string()))?;
    let refs_json =
        serde_json::to_string(&refs).map_err(|e| worker::Error::RustError(e.to_string()))?;

    // Fail closed: make the provider binding active first, and only then expose the
    // rail to PaymentIntent creation. If the second statement fails, pay core stays blocked.
    worker::query!(
        &db,
        "UPDATE payment_provider_bindings SET sync_state='active',external_product_ref=?1,metadata_json=?2,last_error=NULL,last_synced_at=?3,updated_at=?3 WHERE product_id=?4 AND provider='google_play'",
        external_product_ref, metadata, synced_at, product_id
    )?
    .run()
    .await?;
    worker::query!(
        &db,
        "UPDATE payment_product_config SET allowed_rails_json=?1,provider_product_refs_json=?2,updated_at=?3 WHERE product_id=?4",
        &rails_json, &refs_json, synced_at, product_id
    )?
    .run()
    .await?;
    Ok(())
}

async fn send_google_json(
    method: Method,
    url: &str,
    body: &serde_json::Value,
    token: &str,
) -> Result<(u16, Vec<u8>)> {
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {token}"))?;
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(method)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(&body.to_string())));
    let outbound = Request::new_with_init(url, &init)?;
    let mut response = Fetch::Request(outbound).send().await?;
    let status = response.status_code();
    let bytes = response.bytes().await?;
    Ok((status, bytes))
}

async fn sync_google(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    if !env_enabled(&ctx.env, "GOOGLE_PLAY_CATALOG_SYNC_ENABLED") {
        return Response::error("Google catalog sync is not enabled", 503);
    }
    let app_id = ctx
        .param("mini_app_id")
        .ok_or_else(|| worker::Error::RustError("missing app".into()))?;
    let product_id = ctx
        .param("product_id")
        .ok_or_else(|| worker::Error::RustError("missing product".into()))?;
    app_access(&ctx.env, &user, app_id, true).await?;
    let p = product_row(&ctx.env, app_id, product_id).await?;
    let external = google_product_id(app_id, &p.sku);
    let spec = GoogleCatalogProduct {
        package_name: env_text(&ctx.env, "GOOGLE_PLAY_PACKAGE_NAME")?,
        product_id: external.clone(),
        display_name: p.display_name,
        description: p.description,
        product_kind: p.product_kind,
        currency: p.currency,
        amount_minor: p.amount,
        product_tax_category_code: p.tax_code,
    };
    let token = google_token(&ctx.env).await?;

    let conversion_call = build_google_price_conversion_request(&spec)
        .map_err(|e| worker::Error::RustError(e.to_string()))?;
    let (conversion_status, conversion_body) = send_google_json(
        Method::Post,
        &conversion_call.url,
        &conversion_call.body,
        &token,
    )
    .await?;
    if !(200..300).contains(&conversion_status) {
        let error = String::from_utf8_lossy(&conversion_body)
            .chars()
            .take(500)
            .collect::<String>();
        let t = now();
        worker::query!(&ctx.env.d1(DB)?,"UPDATE payment_provider_bindings SET sync_state='error',last_error=?1,updated_at=?2 WHERE product_id=?3 AND provider='google_play'",&error,t,product_id)?.run().await?;
        return Response::from_json(
            &json!({"ok":false,"stage":"convertRegionPrices","status":conversion_status,"error":error}),
        );
    }
    let converted: GoogleConvertedPrices =
        serde_json::from_slice(&conversion_body).map_err(|_| {
            worker::Error::RustError("invalid Google converted pricing response".into())
        })?;
    let call = build_google_sync_request(&spec, &converted)
        .map_err(|e| worker::Error::RustError(e.to_string()))?;
    let method = if call.method == "POST" {
        Method::Post
    } else {
        Method::Patch
    };
    let (status, body) = send_google_json(method, &call.url, &call.body, &token).await?;
    let t = now();
    if !(200..300).contains(&status) {
        let error = String::from_utf8_lossy(&body)
            .chars()
            .take(500)
            .collect::<String>();
        worker::query!(&ctx.env.d1(DB)?,"UPDATE payment_provider_bindings SET sync_state='error',last_error=?1,updated_at=?2 WHERE product_id=?3 AND provider='google_play'",&error,t,product_id)?.run().await?;
        return Response::from_json(
            &json!({"ok":false,"stage":"catalogSync","status":status,"error":error}),
        );
    }
    let metadata = serde_json::json!({
        "regionVersion": converted.region_version.version,
        "convertedRegionCount": converted.converted_region_prices.len(),
    })
    .to_string();
    activate_google_pay_rail(&ctx.env, product_id, &external, &metadata, t).await?;
    Response::from_json(&json!({
        "ok": true,
        "provider": "google_play",
        "externalProductRef": external,
        "status": status,
        "regionVersion": converted.region_version.version,
        "convertedRegionCount": converted.converted_region_prices.len()
    }))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PayoutProfileInput {
    country_code: String,
    legal_entity_type: String,
    preferred_currency: String,
    payout_schedule: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeveloperPayoutRequest {
    payout_account_id: String,
    currency: String,
    amount: i64,
    idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DeveloperPayoutAccountRow {
    payout_account_id: String,
    developer_id: String,
    state: String,
    onboarding_state: String,
    kyc_status: String,
    payouts_enabled: i64,
    currencies_json: String,
}

async fn put_payout_profile(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let developer = profile(&ctx.env, &user)
        .await?
        .ok_or_else(|| worker::Error::RustError("register developer profile first".into()))?;
    let input: PayoutProfileInput = req.json().await?;
    let country = input.country_code.trim().to_ascii_uppercase();
    let currency = input.preferred_currency.trim().to_ascii_uppercase();
    if country.len() != 2 || !country.bytes().all(|b| b.is_ascii_uppercase()) {
        return Response::error("countryCode must be ISO alpha-2", 400);
    }
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Response::error("preferredCurrency must be ISO alpha-3", 400);
    }
    if !matches!(
        input.legal_entity_type.as_str(),
        "individual" | "individual_business" | "company" | "nonprofit"
    ) {
        return Response::error("invalid legalEntityType", 400);
    }
    if !matches!(
        input.payout_schedule.as_str(),
        "manual" | "daily" | "weekly" | "monthly"
    ) {
        return Response::error("invalid payoutSchedule", 400);
    }
    let t = now();
    worker::query!(&ctx.env.d1(DB)?,
        "INSERT INTO developer_payout_profiles (developer_id,country_code,legal_entity_type,preferred_currency,payout_schedule,compliance_state,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,'pending',?6,?6) ON CONFLICT(developer_id) DO UPDATE SET country_code=excluded.country_code,legal_entity_type=excluded.legal_entity_type,preferred_currency=excluded.preferred_currency,payout_schedule=excluded.payout_schedule,compliance_state=CASE WHEN developer_payout_profiles.country_code=excluded.country_code AND developer_payout_profiles.legal_entity_type=excluded.legal_entity_type THEN developer_payout_profiles.compliance_state ELSE 'pending' END,updated_at=excluded.updated_at",
        &developer.developer_id,&country,&input.legal_entity_type,&currency,&input.payout_schedule,t)?.run().await?;
    Response::from_json(
        &json!({"developerId":developer.developer_id,"countryCode":country,"legalEntityType":input.legal_entity_type,"preferredCurrency":currency,"payoutSchedule":input.payout_schedule,"complianceState":"pending"}),
    )
}

async fn payout_overview(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let developer = profile(&ctx.env, &user)
        .await?
        .ok_or_else(|| worker::Error::RustError("developer profile not found".into()))?;
    let db = ctx.env.d1(DB)?;
    let profile_row = worker::query!(&db,"SELECT developer_id,country_code,legal_entity_type,preferred_currency,payout_schedule,compliance_state FROM developer_payout_profiles WHERE developer_id=?1 LIMIT 1",&developer.developer_id)?.first::<Value>(None).await?;
    let accounts = worker::query!(&db,"SELECT payout_account_id,provider,country_code,legal_entity_type,state,onboarding_state,kyc_status,payouts_enabled,currencies_json,purposes_json,is_default,last_capability_sync_at FROM developer_payout_accounts WHERE developer_id=?1 ORDER BY is_default DESC,created_at DESC",&developer.developer_id)?.all().await?.results::<Value>()?;
    let pending_pattern = format!("developer-pending:{}:%", developer.developer_id);
    let available_pattern = format!("developer-available:{}:%", developer.developer_id);
    let reserved_pattern = format!("developer-reserved:{}:%", developer.developer_id);
    let paid_pattern = format!("developer-paid:{}:%", developer.developer_id);
    let balances=worker::query!(&db,"SELECT account_id,currency,balance FROM wallet_balances WHERE account_id LIKE ?1 OR account_id LIKE ?2 OR account_id LIKE ?3 OR account_id LIKE ?4 ORDER BY currency,account_id",&pending_pattern,&available_pattern,&reserved_pattern,&paid_pattern)?.all().await?.results::<Value>()?;
    let settlements=worker::query!(&db,"SELECT reconciliation_id,payment_id,region_code,settlement_source,currency,gross_amount,tax_amount,provider_fee_amount,refund_amount,chargeback_amount,net_receipts,platform_fee_amount,reserve_amount,developer_payable_amount,status,created_at FROM developer_settlement_reconciliations WHERE developer_id=?1 ORDER BY created_at DESC LIMIT 100",&developer.developer_id)?.all().await?.results::<Value>()?;
    let payouts=worker::query!(&db,"SELECT p.payout_id,p.payout_account_id,p.currency,p.amount,p.status,p.provider_reference,p.created_at,p.updated_at,a.provider FROM developer_payouts p JOIN developer_payout_accounts a ON a.payout_account_id=p.payout_account_id WHERE p.developer_id=?1 ORDER BY p.created_at DESC LIMIT 100",&developer.developer_id)?.all().await?.results::<Value>()?;
    let original_splits=worker::query!(&db,"SELECT split_id,payment_id,provider,payout_account_id,currency,amount,platform_fee_amount,provider_reference,state,created_at FROM developer_original_order_splits WHERE developer_id=?1 ORDER BY created_at DESC LIMIT 100",&developer.developer_id)?.all().await?.results::<Value>()?;
    let region = profile_row
        .as_ref()
        .and_then(|v| v.get("country_code"))
        .and_then(Value::as_str)
        .map(|v| if v == "CN" { "CN" } else { "GLOBAL" })
        .unwrap_or("GLOBAL");
    let routes=worker::query!(&db,"SELECT route_id,region_code,purpose,provider,priority,state FROM payout_provider_routes WHERE region_code=?1 ORDER BY purpose,priority",region)?.all().await?.results::<Value>()?;
    Response::from_json(
        &json!({"profile":profile_row,"balances":balances,"accounts":accounts,"settlements":settlements,"payouts":payouts,"originalSplits":original_splits,"routes":routes}),
    )
}

async fn request_payout(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let developer = profile(&ctx.env, &user)
        .await?
        .ok_or_else(|| worker::Error::RustError("developer profile not found".into()))?;
    let input: DeveloperPayoutRequest = req.json().await?;
    if input.amount <= 0
        || input.currency.len() != 3
        || !input.currency.bytes().all(|b| b.is_ascii_uppercase())
        || !is_identifier(&input.payout_account_id)
        || !is_identifier(&input.idempotency_key)
    {
        return Response::error("invalid payout request", 400);
    }
    let db = ctx.env.d1(DB)?;
    let account=worker::query!(&db,"SELECT payout_account_id,developer_id,state,onboarding_state,kyc_status,payouts_enabled,currencies_json FROM developer_payout_accounts WHERE payout_account_id=?1 AND developer_id=?2 LIMIT 1",&input.payout_account_id,&developer.developer_id)?.first::<DeveloperPayoutAccountRow>(None).await?
        .ok_or_else(||worker::Error::RustError("payout account not found".into()))?;
    let currencies: Vec<String> = serde_json::from_str(&account.currencies_json)
        .map_err(|_| worker::Error::RustError("invalid payout account currencies".into()))?;
    if account.state != "active"
        || account.onboarding_state != "verified"
        || account.kyc_status != "verified"
        || account.payouts_enabled != 1
        || !currencies.iter().any(|c| c == &input.currency)
    {
        return Response::error(
            "payout account is not verified/enabled for this currency",
            409,
        );
    }
    let base = ctx
        .env
        .var("FABUSHI_PAY_INTERNAL_URL")
        .ok()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "https://pay.ombhrum.com".into());
    if !base.starts_with("https://") {
        return Response::error("invalid internal pay URL", 500);
    }
    let create_body = json!({"idempotencyKey":input.idempotency_key,"developerId":developer.developer_id,"payoutAccountId":input.payout_account_id,"currency":input.currency,"amount":input.amount});
    let (create_status, created) =
        pay_admin_json(&ctx.env, "/v1/pay/admin/payouts", Some(create_body)).await?;
    if !(200..300).contains(&create_status) {
        return Ok(Response::from_json(&created)?.with_status(create_status));
    }
    let payout_id = created
        .get("payout")
        .and_then(|value| value.get("payoutId"))
        .and_then(Value::as_str)
        .or_else(|| created.get("payoutId").and_then(Value::as_str))
        .ok_or_else(|| worker::Error::RustError("pay service response lacks payoutId".into()))?;
    let (dispatch_status, dispatched) = pay_admin_json(
        &ctx.env,
        &format!("/v1/pay/admin/payouts/{}/dispatch", payout_id),
        Some(json!({})),
    )
    .await?;
    if !(200..300).contains(&dispatch_status) {
        return Ok(Response::from_json(
            &json!({"payout":created.get("payout"),"dispatch":dispatched}),
        )?
        .with_status(dispatch_status));
    }
    Response::from_json(&json!({"payout":created.get("payout"),"dispatch":dispatched}))
}

#[derive(Debug, Clone, Deserialize)]
struct AutoPayoutCandidate {
    developer_id: String,
    preferred_currency: String,
    payout_schedule: String,
    minimum_payout_amount: i64,
    last_scheduled_payout_at: Option<i64>,
    payout_account_id: String,
}

fn payout_schedule_seconds(schedule: &str) -> Option<i64> {
    match schedule {
        "daily" => Some(86_400),
        "weekly" => Some(604_800),
        "monthly" => Some(2_592_000),
        _ => None,
    }
}

fn payout_schedule_due(schedule: &str, last: Option<i64>, current: i64) -> bool {
    let Some(period) = payout_schedule_seconds(schedule) else {
        return false;
    };
    last.map(|value| current.saturating_sub(value) >= period)
        .unwrap_or(true)
}

async fn pay_admin_json(env: &Env, path: &str, body: Option<Value>) -> Result<(u16, Value)> {
    let base = env
        .var("FABUSHI_PAY_INTERNAL_URL")
        .ok()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "https://pay.ombhrum.com".into());
    if !base.starts_with("https://") || base.contains(char::is_whitespace) {
        return Err(worker::Error::RustError(
            "invalid FABUSHI_PAY_INTERNAL_URL".into(),
        ));
    }
    let token = env_text(env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {token}"))?;
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_headers(headers);
    if let Some(body) = body {
        init.with_body(Some(JsValue::from_str(&body.to_string())));
    }
    let request =
        Request::new_with_init(&format!("{}{}", base.trim_end_matches('/'), path), &init)?;
    let mut response = Fetch::Request(request).send().await?;
    let status = response.status_code();
    let bytes = response.bytes().await?;
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)}));
    Ok((status, value))
}

async fn run_payout_maintenance(env: &Env) -> Result<()> {
    // Release matured risk reserves first so the sweep sees the final available balance.
    let (reserve_status, _) =
        pay_admin_json(env, "/v1/pay/admin/settlements/reserves/release-due", None).await?;
    if !(200..300).contains(&reserve_status) {
        return Err(worker::Error::RustError(format!(
            "reserve release sweep failed with HTTP {reserve_status}"
        )));
    }

    let database = env.d1(DB)?;
    let candidates = worker::query!(&database,
        "SELECT p.developer_id,p.preferred_currency,p.payout_schedule,p.minimum_payout_amount,p.last_scheduled_payout_at,a.payout_account_id
         FROM developer_payout_profiles p
         JOIN developer_payout_accounts a ON a.developer_id=p.developer_id
         WHERE p.compliance_state='eligible' AND p.payout_schedule<>'manual'
           AND a.is_default=1 AND a.state='active' AND a.onboarding_state='verified'
           AND a.kyc_status='verified' AND a.payouts_enabled=1
           AND EXISTS (SELECT 1 FROM json_each(a.currencies_json) WHERE value=p.preferred_currency)
           AND EXISTS (SELECT 1 FROM json_each(a.purposes_json) WHERE value='marketplace_payout')
           AND EXISTS (
             SELECT 1 FROM payout_provider_routes r
             WHERE r.region_code=CASE WHEN p.country_code='CN' THEN 'CN' ELSE 'GLOBAL' END
               AND r.purpose='marketplace_payout' AND r.provider=a.provider AND r.state='active'
           )
         ORDER BY p.developer_id,a.is_default DESC,a.created_at ASC").all().await?.results::<AutoPayoutCandidate>()?;
    let current = now();
    let mut seen = std::collections::BTreeSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.developer_id.clone())
            || !payout_schedule_due(
                &candidate.payout_schedule,
                candidate.last_scheduled_payout_at,
                current,
            )
        {
            continue;
        }
        let account_id = format!(
            "developer-available:{}:{}",
            candidate.developer_id, candidate.preferred_currency
        );
        let balance = worker::query!(
            &database,
            "SELECT balance FROM wallet_balances WHERE account_id=?1 LIMIT 1",
            &account_id
        )?
        .first::<Value>(None)
        .await?
        .and_then(|value| value.get("balance").and_then(Value::as_i64))
        .unwrap_or(0);
        let minimum = candidate.minimum_payout_amount.max(1);
        if balance < minimum {
            continue;
        }
        let period = payout_schedule_seconds(&candidate.payout_schedule).unwrap_or(86_400);
        let bucket = current / period;
        let idempotency_key = format!(
            "auto:{}:{}:{}",
            candidate.developer_id, candidate.preferred_currency, bucket
        );
        let create_body = json!({
            "idempotencyKey": idempotency_key,
            "developerId": candidate.developer_id,
            "payoutAccountId": candidate.payout_account_id,
            "currency": candidate.preferred_currency,
            "amount": balance
        });
        let (create_status, created) =
            pay_admin_json(env, "/v1/pay/admin/payouts", Some(create_body)).await?;
        if !(200..300).contains(&create_status) {
            continue;
        }
        let payout_id = created
            .get("payout")
            .and_then(|value| value.get("payoutId"))
            .and_then(Value::as_str)
            .or_else(|| created.get("payoutId").and_then(Value::as_str));
        let Some(payout_id) = payout_id else {
            continue;
        };
        let (dispatch_status, _) = pay_admin_json(
            env,
            &format!("/v1/pay/admin/payouts/{}/dispatch", payout_id),
            Some(json!({})),
        )
        .await?;
        if (200..300).contains(&dispatch_status) {
            worker::query!(&database,"UPDATE developer_payout_profiles SET last_scheduled_payout_at=?1,updated_at=?1 WHERE developer_id=?2",current,&candidate.developer_id)?.run().await?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PayoutOnboardingInput {
    provider: String,
    purpose: String,
}

fn allowed_onboarding_provider(region: &str, provider: &str, purpose: &str) -> bool {
    if region == "CN" {
        match purpose {
            "original_order_split" => matches!(provider, "wechat_platform" | "alipay_platform"),
            "external_proceeds_payout" | "marketplace_payout" => {
                matches!(provider, "lianlian_account_plus" | "huifu_dougong")
            }
            _ => false,
        }
    } else {
        match purpose {
            "marketplace_payout" => matches!(
                provider,
                "stripe_connect" | "adyen_platform" | "paypal_multiparty" | "paypal_payouts"
            ),
            "external_proceeds_payout" => matches!(
                provider,
                "stripe_connect" | "adyen_platform" | "paypal_payouts"
            ),
            _ => false,
        }
    }
}

fn payout_onboarding_env(provider: &str) -> Option<&'static str> {
    match provider {
        "stripe_connect" => Some("PAYOUT_STRIPE_CONNECT_ONBOARDING_URL"),
        "adyen_platform" => Some("PAYOUT_ADYEN_PLATFORM_ONBOARDING_URL"),
        "paypal_multiparty" => Some("PAYOUT_PAYPAL_MULTIPARTY_ONBOARDING_URL"),
        "paypal_payouts" => Some("PAYOUT_PAYPAL_PAYOUTS_ONBOARDING_URL"),
        "wechat_platform" => Some("PAYOUT_WECHAT_PLATFORM_ONBOARDING_URL"),
        "alipay_platform" => Some("PAYOUT_ALIPAY_PLATFORM_ONBOARDING_URL"),
        "lianlian_account_plus" => Some("PAYOUT_LIANLIAN_ACCOUNT_PLUS_ONBOARDING_URL"),
        "huifu_dougong" => Some("PAYOUT_HUIFU_DOUGONG_ONBOARDING_URL"),
        _ => None,
    }
}

async fn create_payout_onboarding(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = require_developer(&req, &ctx.env)?;
    let developer = profile(&ctx.env, &user)
        .await?
        .ok_or_else(|| worker::Error::RustError("developer profile not found".into()))?;
    let input: PayoutOnboardingInput = req.json().await?;
    let database = ctx.env.d1(DB)?;
    let payout_profile = worker::query!(&database,
        "SELECT country_code,legal_entity_type,preferred_currency,compliance_state FROM developer_payout_profiles WHERE developer_id=?1 LIMIT 1",
        &developer.developer_id)?.first::<Value>(None).await?
        .ok_or_else(|| worker::Error::RustError("configure payout profile first".into()))?;
    let country = payout_profile
        .get("country_code")
        .and_then(Value::as_str)
        .unwrap_or("ZZ");
    let region = if country == "CN" { "CN" } else { "GLOBAL" };
    if !allowed_onboarding_provider(region, &input.provider, &input.purpose) {
        return Response::error(
            "provider/purpose is not eligible for this developer region",
            400,
        );
    }
    let active_route = worker::query!(&database,
        "SELECT route_id FROM payout_provider_routes WHERE region_code=?1 AND purpose=?2 AND provider=?3 AND state='active' LIMIT 1",
        region,&input.purpose,&input.provider)?.first::<Value>(None).await?;
    if active_route.is_none() {
        return Response::error(
            "payout provider is not approved/configured for this route",
            503,
        );
    }
    let env_name = payout_onboarding_env(&input.provider)
        .ok_or_else(|| worker::Error::RustError("unsupported payout onboarding provider".into()))?;
    let endpoint = ctx
        .env
        .var(env_name)
        .map_err(|_| worker::Error::RustError(format!("missing {env_name}")))?
        .to_string();
    if !endpoint.starts_with("https://") || endpoint.contains(char::is_whitespace) {
        return Response::error("invalid payout onboarding endpoint", 500);
    }
    let token = env_text(&ctx.env, "PAYOUT_PROVIDER_EXECUTOR_TOKEN")?;
    let session_id = format!("onboard.{}", Uuid::new_v4().simple());
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {token}"))?;
    headers.set("Content-Type", "application/json")?;
    let body = json!({"sessionId":session_id,"developerId":developer.developer_id,"provider":input.provider,"purpose":input.purpose,"countryCode":country,"legalEntityType":payout_profile.get("legal_entity_type").and_then(Value::as_str).unwrap_or("company"),"preferredCurrency":payout_profile.get("preferred_currency").and_then(Value::as_str).unwrap_or("USD")});
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(&body.to_string())));
    let outbound = Request::new_with_init(&endpoint, &init)?;
    let mut response = Fetch::Request(outbound).send().await?;
    let status = response.status_code();
    let bytes = response.bytes().await?;
    if !(200..300).contains(&status) {
        worker::query!(&database,"INSERT INTO developer_payout_onboarding_sessions (session_id,developer_id,provider,country_code,state,last_error,created_at,updated_at) VALUES (?1,?2,?3,?4,'failed',?5,?6,?6)",&session_id,&developer.developer_id,&input.provider,country,&String::from_utf8_lossy(&bytes).chars().take(500).collect::<String>(),now())?.run().await?;
        return Response::error("provider onboarding request failed", 502);
    }
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| worker::Error::RustError("invalid payout onboarding response".into()))?;
    let onboarding_url = payload
        .get("onboardingUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            worker::Error::RustError("provider onboarding response lacks onboardingUrl".into())
        })?;
    if !onboarding_url.starts_with("https://") {
        return Response::error("provider onboarding URL must be HTTPS", 502);
    }
    let provider_session_reference = payload
        .get("providerSessionReference")
        .and_then(Value::as_str);
    let payout_account_id = payload.get("payoutAccountId").and_then(Value::as_str);
    let expires_at = payload.get("expiresAt").and_then(Value::as_i64);
    let t = now();
    worker::query!(&database,"INSERT INTO developer_payout_onboarding_sessions (session_id,developer_id,provider,country_code,payout_account_id,provider_session_reference,state,expires_at,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,'pending',?7,?8,?8)",&session_id,&developer.developer_id,&input.provider,country,payout_account_id,provider_session_reference,expires_at,t)?.run().await?;
    Response::from_json(
        &json!({"sessionId":session_id,"provider":input.provider,"purpose":input.purpose,"onboardingUrl":onboarding_url,"expiresAt":expires_at,"state":"pending"}),
    )
}

#[event(fetch, respond_with_errors)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
      .get("/health",|_,_|Response::from_json(&json!({"ok":true,"service":"fabushi-commerce-control","schema":"fabushi.developer-commerce.v2"})))
      .get_async("/v1/developer/commerce/profile",get_profile).post_async("/v1/developer/commerce/profile",put_profile)
      .get_async("/v1/developer/commerce/payout",payout_overview).post_async("/v1/developer/commerce/payout/profile",put_payout_profile)
      .post_async("/v1/developer/commerce/payout/onboarding",create_payout_onboarding)
      .post_async("/v1/developer/commerce/payout/request",request_payout)
      .get_async("/v1/developer/commerce/miniapps",list_apps).post_async("/v1/developer/commerce/miniapps/:mini_app_id",register_app)
      .get_async("/v1/developer/commerce/miniapps/:mini_app_id/products",list_products).post_async("/v1/developer/commerce/miniapps/:mini_app_id/products",create_product)
      .post_async("/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id",update_product)
      .post_async("/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id/google/sync",sync_google)
      .post_async("/v1/pay/intents/:payment_id/apple/advanced-commerce",apple_request)
      .run(req,env).await
}

#[event(scheduled)]
pub async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    if let Err(error) = run_payout_maintenance(&env).await {
        worker::console_error!("developer payout maintenance failed: {}", error);
    }
}
