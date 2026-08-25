PRAGMA foreign_keys = ON;

-- WeChat/Alipay-origin marketplace payments are settled through the licensed
-- provider's original transaction whenever that product route is approved.
-- They are not converted into a generic Fabushi withdrawal from pooled funds.
CREATE TABLE IF NOT EXISTS developer_original_order_splits (
    split_id TEXT PRIMARY KEY,
    reconciliation_id TEXT NOT NULL UNIQUE REFERENCES developer_settlement_reconciliations(reconciliation_id) ON DELETE RESTRICT,
    payment_id TEXT NOT NULL REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    developer_id TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('wechat_platform', 'alipay_platform')),
    payout_account_id TEXT NOT NULL REFERENCES developer_payout_accounts(payout_account_id) ON DELETE RESTRICT,
    source_provider_reference TEXT NOT NULL,
    currency TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK (amount > 0),
    platform_fee_amount INTEGER NOT NULL CHECK (platform_fee_amount >= 0),
    idempotency_key TEXT NOT NULL UNIQUE,
    provider_reference TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('paid', 'reversed')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS developer_original_order_splits_developer_idx
    ON developer_original_order_splits(developer_id, currency, created_at DESC);
