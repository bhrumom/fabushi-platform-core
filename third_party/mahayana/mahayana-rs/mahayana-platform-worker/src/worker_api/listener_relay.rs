use super::*;

pub(super) async fn listener_register(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to register listeners.",
            );
        }
    };
    let input: ListenerRegistrationRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_listener_registration",
                "Listener registrations must be valid JSON.",
            );
        }
    };
    if input.registrations.len() > 32 {
        return error_response(
            400,
            "too_many_listener_registrations",
            "At most 32 listener platform registrations are allowed.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let now = now_seconds();
    let mut statements = vec![worker::query!(
        &database,
        "DELETE FROM listener_registrations WHERE user_id = ?1",
        &account.user_id
    )?];
    let mut seen = std::collections::BTreeSet::new();
    for registration in input.registrations {
        if !is_listener_platform(&registration.platform)
            || !seen.insert(registration.platform.clone())
        {
            return error_response(
                400,
                "invalid_listener_platform",
                "Listener platforms must be supported and unique.",
            );
        }
        let subscriptions_json = serde_json::to_string(&registration.subscriptions)
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        if subscriptions_json.len() > 64 * 1024 {
            return error_response(
                400,
                "listener_registration_too_large",
                "Listener subscriptions must be at most 64 KiB per platform.",
            );
        }
        statements.push(worker::query!(
            &database,
            "INSERT INTO listener_registrations
             (user_id, platform, subscriptions_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            &account.user_id,
            &registration.platform,
            &subscriptions_json,
            now
        )?);
    }
    database.batch(statements).await?;
    Ok(Response::from_json(&json!({"registered": seen.len()}))?.with_headers(auth_headers()))
}

pub(super) async fn listener_drain(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to drain listener events.",
            );
        }
    };
    let input: ListenerDrainRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_listener_drain",
                "Listener drain requests must be valid JSON.",
            );
        }
    };
    if input.ack_ids.len() > 100
        || input
            .ack_ids
            .iter()
            .any(|id| !valid_relay_identifier(id, 256))
    {
        return error_response(
            400,
            "invalid_listener_ack",
            "At most 100 normalized event IDs may be acknowledged at once.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    if !input.ack_ids.is_empty() {
        let now = now_seconds();
        let mut statements = Vec::with_capacity(input.ack_ids.len());
        for event_id in &input.ack_ids {
            statements.push(worker::query!(
                &database,
                "UPDATE listener_events
                 SET acknowledged_at = COALESCE(acknowledged_at, ?1)
                 WHERE event_id = ?2 AND user_id = ?3",
                now,
                event_id,
                &account.user_id
            )?);
        }
        database.batch(statements).await?;
    }
    let rows = worker::query!(
        &database,
        "SELECT event_id, platform, event_json, created_at
         FROM listener_events
         WHERE user_id = ?1 AND acknowledged_at IS NULL
         ORDER BY created_at ASC, event_id ASC
         LIMIT 100",
        &account.user_id
    )?
    .all()
    .await?
    .results::<ListenerEventRow>()?;
    let events = rows
        .into_iter()
        .filter_map(|row| {
            let event = serde_json::from_str::<Value>(&row.event_json).ok()?;
            Some(json!({
                "id": row.event_id,
                "platform": row.platform,
                "createdAt": exact_nonnegative_i64(row.created_at).unwrap_or_default(),
                "event": event,
            }))
        })
        .collect::<Vec<_>>();
    Ok(Response::from_json(&json!({"events": events}))?.with_headers(auth_headers()))
}

pub(super) async fn listener_ingest(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let authorization = request.headers().get("Authorization")?.unwrap_or_default();
    let supplied = authorization.strip_prefix("Bearer ").unwrap_or_default();
    let expected = match context.env.secret("LISTENER_RELAY_INGEST_TOKEN") {
        Ok(secret) => secret.to_string(),
        Err(_) => {
            return error_response(
                503,
                "listener_ingest_not_configured",
                "Listener relay ingress is not configured.",
            );
        }
    };
    if supplied.is_empty() || !constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
        return error_response(401, "unauthorized", "Invalid listener relay ingress token.");
    }
    let input: ListenerIngressRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_listener_event",
                "Listener ingress requests must be valid JSON.",
            );
        }
    };
    if !valid_relay_identifier(&input.event_id, 256)
        || !valid_relay_identifier(&input.user_id, 256)
        || !is_listener_platform(&input.platform)
    {
        return error_response(
            400,
            "invalid_listener_event",
            "Listener event identifiers or platform are invalid.",
        );
    }
    let event_json = serde_json::to_string(&input.event)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    if event_json.len() > 128 * 1024 {
        return error_response(
            413,
            "listener_event_too_large",
            "Listener events must be at most 128 KiB.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let registration = worker::query!(
        &database,
        "SELECT platform FROM listener_registrations
         WHERE user_id = ?1 AND platform = ?2",
        &input.user_id,
        &input.platform
    )?
    .first::<Value>(None)
    .await?;
    if registration.is_none() {
        return error_response(
            409,
            "listener_not_registered",
            "The target account has not registered this listener platform.",
        );
    }
    let result = worker::query!(
        &database,
        "INSERT INTO listener_events
         (event_id, user_id, platform, event_json, created_at, acknowledged_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL)
         ON CONFLICT(event_id) DO NOTHING",
        &input.event_id,
        &input.user_id,
        &input.platform,
        &event_json,
        now_seconds()
    )?
    .run()
    .await?;
    Ok(Response::from_json(&json!({
        "accepted": d1_changes(Some(&result)) > 0,
        "eventId": input.event_id,
    }))?)
}
