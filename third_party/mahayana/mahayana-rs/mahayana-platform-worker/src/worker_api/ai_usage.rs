use super::*;

pub(super) async fn ai_usage_status(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let user_id = authenticated_user(&request, &context.env)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let status = current_usage_status(&database, &context.env, &user_id, now_seconds()).await?;
    Response::from_json(&status)
}

pub(super) async fn ai_usage_reserve(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
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
    let (token_limit, unlimited) = account_usage_limit(&context.env, &user_id)?;
    if unlimited {
        worker::query!(
            &database,
            "UPDATE ai_usage_budgets SET token_limit = ?1, updated_at = ?2
             WHERE user_id = ?3 AND window_start = ?4",
            token_limit,
            now,
            &user_id,
            window_start
        )?
        .run()
        .await?;
    }
    worker::query!(
        &database,
        "INSERT OR IGNORE INTO ai_usage_budgets
         (user_id, window_start, window_end, token_limit, used_tokens, reserved_tokens, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, 0, ?5)",
        &user_id,
        window_start,
        window_end,
        token_limit,
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

pub(super) async fn ai_usage_capture(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
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

pub(super) async fn ai_usage_release(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
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

async fn current_usage_status(
    database: &worker::D1Database,
    env: &Env,
    user_id: &str,
    now: i64,
) -> Result<AccountUsageStatus> {
    let window_start = usage_window_start(now);
    let window_end = window_start + USAGE_WINDOW_SECONDS;
    let (entitled_limit, unlimited) = account_usage_limit(env, user_id)?;
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
            (
                if unlimited {
                    entitled_limit
                } else {
                    row.token_limit
                },
                row.used_tokens,
                row.reserved_tokens,
            )
        }
        None => (entitled_limit, 0, 0),
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
        unlimited,
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

pub(super) fn d1_changes(result: Option<&worker::D1Result>) -> usize {
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

fn account_usage_limit(env: &Env, user_id: &str) -> Result<(i64, bool)> {
    let configured = ["SUPER_ADMIN_ACCOUNT_IDS", "ADMIN_ACCOUNT_IDS"]
        .into_iter()
        .filter_map(|name| env.var(name).ok())
        .any(|value| {
            value
                .to_string()
                .split(',')
                .any(|candidate| candidate.trim() == user_id.trim())
        });
    let unlimited = is_builtin_super_admin_account_id(user_id) || configured;
    Ok((
        if unlimited {
            UNLIMITED_AI_TOKEN_LIMIT
        } else {
            default_usage_limit(env)?
        },
        unlimited,
    ))
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
