use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use wasm_bindgen::JsValue;
use worker::{Env, Fetch, Headers, Method, Request, RequestInit, Response, Result, RouteContext};

use crate::worker_api::authenticated_user;

const DATABASE_BINDING: &str = "PLATFORM_DB";
const CREDITS_CURRENCY: &str = "FBC";
const APPLE_PRODUCTION_BASE: &str = "https://api.storekit.apple.com";
const APPLE_SANDBOX_BASE: &str = "https://api.storekit-sandbox.apple.com";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_ANDROID_PUBLISHER_BASE: &str =
    "https://androidpublisher.googleapis.com/androidpublisher/v3";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateIntentRequest {
    sku: String,
    rail: String,
    idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppleVerifyRequest {
    transaction_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoogleVerifyRequest {
    purchase_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminSettlementRequest {
    payment_id: String,
    idempotency_key: String,
    #[serde(default)]
    reserve_bps: Option<u16>,
    #[serde(default)]
    hold_period_seconds: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminPayoutAccountRequest {
    payout_account_id: String,
    developer_id: String,
    provider: String,
    external_account_reference: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminPayoutRequest {
    idempotency_key: String,
    developer_id: String,
    payout_account_id: String,
    currency: String,
    amount: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NormalizedProviderWebhook {
    event_id: String,
    event_type: String,
    #[serde(default)]
    payment_id: Option<String>,
    #[serde(default)]
    provider_reference: Option<String>,
    #[serde(default)]
    refund_reference: Option<String>,
    #[serde(default)]
    dispute_reference: Option<String>,
    #[serde(default)]
    payout_id: Option<String>,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    occurred_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProductPolicyRow {
    product_id: String,
    price_id: String,
    plugin_id: String,
    sku: String,
    capability: String,
    currency: String,
    amount: i64,
    developer_id: String,
    product_kind: String,
    platform_fee_bps: i64,
    allowed_rails_json: String,
    provider_product_refs_json: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PaymentIntentRow {
    payment_id: String,
    idempotency_key: String,
    user_id: String,
    mini_app_id: String,
    developer_id: String,
    product_id: String,
    price_id: String,
    entitlement_capability: String,
    sku: String,
    product_kind: String,
    rail: String,
    provider_product_ref: Option<String>,
    currency: String,
    amount: i64,
    platform_fee_bps: i64,
    status: String,
    provider_reference: Option<String>,
    refunded_amount: i64,
    released_developer_amount: i64,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
struct BalanceOnlyRow {
    balance: i64,
}

#[derive(Debug, Deserialize)]
struct PayoutRow {
    payout_id: String,
    developer_id: String,
    payout_account_id: String,
    currency: String,
    amount: i64,
    status: String,
}

#[derive(Debug, Serialize)]
struct AppleApiClaims<'a> {
    iss: &'a str,
    iat: i64,
    exp: i64,
    aud: &'static str,
    bid: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleTransactionResponse {
    signed_transaction_info: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleTransactionPayload {
    transaction_id: String,
    product_id: String,
    bundle_id: String,
    #[serde(default)]
    revocation_date: Option<i64>,
}

#[derive(Debug, Serialize)]
struct GoogleServiceClaims<'a> {
    iss: &'a str,
    scope: &'static str,
    aud: &'static str,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

pub async fn create_intent(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let mini_app_id = route_identifier(&context, "mini_app_id")?.to_string();
    let input: CreateIntentRequest = request.json().await?;
    validate_identifier(&input.sku)?;
    validate_identifier(&input.idempotency_key)?;
    let rail = normalize_rail(&input.rail)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();

    let Some(product) =
        active_payment_product(&database, &mini_app_id, input.sku.trim(), now).await?
    else {
        return error_response(404, "product_not_found", "payment product is not available");
    };
    let allowed_rails =
        serde_json::from_str::<Vec<String>>(&product.allowed_rails_json).map_err(|_| {
            worker::Error::RustError("invalid payment product rail configuration".into())
        })?;
    if !allowed_rails
        .iter()
        .any(|candidate| normalize_rail(candidate).ok() == Some(rail))
    {
        return error_response(
            409,
            "rail_not_allowed",
            "requested payment rail is not enabled for this product",
        );
    }
    if rail == "credits" && product.currency != CREDITS_CURRENCY {
        return error_response(
            409,
            "credits_currency_mismatch",
            "credits purchases must be priced in FBC",
        );
    }
    let provider_refs =
        serde_json::from_str::<BTreeMap<String, String>>(&product.provider_product_refs_json)
            .map_err(|_| {
                worker::Error::RustError("invalid payment provider product configuration".into())
            })?;
    let provider_product_ref = provider_refs.get(rail).cloned();
    if rail != "credits" && provider_product_ref.as_deref().is_none_or(str::is_empty) {
        return error_response(
            409,
            "provider_product_missing",
            "payment provider product identifier is not configured",
        );
    }

    if let Some(existing) =
        payment_by_idempotency(&database, &user_id, &input.idempotency_key).await?
    {
        if existing.mini_app_id != mini_app_id
            || existing.sku != product.sku
            || existing.rail != rail
            || existing.currency != product.currency
            || existing.amount != product.amount
        {
            return error_response(
                409,
                "idempotency_conflict",
                "idempotency key was reused with different payment semantics",
            );
        }
        return payment_response(&existing);
    }

    let payment_id = uuid::Uuid::new_v4().to_string();
    let status = if rail == "credits" {
        "created"
    } else {
        "requires_action"
    };
    worker::query!(
        &database,
        "INSERT INTO payment_intents
         (payment_id, idempotency_key, user_id, mini_app_id, developer_id, product_id, price_id,
          entitlement_capability, sku, product_kind, rail, provider_product_ref, currency, amount,
          platform_fee_bps, status, refunded_amount, released_developer_amount, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 0, 0, ?17, ?17)",
        &payment_id,
        &input.idempotency_key,
        &user_id,
        &mini_app_id,
        &product.developer_id,
        &product.product_id,
        &product.price_id,
        &product.capability,
        &product.sku,
        &product.product_kind,
        rail,
        provider_product_ref.as_deref(),
        &product.currency,
        product.amount,
        product.platform_fee_bps,
        status,
        now
    )?
    .run()
    .await?;

    let payment = payment_by_id(&database, &payment_id)
        .await?
        .ok_or_else(|| worker::Error::RustError("payment intent was not persisted".into()))?;
    payment_response(&payment)
}

pub async fn get_intent(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let payment_id = route_identifier(&context, "payment_id")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(payment) = payment_by_id(&database, payment_id).await? else {
        return error_response(404, "payment_not_found", "payment intent was not found");
    };
    if payment.user_id != user_id {
        return error_response(404, "payment_not_found", "payment intent was not found");
    }
    payment_response(&payment)
}

pub async fn checkout_action(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let payment_id = route_identifier(&context, "payment_id")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(payment) = payment_by_id(&database, payment_id).await? else {
        return error_response(404, "payment_not_found", "payment intent was not found");
    };
    if payment.user_id != user_id {
        return error_response(404, "payment_not_found", "payment intent was not found");
    }
    if matches!(
        payment.status.as_str(),
        "succeeded" | "partially_refunded" | "refunded"
    ) {
        return payment_response(&payment);
    }
    let action = match payment.rail.as_str() {
        "credits" => json!({
            "kind": "credits",
            "confirmPath": format!("/v1/pay/intents/{}/credits/confirm", payment.payment_id),
        }),
        "apple_in_app_purchase" => json!({
            "kind": "appleInAppPurchase",
            "productId": payment.provider_product_ref,
            "verifyPath": format!("/v1/pay/intents/{}/apple/verify", payment.payment_id),
        }),
        "google_play_billing" => json!({
            "kind": "googlePlayBilling",
            "productId": payment.provider_product_ref,
            "verifyPath": format!("/v1/pay/intents/{}/google/verify", payment.payment_id),
        }),
        "web_provider" | "merchant_provider" => {
            let base = env_string(&context.env, "FABUSHI_PAY_CHECKOUT_URL")?;
            let separator = if base.contains('?') { '&' } else { '?' };
            json!({
                "kind": "redirect",
                "url": format!("{base}{separator}paymentId={}", payment.payment_id),
            })
        }
        _ => return error_response(409, "unsupported_rail", "payment rail is not supported"),
    };
    Response::from_json(&json!({"payment": payment_json(&payment), "checkoutAction": action}))
}

pub async fn confirm_credits(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let payment_id = route_identifier(&context, "payment_id")?.to_string();
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(payment) = payment_by_id(&database, &payment_id).await? else {
        return error_response(404, "payment_not_found", "payment intent was not found");
    };
    if payment.user_id != user_id {
        return error_response(404, "payment_not_found", "payment intent was not found");
    }
    if payment.rail != "credits" || payment.currency != CREDITS_CURRENCY {
        return error_response(
            409,
            "invalid_payment_rail",
            "payment is not an FBC credits purchase",
        );
    }
    if matches!(
        payment.status.as_str(),
        "succeeded" | "partially_refunded" | "refunded"
    ) {
        return payment_response(&payment);
    }
    if !matches!(
        payment.status.as_str(),
        "created" | "requires_action" | "processing"
    ) {
        return error_response(
            409,
            "invalid_payment_state",
            "payment can no longer be confirmed",
        );
    }

    post_success(
        &database,
        &payment,
        "credits",
        &format!("credits:{}", payment.payment_id),
        &payment.payment_id,
        now_seconds(),
        true,
    )
    .await?;
    let updated = payment_by_id(&database, &payment_id)
        .await?
        .ok_or_else(|| {
            worker::Error::RustError("payment disappeared after credits capture".into())
        })?;
    if updated.status != "succeeded" {
        let balance =
            wallet_balance(&database, &format!("user:{user_id}:{}", payment.currency)).await?;
        if balance < payment.amount {
            return error_response(
                402,
                "insufficient_credits",
                "insufficient FBC credits balance",
            );
        }
        return error_response(
            500,
            "ledger_invariant_violation",
            "credits payment could not be posted atomically",
        );
    }
    payment_response(&updated)
}

pub async fn verify_apple(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let payment_id = route_identifier(&context, "payment_id")?.to_string();
    let input: AppleVerifyRequest = request.json().await?;
    validate_identifier(&input.transaction_id)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(payment) = payment_by_id(&database, &payment_id).await? else {
        return error_response(404, "payment_not_found", "payment intent was not found");
    };
    if payment.user_id != user_id || payment.rail != "apple_in_app_purchase" {
        return error_response(
            404,
            "payment_not_found",
            "Apple payment intent was not found",
        );
    }
    if matches!(
        payment.status.as_str(),
        "succeeded" | "partially_refunded" | "refunded"
    ) {
        return payment_response(&payment);
    }

    let bundle_id = env_string(&context.env, "APPLE_BUNDLE_ID")?;
    let jwt = apple_server_jwt(&context.env, &bundle_id)?;
    let (body, payload) = fetch_apple_transaction(&input.transaction_id, &jwt).await?;
    if payload.bundle_id != bundle_id
        || Some(payload.product_id.as_str()) != payment.provider_product_ref.as_deref()
        || payload.transaction_id != input.transaction_id
        || payload.revocation_date.is_some()
    {
        return error_response(
            403,
            "apple_transaction_mismatch",
            "Apple transaction does not match this payment intent",
        );
    }
    let event_id = format!("apple:transaction:{}", payload.transaction_id);
    claim_webhook_event(
        &database,
        "apple",
        &event_id,
        &body,
        &payment_id,
        now_seconds(),
    )
    .await?;
    post_success(
        &database,
        &payment,
        "apple",
        &payload.transaction_id,
        &event_id,
        now_seconds(),
        false,
    )
    .await?;
    mark_webhook_processed(&database, "apple", &event_id, &payment_id, now_seconds()).await?;
    let updated = payment_by_id(&database, &payment_id)
        .await?
        .unwrap_or(payment);
    payment_response(&updated)
}

pub async fn verify_google(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let payment_id = route_identifier(&context, "payment_id")?.to_string();
    let input: GoogleVerifyRequest = request.json().await?;
    validate_identifier(&input.purchase_token)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(payment) = payment_by_id(&database, &payment_id).await? else {
        return error_response(404, "payment_not_found", "payment intent was not found");
    };
    if payment.user_id != user_id || payment.rail != "google_play_billing" {
        return error_response(
            404,
            "payment_not_found",
            "Google Play payment intent was not found",
        );
    }
    if matches!(
        payment.status.as_str(),
        "succeeded" | "partially_refunded" | "refunded"
    ) {
        return payment_response(&payment);
    }
    let expected_product = payment.provider_product_ref.as_deref().ok_or_else(|| {
        worker::Error::RustError("Google Play product identifier is not configured".into())
    })?;
    let access_token = google_access_token(&context.env).await?;
    let package_name = env_string(&context.env, "GOOGLE_PLAY_PACKAGE_NAME")?;
    let (body, provider_reference) = if payment.product_kind == "subscription" {
        verify_google_subscription(
            &package_name,
            &input.purchase_token,
            expected_product,
            &access_token,
        )
        .await?
    } else {
        verify_google_product(
            &package_name,
            &input.purchase_token,
            expected_product,
            &access_token,
        )
        .await?
    };
    let token_hash = sha256_hex(input.purchase_token.as_bytes());
    let event_id = format!("google:purchase:{token_hash}");
    claim_webhook_event(
        &database,
        "google",
        &event_id,
        &body,
        &payment_id,
        now_seconds(),
    )
    .await?;
    post_success(
        &database,
        &payment,
        "google",
        &provider_reference,
        &event_id,
        now_seconds(),
        false,
    )
    .await?;
    mark_webhook_processed(&database, "google", &event_id, &payment_id, now_seconds()).await?;
    let updated = payment_by_id(&database, &payment_id)
        .await?
        .unwrap_or(payment);
    payment_response(&updated)
}

pub async fn provider_webhook(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_WEBHOOK_SECRET")?;
    let provider = route_identifier(&context, "provider")?.to_ascii_lowercase();
    if !matches!(provider.as_str(), "web" | "merchant") {
        return error_response(
            404,
            "provider_not_found",
            "normalized webhook provider is not configured",
        );
    }
    let body = request.bytes().await?;
    let event: NormalizedProviderWebhook = serde_json::from_slice(&body)
        .map_err(|_| worker::Error::RustError("invalid normalized provider webhook JSON".into()))?;
    validate_identifier(&event.event_id)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let occurred_at = event.occurred_at.unwrap_or_else(now_seconds);
    let payment_id = event.payment_id.as_deref();
    let claimed = claim_webhook_event(
        &database,
        &provider,
        &event.event_id,
        &body,
        payment_id.unwrap_or_default(),
        now_seconds(),
    )
    .await?;
    if !claimed {
        return Response::from_json(&json!({"ok": true, "duplicate": true}));
    }

    let result = process_normalized_event(&database, &provider, &event, occurred_at).await;
    match result {
        Ok(payment_id) => {
            mark_webhook_processed(
                &database,
                &provider,
                &event.event_id,
                payment_id.as_deref().unwrap_or_default(),
                now_seconds(),
            )
            .await?;
            Response::from_json(&json!({"ok": true, "paymentId": payment_id}))
        }
        Err(error) => {
            mark_webhook_rejected(
                &database,
                &provider,
                &event.event_id,
                "processing_error",
                now_seconds(),
            )
            .await?;
            Err(error)
        }
    }
}

pub async fn admin_release_settlement(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let input: AdminSettlementRequest = request.json().await?;
    validate_identifier(&input.payment_id)?;
    validate_identifier(&input.idempotency_key)?;
    let reserve_bps = input.reserve_bps.unwrap_or(0);
    if reserve_bps > 10_000 {
        return error_response(
            400,
            "invalid_reserve",
            "reserve basis points must be between 0 and 10000",
        );
    }
    let hold_seconds = input.hold_period_seconds.unwrap_or(7 * 24 * 60 * 60).max(0);
    let database = context.env.d1(DATABASE_BINDING)?;
    let Some(payment) = payment_by_id(&database, &input.payment_id).await? else {
        return error_response(404, "payment_not_found", "payment intent was not found");
    };
    if !matches!(payment.status.as_str(), "succeeded" | "partially_refunded") {
        return error_response(
            409,
            "settlement_not_allowed",
            "payment is not settlement eligible",
        );
    }
    let now = now_seconds();
    if payment.created_at.saturating_add(hold_seconds) > now {
        return error_response(
            409,
            "settlement_hold",
            "payment is still inside the settlement hold period",
        );
    }
    let gross_net = developer_net_after_refunds(&payment);
    let target_released = proportional(gross_net, 10_000u16.saturating_sub(reserve_bps));
    let release_amount = target_released.saturating_sub(payment.released_developer_amount);
    if release_amount <= 0 {
        return error_response(
            409,
            "nothing_to_settle",
            "no additional developer revenue is releasable",
        );
    }
    let release_id = uuid::Uuid::new_v4().to_string();
    let pending_account = developer_pending_account(&payment.developer_id, &payment.currency);
    let available_account = developer_available_account(&payment.developer_id, &payment.currency);
    let entry_id = format!("settlement:{release_id}");
    let statements = vec![
        wallet_account_statement(&database, &pending_account, "developer", &format!("{}:pending", payment.developer_id), &payment.currency, now)?,
        wallet_account_statement(&database, &available_account, "developer", &format!("{}:available", payment.developer_id), &payment.currency, now)?,
        worker::query!(&database,
            "INSERT OR IGNORE INTO developer_settlement_releases
             (release_id, payment_id, idempotency_key, developer_id, currency, amount, released_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            &release_id, &payment.payment_id, &input.idempotency_key, &payment.developer_id,
            &payment.currency, release_amount, now)?,
        worker::query!(&database,
            "INSERT OR IGNORE INTO journal_entries (entry_id, reference_type, reference_id, state, created_at)
             SELECT ?1, 'settlement_release', ?2, 'draft', ?3
             WHERE EXISTS (SELECT 1 FROM developer_settlement_releases WHERE release_id = ?2)",
            &entry_id, &release_id, now)?,
        journal_line_statement(&database, &format!("{entry_id}:pending"), &entry_id, &pending_account, &payment.currency, -release_amount, now)?,
        journal_line_statement(&database, &format!("{entry_id}:available"), &entry_id, &available_account, &payment.currency, release_amount, now)?,
        post_balanced_entry_statement(&database, &entry_id, now)?,
        worker::query!(&database,
            "UPDATE payment_intents
             SET released_developer_amount = released_developer_amount + ?1, updated_at = ?2
             WHERE payment_id = ?3
               AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id = ?4 AND state = 'posted')",
            release_amount, now, &payment.payment_id, &entry_id)?,
    ];
    database.batch(statements).await?;
    Response::from_json(&json!({
        "releaseId": release_id,
        "paymentId": payment.payment_id,
        "amount": release_amount,
        "currency": payment.currency,
        "reserveBps": reserve_bps,
    }))
}

pub async fn admin_upsert_payout_account(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let input: AdminPayoutAccountRequest = request.json().await?;
    for value in [
        &input.payout_account_id,
        &input.developer_id,
        &input.provider,
        &input.external_account_reference,
    ] {
        validate_identifier(value)?;
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    worker::query!(&database,
        "INSERT INTO developer_payout_accounts
         (payout_account_id, developer_id, provider, external_account_reference, state, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
         ON CONFLICT(payout_account_id) DO UPDATE SET
           developer_id = excluded.developer_id,
           provider = excluded.provider,
           external_account_reference = excluded.external_account_reference,
           state = 'active', updated_at = excluded.updated_at",
        &input.payout_account_id, &input.developer_id, &input.provider,
        &input.external_account_reference, now)?.run().await?;
    Response::from_json(&json!({"ok": true, "payoutAccountId": input.payout_account_id}))
}

pub async fn admin_create_payout(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let input: AdminPayoutRequest = request.json().await?;
    for value in [
        &input.idempotency_key,
        &input.developer_id,
        &input.payout_account_id,
        &input.currency,
    ] {
        validate_identifier(value)?;
    }
    if input.amount <= 0 {
        return error_response(400, "invalid_amount", "payout amount must be positive");
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    if let Some(existing) = payout_by_idempotency(&database, &input.idempotency_key).await? {
        if existing.developer_id != input.developer_id
            || existing.currency != input.currency
            || existing.amount != input.amount
        {
            return error_response(
                409,
                "idempotency_conflict",
                "payout idempotency key was reused with different semantics",
            );
        }
        return Response::from_json(&json!({"payout": existing}));
    }
    let active_account = worker::query!(
        &database,
        "SELECT payout_account_id FROM developer_payout_accounts
         WHERE payout_account_id = ?1 AND developer_id = ?2 AND state = 'active'",
        &input.payout_account_id,
        &input.developer_id
    )?
    .first::<Value>(None)
    .await?;
    if active_account.is_none() {
        return error_response(
            409,
            "payout_account_unavailable",
            "developer payout account is not active",
        );
    }
    let available_account = developer_available_account(&input.developer_id, &input.currency);
    if wallet_balance(&database, &available_account).await? < input.amount {
        return error_response(
            409,
            "insufficient_developer_balance",
            "developer available balance is insufficient",
        );
    }
    let payout_id = uuid::Uuid::new_v4().to_string();
    let clearing_account = format!(
        "payout-clearing:{}:{}",
        input.payout_account_id, input.currency
    );
    let entry_id = format!("payout:{payout_id}");
    let now = now_seconds();
    database.batch(vec![
        wallet_account_statement(&database, &available_account, "developer", &format!("{}:available", input.developer_id), &input.currency, now)?,
        wallet_account_statement(&database, &clearing_account, "platform", &format!("payout-clearing:{}", input.payout_account_id), &input.currency, now)?,
        worker::query!(&database,
            "INSERT INTO developer_payouts
             (payout_id, idempotency_key, developer_id, payout_account_id, currency, amount, status, created_at, updated_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?7
             WHERE COALESCE((SELECT balance FROM wallet_balances WHERE account_id = ?8), 0) >= ?6
             ON CONFLICT(idempotency_key) DO NOTHING",
            &payout_id, &input.idempotency_key, &input.developer_id, &input.payout_account_id,
            &input.currency, input.amount, now, &available_account)?,
        worker::query!(&database,
            "INSERT OR IGNORE INTO journal_entries (entry_id, reference_type, reference_id, state, created_at)
             SELECT ?1, 'developer_payout', payout_id, 'draft', ?2 FROM developer_payouts WHERE payout_id = ?3",
            &entry_id, now, &payout_id)?,
        journal_line_statement(&database, &format!("{entry_id}:developer"), &entry_id, &available_account, &input.currency, -input.amount, now)?,
        journal_line_statement(&database, &format!("{entry_id}:clearing"), &entry_id, &clearing_account, &input.currency, input.amount, now)?,
        post_balanced_entry_statement(&database, &entry_id, now)?,
    ]).await?;
    let Some(payout) = payout_by_idempotency(&database, &input.idempotency_key).await? else {
        return error_response(
            409,
            "insufficient_developer_balance",
            "developer available balance changed before payout reservation",
        );
    };
    Response::from_json(&json!({"payout": payout}))
}

pub async fn admin_developer_balance(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let developer_id = route_identifier(&context, "developer_id")?;
    let currency = context
        .param("currency")
        .map(String::as_str)
        .unwrap_or(CREDITS_CURRENCY);
    validate_identifier(developer_id)?;
    validate_identifier(currency)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let pending = wallet_balance(
        &database,
        &developer_pending_account(developer_id, currency),
    )
    .await?;
    let available = wallet_balance(
        &database,
        &developer_available_account(developer_id, currency),
    )
    .await?;
    Response::from_json(
        &json!({"developerId": developer_id, "currency": currency, "pending": pending, "available": available}),
    )
}

async fn process_normalized_event(
    database: &worker::D1Database,
    provider: &str,
    event: &NormalizedProviderWebhook,
    occurred_at: i64,
) -> Result<Option<String>> {
    match event.event_type.as_str() {
        "paymentSucceeded" => {
            let payment_id = required_event_value(event.payment_id.as_deref(), "paymentId")?;
            let provider_reference =
                required_event_value(event.provider_reference.as_deref(), "providerReference")?;
            let payment = payment_by_id(database, payment_id)
                .await?
                .ok_or_else(|| worker::Error::RustError("webhook payment not found".into()))?;
            let expected_rail = if provider == "merchant" {
                "merchant_provider"
            } else {
                "web_provider"
            };
            if payment.rail != expected_rail {
                return Err(worker::Error::RustError(
                    "provider rail does not match payment intent".into(),
                ));
            }
            post_success(
                database,
                &payment,
                provider,
                provider_reference,
                &event.event_id,
                occurred_at,
                false,
            )
            .await?;
            Ok(Some(payment_id.to_string()))
        }
        "paymentFailed" | "paymentCancelled" => {
            let payment_id = required_event_value(event.payment_id.as_deref(), "paymentId")?;
            let status = if event.event_type == "paymentFailed" {
                "failed"
            } else {
                "cancelled"
            };
            worker::query!(database,
                "UPDATE payment_intents SET status = ?1, provider_reference = COALESCE(?2, provider_reference), updated_at = ?3
                 WHERE payment_id = ?4 AND status IN ('created', 'requires_action', 'processing')",
                status, event.provider_reference.as_deref(), occurred_at, payment_id)?.run().await?;
            Ok(Some(payment_id.to_string()))
        }
        "refundSucceeded" => {
            let payment_id = required_event_value(event.payment_id.as_deref(), "paymentId")?;
            let refund_reference =
                required_event_value(event.refund_reference.as_deref(), "refundReference")?;
            let amount = event
                .amount
                .ok_or_else(|| worker::Error::RustError("refund amount is required".into()))?;
            let payment = payment_by_id(database, payment_id)
                .await?
                .ok_or_else(|| worker::Error::RustError("refund payment not found".into()))?;
            apply_refund(
                database,
                &payment,
                provider,
                refund_reference,
                amount,
                &event.event_id,
                occurred_at,
            )
            .await?;
            Ok(Some(payment_id.to_string()))
        }
        "chargebackOpened" => {
            let payment_id = required_event_value(event.payment_id.as_deref(), "paymentId")?;
            let reference =
                required_event_value(event.dispute_reference.as_deref(), "disputeReference")?;
            let amount = event
                .amount
                .ok_or_else(|| worker::Error::RustError("dispute amount is required".into()))?;
            let payment = payment_by_id(database, payment_id)
                .await?
                .ok_or_else(|| worker::Error::RustError("dispute payment not found".into()))?;
            if amount <= 0 || amount > payment.amount.saturating_sub(payment.refunded_amount) {
                return Err(worker::Error::RustError("invalid dispute amount".into()));
            }
            worker::query!(database,
                "INSERT OR IGNORE INTO payment_disputes
                 (dispute_id, payment_id, provider, provider_reference, currency, amount, status, opened_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, ?7)",
                uuid::Uuid::new_v4().to_string(), payment_id, provider, reference, &payment.currency, amount, occurred_at)?.run().await?;
            Ok(Some(payment_id.to_string()))
        }
        "chargebackWon" => {
            let reference =
                required_event_value(event.dispute_reference.as_deref(), "disputeReference")?;
            worker::query!(database,
                "UPDATE payment_disputes SET status = 'won', updated_at = ?1 WHERE provider = ?2 AND provider_reference = ?3 AND status = 'open'",
                occurred_at, provider, reference)?.run().await?;
            Ok(event.payment_id.clone())
        }
        "chargebackLost" => {
            let payment_id = required_event_value(event.payment_id.as_deref(), "paymentId")?;
            let reference =
                required_event_value(event.dispute_reference.as_deref(), "disputeReference")?;
            let payment = payment_by_id(database, payment_id)
                .await?
                .ok_or_else(|| worker::Error::RustError("dispute payment not found".into()))?;
            let amount = event
                .amount
                .unwrap_or_else(|| payment.amount.saturating_sub(payment.refunded_amount));
            apply_refund(
                database,
                &payment,
                provider,
                &format!("chargeback:{reference}"),
                amount,
                &event.event_id,
                occurred_at,
            )
            .await?;
            worker::query!(database,
                "UPDATE payment_disputes SET status = 'lost', updated_at = ?1 WHERE provider = ?2 AND provider_reference = ?3",
                occurred_at, provider, reference)?.run().await?;
            Ok(Some(payment_id.to_string()))
        }
        "payoutPaid" => {
            let payout_id = required_event_value(event.payout_id.as_deref(), "payoutId")?;
            worker::query!(database,
                "UPDATE developer_payouts SET status = 'paid', provider_reference = COALESCE(?1, provider_reference), updated_at = ?2
                 WHERE payout_id = ?3 AND status IN ('pending', 'processing')",
                event.provider_reference.as_deref(), occurred_at, payout_id)?.run().await?;
            Ok(None)
        }
        "payoutFailed" => {
            let payout_id = required_event_value(event.payout_id.as_deref(), "payoutId")?;
            reverse_failed_payout(database, payout_id, occurred_at).await?;
            Ok(None)
        }
        _ => Err(worker::Error::RustError(
            "unsupported normalized provider event type".into(),
        )),
    }
}

async fn post_success(
    database: &worker::D1Database,
    payment: &PaymentIntentRow,
    provider: &str,
    provider_reference: &str,
    event_id: &str,
    occurred_at: i64,
    debit_user: bool,
) -> Result<()> {
    if matches!(
        payment.status.as_str(),
        "succeeded" | "partially_refunded" | "refunded"
    ) {
        if payment.provider_reference.as_deref() == Some(provider_reference) {
            return Ok(());
        }
        return Err(worker::Error::RustError(
            "provider reference conflicts with successful payment".into(),
        ));
    }
    if !matches!(
        payment.status.as_str(),
        "created" | "requires_action" | "processing"
    ) {
        return Err(worker::Error::RustError("payment is not capturable".into()));
    }
    let fee = platform_fee(payment, payment.amount);
    let developer_net = payment.amount.saturating_sub(fee);
    let source_account = if debit_user {
        format!("user:{}:{}", payment.user_id, payment.currency)
    } else {
        format!("provider-clearing:{provider}:{}", payment.currency)
    };
    let source_owner_type = if debit_user { "user" } else { "platform" };
    let source_owner_id = if debit_user {
        payment.user_id.clone()
    } else {
        format!("provider-clearing:{provider}")
    };
    let developer_account = developer_pending_account(&payment.developer_id, &payment.currency);
    let platform_account = format!("platform:payment-revenue:{}", payment.currency);
    let order_id = payment.payment_id.clone();
    let entry_id = format!("payment:{}:capture", payment.payment_id);
    let attempt_id = format!("payment-attempt:{}", payment.payment_id);
    let entitlement_id = format!("payment-entitlement:{}", payment.payment_id);
    let audit_id = format!("payment-audit:{}", payment.payment_id);

    let mut statements = vec![
        wallet_account_statement(
            database,
            &source_account,
            source_owner_type,
            &source_owner_id,
            &payment.currency,
            occurred_at,
        )?,
        wallet_account_statement(
            database,
            &developer_account,
            "developer",
            &format!("{}:pending", payment.developer_id),
            &payment.currency,
            occurred_at,
        )?,
        wallet_account_statement(
            database,
            &platform_account,
            "platform",
            "payment-revenue",
            &payment.currency,
            occurred_at,
        )?,
    ];
    let balance_guard = if debit_user {
        "AND COALESCE((SELECT balance FROM wallet_balances WHERE account_id = ?12), 0) >= ?8"
    } else {
        ""
    };
    let order_sql = format!(
        "INSERT INTO orders
         (order_id, buyer_user_id, plugin_id, product_id, price_id, sku, currency, amount, status, idempotency_key, created_at, updated_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?10
         FROM payment_intents pi
         WHERE pi.payment_id = ?11 AND pi.status IN ('created', 'requires_action', 'processing') {balance_guard}
         ON CONFLICT(buyer_user_id, idempotency_key) DO NOTHING"
    );
    statements.push(worker::query!(
        database,
        &order_sql,
        &order_id,
        &payment.user_id,
        &payment.mini_app_id,
        &payment.product_id,
        &payment.price_id,
        &payment.sku,
        &payment.currency,
        payment.amount,
        &payment.idempotency_key,
        occurred_at,
        &payment.payment_id,
        &source_account
    )?);
    statements.push(worker::query!(database,
        "INSERT OR IGNORE INTO journal_entries (entry_id, reference_type, reference_id, state, created_at)
         SELECT ?1, 'payment', ?2, 'draft', ?3 FROM orders WHERE order_id = ?2",
        &entry_id, &order_id, occurred_at)?);
    statements.push(journal_line_statement(
        database,
        &format!("{entry_id}:source"),
        &entry_id,
        &source_account,
        &payment.currency,
        -payment.amount,
        occurred_at,
    )?);
    if developer_net > 0 {
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:developer"),
            &entry_id,
            &developer_account,
            &payment.currency,
            developer_net,
            occurred_at,
        )?);
    }
    if fee > 0 {
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:platform"),
            &entry_id,
            &platform_account,
            &payment.currency,
            fee,
            occurred_at,
        )?);
    }
    statements.push(post_balanced_entry_statement(
        database,
        &entry_id,
        occurred_at,
    )?);
    statements.push(worker::query!(
        database,
        "UPDATE payment_intents SET status = 'succeeded', provider_reference = ?1, updated_at = ?2
         WHERE payment_id = ?3 AND status IN ('created', 'requires_action', 'processing')
           AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id = ?4 AND state = 'posted')",
        provider_reference,
        occurred_at,
        &payment.payment_id,
        &entry_id
    )?);
    statements.push(worker::query!(database,
        "UPDATE orders SET status = 'fulfilled', updated_at = ?1 WHERE order_id = ?2
           AND EXISTS (SELECT 1 FROM payment_intents WHERE payment_id = ?2 AND status = 'succeeded')",
        occurred_at, &order_id)?);
    statements.push(worker::query!(database,
        "INSERT OR IGNORE INTO payment_attempts
         (attempt_id, order_id, provider, provider_event_id, provider_payment_id, status, request_fingerprint, created_at, updated_at)
         SELECT ?1, ?2, ?3, ?4, ?5, 'captured', ?6, ?7, ?7 FROM orders WHERE order_id = ?2",
        &attempt_id, &order_id, provider, event_id, provider_reference,
        sha256_hex(format!("{}:{provider_reference}", payment.payment_id).as_bytes()), occurred_at)?);
    statements.push(worker::query!(
        database,
        "INSERT OR IGNORE INTO entitlements
         (entitlement_id, user_id, plugin_id, product_id, order_id, capability, status, granted_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7
         FROM payment_intents WHERE payment_id = ?5 AND status = 'succeeded'",
        &entitlement_id,
        &payment.user_id,
        &payment.mini_app_id,
        &payment.product_id,
        &order_id,
        &payment.entitlement_capability,
        occurred_at
    )?);
    statements.push(worker::query!(database,
        "INSERT OR IGNORE INTO audit_events
         (event_id, actor_type, actor_id, event_type, subject_type, subject_id, payload_json, created_at)
         SELECT ?1, 'system', ?2, 'payment.succeeded', 'payment', ?3, '{}', ?4
         WHERE EXISTS (SELECT 1 FROM payment_intents WHERE payment_id = ?3 AND status = 'succeeded')",
        &audit_id, provider, &payment.payment_id, occurred_at)?);
    database.batch(statements).await?;
    Ok(())
}

async fn apply_refund(
    database: &worker::D1Database,
    payment: &PaymentIntentRow,
    provider: &str,
    refund_reference: &str,
    amount: i64,
    event_id: &str,
    occurred_at: i64,
) -> Result<()> {
    if amount <= 0 || amount > payment.amount.saturating_sub(payment.refunded_amount) {
        return Err(worker::Error::RustError(
            "refund exceeds remaining refundable amount".into(),
        ));
    }
    if !matches!(payment.status.as_str(), "succeeded" | "partially_refunded") {
        return Err(worker::Error::RustError("payment is not refundable".into()));
    }
    let new_refunded = payment.refunded_amount.saturating_add(amount);
    let fee_refund = platform_fee(payment, new_refunded)
        .saturating_sub(platform_fee(payment, payment.refunded_amount));
    let developer_refund = amount.saturating_sub(fee_refund);
    let developer_net_before = developer_net_after_refunds(payment);
    let pending_before = developer_net_before
        .saturating_sub(payment.released_developer_amount)
        .max(0);
    let pending_debit = developer_refund.min(pending_before);
    let available_debit = developer_refund.saturating_sub(pending_debit);
    if available_debit > payment.released_developer_amount {
        return Err(worker::Error::RustError(
            "refund accounting invariant was violated".into(),
        ));
    }
    let source_account = if payment.rail == "credits" {
        format!("user:{}:{}", payment.user_id, payment.currency)
    } else {
        format!("provider-clearing:{provider}:{}", payment.currency)
    };
    let source_owner_type = if payment.rail == "credits" {
        "user"
    } else {
        "platform"
    };
    let source_owner_id = if payment.rail == "credits" {
        payment.user_id.clone()
    } else {
        format!("provider-clearing:{provider}")
    };
    let pending_account = developer_pending_account(&payment.developer_id, &payment.currency);
    let available_account = developer_available_account(&payment.developer_id, &payment.currency);
    let platform_account = format!("platform:payment-revenue:{}", payment.currency);
    let refund_id = uuid::Uuid::new_v4().to_string();
    let entry_id = format!("refund:{refund_id}");
    let mut statements = vec![
        wallet_account_statement(database, &source_account, source_owner_type, &source_owner_id, &payment.currency, occurred_at)?,
        wallet_account_statement(database, &pending_account, "developer", &format!("{}:pending", payment.developer_id), &payment.currency, occurred_at)?,
        wallet_account_statement(database, &available_account, "developer", &format!("{}:available", payment.developer_id), &payment.currency, occurred_at)?,
        wallet_account_statement(database, &platform_account, "platform", "payment-revenue", &payment.currency, occurred_at)?,
        worker::query!(database,
            "INSERT OR IGNORE INTO fabushi_payment_refunds
             (refund_id, payment_id, idempotency_key, provider_refund_id, currency, amount, status, reason, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'succeeded', 'provider_refund', ?7, ?7)",
            &refund_id, &payment.payment_id, event_id, refund_reference, &payment.currency, amount, occurred_at)?,
        worker::query!(database,
            "INSERT OR IGNORE INTO journal_entries (entry_id, reference_type, reference_id, state, created_at)
             SELECT ?1, 'payment_refund', ?2, 'draft', ?3
             WHERE EXISTS (SELECT 1 FROM fabushi_payment_refunds WHERE refund_id = ?2)",
            &entry_id, &refund_id, occurred_at)?,
    ];
    if pending_debit > 0 {
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:pending"),
            &entry_id,
            &pending_account,
            &payment.currency,
            -pending_debit,
            occurred_at,
        )?);
    }
    if available_debit > 0 {
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:available"),
            &entry_id,
            &available_account,
            &payment.currency,
            -available_debit,
            occurred_at,
        )?);
    }
    if fee_refund > 0 {
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:platform"),
            &entry_id,
            &platform_account,
            &payment.currency,
            -fee_refund,
            occurred_at,
        )?);
    }
    statements.push(journal_line_statement(
        database,
        &format!("{entry_id}:source"),
        &entry_id,
        &source_account,
        &payment.currency,
        amount,
        occurred_at,
    )?);
    statements.push(post_balanced_entry_statement(
        database,
        &entry_id,
        occurred_at,
    )?);
    let next_status = if new_refunded == payment.amount {
        "refunded"
    } else {
        "partially_refunded"
    };
    statements.push(worker::query!(
        database,
        "UPDATE payment_intents
         SET refunded_amount = ?1, released_developer_amount = released_developer_amount - ?2,
             status = ?3, updated_at = ?4
         WHERE payment_id = ?5
           AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id = ?6 AND state = 'posted')",
        new_refunded,
        available_debit,
        next_status,
        occurred_at,
        &payment.payment_id,
        &entry_id
    )?);
    if next_status == "refunded" {
        statements.push(worker::query!(database,
            "UPDATE entitlements SET status = 'revoked', revoked_at = ?1 WHERE order_id = ?2 AND status = 'active'",
            occurred_at, &payment.payment_id)?);
        statements.push(worker::query!(
            database,
            "UPDATE orders SET status = 'refunded', updated_at = ?1 WHERE order_id = ?2",
            occurred_at,
            &payment.payment_id
        )?);
    }
    database.batch(statements).await?;
    Ok(())
}

async fn reverse_failed_payout(
    database: &worker::D1Database,
    payout_id: &str,
    occurred_at: i64,
) -> Result<()> {
    let Some(payout) = payout_by_id(database, payout_id).await? else {
        return Err(worker::Error::RustError("payout not found".into()));
    };
    if payout.status == "failed" {
        return Ok(());
    }
    if !matches!(payout.status.as_str(), "pending" | "processing") {
        return Err(worker::Error::RustError(
            "payout cannot be failed from its current state".into(),
        ));
    }
    let available_account = developer_available_account(&payout.developer_id, &payout.currency);
    let clearing_account = format!(
        "payout-clearing:{}:{}",
        payout.payout_account_id, payout.currency
    );
    let entry_id = format!("payout-reversal:{payout_id}");
    database.batch(vec![
        wallet_account_statement(database, &available_account, "developer", &format!("{}:available", payout.developer_id), &payout.currency, occurred_at)?,
        wallet_account_statement(database, &clearing_account, "platform", &format!("payout-clearing:{}", payout.payout_account_id), &payout.currency, occurred_at)?,
        worker::query!(database,
            "INSERT OR IGNORE INTO journal_entries (entry_id, reference_type, reference_id, state, created_at)
             VALUES (?1, 'payout_reversal', ?2, 'draft', ?3)", &entry_id, payout_id, occurred_at)?,
        journal_line_statement(database, &format!("{entry_id}:clearing"), &entry_id, &clearing_account, &payout.currency, -payout.amount, occurred_at)?,
        journal_line_statement(database, &format!("{entry_id}:developer"), &entry_id, &available_account, &payout.currency, payout.amount, occurred_at)?,
        post_balanced_entry_statement(database, &entry_id, occurred_at)?,
        worker::query!(database,
            "UPDATE developer_payouts SET status = 'failed', updated_at = ?1 WHERE payout_id = ?2
             AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id = ?3 AND state = 'posted')",
            occurred_at, payout_id, &entry_id)?,
    ]).await?;
    Ok(())
}

async fn active_payment_product(
    database: &worker::D1Database,
    mini_app_id: &str,
    sku: &str,
    now: i64,
) -> Result<Option<ProductPolicyRow>> {
    worker::query!(
        database,
        "SELECT p.product_id, pr.price_id, p.plugin_id, p.sku,
                p.entitlement_capability AS capability, pr.currency, pr.amount,
                pc.developer_id, pc.product_kind, pc.platform_fee_bps,
                pc.allowed_rails_json, pc.provider_product_refs_json
         FROM products p
         JOIN prices pr ON pr.product_id = p.product_id
         JOIN payment_product_config pc ON pc.product_id = p.product_id
         WHERE p.plugin_id = ?1 AND p.sku = ?2 AND p.active = 1 AND pc.active = 1
           AND pr.active = 1 AND pr.starts_at <= ?3 AND (pr.ends_at IS NULL OR pr.ends_at > ?3)
         ORDER BY pr.starts_at DESC LIMIT 1",
        mini_app_id,
        sku,
        now
    )?
    .first::<ProductPolicyRow>(None)
    .await
}

async fn payment_by_id(
    database: &worker::D1Database,
    payment_id: &str,
) -> Result<Option<PaymentIntentRow>> {
    worker::query!(database,
        "SELECT payment_id, idempotency_key, user_id, mini_app_id, developer_id, product_id, price_id,
                entitlement_capability, sku, product_kind, rail, provider_product_ref, currency, amount,
                platform_fee_bps, status, provider_reference, refunded_amount, released_developer_amount,
                created_at, updated_at
         FROM payment_intents WHERE payment_id = ?1", payment_id)?
        .first::<PaymentIntentRow>(None).await
}

async fn payment_by_idempotency(
    database: &worker::D1Database,
    user_id: &str,
    idempotency_key: &str,
) -> Result<Option<PaymentIntentRow>> {
    worker::query!(database,
        "SELECT payment_id, idempotency_key, user_id, mini_app_id, developer_id, product_id, price_id,
                entitlement_capability, sku, product_kind, rail, provider_product_ref, currency, amount,
                platform_fee_bps, status, provider_reference, refunded_amount, released_developer_amount,
                created_at, updated_at
         FROM payment_intents WHERE user_id = ?1 AND idempotency_key = ?2", user_id, idempotency_key)?
        .first::<PaymentIntentRow>(None).await
}

async fn payout_by_idempotency(
    database: &worker::D1Database,
    key: &str,
) -> Result<Option<PayoutRow>> {
    worker::query!(
        database,
        "SELECT payout_id, developer_id, payout_account_id, currency, amount, status
         FROM developer_payouts WHERE idempotency_key = ?1",
        key
    )?
    .first::<PayoutRow>(None)
    .await
}

async fn payout_by_id(database: &worker::D1Database, payout_id: &str) -> Result<Option<PayoutRow>> {
    worker::query!(
        database,
        "SELECT payout_id, developer_id, payout_account_id, currency, amount, status
         FROM developer_payouts WHERE payout_id = ?1",
        payout_id
    )?
    .first::<PayoutRow>(None)
    .await
}

async fn wallet_balance(database: &worker::D1Database, account_id: &str) -> Result<i64> {
    Ok(worker::query!(
        database,
        "SELECT balance FROM wallet_balances WHERE account_id = ?1",
        account_id
    )?
    .first::<BalanceOnlyRow>(None)
    .await?
    .map(|row| row.balance)
    .unwrap_or(0))
}

async fn claim_webhook_event(
    database: &worker::D1Database,
    provider: &str,
    event_id: &str,
    body: &[u8],
    payment_id: &str,
    received_at: i64,
) -> Result<bool> {
    let payload_sha256 = sha256_hex(body);
    let result = worker::query!(
        database,
        "INSERT OR IGNORE INTO payment_webhook_events
         (provider, event_id, payload_sha256, state, payment_id, received_at)
         VALUES (?1, ?2, ?3, 'processing', NULLIF(?4, ''), ?5)",
        provider,
        event_id,
        &payload_sha256,
        payment_id,
        received_at
    )?
    .run()
    .await?;
    Ok(d1_changes(&result) > 0)
}

async fn mark_webhook_processed(
    database: &worker::D1Database,
    provider: &str,
    event_id: &str,
    payment_id: &str,
    processed_at: i64,
) -> Result<()> {
    worker::query!(database,
        "UPDATE payment_webhook_events SET state = 'processed', payment_id = COALESCE(NULLIF(?1, ''), payment_id),
                processed_at = ?2, error_code = NULL WHERE provider = ?3 AND event_id = ?4",
        payment_id, processed_at, provider, event_id)?.run().await?;
    Ok(())
}

async fn mark_webhook_rejected(
    database: &worker::D1Database,
    provider: &str,
    event_id: &str,
    error_code: &str,
    processed_at: i64,
) -> Result<()> {
    worker::query!(
        database,
        "UPDATE payment_webhook_events SET state = 'rejected', processed_at = ?1, error_code = ?2
         WHERE provider = ?3 AND event_id = ?4",
        processed_at,
        error_code,
        provider,
        event_id
    )?
    .run()
    .await?;
    Ok(())
}

fn wallet_account_statement<'a>(
    database: &'a worker::D1Database,
    account_id: &'a str,
    owner_type: &'a str,
    owner_id: &'a str,
    currency: &'a str,
    now: i64,
) -> Result<worker::D1PreparedStatement> {
    worker::query!(database,
        "INSERT OR IGNORE INTO wallet_accounts (account_id, owner_type, owner_id, currency, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)", account_id, owner_type, owner_id, currency, now)
}

fn journal_line_statement<'a>(
    database: &'a worker::D1Database,
    line_id: &'a str,
    entry_id: &'a str,
    account_id: &'a str,
    currency: &'a str,
    amount: i64,
    now: i64,
) -> Result<worker::D1PreparedStatement> {
    worker::query!(database,
        "INSERT OR IGNORE INTO journal_lines (line_id, entry_id, account_id, currency, amount, created_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6
         WHERE ?5 <> 0 AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id = ?2 AND state = 'draft')",
        line_id, entry_id, account_id, currency, amount, now)
}

fn post_balanced_entry_statement<'a>(
    database: &'a worker::D1Database,
    entry_id: &'a str,
    now: i64,
) -> Result<worker::D1PreparedStatement> {
    worker::query!(database,
        "UPDATE journal_entries SET state = 'posted', posted_at = ?1
         WHERE entry_id = ?2 AND state = 'draft'
           AND (SELECT COUNT(*) FROM journal_lines WHERE entry_id = ?2) >= 2
           AND NOT EXISTS (
             SELECT currency FROM journal_lines WHERE entry_id = ?2 GROUP BY currency HAVING SUM(amount) <> 0
           )", now, entry_id)
}

fn payment_json(payment: &PaymentIntentRow) -> Value {
    json!({
        "schema": "mahayana.miniapp.payment.v1",
        "paymentId": payment.payment_id,
        "idempotencyKey": payment.idempotency_key,
        "miniAppId": payment.mini_app_id,
        "sku": payment.sku,
        "productKind": payment.product_kind,
        "rail": rail_api_name(&payment.rail),
        "amount": payment.amount,
        "currency": payment.currency,
        "status": status_api_name(&payment.status),
        "providerReference": payment.provider_reference,
        "refundedAmount": payment.refunded_amount,
        "createdAt": payment.created_at,
        "updatedAt": payment.updated_at,
    })
}

fn payment_response(payment: &PaymentIntentRow) -> Result<Response> {
    Response::from_json(&payment_json(payment))
}

fn platform_fee(payment: &PaymentIntentRow, gross: i64) -> i64 {
    proportional(
        gross,
        u16::try_from(payment.platform_fee_bps).unwrap_or(10_000),
    )
}

fn developer_net_after_refunds(payment: &PaymentIntentRow) -> i64 {
    let full_net = payment
        .amount
        .saturating_sub(platform_fee(payment, payment.amount));
    let refunded_net = payment
        .refunded_amount
        .saturating_sub(platform_fee(payment, payment.refunded_amount));
    full_net.saturating_sub(refunded_net)
}

fn proportional(amount: i64, bps: u16) -> i64 {
    let value = i128::from(amount) * i128::from(bps) / 10_000;
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn developer_pending_account(developer_id: &str, currency: &str) -> String {
    format!("developer-pending:{developer_id}:{currency}")
}

fn developer_available_account(developer_id: &str, currency: &str) -> String {
    format!("developer-available:{developer_id}:{currency}")
}

fn normalize_rail(value: &str) -> Result<&'static str> {
    match value.trim() {
        "credits" => Ok("credits"),
        "appleInAppPurchase" | "apple_in_app_purchase" => Ok("apple_in_app_purchase"),
        "googlePlayBilling" | "google_play_billing" => Ok("google_play_billing"),
        "webProvider" | "web_provider" => Ok("web_provider"),
        "merchantProvider" | "merchant_provider" => Ok("merchant_provider"),
        _ => Err(worker::Error::RustError("unsupported payment rail".into())),
    }
}

fn rail_api_name(value: &str) -> &str {
    match value {
        "apple_in_app_purchase" => "appleInAppPurchase",
        "google_play_billing" => "googlePlayBilling",
        "web_provider" => "webProvider",
        "merchant_provider" => "merchantProvider",
        other => other,
    }
}

fn status_api_name(value: &str) -> &str {
    match value {
        "requires_action" => "requiresAction",
        "partially_refunded" => "partiallyRefunded",
        other => other,
    }
}

fn validate_identifier(value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return Err(worker::Error::RustError(
            "invalid payment identifier".into(),
        ));
    }
    Ok(())
}

fn required_event_value<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str> {
    let value = value
        .ok_or_else(|| worker::Error::RustError(format!("provider event is missing {field}")))?;
    validate_identifier(value)?;
    Ok(value)
}

fn route_identifier<'a>(context: &'a RouteContext<()>, name: &str) -> Result<&'a str> {
    context
        .param(name)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| worker::Error::RustError(format!("missing route identifier {name}")))
}

fn require_bearer_secret(request: &Request, env: &Env, name: &str) -> Result<()> {
    let expected = env_string(env, name)?;
    let authorization = request.headers().get("Authorization")?.unwrap_or_default();
    if authorization.strip_prefix("Bearer ") != Some(expected.as_str()) {
        return Err(worker::Error::RustError(
            "unauthorized payment service request".into(),
        ));
    }
    Ok(())
}

fn env_string(env: &Env, name: &str) -> Result<String> {
    if let Ok(value) = env.secret(name) {
        let value = value.to_string();
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    if let Ok(value) = env.var(name) {
        let value = value.to_string();
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    Err(worker::Error::RustError(format!(
        "missing required payment configuration {name}"
    )))
}

fn now_seconds() -> i64 {
    (worker::Date::now().as_millis() / 1000.0) as i64
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn d1_changes(result: &worker::D1Result) -> usize {
    result
        .meta()
        .ok()
        .flatten()
        .and_then(|meta| meta.changes)
        .unwrap_or(0) as usize
}

fn error_response(status: u16, code: &str, message: &str) -> Result<Response> {
    Ok(Response::from_json(&json!({"error": code, "message": message}))?.with_status(status))
}

fn apple_server_jwt(env: &Env, bundle_id: &str) -> Result<String> {
    let issuer_id = env_string(env, "APPLE_ISSUER_ID")?;
    let key_id = env_string(env, "APPLE_KEY_ID")?;
    let private_key = env_string(env, "APPLE_PRIVATE_KEY")?.replace("\\n", "\n");
    let now = now_seconds();
    let claims = AppleApiClaims {
        iss: &issuer_id,
        iat: now,
        exp: now + 20 * 60,
        aud: "appstoreconnect-v1",
        bid: bundle_id,
    };
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key_id);
    header.typ = Some("JWT".into());
    let key = EncodingKey::from_ec_pem(private_key.as_bytes())
        .map_err(|error| worker::Error::RustError(format!("invalid Apple private key: {error}")))?;
    encode(&header, &claims, &key).map_err(|error| {
        worker::Error::RustError(format!("failed to sign Apple server JWT: {error}"))
    })
}

async fn fetch_apple_transaction(
    transaction_id: &str,
    jwt: &str,
) -> Result<(Vec<u8>, AppleTransactionPayload)> {
    let mut last_status = 0;
    for base in [APPLE_PRODUCTION_BASE, APPLE_SANDBOX_BASE] {
        let url = format!("{base}/inApps/v1/transactions/{transaction_id}");
        let mut outbound = Request::new(&url, Method::Get)?;
        outbound
            .headers_mut()?
            .set("Authorization", &format!("Bearer {jwt}"))?;
        outbound.headers_mut()?.set("Accept", "application/json")?;
        let mut response = Fetch::Request(outbound).send().await?;
        last_status = response.status_code();
        let body = response.bytes().await?;
        if last_status == 200 {
            let envelope: AppleTransactionResponse =
                serde_json::from_slice(&body).map_err(|_| {
                    worker::Error::RustError("invalid Apple transaction response".into())
                })?;
            let payload: AppleTransactionPayload =
                decode_jws_payload(&envelope.signed_transaction_info)?;
            return Ok((body, payload));
        }
        if last_status != 404 {
            break;
        }
    }
    Err(worker::Error::RustError(format!(
        "Apple transaction verification failed with HTTP {last_status}"
    )))
}

fn decode_jws_payload<T: DeserializeOwned>(jws: &str) -> Result<T> {
    let payload = jws
        .split('.')
        .nth(1)
        .ok_or_else(|| worker::Error::RustError("invalid JWS payload".into()))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .map_err(|_| worker::Error::RustError("invalid base64url JWS payload".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| worker::Error::RustError("invalid JSON inside JWS payload".into()))
}

async fn google_access_token(env: &Env) -> Result<String> {
    let service_account_email = env_string(env, "GOOGLE_PLAY_SERVICE_ACCOUNT_EMAIL")?;
    let private_key = env_string(env, "GOOGLE_PLAY_PRIVATE_KEY")?.replace("\\n", "\n");
    let now = now_seconds();
    let claims = GoogleServiceClaims {
        iss: &service_account_email,
        scope: "https://www.googleapis.com/auth/androidpublisher",
        aud: GOOGLE_OAUTH_TOKEN_URL,
        iat: now,
        exp: now + 60 * 60,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("JWT".into());
    let key = EncodingKey::from_rsa_pem(private_key.as_bytes()).map_err(|error| {
        worker::Error::RustError(format!("invalid Google Play private key: {error}"))
    })?;
    let assertion = encode(&header, &claims, &key).map_err(|error| {
        worker::Error::RustError(format!("failed to sign Google service JWT: {error}"))
    })?;
    let form = format!(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={assertion}"
    );
    let headers = Headers::new();
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;
    headers.set("Accept", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(&form)));
    let outbound = Request::new_with_init(GOOGLE_OAUTH_TOKEN_URL, &init)?;
    let mut response = Fetch::Request(outbound).send().await?;
    let status = response.status_code();
    let body = response.bytes().await?;
    if status != 200 {
        return Err(worker::Error::RustError(format!(
            "Google OAuth token exchange failed with HTTP {status}"
        )));
    }
    let token: GoogleTokenResponse = serde_json::from_slice(&body)
        .map_err(|_| worker::Error::RustError("invalid Google OAuth token response".into()))?;
    Ok(token.access_token)
}

async fn verify_google_product(
    package_name: &str,
    purchase_token: &str,
    product_id: &str,
    access_token: &str,
) -> Result<(Vec<u8>, String)> {
    let url = format!(
        "{GOOGLE_ANDROID_PUBLISHER_BASE}/applications/{package_name}/purchases/products/{product_id}/tokens/{purchase_token}"
    );
    let body = google_authorized_get(&url, access_token).await?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| worker::Error::RustError("invalid Google Play product response".into()))?;
    if value.get("purchaseState").and_then(Value::as_i64) != Some(0) {
        return Err(worker::Error::RustError(
            "Google Play product purchase is not in purchased state".into(),
        ));
    }
    let order_id = value
        .get("orderId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            worker::Error::RustError("Google Play product response is missing orderId".into())
        })?;
    Ok((body, order_id.to_string()))
}

async fn verify_google_subscription(
    package_name: &str,
    purchase_token: &str,
    expected_product_id: &str,
    access_token: &str,
) -> Result<(Vec<u8>, String)> {
    let url = format!(
        "{GOOGLE_ANDROID_PUBLISHER_BASE}/applications/{package_name}/purchases/subscriptionsv2/tokens/{purchase_token}"
    );
    let body = google_authorized_get(&url, access_token).await?;
    let value: Value = serde_json::from_slice(&body).map_err(|_| {
        worker::Error::RustError("invalid Google Play subscription response".into())
    })?;
    let state = value
        .get("subscriptionState")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(
        state,
        "SUBSCRIPTION_STATE_ACTIVE"
            | "SUBSCRIPTION_STATE_IN_GRACE_PERIOD"
            | "SUBSCRIPTION_STATE_CANCELED"
    ) {
        return Err(worker::Error::RustError(
            "Google Play subscription is not entitlement eligible".into(),
        ));
    }
    let has_product = value
        .get("lineItems")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("productId").and_then(Value::as_str) == Some(expected_product_id)
            })
        });
    if !has_product {
        return Err(worker::Error::RustError(
            "Google Play subscription product does not match payment intent".into(),
        ));
    }
    let order_id = value
        .get("latestOrderId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            worker::Error::RustError(
                "Google Play subscription response is missing latestOrderId".into(),
            )
        })?;
    Ok((body, order_id.to_string()))
}

async fn google_authorized_get(url: &str, access_token: &str) -> Result<Vec<u8>> {
    let mut outbound = Request::new(url, Method::Get)?;
    outbound
        .headers_mut()?
        .set("Authorization", &format!("Bearer {access_token}"))?;
    outbound.headers_mut()?.set("Accept", "application/json")?;
    let mut response = Fetch::Request(outbound).send().await?;
    let status = response.status_code();
    let body = response.bytes().await?;
    if status != 200 {
        return Err(worker::Error::RustError(format!(
            "Google Play purchase verification failed with HTTP {status}"
        )));
    }
    Ok(body)
}
