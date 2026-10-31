use super::*;
use crate::capability_access::{
    EntitlementAccessInput, active_purchase_rails, evaluate_entitlement_access,
};

pub(super) async fn wallet_balance(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
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

pub(super) async fn wallet_history(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
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

pub(super) async fn commerce_quote(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
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

pub(super) async fn commerce_purchase(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
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

pub(super) async fn commerce_entitlement(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let user_id = match authenticated_user(&request, &context.env) {
        Ok(user_id) => user_id,
        Err(_) => match authenticated_plugin_account(&request, &context.env, plugin_id).await {
            Ok(account) => account.user_id,
            Err(_) => {
                return error_response(
                    401,
                    "authentication_required",
                    "active Fabushi account or matching Mini App session required",
                );
            }
        },
    };
    let capability = route_identifier(&context, "capability")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();

    #[derive(Deserialize)]
    struct EntitlementAccessRow {
        entitlement_id: String,
        user_id: String,
        plugin_id: String,
        capability: String,
        status: String,
        granted_at: i64,
        expires_at: Option<i64>,
        product_kind: Option<String>,
        subscription_period_seconds: Option<i64>,
        subscription_status: Option<String>,
        subscription_period_end: Option<i64>,
    }

    #[derive(Deserialize)]
    struct CountRow {
        count: i64,
    }

    #[derive(Deserialize)]
    struct PurchaseOptionRow {
        product_id: String,
        sku: String,
        display_name: String,
        product_kind: String,
        subscription_period_seconds: Option<i64>,
        currency: String,
        amount: i64,
        allowed_rails_json: String,
        active_providers: String,
    }

    let rows = worker::query!(
        &database,
        "SELECT e.entitlement_id, e.user_id, e.plugin_id, e.capability, e.status,
                e.granted_at, e.expires_at,
                COALESCE(c.product_kind, pc.product_kind) AS product_kind,
                c.subscription_period_seconds AS subscription_period_seconds,
                (SELECT ms.status FROM monetization_subscriptions ms
                  WHERE ms.user_id = e.user_id AND ms.product_id = e.product_id
                    AND ms.entitlement_capability = e.capability
                  ORDER BY ms.updated_at DESC LIMIT 1) AS subscription_status,
                (SELECT ms.current_period_end FROM monetization_subscriptions ms
                  WHERE ms.user_id = e.user_id AND ms.product_id = e.product_id
                    AND ms.entitlement_capability = e.capability
                  ORDER BY ms.updated_at DESC LIMIT 1) AS subscription_period_end
           FROM entitlements e
           LEFT JOIN payment_product_catalog c ON c.product_id = e.product_id
           LEFT JOIN payment_product_config pc ON pc.product_id = e.product_id
          WHERE e.user_id = ?1 AND e.plugin_id = ?2 AND e.capability = ?3
          ORDER BY e.granted_at DESC LIMIT 50",
        &user_id,
        plugin_id,
        capability
    )?
    .all()
    .await?
    .results::<EntitlementAccessRow>()?;

    let mut entitlement = None;
    let mut effective_expires_at = None;
    let mut access_reason = "not_entitled";
    for row in rows {
        let decision = evaluate_entitlement_access(
            EntitlementAccessInput {
                status: &row.status,
                product_kind: row.product_kind.as_deref(),
                granted_at: row.granted_at,
                entitlement_expires_at: row.expires_at,
                subscription_status: row.subscription_status.as_deref(),
                subscription_period_end: row.subscription_period_end,
                subscription_period_seconds: row.subscription_period_seconds,
            },
            now,
        );
        effective_expires_at = decision.effective_expires_at;
        access_reason = decision.reason;
        if decision.allowed {
            entitlement = Some(Entitlement {
                entitlement_id: row.entitlement_id,
                user_id: row.user_id,
                plugin_id: row.plugin_id,
                capability: row.capability,
                status: EntitlementStatus::Active,
                expires_at: decision.effective_expires_at,
            });
            break;
        }
    }

    let protected_count = worker::query!(
        &database,
        "SELECT COUNT(*) AS count FROM products
          WHERE plugin_id = ?1 AND entitlement_capability = ?2",
        plugin_id,
        capability
    )?
    .first::<CountRow>(None)
    .await?
    .map(|row| row.count)
    .unwrap_or(0);
    let protected = protected_count > 0;

    let option_rows = worker::query!(
        &database,
        "SELECT c.product_id, c.sku, c.display_name, c.product_kind,
                c.subscription_period_seconds, pr.currency, pr.amount,
                pc.allowed_rails_json,
                COALESCE((SELECT GROUP_CONCAT(pb.provider, ',')
                  FROM payment_provider_bindings pb
                 WHERE pb.product_id = c.product_id AND pb.sync_state = 'active'), '') AS active_providers
           FROM payment_product_catalog c
           JOIN products p ON p.product_id = c.product_id
           JOIN prices pr ON pr.product_id = c.product_id
           JOIN payment_product_config pc ON pc.product_id = c.product_id
          WHERE c.mini_app_id = ?1 AND c.entitlement_capability = ?2
            AND c.catalog_status = 'active' AND p.active = 1 AND pc.active = 1
            AND pr.active = 1 AND pr.starts_at <= ?3
            AND (pr.ends_at IS NULL OR pr.ends_at > ?3)
          ORDER BY c.product_kind = 'subscription' DESC, pr.amount ASC",
        plugin_id,
        capability,
        now
    )?
    .all()
    .await?
    .results::<PurchaseOptionRow>()?;

    let purchase_options = option_rows
        .into_iter()
        .map(|row| {
            let allowed_rails =
                serde_json::from_str::<Vec<String>>(&row.allowed_rails_json).unwrap_or_default();
            let rails = active_purchase_rails(&allowed_rails, &row.active_providers);
            json!({
                "productId": row.product_id,
                "sku": row.sku,
                "displayName": row.display_name,
                "productKind": row.product_kind,
                "subscriptionPeriodSeconds": row.subscription_period_seconds,
                "currency": row.currency,
                "amount": row.amount,
                "activeRails": rails,
            })
        })
        .collect::<Vec<_>>();

    let allowed = entitlement.is_some() || !protected;
    if !protected && entitlement.is_none() {
        access_reason = "unprotected_capability";
        effective_expires_at = None;
    }

    Response::from_json(&json!({
        "entitlement": entitlement,
        "access": {
            "protected": protected,
            "allowed": allowed,
            "reason": access_reason,
            "effectiveExpiresAt": effective_expires_at,
        },
        "purchaseOptions": purchase_options,
    }))
}

pub(super) async fn delegated_plugin_token(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_session_account(&request, &context.env).await {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "session_required",
                "active Fabushi account session required for Mini App credential bootstrap",
            );
        }
    };
    let Some(session_id) = account.session_id.clone() else {
        return error_response(
            401,
            "session_required",
            "active Fabushi account session required",
        );
    };
    let delegated: DelegatedTokenRequest = request.json().await?;
    validate_delegated_request(&delegated)?;
    let now = now_seconds() as usize;
    let expires_at = now + 300;
    let claims = PluginAccessTokenClaims {
        iss: ACCESS_TOKEN_ISSUER.to_string(),
        sub: account.user_id,
        aud: format!("plugin:{}", delegated.plugin_id),
        scope: delegated.scopes,
        device_id: delegated.device_id,
        sid: session_id,
        jti: Uuid::new_v4().to_string(),
        iat: now,
        exp: expires_at,
        token_use: "plugin".to_string(),
    };
    // Reuse the canonical account signing key so every verifier can use the existing
    // public JWKS. token_use + audience + exact scope keep delegated tokens distinct.
    let private_key = context
        .env
        .secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?
        .to_string();
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("JWT".to_string());
    header.kid = Some(context.env.var("ACCESS_TOKEN_KEY_ID")?.to_string());
    let key = EncodingKey::from_rsa_pem(private_key.as_bytes()).map_err(jwt_error)?;
    let token = encode(&header, &claims, &key).map_err(jwt_error)?;
    Response::from_json(&json!({
        "accessToken": token,
        "tokenType": "Bearer",
        "expiresIn": 300,
        "expiresAt": expires_at,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DelegatedTokenIntrospectRequest {
    plugin_id: String,
}

pub(super) async fn delegated_plugin_token_introspect(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    if request.method() != Method::Post {
        return error_response(405, "method_not_allowed", "POST required");
    }
    let body: DelegatedTokenIntrospectRequest = request.json().await?;
    if !is_identifier(&body.plugin_id) {
        return error_response(400, "invalid_plugin_id", "invalid delegated plugin id");
    }
    let account = match authenticated_plugin_account(&request, &context.env, &body.plugin_id).await
    {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "invalid_plugin_token",
                "Mini App credential is invalid, expired, or its Fabushi session was revoked",
            );
        }
    };
    Response::from_json(&json!({
        "active": true,
        "pluginId": body.plugin_id,
        "sessionBound": true,
        "user": { "id": account.user_id },
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
    let expected_scope = format!("miniapp:{}", request.plugin_id);
    if request.scopes.len() != 1
        || request.scopes.first().map(String::as_str) != Some(expected_scope.as_str())
        || request
            .scopes
            .iter()
            .any(|scope| scope.len() > 96 || !is_scope(scope))
    {
        return Err(worker::Error::RustError(
            "delegated token must contain only the matching Mini App scope".into(),
        ));
    }
    Ok(())
}

pub(super) async fn purchases(request: Request, context: RouteContext<()>) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    purchases_response(&context.env, &user_id).await
}

pub(super) async fn purchases_restore(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    if request.method() != Method::Post {
        return error_response(405, "method_not_allowed", "POST required");
    }
    let user_id = authenticated_user(&request, &context.env)?;
    purchases_response(&context.env, &user_id).await
}

pub(super) async fn purchases_response(env: &Env, user_id: &str) -> Result<Response> {
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

pub(super) async fn active_price(
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

pub(super) async fn order_by_idempotency(
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

pub(super) async fn entitlement_for_order(
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
