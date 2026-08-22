#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

#[cfg(target_arch = "wasm32")]
mod worker_api {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
    use serde::Deserialize;
    use worker::{Env, Request, Result};

    const ACCESS_TOKEN_ISSUER: &str = "https://api.ombhrum.com";
    const ACCESS_TOKEN_AUDIENCE: &str = "mahayana-platform";

    #[derive(Debug, Deserialize)]
    struct AccessTokenClaims {
        sub: String,
        #[serde(default)]
        scope: Vec<String>,
        token_use: String,
    }

    pub(crate) fn authenticated_user(request: &Request, env: &Env) -> Result<String> {
        let authorization = request
            .headers()
            .get("Authorization")?
            .ok_or_else(|| worker::Error::RustError("missing Authorization header".into()))?;
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| worker::Error::RustError("invalid Authorization header".into()))?;
        let public_key = env.secret("ACCESS_TOKEN_PUBLIC_KEY_PEM")?.to_string();
        let key = DecodingKey::from_rsa_pem(public_key.as_bytes()).map_err(jwt_error)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ACCESS_TOKEN_ISSUER]);
        validation.set_audience(&[ACCESS_TOKEN_AUDIENCE]);
        let claims = decode::<AccessTokenClaims>(token, &key, &validation)
            .map_err(jwt_error)?
            .claims;
        if claims.token_use != "access"
            || !claims
                .scope
                .iter()
                .any(|scope| scope == "commerce.purchase")
        {
            return Err(worker::Error::RustError(
                "access token cannot perform payment operations".into(),
            ));
        }
        Ok(claims.sub)
    }

    fn jwt_error(error: jsonwebtoken::errors::Error) -> worker::Error {
        worker::Error::RustError(format!("invalid account access token: {error}"))
    }
}

#[cfg(target_arch = "wasm32")]
mod payment_api {
    include!("../../mahayana-platform-worker/src/payment_api.rs");

    impl serde::Serialize for PayoutRow {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            use serde::ser::SerializeStruct;
            let mut state = serializer.serialize_struct("PayoutRow", 6)?;
            state.serialize_field("payoutId", &self.payout_id)?;
            state.serialize_field("developerId", &self.developer_id)?;
            state.serialize_field("payoutAccountId", &self.payout_account_id)?;
            state.serialize_field("currency", &self.currency)?;
            state.serialize_field("amount", &self.amount)?;
            state.serialize_field("status", &self.status)?;
            state.end()
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct BoundAppleEnvelope {
        signed_transaction_info: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct BoundAppleTransaction {
        transaction_id: String,
        product_id: String,
        bundle_id: String,
        app_account_token: Option<String>,
        revocation_date: Option<i64>,
    }

    #[derive(Debug, Deserialize)]
    struct WebhookInboxRow {
        state: String,
        payload_sha256: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct AdminProductRequest {
        product_id: String,
        price_id: String,
        mini_app_id: String,
        sku: String,
        developer_id: String,
        capability: String,
        product_kind: String,
        currency: String,
        amount: i64,
        platform_fee_bps: u16,
        allowed_rails: Vec<String>,
        #[serde(default)]
        provider_product_refs: BTreeMap<String, String>,
    }

    pub async fn verify_apple_bound(
        mut request: Request,
        context: RouteContext<()>,
    ) -> Result<Response> {
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
        let (body, _) = fetch_apple_transaction(&input.transaction_id, &jwt).await?;
        let envelope: BoundAppleEnvelope = serde_json::from_slice(&body)
            .map_err(|_| worker::Error::RustError("invalid Apple transaction response".into()))?;
        let payload: BoundAppleTransaction = decode_jws_payload(&envelope.signed_transaction_info)?;
        if payload.bundle_id != bundle_id
            || Some(payload.product_id.as_str()) != payment.provider_product_ref.as_deref()
            || payload.transaction_id != input.transaction_id
            || payload.revocation_date.is_some()
            || payload.app_account_token.as_deref() != Some(payment.payment_id.as_str())
        {
            return error_response(
                403,
                "apple_transaction_mismatch",
                "Apple transaction does not belong to this Payment Intent",
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
        post_external_success(
            &database,
            &payment,
            "apple",
            &payload.transaction_id,
            &event_id,
            now_seconds(),
        )
        .await?;
        mark_webhook_processed(&database, "apple", &event_id, &payment_id, now_seconds()).await?;
        let updated = payment_by_id(&database, &payment_id)
            .await?
            .unwrap_or(payment);
        payment_response(&updated)
    }

    pub async fn verify_google_bound(
        mut request: Request,
        context: RouteContext<()>,
    ) -> Result<Response> {
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
        let response_json: Value = serde_json::from_slice(&body).map_err(|_| {
            worker::Error::RustError("invalid Google Play purchase response".into())
        })?;
        let bound_account = if payment.product_kind == "subscription" {
            response_json
                .get("externalAccountIdentifiers")
                .and_then(|value| value.get("obfuscatedExternalAccountId"))
                .and_then(Value::as_str)
        } else {
            response_json
                .get("obfuscatedExternalAccountId")
                .and_then(Value::as_str)
        };
        if bound_account != Some(payment.payment_id.as_str()) {
            return error_response(
                403,
                "google_purchase_account_mismatch",
                "Google Play purchase does not belong to this Payment Intent",
            );
        }
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
        post_external_success(
            &database,
            &payment,
            "google",
            &provider_reference,
            &event_id,
            now_seconds(),
        )
        .await?;
        mark_webhook_processed(&database, "google", &event_id, &payment_id, now_seconds()).await?;
        let updated = payment_by_id(&database, &payment_id)
            .await?
            .unwrap_or(payment);
        payment_response(&updated)
    }

    pub async fn provider_webhook_bound(
        mut request: Request,
        context: RouteContext<()>,
    ) -> Result<Response> {
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
        let event: NormalizedProviderWebhook = serde_json::from_slice(&body).map_err(|_| {
            worker::Error::RustError("invalid normalized provider webhook JSON".into())
        })?;
        validate_identifier(&event.event_id)?;
        let database = context.env.d1(DATABASE_BINDING)?;
        let occurred_at = event.occurred_at.unwrap_or_else(now_seconds);
        let payment_id = event.payment_id.as_deref().unwrap_or_default();
        let claimed = claim_webhook_event(
            &database,
            &provider,
            &event.event_id,
            &body,
            payment_id,
            now_seconds(),
        )
        .await?;
        if !claimed {
            let existing = worker::query!(
                &database,
                "SELECT state, payload_sha256 FROM payment_webhook_events WHERE provider = ?1 AND event_id = ?2",
                &provider,
                &event.event_id
            )?
            .first::<WebhookInboxRow>(None)
            .await?;
            let Some(existing) = existing else {
                return Err(worker::Error::RustError(
                    "webhook inbox claim disappeared".into(),
                ));
            };
            if existing.payload_sha256 != sha256_hex(&body) {
                return error_response(
                    409,
                    "webhook_event_conflict",
                    "provider event id was reused with a different payload",
                );
            }
            match existing.state.as_str() {
                "processed" | "processing" => {
                    return Response::from_json(&json!({"ok": true, "duplicate": true}));
                }
                "rejected" => {
                    worker::query!(
                        &database,
                        "UPDATE payment_webhook_events SET state = 'processing', processed_at = NULL, error_code = NULL
                         WHERE provider = ?1 AND event_id = ?2 AND state = 'rejected'",
                        &provider,
                        &event.event_id
                    )?
                    .run()
                    .await?;
                }
                _ => {
                    return error_response(
                        409,
                        "webhook_event_busy",
                        "provider event cannot be reclaimed from its current state",
                    );
                }
            }
        }

        let result = if event.event_type == "paymentSucceeded" {
            let payment_id = required_event_value(event.payment_id.as_deref(), "paymentId")?;
            let provider_reference =
                required_event_value(event.provider_reference.as_deref(), "providerReference")?;
            let payment = payment_by_id(&database, payment_id)
                .await?
                .ok_or_else(|| worker::Error::RustError("webhook payment not found".into()))?;
            let expected_rail = if provider == "merchant" {
                "merchant_provider"
            } else {
                "web_provider"
            };
            if payment.rail != expected_rail {
                Err(worker::Error::RustError(
                    "provider rail does not match payment intent".into(),
                ))
            } else {
                post_external_success(
                    &database,
                    &payment,
                    &provider,
                    provider_reference,
                    &event.event_id,
                    occurred_at,
                )
                .await
                .map(|_| Some(payment_id.to_string()))
            }
        } else {
            process_normalized_event(&database, &provider, &event, occurred_at).await
        };
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

    async fn post_external_success(
        database: &worker::D1Database,
        payment: &PaymentIntentRow,
        provider: &str,
        provider_reference: &str,
        event_id: &str,
        occurred_at: i64,
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
        let source_account = format!("provider-clearing:{provider}:{}", payment.currency);
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
                "platform",
                &format!("provider-clearing:{provider}"),
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
            worker::query!(
                database,
                "INSERT INTO orders
                 (order_id, buyer_user_id, plugin_id, product_id, price_id, sku, currency, amount,
                  status, idempotency_key, created_at, updated_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?10
                 FROM payment_intents pi
                 WHERE pi.payment_id = ?11 AND pi.status IN ('created', 'requires_action', 'processing')
                 ON CONFLICT(buyer_user_id, idempotency_key) DO NOTHING",
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
                &payment.payment_id
            )?,
            worker::query!(
                database,
                "INSERT OR IGNORE INTO journal_entries (entry_id, reference_type, reference_id, state, created_at)
                 SELECT ?1, 'payment', ?2, 'draft', ?3 FROM orders WHERE order_id = ?2",
                &entry_id,
                &order_id,
                occurred_at
            )?,
            journal_line_statement(
                database,
                &format!("{entry_id}:source"),
                &entry_id,
                &source_account,
                &payment.currency,
                -payment.amount,
                occurred_at,
            )?,
        ];
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
        statements.extend([
            post_balanced_entry_statement(database, &entry_id, occurred_at)?,
            worker::query!(
                database,
                "UPDATE payment_intents SET status = 'succeeded', provider_reference = ?1, updated_at = ?2
                 WHERE payment_id = ?3 AND status IN ('created', 'requires_action', 'processing')
                   AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id = ?4 AND state = 'posted')",
                provider_reference,
                occurred_at,
                &payment.payment_id,
                &entry_id
            )?,
            worker::query!(
                database,
                "UPDATE orders SET status = 'fulfilled', updated_at = ?1 WHERE order_id = ?2
                   AND EXISTS (SELECT 1 FROM payment_intents WHERE payment_id = ?2 AND status = 'succeeded')",
                occurred_at,
                &order_id
            )?,
            worker::query!(
                database,
                "INSERT OR IGNORE INTO payment_attempts
                 (attempt_id, order_id, provider, provider_event_id, provider_payment_id, status,
                  request_fingerprint, created_at, updated_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, 'captured', ?6, ?7, ?7 FROM orders WHERE order_id = ?2",
                &attempt_id,
                &order_id,
                provider,
                event_id,
                provider_reference,
                sha256_hex(format!("{}:{provider_reference}", payment.payment_id).as_bytes()),
                occurred_at
            )?,
            worker::query!(
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
            )?,
            worker::query!(
                database,
                "INSERT OR IGNORE INTO audit_events
                 (event_id, actor_type, actor_id, event_type, subject_type, subject_id, payload_json, created_at)
                 SELECT ?1, 'system', ?2, 'payment.succeeded', 'payment', ?3, '{}', ?4
                 WHERE EXISTS (SELECT 1 FROM payment_intents WHERE payment_id = ?3 AND status = 'succeeded')",
                &audit_id,
                provider,
                &payment.payment_id,
                occurred_at
            )?,
        ]);
        database.batch(statements).await?;
        let updated = payment_by_id(database, &payment.payment_id)
            .await?
            .ok_or_else(|| worker::Error::RustError("payment disappeared after capture".into()))?;
        if updated.status != "succeeded" {
            return Err(worker::Error::RustError(
                "external payment ledger did not post atomically".into(),
            ));
        }
        Ok(())
    }

    pub async fn admin_upsert_product(
        mut request: Request,
        context: RouteContext<()>,
    ) -> Result<Response> {
        require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
        let input: AdminProductRequest = request.json().await?;
        for value in [
            input.product_id.as_str(),
            input.price_id.as_str(),
            input.mini_app_id.as_str(),
            input.sku.as_str(),
            input.developer_id.as_str(),
            input.capability.as_str(),
            input.currency.as_str(),
        ] {
            validate_identifier(value)?;
        }
        if input.amount <= 0 {
            return error_response(400, "invalid_amount", "product amount must be positive");
        }
        let product_kind = match input.product_kind.as_str() {
            "digitalConsumable" | "digital_consumable" => "digital_consumable",
            "digitalDurable" | "digital_durable" => "digital_durable",
            "subscription" => "subscription",
            "physical" => "physical",
            "service" => "service",
            _ => {
                return error_response(
                    400,
                    "invalid_product_kind",
                    "unsupported payment product kind",
                );
            }
        };
        if input.allowed_rails.is_empty() {
            return error_response(
                400,
                "missing_payment_rails",
                "at least one payment rail must be enabled",
            );
        }
        let mut rails = Vec::with_capacity(input.allowed_rails.len());
        for rail in &input.allowed_rails {
            let normalized = normalize_rail(rail)?.to_string();
            if !rails.contains(&normalized) {
                rails.push(normalized);
            }
        }
        let mut provider_refs = BTreeMap::<String, String>::new();
        for (rail, product_ref) in &input.provider_product_refs {
            validate_identifier(product_ref)?;
            provider_refs.insert(normalize_rail(rail)?.to_string(), product_ref.clone());
        }
        for rail in &rails {
            if rail != "credits" && !provider_refs.contains_key(rail) {
                return error_response(
                    400,
                    "provider_product_missing",
                    "every external payment rail requires a provider product identifier",
                );
            }
        }
        if rails.iter().any(|rail| rail == "credits") && input.currency != CREDITS_CURRENCY {
            return error_response(
                400,
                "credits_currency_mismatch",
                "credits-enabled products must be priced in FBC",
            );
        }

        let rails_json = serde_json::to_string(&rails).map_err(|error| {
            worker::Error::RustError(format!("failed to encode payment rail policy: {error}"))
        })?;
        let provider_refs_json = serde_json::to_string(&provider_refs).map_err(|error| {
            worker::Error::RustError(format!("failed to encode provider product policy: {error}"))
        })?;
        let consumption_mode = if product_kind == "digital_consumable" {
            "consumable"
        } else {
            "durable"
        };
        let database = context.env.d1(DATABASE_BINDING)?;
        let now = now_seconds();
        database
            .batch(vec![
                worker::query!(
                    &database,
                    "INSERT INTO products
                     (product_id, plugin_id, sku, seller_user_id, entitlement_capability,
                      consumption_mode, active, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)
                     ON CONFLICT(product_id) DO UPDATE SET
                       plugin_id = excluded.plugin_id,
                       sku = excluded.sku,
                       seller_user_id = excluded.seller_user_id,
                       entitlement_capability = excluded.entitlement_capability,
                       consumption_mode = excluded.consumption_mode,
                       active = 1,
                       updated_at = excluded.updated_at",
                    &input.product_id,
                    &input.mini_app_id,
                    &input.sku,
                    &input.developer_id,
                    &input.capability,
                    consumption_mode,
                    now
                )?,
                worker::query!(
                    &database,
                    "UPDATE prices SET active = 0, ends_at = COALESCE(ends_at, ?1)
                     WHERE product_id = ?2 AND currency = ?3 AND active = 1 AND price_id <> ?4",
                    now,
                    &input.product_id,
                    &input.currency,
                    &input.price_id
                )?,
                worker::query!(
                    &database,
                    "INSERT INTO prices
                     (price_id, product_id, currency, amount, active, starts_at, ends_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, 1, ?5, NULL, ?5)
                     ON CONFLICT(price_id) DO UPDATE SET
                       product_id = excluded.product_id,
                       currency = excluded.currency,
                       amount = excluded.amount,
                       active = 1,
                       starts_at = excluded.starts_at,
                       ends_at = NULL",
                    &input.price_id,
                    &input.product_id,
                    &input.currency,
                    input.amount,
                    now
                )?,
                worker::query!(
                    &database,
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
                       active = 1,
                       updated_at = excluded.updated_at",
                    &input.product_id,
                    &input.developer_id,
                    product_kind,
                    i64::from(input.platform_fee_bps),
                    &rails_json,
                    &provider_refs_json,
                    now
                )?,
            ])
            .await?;

        Response::from_json(&json!({
            "ok": true,
            "productId": input.product_id,
            "priceId": input.price_id,
            "miniAppId": input.mini_app_id,
            "sku": input.sku,
            "developerId": input.developer_id,
            "productKind": product_kind,
            "currency": input.currency,
            "amount": input.amount,
            "platformFeeBps": input.platform_fee_bps,
            "allowedRails": rails,
            "providerProductRefs": provider_refs,
        }))
    }
}

#[cfg(target_arch = "wasm32")]
use worker::{Context, Env, Request, Response, Result, Router, event};

#[cfg(target_arch = "wasm32")]
#[event(fetch, respond_with_errors)]
pub async fn main(request: Request, env: Env, _context: Context) -> Result<Response> {
    Router::new()
        .get("/health", |_, _| {
            Response::from_json(&serde_json::json!({
                "ok": true,
                "service": "fabushi-pay",
                "schema": "mahayana.miniapp.payment.v1"
            }))
        })
        .post_async(
            "/v1/miniapps/:mini_app_id/pay/intents",
            payment_api::create_intent,
        )
        .get_async("/v1/pay/intents/:payment_id", payment_api::get_intent)
        .post_async(
            "/v1/pay/intents/:payment_id/checkout",
            payment_api::checkout_action,
        )
        .post_async(
            "/v1/pay/intents/:payment_id/credits/confirm",
            payment_api::confirm_credits,
        )
        .post_async(
            "/v1/pay/intents/:payment_id/apple/verify",
            payment_api::verify_apple_bound,
        )
        .post_async(
            "/v1/pay/intents/:payment_id/google/verify",
            payment_api::verify_google_bound,
        )
        .post_async(
            "/v1/pay/providers/:provider/webhook",
            payment_api::provider_webhook_bound,
        )
        .post_async("/v1/pay/admin/products", payment_api::admin_upsert_product)
        .post_async(
            "/v1/pay/admin/settlements/release",
            payment_api::admin_release_settlement,
        )
        .post_async(
            "/v1/pay/admin/payout-accounts",
            payment_api::admin_upsert_payout_account,
        )
        .post_async("/v1/pay/admin/payouts", payment_api::admin_create_payout)
        .get_async(
            "/v1/pay/admin/developers/:developer_id/balance/:currency",
            payment_api::admin_developer_balance,
        )
        .run(request, env)
        .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn payment_service_is_intentionally_standalone() {
        assert_eq!(env!("CARGO_PKG_NAME"), "mahayana-pay-worker");
    }
}
