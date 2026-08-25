#[derive(Debug, Clone, Deserialize)]
struct ReconciledRefundRow {
    reconciliation_id: String,
    tax_amount: i64,
    provider_fee_amount: i64,
    chargeback_amount: i64,
    net_receipts: i64,
    platform_fee_bps: i64,
    platform_fee_amount: i64,
    reserve_bps: i64,
    reserve_amount: i64,
    developer_payable_amount: i64,
}

async fn reconciled_refund_row(
    database: &worker::D1Database,
    payment_id: &str,
) -> Result<Option<ReconciledRefundRow>> {
    worker::query!(
        database,
        "SELECT reconciliation_id,tax_amount,provider_fee_amount,chargeback_amount,net_receipts,platform_fee_bps,platform_fee_amount,reserve_bps,reserve_amount,developer_payable_amount
         FROM developer_settlement_reconciliations
         WHERE payment_id=?1 AND status='released'
         ORDER BY created_at DESC LIMIT 1",
        payment_id
    )?
    .first::<ReconciledRefundRow>(None)
    .await
}

async fn apply_refund_after_reconciliation(
    database: &worker::D1Database,
    payment: &PaymentIntentRow,
    reconciliation: &ReconciledRefundRow,
    provider: &str,
    refund_reference: &str,
    amount: i64,
    event_id: &str,
    occurred_at: i64,
) -> Result<()> {
    let new_refunded = payment.refunded_amount.saturating_add(amount);
    let fixed_deductions = reconciliation
        .tax_amount
        .saturating_add(reconciliation.provider_fee_amount)
        .saturating_add(reconciliation.chargeback_amount);
    let remaining_after_refund = payment.amount.saturating_sub(new_refunded);
    let new_net_receipts = remaining_after_refund.saturating_sub(fixed_deductions);
    let platform_bps = u16::try_from(reconciliation.platform_fee_bps).unwrap_or(10_000);
    let reserve_bps = u16::try_from(reconciliation.reserve_bps).unwrap_or(10_000);
    let new_platform_fee = proportional(new_net_receipts, platform_bps);
    let new_after_platform = new_net_receipts.saturating_sub(new_platform_fee);
    let new_reserve = proportional(new_after_platform, reserve_bps);
    let new_payable = new_after_platform.saturating_sub(new_reserve);

    let platform_debit = reconciliation
        .platform_fee_amount
        .saturating_sub(new_platform_fee);
    let reserve_debit = reconciliation.reserve_amount.saturating_sub(new_reserve);
    let available_debit = reconciliation
        .developer_payable_amount
        .saturating_sub(new_payable);
    let recognized_reduction = platform_debit
        .saturating_add(reserve_debit)
        .saturating_add(available_debit);
    let platform_refund_loss = amount.saturating_sub(recognized_reduction);

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
    let available_account = developer_available_account(&payment.developer_id, &payment.currency);
    let reserved_account = developer_reserved_account(&payment.developer_id, &payment.currency);
    let platform_account = format!("platform:payment-revenue:{}", payment.currency);
    let refund_loss_account = format!("platform:refund-loss:{}", payment.currency);
    let refund_id = uuid::Uuid::new_v4().to_string();
    let entry_id = format!("refund:{refund_id}");

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
            &available_account,
            "developer",
            &format!("{}:available", payment.developer_id),
            &payment.currency,
            occurred_at,
        )?,
        wallet_account_statement(
            database,
            &reserved_account,
            "developer",
            &format!("{}:reserved", payment.developer_id),
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
        wallet_account_statement(
            database,
            &refund_loss_account,
            "platform",
            "refund-loss",
            &payment.currency,
            occurred_at,
        )?,
        worker::query!(
            database,
            "INSERT OR IGNORE INTO fabushi_payment_refunds
             (refund_id,payment_id,idempotency_key,provider_refund_id,currency,amount,status,reason,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,'succeeded','provider_refund_after_reconciliation',?7,?7)",
            &refund_id,
            &payment.payment_id,
            event_id,
            refund_reference,
            &payment.currency,
            amount,
            occurred_at
        )?,
        worker::query!(
            database,
            "INSERT OR IGNORE INTO journal_entries (entry_id,reference_type,reference_id,state,created_at)
             SELECT ?1,'payment_refund',?2,'draft',?3
             WHERE EXISTS (SELECT 1 FROM fabushi_payment_refunds WHERE refund_id=?2)",
            &entry_id,
            &refund_id,
            occurred_at
        )?,
    ];

    if reserve_debit > 0 {
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:reserved"),
            &entry_id,
            &reserved_account,
            &payment.currency,
            -reserve_debit,
            occurred_at,
        )?);
    }
    if available_debit > 0 {
        // Available balance may become negative after a developer has already
        // been paid. Future payouts are therefore blocked until future revenue
        // offsets the refund liability; no hidden off-ledger debt is created.
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
    if platform_debit > 0 {
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:platform"),
            &entry_id,
            &platform_account,
            &payment.currency,
            -platform_debit,
            occurred_at,
        )?);
    }
    if platform_refund_loss > 0 {
        // Fixed store/payment fees or taxes can make a full customer refund
        // larger than the revenue previously recognized. Record that delta as
        // a platform refund loss instead of charging it to the developer.
        statements.push(journal_line_statement(
            database,
            &format!("{entry_id}:refund-loss"),
            &entry_id,
            &refund_loss_account,
            &payment.currency,
            -platform_refund_loss,
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
        "UPDATE developer_settlement_reconciliations
         SET refund_amount=?1,net_receipts=?2,platform_fee_amount=?3,reserve_amount=?4,developer_payable_amount=?5,updated_at=?6
         WHERE reconciliation_id=?7
           AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id=?8 AND state='posted')",
        new_refunded,
        new_net_receipts,
        new_platform_fee,
        new_reserve,
        new_payable,
        occurred_at,
        &reconciliation.reconciliation_id,
        &entry_id
    )?);
    statements.push(worker::query!(
        database,
        "UPDATE payment_intents
         SET refunded_amount=?1,released_developer_amount=?2,status=?3,updated_at=?4
         WHERE payment_id=?5
           AND EXISTS (SELECT 1 FROM journal_entries WHERE entry_id=?6 AND state='posted')",
        new_refunded,
        new_after_platform,
        next_status,
        occurred_at,
        &payment.payment_id,
        &entry_id
    )?);
    if next_status == "refunded" {
        statements.push(worker::query!(
            database,
            "UPDATE entitlements SET status='revoked',revoked_at=?1 WHERE order_id=?2 AND status='active'",
            occurred_at,
            &payment.payment_id
        )?);
        statements.push(worker::query!(
            database,
            "UPDATE orders SET status='refunded',updated_at=?1 WHERE order_id=?2",
            occurred_at,
            &payment.payment_id
        )?);
    }
    database.batch(statements).await?;
    Ok(())
}
