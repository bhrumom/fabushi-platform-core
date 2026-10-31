include!("original_order_split.rs");
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminSettlementReconciliationRequest {
    payment_id: String,
    idempotency_key: String,
    region_code: String,
    settlement_source: String,
    #[serde(default)]
    tax_amount: i64,
    #[serde(default)]
    provider_fee_amount: i64,
    #[serde(default)]
    chargeback_amount: i64,
    #[serde(default)]
    reserve_bps: u16,
    #[serde(default)]
    reserve_hold_seconds: i64,
    #[serde(default)]
    provider_settlement_reference: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminPayoutAccountV2Request {
    payout_account_id: String,
    developer_id: String,
    provider: String,
    external_account_reference: String,
    country_code: String,
    legal_entity_type: String,
    onboarding_state: String,
    kyc_status: String,
    payouts_enabled: bool,
    currencies: Vec<String>,
    purposes: Vec<String>,
    #[serde(default)]
    provider_metadata: Value,
    #[serde(default)]
    is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct PayoutCapabilityRowV2 {
    payout_account_id: String,
    developer_id: String,
    provider: String,
    external_account_reference: String,
    country_code: String,
    state: String,
    onboarding_state: String,
    kyc_status: String,
    payouts_enabled: i64,
    currencies_json: String,
    purposes_json: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PayoutDispatchRow {
    payout_id: String,
    idempotency_key: String,
    developer_id: String,
    payout_account_id: String,
    currency: String,
    amount: i64,
    status: String,
    provider: String,
    external_account_reference: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PayoutAttemptStateRow {
    attempt_id: String,
    state: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdminPayoutRouteStateRequest {
    state: String,
}

fn supported_payout_provider(provider: &str) -> bool {
    matches!(
        provider,
        "stripe_connect"
            | "adyen_platform"
            | "paypal_multiparty"
            | "paypal_payouts"
            | "wechat_platform"
            | "alipay_platform"
            | "lianlian_account_plus"
            | "huifu_dougong"
    )
}

fn valid_country_code(value: &str) -> bool {
    value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn valid_iso_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn valid_payout_purpose(value: &str) -> bool {
    matches!(
        value,
        "original_order_split" | "external_proceeds_payout" | "marketplace_payout"
    )
}

fn payout_executor_env(provider: &str) -> Option<&'static str> {
    match provider {
        "stripe_connect" => Some("PAYOUT_STRIPE_CONNECT_EXECUTOR_URL"),
        "adyen_platform" => Some("PAYOUT_ADYEN_PLATFORM_EXECUTOR_URL"),
        "paypal_multiparty" => Some("PAYOUT_PAYPAL_MULTIPARTY_EXECUTOR_URL"),
        "paypal_payouts" => Some("PAYOUT_PAYPAL_PAYOUTS_EXECUTOR_URL"),
        "wechat_platform" => Some("PAYOUT_WECHAT_PLATFORM_EXECUTOR_URL"),
        "alipay_platform" => Some("PAYOUT_ALIPAY_PLATFORM_EXECUTOR_URL"),
        "lianlian_account_plus" => Some("PAYOUT_LIANLIAN_ACCOUNT_PLUS_EXECUTOR_URL"),
        "huifu_dougong" => Some("PAYOUT_HUIFU_DOUGONG_EXECUTOR_URL"),
        _ => None,
    }
}

fn developer_reserved_account(developer_id: &str, currency: &str) -> String {
    format!("developer-reserved:{developer_id}:{currency}")
}

pub async fn admin_reconcile_settlement(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let input: AdminSettlementReconciliationRequest = request.json().await?;
    validate_identifier(&input.payment_id)?;
    validate_identifier(&input.idempotency_key)?;
    if !matches!(input.region_code.as_str(), "CN" | "GLOBAL") {
        return error_response(400, "invalid_region", "regionCode must be CN or GLOBAL");
    }
    if !matches!(
        input.settlement_source.as_str(),
        "wechat_order"
            | "alipay_order"
            | "apple_store_proceeds"
            | "google_store_proceeds"
            | "web_marketplace"
            | "other_external_proceeds"
    ) {
        return error_response(
            400,
            "invalid_settlement_source",
            "unsupported settlement source",
        );
    }
    if input.tax_amount < 0 || input.provider_fee_amount < 0 || input.chargeback_amount < 0 {
        return error_response(
            400,
            "invalid_settlement_amount",
            "settlement deductions must be non-negative",
        );
    }
    let reserve_hold_seconds = if input.reserve_bps > 0 && input.reserve_hold_seconds == 0 {
        604800
    } else {
        input.reserve_hold_seconds
    };
    if reserve_hold_seconds < 0 || reserve_hold_seconds > 7_776_000 {
        return error_response(
            400,
            "invalid_reserve_hold",
            "reserveHoldSeconds must be between 0 and 90 days",
        );
    }
    if input.reserve_bps > 10_000 {
        return error_response(
            400,
            "invalid_reserve",
            "reserve basis points must be between 0 and 10000",
        );
    }
    if let Some(reference) = input.provider_settlement_reference.as_deref() {
        validate_identifier(reference)?;
    }

    let database = context.env.d1(DATABASE_BINDING)?;
    if let Some(existing) = worker::query!(
        &database,
        "SELECT reconciliation_id,payment_id,developer_id,currency,gross_amount,tax_amount,provider_fee_amount,refund_amount,chargeback_amount,net_receipts,platform_fee_amount,reserve_amount,developer_payable_amount,status FROM developer_settlement_reconciliations WHERE idempotency_key=?1 LIMIT 1",
        &input.idempotency_key
    )?
    .first::<Value>(None)
    .await?
    {
        return Response::from_json(&json!({"reconciliation":existing,"duplicate":true}));
    }

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
    if payment.released_developer_amount != 0 {
        return error_response(
            409,
            "settlement_already_released",
            "fiat settlement must be reconciled before any developer release",
        );
    }

    let gross_after_refunds = payment.amount.saturating_sub(payment.refunded_amount);
    let deductions = input
        .tax_amount
        .saturating_add(input.provider_fee_amount)
        .saturating_add(input.chargeback_amount);
    if deductions > gross_after_refunds {
        return error_response(
            400,
            "settlement_deductions_exceed_receipts",
            "tax/provider/chargeback deductions exceed remaining gross receipts",
        );
    }
    let net_receipts = gross_after_refunds - deductions;
    let platform_bps = u16::try_from(payment.platform_fee_bps).unwrap_or(10_000);
    let desired_platform_fee = proportional(net_receipts, platform_bps);
    let current_platform_fee = platform_fee(&payment, payment.amount)
        .saturating_sub(platform_fee(&payment, payment.refunded_amount));
    let platform_fee_reduction = current_platform_fee.saturating_sub(desired_platform_fee);
    let current_developer_net = developer_net_after_refunds(&payment);
    let developer_before_reserve = net_receipts.saturating_sub(desired_platform_fee);
    let developer_adjustment = current_developer_net.saturating_sub(developer_before_reserve);
    if developer_adjustment.saturating_add(platform_fee_reduction) != deductions {
        return error_response(
            500,
            "settlement_invariant_violation",
            "settlement waterfall is not balanced",
        );
    }
    let reserve_amount = proportional(developer_before_reserve, input.reserve_bps);
    let developer_payable = developer_before_reserve.saturating_sub(reserve_amount);
    let pending_account = developer_pending_account(&payment.developer_id, &payment.currency);
    let available_account = developer_available_account(&payment.developer_id, &payment.currency);
    let reserved_account = developer_reserved_account(&payment.developer_id, &payment.currency);
    let platform_account = format!("platform:payment-revenue:{}", payment.currency);
    let provider_cost_account = format!("platform:provider-cost:{}", payment.currency);
    let tax_account = format!("platform:tax-liability:{}", payment.currency);
    let chargeback_account = format!("platform:chargeback-cost:{}", payment.currency);
    let reconciliation_id = uuid::Uuid::new_v4().to_string();
    let adjustment_entry = format!("settlement-adjustment:{reconciliation_id}");
    let release_entry = format!("settlement-release:{reconciliation_id}");
    let release_id = format!("reconciliation-release:{reconciliation_id}");
    let now = now_seconds();
    let reserve_release_after = now.saturating_add(reserve_hold_seconds);

    let original_split = if matches!(
        input.settlement_source.as_str(),
        "wechat_order" | "alipay_order"
    ) && developer_payable > 0
    {
        Some(
            execute_original_order_split(
                &context.env,
                &database,
                &payment,
                &input,
                developer_payable,
                desired_platform_fee,
            )
            .await?,
        )
    } else {
        None
    };
    let paid_account = developer_paid_account(&payment.developer_id, &payment.currency);

    let mut statements = vec![
        wallet_account_statement(&database, &pending_account, "developer", &format!("{}:pending", payment.developer_id), &payment.currency, now)?,
        wallet_account_statement(&database, &available_account, "developer", &format!("{}:available", payment.developer_id), &payment.currency, now)?,
        wallet_account_statement(&database, &reserved_account, "developer", &format!("{}:reserved", payment.developer_id), &payment.currency, now)?,
        wallet_account_statement(&database, &platform_account, "platform", "payment-revenue", &payment.currency, now)?,
        wallet_account_statement(&database, &provider_cost_account, "platform", "provider-cost", &payment.currency, now)?,
        wallet_account_statement(&database, &tax_account, "platform", "tax-liability", &payment.currency, now)?,
        wallet_account_statement(&database, &chargeback_account, "platform", "chargeback-cost", &payment.currency, now)?,
        worker::query!(
            &database,
            "INSERT INTO developer_settlement_reconciliations
             (reconciliation_id,payment_id,idempotency_key,developer_id,region_code,settlement_source,currency,gross_amount,tax_amount,provider_fee_amount,refund_amount,chargeback_amount,net_receipts,platform_fee_bps,platform_fee_amount,reserve_bps,reserve_amount,reserve_release_after,developer_payable_amount,provider_settlement_reference,status,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,'reconciled',?21,?21)",
            &reconciliation_id,&payment.payment_id,&input.idempotency_key,&payment.developer_id,&input.region_code,&input.settlement_source,&payment.currency,payment.amount,input.tax_amount,input.provider_fee_amount,payment.refunded_amount,input.chargeback_amount,net_receipts,i64::from(platform_bps),desired_platform_fee,i64::from(input.reserve_bps),reserve_amount,reserve_release_after,developer_payable,input.provider_settlement_reference.as_deref(),now
        )?,
    ];
    if original_split.is_some() {
        statements.push(wallet_account_statement(
            &database,
            &paid_account,
            "developer",
            &format!("{}:paid", payment.developer_id),
            &payment.currency,
            now,
        )?);
    }

    if deductions > 0 {
        statements.push(worker::query!(&database,
            "INSERT INTO journal_entries (entry_id,reference_type,reference_id,state,created_at) VALUES (?1,'settlement_reconciliation',?2,'draft',?3)",
            &adjustment_entry,&reconciliation_id,now)?);
        if developer_adjustment > 0 {
            statements.push(journal_line_statement(
                &database,
                &format!("{adjustment_entry}:developer"),
                &adjustment_entry,
                &pending_account,
                &payment.currency,
                -developer_adjustment,
                now,
            )?);
        }
        if platform_fee_reduction > 0 {
            statements.push(journal_line_statement(
                &database,
                &format!("{adjustment_entry}:platform"),
                &adjustment_entry,
                &platform_account,
                &payment.currency,
                -platform_fee_reduction,
                now,
            )?);
        }
        if input.provider_fee_amount > 0 {
            statements.push(journal_line_statement(
                &database,
                &format!("{adjustment_entry}:provider"),
                &adjustment_entry,
                &provider_cost_account,
                &payment.currency,
                input.provider_fee_amount,
                now,
            )?);
        }
        if input.tax_amount > 0 {
            statements.push(journal_line_statement(
                &database,
                &format!("{adjustment_entry}:tax"),
                &adjustment_entry,
                &tax_account,
                &payment.currency,
                input.tax_amount,
                now,
            )?);
        }
        if input.chargeback_amount > 0 {
            statements.push(journal_line_statement(
                &database,
                &format!("{adjustment_entry}:chargeback"),
                &adjustment_entry,
                &chargeback_account,
                &payment.currency,
                input.chargeback_amount,
                now,
            )?);
        }
        statements.push(post_balanced_entry_statement(
            &database,
            &adjustment_entry,
            now,
        )?);
    }

    if reserve_amount > 0 {
        let reserve_entry = format!("settlement-reserve:{reconciliation_id}");
        statements.push(worker::query!(&database,
            "INSERT INTO journal_entries (entry_id,reference_type,reference_id,state,created_at) VALUES (?1,'settlement_reserve',?2,'draft',?3)",
            &reserve_entry,&reconciliation_id,now)?);
        statements.push(journal_line_statement(
            &database,
            &format!("{reserve_entry}:pending"),
            &reserve_entry,
            &pending_account,
            &payment.currency,
            -reserve_amount,
            now,
        )?);
        statements.push(journal_line_statement(
            &database,
            &format!("{reserve_entry}:reserved"),
            &reserve_entry,
            &reserved_account,
            &payment.currency,
            reserve_amount,
            now,
        )?);
        statements.push(post_balanced_entry_statement(
            &database,
            &reserve_entry,
            now,
        )?);
    }

    if developer_payable > 0 {
        statements.push(worker::query!(&database,
            "INSERT INTO developer_settlement_releases (release_id,payment_id,idempotency_key,developer_id,currency,amount,released_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            &release_id,&payment.payment_id,&format!("release:{}",input.idempotency_key),&payment.developer_id,&payment.currency,developer_payable,now)?);
        statements.push(worker::query!(&database,
            "INSERT INTO journal_entries (entry_id,reference_type,reference_id,state,created_at) VALUES (?1,?2,?3,'draft',?4)",
            &release_entry,if original_split.is_some(){"settlement_original_order_split"}else{"settlement_release"},&release_id,now)?);
        statements.push(journal_line_statement(
            &database,
            &format!("{release_entry}:pending"),
            &release_entry,
            &pending_account,
            &payment.currency,
            -developer_payable,
            now,
        )?);
        if let Some(split) = original_split.as_ref() {
            statements.push(journal_line_statement(
                &database,
                &format!("{release_entry}:paid"),
                &release_entry,
                &paid_account,
                &payment.currency,
                developer_payable,
                now,
            )?);
            statements.push(worker::query!(&database,
                "INSERT INTO developer_original_order_splits (split_id,reconciliation_id,payment_id,developer_id,provider,payout_account_id,source_provider_reference,currency,amount,platform_fee_amount,idempotency_key,provider_reference,state,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'paid',?13,?13)",
                &split.split_id,&reconciliation_id,&payment.payment_id,&payment.developer_id,&split.provider,&split.payout_account_id,&split.source_provider_reference,&payment.currency,developer_payable,desired_platform_fee,&split.idempotency_key,&split.provider_reference,now)?);
        } else {
            statements.push(journal_line_statement(
                &database,
                &format!("{release_entry}:available"),
                &release_entry,
                &available_account,
                &payment.currency,
                developer_payable,
                now,
            )?);
        }
        statements.push(post_balanced_entry_statement(
            &database,
            &release_entry,
            now,
        )?);
    }

    statements.push(worker::query!(
        &database,
        "UPDATE payment_intents SET released_developer_amount=?1,updated_at=?2 WHERE payment_id=?3",
        developer_before_reserve,
        now,
        &payment.payment_id
    )?);
    statements.push(worker::query!(&database,
        "UPDATE developer_settlement_reconciliations SET status='released',updated_at=?1 WHERE reconciliation_id=?2",
        now,&reconciliation_id)?);
    database.batch(statements).await?;

    Response::from_json(&json!({
        "reconciliationId": reconciliation_id,
        "paymentId": payment.payment_id,
        "currency": payment.currency,
        "grossAmount": payment.amount,
        "refundAmount": payment.refunded_amount,
        "taxAmount": input.tax_amount,
        "providerFeeAmount": input.provider_fee_amount,
        "chargebackAmount": input.chargeback_amount,
        "netReceipts": net_receipts,
        "platformFeeBps": platform_bps,
        "platformFeeAmount": desired_platform_fee,
        "reserveBps": input.reserve_bps,
        "reserveAmount": reserve_amount,
        "reserveReleaseAfter": reserve_release_after,
        "developerPayableAmount": developer_payable,
        "settlementMode": if original_split.is_some() {"original_order_split"} else {"developer_payout"},
        "originalSplitProviderReference": original_split.as_ref().map(|value| value.provider_reference.clone()),
        "status": "released"
    }))
}

pub async fn admin_upsert_payout_account_v2(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let input: AdminPayoutAccountV2Request = request.json().await?;
    for value in [
        &input.payout_account_id,
        &input.developer_id,
        &input.provider,
        &input.external_account_reference,
    ] {
        validate_identifier(value)?;
    }
    if !supported_payout_provider(&input.provider) {
        return error_response(
            400,
            "unsupported_payout_provider",
            "unsupported payout provider",
        );
    }
    if !valid_country_code(&input.country_code) {
        return error_response(
            400,
            "invalid_country",
            "countryCode must be ISO alpha-2 uppercase",
        );
    }
    if !matches!(
        input.legal_entity_type.as_str(),
        "individual" | "individual_business" | "company" | "nonprofit"
    ) {
        return error_response(400, "invalid_legal_entity", "unsupported legal entity type");
    }
    if !matches!(
        input.onboarding_state.as_str(),
        "not_started" | "pending" | "requirements_due" | "verified" | "rejected"
    ) || !matches!(
        input.kyc_status.as_str(),
        "unverified" | "pending" | "verified" | "restricted" | "rejected"
    ) {
        return error_response(
            400,
            "invalid_capability_state",
            "invalid onboarding/KYC state",
        );
    }
    if input.currencies.is_empty()
        || input
            .currencies
            .iter()
            .any(|currency| !valid_iso_currency(currency))
    {
        return error_response(
            400,
            "invalid_currency",
            "at least one ISO currency is required",
        );
    }
    if input.purposes.is_empty()
        || input
            .purposes
            .iter()
            .any(|purpose| !valid_payout_purpose(purpose))
    {
        return error_response(
            400,
            "invalid_payout_purpose",
            "at least one supported payout purpose is required",
        );
    }
    let state = if input.onboarding_state == "verified"
        && input.kyc_status == "verified"
        && input.payouts_enabled
    {
        "active"
    } else {
        "pending"
    };
    let currencies_json = serde_json::to_string(&input.currencies)
        .map_err(|e| worker::Error::RustError(e.to_string()))?;
    let purposes_json = serde_json::to_string(&input.purposes)
        .map_err(|e| worker::Error::RustError(e.to_string()))?;
    let metadata_json = input.provider_metadata.to_string();
    let now = now_seconds();
    let database = context.env.d1(DATABASE_BINDING)?;
    worker::query!(&database,
        "INSERT INTO developer_payout_accounts
         (payout_account_id,developer_id,provider,external_account_reference,state,created_at,updated_at,country_code,legal_entity_type,onboarding_state,kyc_status,payouts_enabled,currencies_json,purposes_json,provider_metadata_json,is_default,last_capability_sync_at)
         VALUES (?1,?2,?3,?4,?5,?6,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?6)
         ON CONFLICT(payout_account_id) DO UPDATE SET developer_id=excluded.developer_id,provider=excluded.provider,external_account_reference=excluded.external_account_reference,state=excluded.state,country_code=excluded.country_code,legal_entity_type=excluded.legal_entity_type,onboarding_state=excluded.onboarding_state,kyc_status=excluded.kyc_status,payouts_enabled=excluded.payouts_enabled,currencies_json=excluded.currencies_json,purposes_json=excluded.purposes_json,provider_metadata_json=excluded.provider_metadata_json,is_default=excluded.is_default,last_capability_sync_at=excluded.last_capability_sync_at,updated_at=excluded.updated_at",
        &input.payout_account_id,&input.developer_id,&input.provider,&input.external_account_reference,state,now,&input.country_code,&input.legal_entity_type,&input.onboarding_state,&input.kyc_status,if input.payouts_enabled {1} else {0},&currencies_json,&purposes_json,&metadata_json,if input.is_default {1} else {0})?.run().await?;
    worker::query!(&database,
        "UPDATE developer_payout_profiles SET compliance_state=CASE WHEN EXISTS (SELECT 1 FROM developer_payout_accounts a WHERE a.developer_id=?1 AND a.state='active' AND a.kyc_status='verified' AND a.payouts_enabled=1) THEN 'eligible' ELSE 'pending' END,updated_at=?2 WHERE developer_id=?1",
        &input.developer_id,now)?.run().await?;
    Response::from_json(
        &json!({"ok":true,"payoutAccountId":input.payout_account_id,"state":state,"kycStatus":input.kyc_status,"payoutsEnabled":input.payouts_enabled}),
    )
}

pub async fn admin_set_payout_route(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let route_id = route_identifier(&context, "route_id")?;
    let input: AdminPayoutRouteStateRequest = request.json().await?;
    if !matches!(
        input.state.as_str(),
        "active" | "suspended" | "disabled" | "pending_configuration"
    ) {
        return error_response(400, "invalid_route_state", "unsupported payout route state");
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let result = worker::query!(
        &database,
        "UPDATE payout_provider_routes SET state=?1,updated_at=?2 WHERE route_id=?3",
        &input.state,
        now_seconds(),
        route_id
    )?
    .run()
    .await?;
    if d1_changes(&result) == 0 {
        return error_response(404, "route_not_found", "payout route was not found");
    }
    Response::from_json(&json!({"ok":true,"routeId":route_id,"state":input.state}))
}

pub async fn admin_create_payout_v2(
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
    if input.amount <= 0 || !valid_iso_currency(&input.currency) {
        return error_response(
            400,
            "invalid_payout",
            "positive amount and ISO currency are required",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    if let Some(existing) = payout_by_idempotency(&database, &input.idempotency_key).await? {
        if existing.developer_id != input.developer_id
            || existing.payout_account_id != input.payout_account_id
            || existing.currency != input.currency
            || existing.amount != input.amount
        {
            return error_response(
                409,
                "idempotency_conflict",
                "payout idempotency key was reused with different semantics",
            );
        }
        return Response::from_json(&json!({"payout":existing,"duplicate":true}));
    }
    let account=worker::query!(&database,
        "SELECT payout_account_id,developer_id,provider,external_account_reference,country_code,state,onboarding_state,kyc_status,payouts_enabled,currencies_json,purposes_json FROM developer_payout_accounts WHERE payout_account_id=?1 AND developer_id=?2 LIMIT 1",
        &input.payout_account_id,&input.developer_id)?.first::<PayoutCapabilityRowV2>(None).await?
        .ok_or_else(||worker::Error::RustError("payout account was not found".into()))?;
    let currencies: Vec<String> = serde_json::from_str(&account.currencies_json)
        .map_err(|_| worker::Error::RustError("invalid payout currency capabilities".into()))?;
    let purposes: Vec<String> = serde_json::from_str(&account.purposes_json)
        .map_err(|_| worker::Error::RustError("invalid payout purpose capabilities".into()))?;
    if account.state != "active"
        || account.onboarding_state != "verified"
        || account.kyc_status != "verified"
        || account.payouts_enabled != 1
        || !currencies.iter().any(|v| v == &input.currency)
        || !purposes.iter().any(|v| v == "marketplace_payout")
    {
        return error_response(
            409,
            "payout_account_unavailable",
            "payout account is not verified/enabled for this currency and purpose",
        );
    }
    let region = if account.country_code == "CN" {
        "CN"
    } else {
        "GLOBAL"
    };
    let route=worker::query!(&database,
        "SELECT route_id FROM payout_provider_routes WHERE region_code=?1 AND purpose='marketplace_payout' AND provider=?2 AND state='active' LIMIT 1",
        region,&account.provider)?.first::<Value>(None).await?;
    if route.is_none() {
        return error_response(
            503,
            "payout_route_unavailable",
            "provider route is not active for this region",
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
    let attempt_id = uuid::Uuid::new_v4().to_string();
    let clearing_account = format!(
        "payout-clearing:{}:{}",
        input.payout_account_id, input.currency
    );
    let entry_id = format!("payout:{payout_id}");
    let now = now_seconds();
    let fingerprint = sha256_hex(
        format!(
            "{}:{}:{}:{}:{}",
            input.developer_id,
            input.payout_account_id,
            input.currency,
            input.amount,
            input.idempotency_key
        )
        .as_bytes(),
    );
    database.batch(vec![
        wallet_account_statement(&database,&available_account,"developer",&format!("{}:available",input.developer_id),&input.currency,now)?,
        wallet_account_statement(&database,&clearing_account,"platform",&format!("payout-clearing:{}",input.payout_account_id),&input.currency,now)?,
        worker::query!(&database,"INSERT INTO developer_payouts (payout_id,idempotency_key,developer_id,payout_account_id,currency,amount,status,created_at,updated_at) SELECT ?1,?2,?3,?4,?5,?6,'pending',?7,?7 WHERE COALESCE((SELECT balance FROM wallet_balances WHERE account_id=?8),0)>=?6",&payout_id,&input.idempotency_key,&input.developer_id,&input.payout_account_id,&input.currency,input.amount,now,&available_account)?,
        worker::query!(&database,"INSERT OR IGNORE INTO journal_entries (entry_id,reference_type,reference_id,state,created_at) SELECT ?1,'developer_payout',payout_id,'draft',?2 FROM developer_payouts WHERE payout_id=?3",&entry_id,now,&payout_id)?,
        journal_line_statement(&database,&format!("{entry_id}:developer"),&entry_id,&available_account,&input.currency,-input.amount,now)?,
        journal_line_statement(&database,&format!("{entry_id}:clearing"),&entry_id,&clearing_account,&input.currency,input.amount,now)?,
        post_balanced_entry_statement(&database,&entry_id,now)?,
        worker::query!(&database,"INSERT INTO developer_payout_attempts (attempt_id,payout_id,provider,idempotency_key,state,request_fingerprint,created_at,updated_at) SELECT ?1,?2,?3,?4,'created',?5,?6,?6 FROM developer_payouts WHERE payout_id=?2",&attempt_id,&payout_id,&account.provider,&format!("dispatch:{}",input.idempotency_key),&fingerprint,now)?,
    ]).await?;
    let Some(payout) = payout_by_idempotency(&database, &input.idempotency_key).await? else {
        return error_response(
            409,
            "insufficient_developer_balance",
            "developer balance changed before payout reservation",
        );
    };
    Response::from_json(
        &json!({"payout":payout,"attemptId":attempt_id,"provider":account.provider,"state":"created"}),
    )
}

pub async fn admin_dispatch_payout(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let payout_id = route_identifier(&context, "payout_id")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let payout=worker::query!(&database,
        "SELECT p.payout_id,p.idempotency_key,p.developer_id,p.payout_account_id,p.currency,p.amount,p.status,a.provider,a.external_account_reference FROM developer_payouts p JOIN developer_payout_accounts a ON a.payout_account_id=p.payout_account_id WHERE p.payout_id=?1 LIMIT 1",payout_id)?.first::<PayoutDispatchRow>(None).await?
        .ok_or_else(||worker::Error::RustError("payout not found".into()))?;
    if !matches!(payout.status.as_str(), "pending" | "processing") {
        return error_response(409, "payout_not_dispatchable", "payout is not dispatchable");
    }
    let attempt=worker::query!(&database,"SELECT attempt_id,state FROM developer_payout_attempts WHERE payout_id=?1 ORDER BY created_at DESC LIMIT 1",payout_id)?.first::<PayoutAttemptStateRow>(None).await?
        .ok_or_else(||worker::Error::RustError("payout attempt not found".into()))?;
    if matches!(attempt.state.as_str(), "submitted" | "processing" | "paid") {
        return Response::from_json(
            &json!({"ok":true,"payoutId":payout_id,"state":attempt.state,"duplicate":true}),
        );
    }
    let env_name = payout_executor_env(&payout.provider)
        .ok_or_else(|| worker::Error::RustError("unsupported payout provider".into()))?;
    let executor_url = match context.env.var(env_name) {
        Ok(v) => v.to_string(),
        Err(_) => {
            worker::query!(&database,"UPDATE developer_payout_attempts SET state='configuration_required',last_error='executor URL is not configured',updated_at=?1 WHERE attempt_id=?2",now_seconds(),&attempt.attempt_id)?.run().await?;
            return error_response(
                503,
                "payout_provider_not_configured",
                "payout provider executor is not configured",
            );
        }
    };
    if !executor_url.starts_with("https://") || executor_url.contains(char::is_whitespace) {
        worker::query!(&database,"UPDATE developer_payout_attempts SET state='configuration_required',last_error='executor URL must be HTTPS',updated_at=?1 WHERE attempt_id=?2",now_seconds(),&attempt.attempt_id)?.run().await?;
        reverse_failed_payout(&database, payout_id, now_seconds()).await?;
        return error_response(
            500,
            "invalid_executor_url",
            "payout executor URL must be HTTPS",
        );
    }
    let token = context
        .env
        .secret("PAYOUT_PROVIDER_EXECUTOR_TOKEN")?
        .to_string();
    let body = json!({"payoutId":payout.payout_id,"idempotencyKey":payout.idempotency_key,"developerId":payout.developer_id,"payoutAccountId":payout.payout_account_id,"externalAccountReference":payout.external_account_reference,"currency":payout.currency,"amount":payout.amount,"provider":payout.provider});
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {token}"))?;
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(&body.to_string())));
    let outbound = Request::new_with_init(&executor_url, &init)?;
    let mut response = Fetch::Request(outbound).send().await?;
    let status = response.status_code();
    let bytes = response.bytes().await?;
    if !(200..300).contains(&status) {
        let error = String::from_utf8_lossy(&bytes)
            .chars()
            .take(500)
            .collect::<String>();
        worker::query!(&database,"UPDATE developer_payout_attempts SET state='failed',response_code=?1,last_error=?2,updated_at=?3 WHERE attempt_id=?4",status.to_string(),&error,now_seconds(),&attempt.attempt_id)?.run().await?;
        reverse_failed_payout(&database, payout_id, now_seconds()).await?;
        return error_response(
            502,
            "payout_dispatch_failed",
            "provider executor rejected payout",
        );
    }
    let payload: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    let provider_reference = payload.get("providerReference").and_then(Value::as_str);
    worker::query!(&database,"UPDATE developer_payout_attempts SET state='submitted',provider_reference=?1,response_code=?2,last_error=NULL,updated_at=?3 WHERE attempt_id=?4",provider_reference,status.to_string(),now_seconds(),&attempt.attempt_id)?.run().await?;
    worker::query!(&database,"UPDATE developer_payouts SET status='processing',provider_reference=COALESCE(?1,provider_reference),updated_at=?2 WHERE payout_id=?3 AND status='pending'",provider_reference,now_seconds(),payout_id)?.run().await?;
    Response::from_json(
        &json!({"ok":true,"payoutId":payout_id,"provider":payout.provider,"providerReference":provider_reference,"status":"processing"}),
    )
}
