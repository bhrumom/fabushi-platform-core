#[derive(Debug, Clone, Deserialize)]
struct DueReserveRow {
    reconciliation_id: String,
    developer_id: String,
    currency: String,
    reserve_amount: i64,
}

async fn release_one_reserve(
    database: &worker::D1Database,
    row: &DueReserveRow,
    occurred_at: i64,
) -> Result<bool> {
    if row.reserve_amount <= 0 {
        worker::query!(
            database,
            "UPDATE developer_settlement_reconciliations
             SET reserve_released_at=COALESCE(reserve_released_at,?1),updated_at=?1
             WHERE reconciliation_id=?2 AND reserve_released_at IS NULL",
            occurred_at,
            &row.reconciliation_id
        )?
        .run()
        .await?;
        return Ok(false);
    }

    let reserved_account = developer_reserved_account(&row.developer_id, &row.currency);
    let available_account = developer_available_account(&row.developer_id, &row.currency);
    let entry_id = format!("reserve-release:{}", row.reconciliation_id);
    let release_id = format!("reserve-release:{}", row.reconciliation_id);
    database
        .batch(vec![
            wallet_account_statement(
                database,
                &reserved_account,
                "developer",
                &format!("{}:reserved", row.developer_id),
                &row.currency,
                occurred_at,
            )?,
            wallet_account_statement(
                database,
                &available_account,
                "developer",
                &format!("{}:available", row.developer_id),
                &row.currency,
                occurred_at,
            )?,
            worker::query!(
                database,
                "INSERT OR IGNORE INTO developer_settlement_releases
                 (release_id,payment_id,idempotency_key,developer_id,currency,amount,released_at)
                 SELECT ?1,payment_id,?2,developer_id,currency,reserve_amount,?3
                 FROM developer_settlement_reconciliations
                 WHERE reconciliation_id=?4 AND reserve_released_at IS NULL AND reserve_amount>0",
                &release_id,
                &format!("reserve:{}", row.reconciliation_id),
                occurred_at,
                &row.reconciliation_id
            )?,
            worker::query!(
                database,
                "INSERT OR IGNORE INTO journal_entries
                 (entry_id,reference_type,reference_id,state,created_at)
                 SELECT ?1,'settlement_reserve_release',?2,'draft',?3
                 WHERE EXISTS (SELECT 1 FROM developer_settlement_releases WHERE release_id=?2)",
                &entry_id,
                &release_id,
                occurred_at
            )?,
            journal_line_statement(
                database,
                &format!("{entry_id}:reserved"),
                &entry_id,
                &reserved_account,
                &row.currency,
                -row.reserve_amount,
                occurred_at,
            )?,
            journal_line_statement(
                database,
                &format!("{entry_id}:available"),
                &entry_id,
                &available_account,
                &row.currency,
                row.reserve_amount,
                occurred_at,
            )?,
            post_balanced_entry_statement(database, &entry_id, occurred_at)?,
            worker::query!(
                database,
                "UPDATE developer_settlement_reconciliations
                 SET developer_payable_amount=developer_payable_amount+reserve_amount,
                     reserve_amount=0,reserve_released_at=?1,updated_at=?1
                 WHERE reconciliation_id=?2 AND reserve_released_at IS NULL
                   AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id=?3 AND state='posted')",
                occurred_at,
                &row.reconciliation_id,
                &entry_id
            )?,
        ])
        .await?;
    Ok(true)
}

pub async fn release_due_reserves(
    database: &worker::D1Database,
    occurred_at: i64,
    limit: u32,
) -> Result<u32> {
    let limit = i64::from(limit.clamp(1, 500));
    let rows = worker::query!(
        database,
        "SELECT reconciliation_id,developer_id,currency,reserve_amount
         FROM developer_settlement_reconciliations
         WHERE status='released' AND reserve_released_at IS NULL
           AND reserve_release_after<=?1
         ORDER BY reserve_release_after,reconciliation_id LIMIT ?2",
        occurred_at,
        limit
    )?
    .all()
    .await?
    .results::<DueReserveRow>()?;
    let mut released = 0u32;
    for row in rows {
        if release_one_reserve(database, &row, occurred_at).await? {
            released = released.saturating_add(1);
        }
    }
    Ok(released)
}

pub async fn admin_release_due_reserves(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let released = release_due_reserves(&database, now_seconds(), 250).await?;
    Response::from_json(&json!({"ok":true,"released":released}))
}

pub async fn admin_release_reserve(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    require_bearer_secret(&request, &context.env, "FABUSHI_PAY_ADMIN_TOKEN")?;
    let reconciliation_id = route_identifier(&context, "reconciliation_id")?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT reconciliation_id,developer_id,currency,reserve_amount
         FROM developer_settlement_reconciliations
         WHERE reconciliation_id=?1 AND status='released' AND reserve_released_at IS NULL
           AND reserve_release_after<=?2 LIMIT 1",
        reconciliation_id,
        now_seconds()
    )?
    .first::<DueReserveRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(
            409,
            "reserve_not_releasable",
            "reserve is missing, already released, or still inside its hold window",
        );
    };
    let released = release_one_reserve(&database, &row, now_seconds()).await?;
    Response::from_json(
        &json!({"ok":true,"reconciliationId":reconciliation_id,"released":released}),
    )
}
