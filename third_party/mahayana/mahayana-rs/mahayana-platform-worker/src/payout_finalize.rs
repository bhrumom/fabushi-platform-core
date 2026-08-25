#[derive(Debug, Clone, Deserialize)]
struct PayoutFinalizeRow {
    payout_id: String,
    payout_account_id: String,
    developer_id: String,
    currency: String,
    amount: i64,
    status: String,
    provider: String,
}

async fn finalize_paid_payout(
    database: &worker::D1Database,
    payout_id: &str,
    provider_reference: Option<&str>,
    occurred_at: i64,
) -> Result<()> {
    let payout = worker::query!(
        database,
        "SELECT p.payout_id,p.payout_account_id,p.developer_id,p.currency,p.amount,p.status,a.provider
         FROM developer_payouts p
         JOIN developer_payout_accounts a ON a.payout_account_id=p.payout_account_id
         WHERE p.payout_id=?1 LIMIT 1",
        payout_id
    )?
    .first::<PayoutFinalizeRow>(None)
    .await?
    .ok_or_else(|| worker::Error::RustError("payout not found".into()))?;

    if payout.status == "paid" {
        worker::query!(
            database,
            "UPDATE developer_payout_attempts
             SET state='paid',provider_reference=COALESCE(?1,provider_reference),last_error=NULL,updated_at=?2
             WHERE payout_id=?3 AND state IN ('created','submitted','processing','paid')",
            provider_reference,
            occurred_at,
            payout_id
        )?
        .run()
        .await?;
        return Ok(());
    }
    if !matches!(payout.status.as_str(), "pending" | "processing") {
        return Err(worker::Error::RustError(
            "payout is not eligible for paid finalization".into(),
        ));
    }

    let clearing_account = format!(
        "payout-clearing:{}:{}",
        payout.payout_account_id, payout.currency
    );
    let external_account = format!("external-payout:{}:{}", payout.provider, payout.currency);
    let entry_id = format!("payout:{}:paid", payout.payout_id);

    database
        .batch(vec![
            wallet_account_statement(
                database,
                &clearing_account,
                "platform",
                &format!("payout-clearing:{}", payout.payout_account_id),
                &payout.currency,
                occurred_at,
            )?,
            wallet_account_statement(
                database,
                &external_account,
                "platform",
                &format!("external-payout:{}", payout.provider),
                &payout.currency,
                occurred_at,
            )?,
            worker::query!(
                database,
                "INSERT OR IGNORE INTO journal_entries
                 (entry_id,reference_type,reference_id,state,created_at)
                 SELECT ?1,'developer_payout_paid',?2,'draft',?3
                 WHERE EXISTS (
                    SELECT 1 FROM developer_payouts
                    WHERE payout_id=?2 AND status IN ('pending','processing')
                 )",
                &entry_id,
                payout_id,
                occurred_at
            )?,
            journal_line_statement(
                database,
                &format!("{entry_id}:clearing"),
                &entry_id,
                &clearing_account,
                &payout.currency,
                -payout.amount,
                occurred_at,
            )?,
            journal_line_statement(
                database,
                &format!("{entry_id}:external"),
                &entry_id,
                &external_account,
                &payout.currency,
                payout.amount,
                occurred_at,
            )?,
            post_balanced_entry_statement(database, &entry_id, occurred_at)?,
            worker::query!(
                database,
                "UPDATE developer_payouts
                 SET status='paid',provider_reference=COALESCE(?1,provider_reference),updated_at=?2
                 WHERE payout_id=?3 AND status IN ('pending','processing')
                   AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id=?4 AND state='posted')",
                provider_reference,
                occurred_at,
                payout_id,
                &entry_id
            )?,
            worker::query!(
                database,
                "UPDATE developer_payout_attempts
                 SET state='paid',provider_reference=COALESCE(?1,provider_reference),last_error=NULL,updated_at=?2
                 WHERE payout_id=?3 AND state IN ('created','submitted','processing')
                   AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id=?4 AND state='posted')",
                provider_reference,
                occurred_at,
                payout_id,
                &entry_id
            )?,
        ])
        .await?;
    Ok(())
}
