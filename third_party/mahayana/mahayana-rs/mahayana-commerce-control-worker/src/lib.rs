#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde::{Deserialize, Serialize};

const MAX_PRICE_MINOR: i64 = 100_000_000_000;
const THIRTY_DAYS_SECONDS: i64 = 2_592_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeveloperProductDraft {
    pub sku: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub product_kind: String,
    pub entitlement_capability: String,
    pub currency: String,
    pub amount: i64,
    #[serde(default)]
    pub subscription_period_seconds: Option<i64>,
    #[serde(default)]
    pub rails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBindingPlan {
    pub provider: String,
    pub external_product_ref: Option<String>,
    pub generic_product_id: Option<String>,
    pub sync_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfiguration {
    pub apple_advanced_commerce_enabled: bool,
    pub apple_one_time_generic_product_id: Option<String>,
    pub apple_subscription_generic_product_id: Option<String>,
    pub google_catalog_sync_enabled: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("invalid identifier")]
    InvalidIdentifier,
    #[error("invalid product kind")]
    InvalidProductKind,
    #[error("invalid currency")]
    InvalidCurrency,
    #[error("invalid price")]
    InvalidPrice,
    #[error("invalid subscription period")]
    InvalidSubscriptionPeriod,
    #[error("invalid payment rail")]
    InvalidRail,
    #[error("credits rail requires FBC pricing")]
    CreditsCurrencyMismatch,
    #[error("Apple Mini Apps SKU is too long")]
    AppleSkuTooLong,
    #[error("price cannot be represented in Apple milliunits")]
    ApplePriceOverflow,
}

pub fn validate_product_draft(input: &DeveloperProductDraft) -> Result<(), CatalogError> {
    if !is_identifier(&input.sku) || !is_identifier(&input.entitlement_capability) {
        return Err(CatalogError::InvalidIdentifier);
    }
    if input.display_name.trim().is_empty() || input.display_name.chars().count() > 80 {
        return Err(CatalogError::InvalidIdentifier);
    }
    if input.description.chars().count() > 500 {
        return Err(CatalogError::InvalidIdentifier);
    }
    if !matches!(
        input.product_kind.as_str(),
        "digital_consumable" | "digital_durable" | "subscription" | "physical" | "service"
    ) {
        return Err(CatalogError::InvalidProductKind);
    }
    if !is_currency(&input.currency) {
        return Err(CatalogError::InvalidCurrency);
    }
    if input.amount <= 0 || input.amount > MAX_PRICE_MINOR {
        return Err(CatalogError::InvalidPrice);
    }
    if input.product_kind == "subscription" {
        if input.subscription_period_seconds != Some(THIRTY_DAYS_SECONDS) {
            return Err(CatalogError::InvalidSubscriptionPeriod);
        }
    } else if input.subscription_period_seconds.is_some() {
        return Err(CatalogError::InvalidSubscriptionPeriod);
    }
    let rails = normalized_rails(input)?;
    if rails.iter().any(|rail| rail == "credits") && input.currency != "FBC" {
        return Err(CatalogError::CreditsCurrencyMismatch);
    }
    Ok(())
}

pub fn normalized_rails(input: &DeveloperProductDraft) -> Result<Vec<String>, CatalogError> {
    let defaults: &[&str] = match input.product_kind.as_str() {
        "digital_consumable" | "digital_durable" | "subscription" => {
            &["apple_advanced_commerce", "google_play", "web_provider"]
        }
        "physical" | "service" => &["merchant_provider", "web_provider"],
        _ => return Err(CatalogError::InvalidProductKind),
    };
    let source: Vec<String> = if input.rails.is_empty() {
        defaults.iter().map(|rail| (*rail).to_string()).collect()
    } else {
        input.rails.clone()
    };
    let mut result = Vec::new();
    for rail in source {
        let rail = match rail.trim() {
            "apple" | "apple_advanced_commerce" => "apple_advanced_commerce",
            "google" | "google_play" => "google_play",
            "web" | "web_provider" => "web_provider",
            "merchant" | "merchant_provider" => "merchant_provider",
            "credits" => "credits",
            _ => return Err(CatalogError::InvalidRail),
        };
        if !result.iter().any(|existing| existing == rail) {
            result.push(rail.to_string());
        }
    }
    Ok(result)
}

pub fn plan_provider_bindings(
    mini_app_id: &str,
    input: &DeveloperProductDraft,
    configuration: &ProviderConfiguration,
) -> Result<Vec<ProviderBindingPlan>, CatalogError> {
    validate_product_draft(input)?;
    if !is_identifier(mini_app_id) {
        return Err(CatalogError::InvalidIdentifier);
    }
    let mut plans = Vec::new();
    for rail in normalized_rails(input)? {
        let plan = match rail.as_str() {
            "apple_advanced_commerce" => {
                let generic = if input.product_kind == "subscription" {
                    configuration.apple_subscription_generic_product_id.clone()
                } else {
                    configuration.apple_one_time_generic_product_id.clone()
                };
                ProviderBindingPlan {
                    provider: rail,
                    external_product_ref: generic.clone(),
                    generic_product_id: generic.clone(),
                    sync_state: if configuration.apple_advanced_commerce_enabled && generic.is_some() {
                        "active".into()
                    } else {
                        "pending_configuration".into()
                    },
                }
            }
            "google_play" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: Some(google_product_id(mini_app_id, &input.sku)),
                generic_product_id: None,
                sync_state: if configuration.google_catalog_sync_enabled {
                    "pending_sync".into()
                } else {
                    "pending_configuration".into()
                },
            },
            "web_provider" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: Some(format!("fabushi.{mini_app_id}.{}", input.sku)),
                generic_product_id: None,
                sync_state: "active".into(),
            },
            "merchant_provider" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: Some(format!("fabushi.merchant.{mini_app_id}.{}", input.sku)),
                generic_product_id: None,
                sync_state: "active".into(),
            },
            "credits" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: None,
                generic_product_id: None,
                sync_state: "active".into(),
            },
            _ => return Err(CatalogError::InvalidRail),
        };
        plans.push(plan);
    }
    Ok(plans)
}

pub fn apple_partner_sku(
    mini_app_sku: &str,
    partner_name: &str,
    partner_id: &str,
) -> Result<String, CatalogError> {
    if !is_identifier(mini_app_sku)
        || !is_identifier(partner_name)
        || !is_identifier(partner_id)
    {
        return Err(CatalogError::InvalidIdentifier);
    }
    let sku = format!("{mini_app_sku}|{partner_name}|{partner_id}");
    if sku.len() > 128 {
        return Err(CatalogError::AppleSkuTooLong);
    }
    Ok(sku)
}

pub fn minor_units_to_apple_milliunits(currency: &str, amount: i64) -> Result<i64, CatalogError> {
    if !is_currency(currency) || amount <= 0 {
        return Err(CatalogError::InvalidPrice);
    }
    let exponent = iso_currency_exponent(currency);
    let multiplier = 10_i64
        .checked_pow(3_u32.saturating_sub(exponent as u32))
        .ok_or(CatalogError::ApplePriceOverflow)?;
    amount
        .checked_mul(multiplier)
        .ok_or(CatalogError::ApplePriceOverflow)
}

fn iso_currency_exponent(currency: &str) -> u8 {
    match currency {
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG"
        | "RWF" | "UGX" | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        _ => 2,
    }
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
        })
}

fn is_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn google_product_id(mini_app_id: &str, sku: &str) -> String {
    let mut value = format!("{}.{}", mini_app_id, sku).to_ascii_lowercase();
    value = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' { ch } else { '_' })
        .collect();
    value.truncate(128);
    value
}

#[cfg(target_arch = "wasm32")]
mod worker_api {
    use super::*;
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use uuid::Uuid;
    use worker::{Context, Env, Request, Response, Result, RouteContext, Router, event};

    const DATABASE_BINDING: &str = "PLATFORM_DB";
    const ACCESS_TOKEN_ISSUER: &str = "https://api.ombhrum.com";
    const ACCESS_TOKEN_AUDIENCE: &str = "mahayana-platform";

    #[derive(Debug, Deserialize)]
    struct AccessTokenClaims {
        sub: String,
        #[serde(default)]
        scope: Vec<String>,
        token_use: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct DeveloperProfileRequest {
        display_name: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct RegisterMiniAppRequest {
        display_name: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct ProductStatusRequest {
        status: String,
    }

    #[derive(Debug, Deserialize)]
    struct DeveloperProfileRow {
        developer_id: String,
        owner_user_id: String,
        display_name: String,
        status: String,
    }

    #[derive(Debug, Deserialize)]
    struct OwnedMiniAppRow {
        mini_app_id: String,
        developer_id: String,
        display_name: String,
        status: String,
        role: String,
    }

    #[derive(Debug, Deserialize)]
    struct ProductRow {
        product_id: String,
        mini_app_id: String,
        developer_id: String,
        sku: String,
        display_name: String,
        description: String,
        product_kind: String,
        entitlement_capability: String,
        subscription_period_seconds: Option<i64>,
        catalog_status: String,
        currency: String,
        amount: i64,
        price_id: String,
    }

    fn now_seconds() -> i64 {
        (js_sys::Date::now() / 1000.0) as i64
    }

    fn authenticated_developer(request: &Request, env: &Env) -> Result<String> {
        let authorization = request
            .headers()
            .get("Authorization")?
            .ok_or_else(|| worker::Error::RustError("missing Authorization header".into()))?;
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| worker::Error::RustError("invalid Authorization header".into()))?;
        let public_key = env.secret("ACCESS_TOKEN_PUBLIC_KEY_PEM")?.to_string();
        let key = DecodingKey::from_rsa_pem(public_key.as_bytes())
            .map_err(|error| worker::Error::RustError(format!("invalid access key: {error}")))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ACCESS_TOKEN_ISSUER]);
        validation.set_audience(&[ACCESS_TOKEN_AUDIENCE]);
        let claims = decode::<AccessTokenClaims>(token, &key, &validation)
            .map_err(|error| worker::Error::RustError(format!("invalid access token: {error}")))?
            .claims;
        if claims.token_use != "access"
            || !claims.scope.iter().any(|scope| scope == "commerce.developer.manage")
        {
            return Err(worker::Error::RustError(
                "access token cannot manage Developer Commerce".into(),
            ));
        }
        Ok(claims.sub)
    }

    fn default_platform_fee_bps(env: &Env) -> Result<i64> {
        let value = env
            .var("FABUSHI_PAY_DEFAULT_PLATFORM_FEE_BPS")
            .ok()
            .and_then(|value| value.to_string().parse::<i64>().ok())
            .unwrap_or(1000);
        if !(0..=10_000).contains(&value) {
            return Err(worker::Error::RustError(
                "invalid FABUSHI_PAY_DEFAULT_PLATFORM_FEE_BPS".into(),
            ));
        }
        Ok(value)
    }

    fn provider_configuration(env: &Env) -> ProviderConfiguration {
        let enabled = |name: &str| {
            env.var(name)
                .ok()
                .map(|value| value.to_string().eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        };
        let text = |name: &str| {
            env.var(name)
                .ok()
                .map(|value| value.to_string())
                .filter(|value| !value.trim().is_empty())
        };
        ProviderConfiguration {
            apple_advanced_commerce_enabled: enabled("APPLE_ADVANCED_COMMERCE_ENABLED"),
            apple_one_time_generic_product_id: text("APPLE_ADVANCED_COMMERCE_ONETIME_PRODUCT_ID"),
            apple_subscription_generic_product_id: text("APPLE_ADVANCED_COMMERCE_SUBSCRIPTION_PRODUCT_ID"),
            google_catalog_sync_enabled: enabled("GOOGLE_PLAY_CATALOG_SYNC_ENABLED"),
        }
    }

    async fn profile_for_user(env: &Env, user_id: &str) -> Result<Option<DeveloperProfileRow>> {
        env.d1(DATABASE_BINDING)?
            .prepare(
                "SELECT developer_id, owner_user_id, display_name, status
                 FROM developer_commerce_profiles WHERE owner_user_id = ?1 LIMIT 1",
            )
            .bind(&[user_id.into()])?
            .first::<DeveloperProfileRow>(None)
            .await
    }

    async fn require_profile(env: &Env, user_id: &str) -> Result<DeveloperProfileRow> {
        profile_for_user(env, user_id)
            .await?
            .ok_or_else(|| worker::Error::RustError("developer profile is not registered".into()))
    }

    async fn require_app_role(
        env: &Env,
        user_id: &str,
        mini_app_id: &str,
        write: bool,
    ) -> Result<OwnedMiniAppRow> {
        let row = env
            .d1(DATABASE_BINDING)?
            .prepare(
                "SELECT o.mini_app_id, o.developer_id, o.display_name, o.status, m.role
                 FROM mini_app_commerce_owners o
                 JOIN mini_app_commerce_members m ON m.mini_app_id = o.mini_app_id
                 WHERE o.mini_app_id = ?1 AND m.user_id = ?2 AND m.active = 1
                   AND o.status = 'active' LIMIT 1",
            )
            .bind(&[mini_app_id.into(), user_id.into()])?
            .first::<OwnedMiniAppRow>(None)
            .await?
            .ok_or_else(|| worker::Error::RustError("Mini App commerce access denied".into()))?;
        if write && !matches!(row.role.as_str(), "owner" | "admin" | "catalog_manager") {
            return Err(worker::Error::RustError(
                "Mini App commerce role is read-only".into(),
            ));
        }
        Ok(row)
    }

    async fn get_profile(request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        Response::from_json(&json!({"profile": profile_for_user(&context.env, &user_id).await?}))
    }

    async fn upsert_profile(mut request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let input: DeveloperProfileRequest = request.json().await?;
        let display_name = input.display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 80 {
            return Response::error("invalid developer display name", 400);
        }
        let existing = profile_for_user(&context.env, &user_id).await?;
        let developer_id = existing
            .as_ref()
            .map(|profile| profile.developer_id.clone())
            .unwrap_or_else(|| format!("dev.{}", Uuid::new_v4().simple()));
        let now = now_seconds();
        context
            .env
            .d1(DATABASE_BINDING)?
            .prepare(
                "INSERT INTO developer_commerce_profiles
                 (developer_id, owner_user_id, display_name, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'active', ?4, ?4)
                 ON CONFLICT(owner_user_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    updated_at = excluded.updated_at",
            )
            .bind(&[
                developer_id.clone().into(),
                user_id.clone().into(),
                display_name.into(),
                now.into(),
            ])?
            .run()
            .await?;
        Response::from_json(&json!({
            "developerId": developer_id,
            "displayName": display_name,
            "status": "active"
        }))
    }

    async fn list_apps(request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let rows = context
            .env
            .d1(DATABASE_BINDING)?
            .prepare(
                "SELECT o.mini_app_id, o.developer_id, o.display_name, o.status, m.role
                 FROM mini_app_commerce_owners o
                 JOIN mini_app_commerce_members m ON m.mini_app_id = o.mini_app_id
                 WHERE m.user_id = ?1 AND m.active = 1 ORDER BY o.created_at DESC",
            )
            .bind(&[user_id.into()])?
            .all()
            .await?
            .results::<Value>()?;
        Response::from_json(&json!({"miniApps": rows}))
    }

    async fn register_app(mut request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let profile = require_profile(&context.env, &user_id).await?;
        let mini_app_id = context
            .param("mini_app_id")
            .ok_or_else(|| worker::Error::RustError("missing mini app id".into()))?;
        if !is_identifier(mini_app_id) {
            return Response::error("invalid mini app id", 400);
        }
        let input: RegisterMiniAppRequest = request.json().await?;
        let display_name = input.display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 80 {
            return Response::error("invalid Mini App display name", 400);
        }
        let now = now_seconds();
        let database = context.env.d1(DATABASE_BINDING)?;
        database
            .batch(vec![
                database
                    .prepare(
                        "INSERT INTO mini_app_commerce_owners
                         (mini_app_id, developer_id, owner_user_id, display_name, status, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
                         ON CONFLICT(mini_app_id) DO UPDATE SET
                           display_name = CASE WHEN owner_user_id = excluded.owner_user_id THEN excluded.display_name ELSE display_name END,
                           updated_at = CASE WHEN owner_user_id = excluded.owner_user_id THEN excluded.updated_at ELSE updated_at END",
                    )
                    .bind(&[
                        mini_app_id.into(),
                        profile.developer_id.clone().into(),
                        user_id.clone().into(),
                        display_name.into(),
                        now.into(),
                    ])?,
                database
                    .prepare(
                        "INSERT INTO mini_app_commerce_members
                         (mini_app_id, user_id, role, active, created_at, updated_at)
                         SELECT ?1, ?2, 'owner', 1, ?3, ?3
                         WHERE EXISTS (
                           SELECT 1 FROM mini_app_commerce_owners
                           WHERE mini_app_id = ?1 AND owner_user_id = ?2
                         )
                         ON CONFLICT(mini_app_id, user_id) DO UPDATE SET
                           role = 'owner', active = 1, updated_at = excluded.updated_at",
                    )
                    .bind(&[mini_app_id.into(), user_id.clone().into(), now.into()])?,
            ])
            .await?;
        let app = require_app_role(&context.env, &user_id, mini_app_id, true).await?;
        Response::from_json(&json!({"miniApp": app}))
    }

    async fn list_products(request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let mini_app_id = context
            .param("mini_app_id")
            .ok_or_else(|| worker::Error::RustError("missing mini app id".into()))?;
        require_app_role(&context.env, &user_id, mini_app_id, false).await?;
        let rows = context
            .env
            .d1(DATABASE_BINDING)?
            .prepare(
                "SELECT c.product_id, c.mini_app_id, c.developer_id, c.sku, c.display_name,
                        c.description, c.product_kind, c.entitlement_capability,
                        c.subscription_period_seconds, c.catalog_status,
                        p.currency, p.amount, p.price_id
                 FROM payment_product_catalog c
                 JOIN prices p ON p.product_id = c.product_id AND p.active = 1
                 WHERE c.mini_app_id = ?1 ORDER BY c.created_at DESC",
            )
            .bind(&[mini_app_id.into()])?
            .all()
            .await?
            .results::<Value>()?;
        Response::from_json(&json!({"products": rows}))
    }

    async fn create_product(mut request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let mini_app_id = context
            .param("mini_app_id")
            .ok_or_else(|| worker::Error::RustError("missing mini app id".into()))?;
        let app = require_app_role(&context.env, &user_id, mini_app_id, true).await?;
        let input: DeveloperProductDraft = request.json().await?;
        validate_product_draft(&input)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let configuration = provider_configuration(&context.env);
        let bindings = plan_provider_bindings(mini_app_id, &input, &configuration)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let product_id = format!("prod.{}", Uuid::new_v4().simple());
        let price_id = format!("price.{}", Uuid::new_v4().simple());
        let revision_id = format!("rev.{}", Uuid::new_v4().simple());
        let event_id = format!("audit.{}", Uuid::new_v4().simple());
        persist_product(
            &context.env,
            &user_id,
            &app.developer_id,
            mini_app_id,
            &product_id,
            &price_id,
            &revision_id,
            &event_id,
            &input,
            &bindings,
            false,
        )
        .await?;
        Response::from_json(&json!({
            "productId": product_id,
            "priceId": price_id,
            "miniAppId": mini_app_id,
            "sku": input.sku,
            "currency": input.currency,
            "amount": input.amount,
            "providerBindings": bindings,
            "pricingAuthority": "fabushi-pay"
        }))
    }

    async fn update_product(mut request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let mini_app_id = context
            .param("mini_app_id")
            .ok_or_else(|| worker::Error::RustError("missing mini app id".into()))?;
        let product_id = context
            .param("product_id")
            .ok_or_else(|| worker::Error::RustError("missing product id".into()))?;
        let app = require_app_role(&context.env, &user_id, mini_app_id, true).await?;
        let input: DeveloperProductDraft = request.json().await?;
        validate_product_draft(&input)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let existing = product_for_owner(&context.env, mini_app_id, product_id).await?;
        if existing.sku != input.sku || existing.product_kind != input.product_kind {
            return Response::error("SKU and product kind are immutable", 409);
        }
        let configuration = provider_configuration(&context.env);
        let bindings = plan_provider_bindings(mini_app_id, &input, &configuration)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let price_id = format!("price.{}", Uuid::new_v4().simple());
        let revision_id = format!("rev.{}", Uuid::new_v4().simple());
        let event_id = format!("audit.{}", Uuid::new_v4().simple());
        persist_product(
            &context.env,
            &user_id,
            &app.developer_id,
            mini_app_id,
            product_id,
            &price_id,
            &revision_id,
            &event_id,
            &input,
            &bindings,
            true,
        )
        .await?;
        Response::from_json(&json!({
            "productId": product_id,
            "priceId": price_id,
            "currency": input.currency,
            "amount": input.amount,
            "providerBindings": bindings,
            "priceRevisionCreated": true
        }))
    }

    async fn set_product_status(mut request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let mini_app_id = context
            .param("mini_app_id")
            .ok_or_else(|| worker::Error::RustError("missing mini app id".into()))?;
        let product_id = context
            .param("product_id")
            .ok_or_else(|| worker::Error::RustError("missing product id".into()))?;
        require_app_role(&context.env, &user_id, mini_app_id, true).await?;
        let input: ProductStatusRequest = request.json().await?;
        if !matches!(input.status.as_str(), "active" | "archived") {
            return Response::error("status must be active or archived", 400);
        }
        product_for_owner(&context.env, mini_app_id, product_id).await?;
        let active = i32::from(input.status == "active");
        let now = now_seconds();
        let database = context.env.d1(DATABASE_BINDING)?;
        database
            .batch(vec![
                database
                    .prepare("UPDATE payment_product_catalog SET catalog_status = ?1, updated_by_user_id = ?2, updated_at = ?3 WHERE product_id = ?4 AND mini_app_id = ?5")
                    .bind(&[input.status.clone().into(), user_id.clone().into(), now.into(), product_id.into(), mini_app_id.into()])?,
                database
                    .prepare("UPDATE products SET active = ?1, updated_at = ?2 WHERE product_id = ?3 AND plugin_id = ?4")
                    .bind(&[active.into(), now.into(), product_id.into(), mini_app_id.into()])?,
                database
                    .prepare("UPDATE payment_product_config SET active = ?1, updated_at = ?2 WHERE product_id = ?3")
                    .bind(&[active.into(), now.into(), product_id.into()])?,
                database
                    .prepare("UPDATE payment_provider_bindings SET sync_state = CASE WHEN ?1 = 1 THEN sync_state ELSE 'archived' END, updated_at = ?2 WHERE product_id = ?3")
                    .bind(&[active.into(), now.into(), product_id.into()])?,
            ])
            .await?;
        Response::from_json(&json!({"productId": product_id, "status": input.status}))
    }

    async fn sync_product(request: Request, context: RouteContext<()>) -> Result<Response> {
        let user_id = authenticated_developer(&request, &context.env)?;
        let mini_app_id = context
            .param("mini_app_id")
            .ok_or_else(|| worker::Error::RustError("missing mini app id".into()))?;
        let product_id = context
            .param("product_id")
            .ok_or_else(|| worker::Error::RustError("missing product id".into()))?;
        require_app_role(&context.env, &user_id, mini_app_id, true).await?;
        let product = product_for_owner(&context.env, mini_app_id, product_id).await?;
        let configuration = provider_configuration(&context.env);
        let draft = DeveloperProductDraft {
            sku: product.sku,
            display_name: product.display_name,
            description: product.description,
            product_kind: product.product_kind,
            entitlement_capability: product.entitlement_capability,
            currency: product.currency,
            amount: product.amount,
            subscription_period_seconds: product.subscription_period_seconds,
            rails: Vec::new(),
        };
        let plans = plan_provider_bindings(mini_app_id, &draft, &configuration)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let database = context.env.d1(DATABASE_BINDING)?;
        let now = now_seconds();
        for plan in &plans {
            database
                .prepare(
                    "INSERT INTO payment_provider_bindings
                     (product_id, provider, external_product_ref, generic_product_id, sync_state,
                      metadata_json, last_error, last_synced_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, '{}', NULL,
                             CASE WHEN ?5 = 'active' THEN ?6 ELSE NULL END, ?6, ?6)
                     ON CONFLICT(product_id, provider) DO UPDATE SET
                       external_product_ref = excluded.external_product_ref,
                       generic_product_id = excluded.generic_product_id,
                       sync_state = excluded.sync_state,
                       last_error = NULL,
                       last_synced_at = excluded.last_synced_at,
                       updated_at = excluded.updated_at",
                )
                .bind(&[
                    product_id.into(),
                    plan.provider.clone().into(),
                    plan.external_product_ref.clone().into(),
                    plan.generic_product_id.clone().into(),
                    plan.sync_state.clone().into(),
                    now.into(),
                ])?
                .run()
                .await?;
        }
        Response::from_json(&json!({
            "productId": product_id,
            "providerBindings": plans,
            "note": "Apple becomes active only when Advanced Commerce entitlement and generic IDs are configured; Google remains pending_sync until Publisher API synchronization succeeds."
        }))
    }

    async fn product_for_owner(env: &Env, mini_app_id: &str, product_id: &str) -> Result<ProductRow> {
        env.d1(DATABASE_BINDING)?
            .prepare(
                "SELECT c.product_id, c.mini_app_id, c.developer_id, c.sku, c.display_name,
                        c.description, c.product_kind, c.entitlement_capability,
                        c.subscription_period_seconds, c.catalog_status,
                        p.currency, p.amount, p.price_id
                 FROM payment_product_catalog c
                 JOIN prices p ON p.product_id = c.product_id AND p.active = 1
                 WHERE c.mini_app_id = ?1 AND c.product_id = ?2 LIMIT 1",
            )
            .bind(&[mini_app_id.into(), product_id.into()])?
            .first::<ProductRow>(None)
            .await?
            .ok_or_else(|| worker::Error::RustError("product not found".into()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_product(
        env: &Env,
        user_id: &str,
        developer_id: &str,
        mini_app_id: &str,
        product_id: &str,
        price_id: &str,
        revision_id: &str,
        event_id: &str,
        input: &DeveloperProductDraft,
        bindings: &[ProviderBindingPlan],
        update: bool,
    ) -> Result<()> {
        let now = now_seconds();
        let fee_bps = default_platform_fee_bps(env)?;
        let rails = normalized_rails(input)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let pay_rails: Vec<String> = rails
            .iter()
            .map(|rail| match rail.as_str() {
                "apple_advanced_commerce" => "apple_in_app_purchase".to_string(),
                "google_play" => "google_play_billing".to_string(),
                other => other.to_string(),
            })
            .collect();
        let provider_refs: BTreeMap<String, String> = bindings
            .iter()
            .filter_map(|binding| {
                let key = match binding.provider.as_str() {
                    "apple_advanced_commerce" => "apple_in_app_purchase",
                    "google_play" => "google_play_billing",
                    other => other,
                };
                binding
                    .external_product_ref
                    .clone()
                    .map(|value| (key.to_string(), value))
            })
            .collect();
        let rails_json = serde_json::to_string(&pay_rails)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let refs_json = serde_json::to_string(&provider_refs)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let consumption_mode = if input.product_kind == "digital_consumable" {
            "consumable"
        } else {
            "durable"
        };
        let database = env.d1(DATABASE_BINDING)?;
        let mut statements = Vec::new();
        if update {
            statements.push(
                database
                    .prepare("UPDATE prices SET active = 0, ends_at = COALESCE(ends_at, ?1) WHERE product_id = ?2 AND active = 1")
                    .bind(&[now.into(), product_id.into()])?,
            );
        }
        statements.extend(vec![
            database
                .prepare(
                    "INSERT INTO payment_product_catalog
                     (product_id, mini_app_id, developer_id, sku, display_name, description,
                      product_kind, entitlement_capability, subscription_period_seconds,
                      catalog_status, created_by_user_id, updated_by_user_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?10, ?11, ?11)
                     ON CONFLICT(product_id) DO UPDATE SET
                       display_name = excluded.display_name,
                       description = excluded.description,
                       entitlement_capability = excluded.entitlement_capability,
                       subscription_period_seconds = excluded.subscription_period_seconds,
                       updated_by_user_id = excluded.updated_by_user_id,
                       updated_at = excluded.updated_at",
                )
                .bind(&[
                    product_id.into(), mini_app_id.into(), developer_id.into(), input.sku.clone().into(),
                    input.display_name.clone().into(), input.description.clone().into(), input.product_kind.clone().into(),
                    input.entitlement_capability.clone().into(), input.subscription_period_seconds.into(), user_id.into(), now.into(),
                ])?,
            database
                .prepare(
                    "INSERT INTO products
                     (product_id, plugin_id, sku, seller_user_id, entitlement_capability,
                      consumption_mode, active, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)
                     ON CONFLICT(product_id) DO UPDATE SET
                       entitlement_capability = excluded.entitlement_capability,
                       active = 1, updated_at = excluded.updated_at",
                )
                .bind(&[
                    product_id.into(), mini_app_id.into(), input.sku.clone().into(), developer_id.into(),
                    input.entitlement_capability.clone().into(), consumption_mode.into(), now.into(),
                ])?,
            database
                .prepare(
                    "INSERT INTO prices
                     (price_id, product_id, currency, amount, active, starts_at, ends_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, 1, ?5, NULL, ?5)",
                )
                .bind(&[price_id.into(), product_id.into(), input.currency.clone().into(), input.amount.into(), now.into()])?,
            database
                .prepare(
                    "INSERT INTO payment_product_config
                     (product_id, developer_id, product_kind, platform_fee_bps,
                      allowed_rails_json, provider_product_refs_json, active, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)
                     ON CONFLICT(product_id) DO UPDATE SET
                       developer_id = excluded.developer_id,
                       product_kind = excluded.product_kind,
                       platform_fee_bps = excluded.platform_fee_bps,
                       allowed_rails_json = excluded.allowed_rails_json,
                       provider_product_refs_json = excluded.provider_product_refs_json,
                       active = 1, updated_at = excluded.updated_at",
                )
                .bind(&[
                    product_id.into(), developer_id.into(), input.product_kind.clone().into(), fee_bps.into(),
                    rails_json.into(), refs_json.into(), now.into(),
                ])?,
            database
                .prepare(
                    "INSERT INTO payment_price_revisions
                     (revision_id, product_id, price_id, currency, amount, actor_user_id, reason, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .bind(&[
                    revision_id.into(), product_id.into(), price_id.into(), input.currency.clone().into(),
                    input.amount.into(), user_id.into(), if update { "developer_update" } else { "developer_create" }.into(), now.into(),
                ])?,
            database
                .prepare(
                    "INSERT INTO developer_commerce_audit_events
                     (event_id, developer_id, mini_app_id, product_id, actor_user_id, event_type, payload_json, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}', ?7)",
                )
                .bind(&[
                    event_id.into(), developer_id.into(), mini_app_id.into(), product_id.into(), user_id.into(),
                    if update { "product.updated" } else { "product.created" }.into(), now.into(),
                ])?,
        ]);
        database.batch(statements).await?;
        for binding in bindings {
            database
                .prepare(
                    "INSERT INTO payment_provider_bindings
                     (product_id, provider, external_product_ref, generic_product_id, sync_state,
                      metadata_json, last_error, last_synced_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, '{}', NULL,
                             CASE WHEN ?5 = 'active' THEN ?6 ELSE NULL END, ?6, ?6)
                     ON CONFLICT(product_id, provider) DO UPDATE SET
                       external_product_ref = excluded.external_product_ref,
                       generic_product_id = excluded.generic_product_id,
                       sync_state = excluded.sync_state,
                       last_error = NULL,
                       last_synced_at = excluded.last_synced_at,
                       updated_at = excluded.updated_at",
                )
                .bind(&[
                    product_id.into(), binding.provider.clone().into(), binding.external_product_ref.clone().into(),
                    binding.generic_product_id.clone().into(), binding.sync_state.clone().into(), now.into(),
                ])?
                .run()
                .await?;
        }
        Ok(())
    }

    #[event(fetch, respond_with_errors)]
    pub async fn main(request: Request, env: Env, _context: Context) -> Result<Response> {
        Router::new()
            .get("/health", |_, _| Response::from_json(&json!({
                "ok": true,
                "service": "fabushi-commerce-control",
                "schema": "fabushi.developer-commerce.v1"
            })))
            .get_async("/v1/developer/commerce/profile", get_profile)
            .post_async("/v1/developer/commerce/profile", upsert_profile)
            .get_async("/v1/developer/commerce/miniapps", list_apps)
            .post_async("/v1/developer/commerce/miniapps/:mini_app_id", register_app)
            .get_async("/v1/developer/commerce/miniapps/:mini_app_id/products", list_products)
            .post_async("/v1/developer/commerce/miniapps/:mini_app_id/products", create_product)
            .post_async("/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id", update_product)
            .post_async("/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id/status", set_product_status)
            .post_async("/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id/sync", sync_product)
            .run(request, env)
            .await
    }
}

#[cfg(target_arch = "wasm32")]
pub use worker_api::main;

#[cfg(test)]
mod tests {
    use super::*;

    fn monthly_cny() -> DeveloperProductDraft {
        DeveloperProductDraft {
            sku: "prayer-wheel.monthly".into(),
            display_name: "本地转经轮月付".into(),
            description: "30 day access".into(),
            product_kind: "subscription".into(),
            entitlement_capability: "local-prayer-wheel".into(),
            currency: "CNY".into(),
            amount: 3000,
            subscription_period_seconds: Some(THIRTY_DAYS_SECONDS),
            rails: vec![],
        }
    }

    #[test]
    fn digital_products_default_to_direct_fiat_store_and_web_rails() {
        let input = monthly_cny();
        validate_product_draft(&input).unwrap();
        assert_eq!(
            normalized_rails(&input).unwrap(),
            vec!["apple_advanced_commerce", "google_play", "web_provider"]
        );
    }

    #[test]
    fn developer_payload_does_not_contain_owner_or_platform_fee_authority() {
        let json = serde_json::to_value(monthly_cny()).unwrap();
        assert!(json.get("developerId").is_none());
        assert!(json.get("ownerUserId").is_none());
        assert!(json.get("platformFeeBps").is_none());
        assert_eq!(json.get("currency").unwrap(), "CNY");
        assert_eq!(json.get("amount").unwrap(), 3000);
    }

    #[test]
    fn credits_are_optional_and_cannot_be_mixed_into_fiat_price() {
        let mut input = monthly_cny();
        input.rails = vec!["credits".into()];
        assert_eq!(
            validate_product_draft(&input),
            Err(CatalogError::CreditsCurrencyMismatch)
        );
    }

    #[test]
    fn provider_state_fails_closed_without_store_configuration() {
        let input = monthly_cny();
        let plans = plan_provider_bindings(
            "global-dharma",
            &input,
            &ProviderConfiguration {
                apple_advanced_commerce_enabled: false,
                apple_one_time_generic_product_id: None,
                apple_subscription_generic_product_id: None,
                google_catalog_sync_enabled: false,
            },
        )
        .unwrap();
        assert_eq!(plans[0].sync_state, "pending_configuration");
        assert_eq!(plans[1].sync_state, "pending_configuration");
        assert_eq!(plans[2].sync_state, "active");
    }

    #[test]
    fn configured_apple_uses_one_generic_subscription_product_for_dynamic_skus() {
        let input = monthly_cny();
        let plans = plan_provider_bindings(
            "global-dharma",
            &input,
            &ProviderConfiguration {
                apple_advanced_commerce_enabled: true,
                apple_one_time_generic_product_id: Some("com.ombhrum.fabushi.miniapp.onetime".into()),
                apple_subscription_generic_product_id: Some("com.ombhrum.fabushi.miniapp.subscription".into()),
                google_catalog_sync_enabled: true,
            },
        )
        .unwrap();
        assert_eq!(plans[0].sync_state, "active");
        assert_eq!(
            plans[0].generic_product_id.as_deref(),
            Some("com.ombhrum.fabushi.miniapp.subscription")
        );
        assert_eq!(plans[1].sync_state, "pending_sync");
    }

    #[test]
    fn apple_partner_sku_and_milliunit_conversion_follow_advanced_commerce_contract() {
        assert_eq!(
            apple_partner_sku("prayer-wheel.monthly", "Fabushi", "official_fabushi").unwrap(),
            "prayer-wheel.monthly|Fabushi|official_fabushi"
        );
        assert_eq!(minor_units_to_apple_milliunits("CNY", 3000).unwrap(), 30_000);
        assert_eq!(minor_units_to_apple_milliunits("JPY", 700).unwrap(), 700_000);
        assert_eq!(minor_units_to_apple_milliunits("KWD", 1500).unwrap(), 1500);
    }

    #[test]
    fn subscriptions_are_fixed_to_current_thirty_day_catalog_contract() {
        let mut input = monthly_cny();
        input.subscription_period_seconds = Some(86_400);
        assert_eq!(
            validate_product_draft(&input),
            Err(CatalogError::InvalidSubscriptionPeriod)
        );
    }
}
