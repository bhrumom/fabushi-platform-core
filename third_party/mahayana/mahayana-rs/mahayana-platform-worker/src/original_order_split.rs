#[derive(Debug, Clone, Deserialize)]
struct OriginalSplitAccountRow {
    payout_account_id: String,
    external_account_reference: String,
    provider: String,
}

#[derive(Debug, Clone)]
struct OriginalSplitResult {
    split_id: String,
    payout_account_id: String,
    provider: String,
    source_provider_reference: String,
    provider_reference: String,
    idempotency_key: String,
}

fn developer_paid_account(developer_id: &str, currency: &str) -> String {
    format!("developer-paid:{developer_id}:{currency}")
}

async fn execute_original_order_split(
    env: &Env,
    database: &worker::D1Database,
    payment: &PaymentIntentRow,
    input: &AdminSettlementReconciliationRequest,
    developer_amount: i64,
    platform_fee_amount: i64,
) -> Result<OriginalSplitResult> {
    if input.region_code != "CN" {
        return Err(worker::Error::RustError(
            "original-order split is restricted to the configured mainland China route".into(),
        ));
    }
    if input.reserve_bps != 0 {
        return Err(worker::Error::RustError(
            "original-order split uses the provider hold/split lifecycle and does not support a Fabushi reserve".into(),
        ));
    }
    let provider = match input.settlement_source.as_str() {
        "wechat_order" => "wechat_platform",
        "alipay_order" => "alipay_platform",
        _ => {
            return Err(worker::Error::RustError(
                "settlement source is not an original-order split source".into(),
            ));
        }
    };
    let source_provider_reference = payment
        .provider_reference
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            worker::Error::RustError(
                "original-order split requires the source provider payment reference".into(),
            )
        })?;

    let account = worker::query!(
        database,
        "SELECT a.payout_account_id,a.external_account_reference,a.provider
         FROM developer_payout_accounts a
         JOIN payout_provider_routes r
           ON r.region_code='CN' AND r.purpose='original_order_split'
          AND r.provider=a.provider AND r.state='active'
         WHERE a.developer_id=?1 AND a.provider=?2 AND a.country_code='CN'
           AND a.state='active' AND a.onboarding_state='verified'
           AND a.kyc_status='verified' AND a.payouts_enabled=1
           AND EXISTS (SELECT 1 FROM json_each(a.currencies_json) WHERE value=?3)
           AND EXISTS (SELECT 1 FROM json_each(a.purposes_json) WHERE value='original_order_split')
         ORDER BY a.is_default DESC,a.created_at ASC LIMIT 1",
        &payment.developer_id,
        provider,
        &payment.currency
    )?
    .first::<OriginalSplitAccountRow>(None)
    .await?
    .ok_or_else(|| {
        worker::Error::RustError(
            "no verified original-order split account is active for this developer".into(),
        )
    })?;

    let env_name = payout_executor_env(provider).ok_or_else(|| {
        worker::Error::RustError("unsupported original-order split provider".into())
    })?;
    let executor_url = env
        .var(env_name)
        .map_err(|_| worker::Error::RustError(format!("missing {env_name}")))?
        .to_string();
    if !executor_url.starts_with("https://") || executor_url.contains(char::is_whitespace) {
        return Err(worker::Error::RustError(
            "original-order split executor URL must be HTTPS".into(),
        ));
    }
    let executor_token = env.secret("PAYOUT_PROVIDER_EXECUTOR_TOKEN")?.to_string();
    let idempotency_key = format!("split:{}", input.idempotency_key);
    let request_body = json!({
        "action":"originalOrderSplit",
        "idempotencyKey":idempotency_key,
        "paymentId":payment.payment_id,
        "developerId":payment.developer_id,
        "provider":provider,
        "sourcePaymentReference":source_provider_reference,
        "externalAccountReference":account.external_account_reference,
        "currency":payment.currency,
        "developerAmount":developer_amount,
        "platformFeeAmount":platform_fee_amount
    });
    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {executor_token}"))?;
    headers.set("Content-Type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(JsValue::from_str(&request_body.to_string())));
    let outbound = Request::new_with_init(&executor_url, &init)?;
    let mut response = Fetch::Request(outbound).send().await?;
    let status = response.status_code();
    let bytes = response.bytes().await?;
    if !(200..300).contains(&status) {
        let detail = String::from_utf8_lossy(&bytes)
            .chars()
            .take(500)
            .collect::<String>();
        return Err(worker::Error::RustError(format!(
            "original-order split provider rejected request: HTTP {status}: {detail}"
        )));
    }
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| worker::Error::RustError("invalid original-order split response".into()))?;
    let provider_reference = payload
        .get("providerReference")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            worker::Error::RustError("original-order split response lacks providerReference".into())
        })?
        .to_string();
    Ok(OriginalSplitResult {
        split_id: format!("split.{}", uuid::Uuid::new_v4().simple()),
        payout_account_id: account.payout_account_id,
        provider: account.provider,
        source_provider_reference: source_provider_reference.to_string(),
        provider_reference,
        idempotency_key,
    })
}
