PRAGMA foreign_keys = ON;

-- Fabushi Pay extends the existing marketplace products/prices and the shared
-- journal. Product prices remain server-authoritative; this table only adds
-- payment policy and provider identifiers to an existing product.
CREATE TABLE IF NOT EXISTS payment_product_config (
    product_id TEXT PRIMARY KEY REFERENCES products(product_id) ON DELETE RESTRICT,
    developer_id TEXT NOT NULL,
    product_kind TEXT NOT NULL CHECK (product_kind IN (
        'digital_consumable', 'digital_durable', 'subscription', 'physical', 'service'
    )),
    platform_fee_bps INTEGER NOT NULL DEFAULT 0 CHECK (platform_fee_bps BETWEEN 0 AND 10000),
    allowed_rails_json TEXT NOT NULL,
    provider_product_refs_json TEXT NOT NULL DEFAULT '{}',
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS payment_product_config_developer_idx
    ON payment_product_config(developer_id, active);

CREATE TABLE IF NOT EXISTS payment_intents (
    payment_id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL,
    user_id TEXT NOT NULL,
    mini_app_id TEXT NOT NULL,
    developer_id TEXT NOT NULL,
    product_id TEXT NOT NULL REFERENCES products(product_id) ON DELETE RESTRICT,
    price_id TEXT NOT NULL REFERENCES prices(price_id) ON DELETE RESTRICT,
    entitlement_capability TEXT NOT NULL,
    sku TEXT NOT NULL,
    product_kind TEXT NOT NULL CHECK (product_kind IN (
        'digital_consumable', 'digital_durable', 'subscription', 'physical', 'service'
    )),
    rail TEXT NOT NULL CHECK (rail IN (
        'credits', 'apple_in_app_purchase', 'google_play_billing', 'web_provider', 'merchant_provider'
    )),
    provider_product_ref TEXT,
    currency TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK (amount > 0),
    platform_fee_bps INTEGER NOT NULL CHECK (platform_fee_bps BETWEEN 0 AND 10000),
    status TEXT NOT NULL CHECK (status IN (
        'created', 'requires_action', 'processing', 'succeeded', 'failed', 'cancelled',
        'partially_refunded', 'refunded'
    )),
    provider_reference TEXT,
    refunded_amount INTEGER NOT NULL DEFAULT 0 CHECK (refunded_amount >= 0),
    released_developer_amount INTEGER NOT NULL DEFAULT 0 CHECK (released_developer_amount >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (user_id, idempotency_key),
    CHECK (refunded_amount <= amount)
);

CREATE UNIQUE INDEX IF NOT EXISTS payment_intents_provider_reference_idx
    ON payment_intents(rail, provider_reference)
    WHERE provider_reference IS NOT NULL;
CREATE INDEX IF NOT EXISTS payment_intents_user_idx
    ON payment_intents(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS payment_intents_developer_idx
    ON payment_intents(developer_id, status, created_at DESC);

-- Inbox/outbox-style event ownership makes provider delivery at-least-once
-- safe. The event row is claimed before any state transition is attempted.
CREATE TABLE IF NOT EXISTS payment_webhook_events (
    provider TEXT NOT NULL,
    event_id TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('received', 'processing', 'processed', 'rejected')),
    payment_id TEXT REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    received_at INTEGER NOT NULL,
    processed_at INTEGER,
    error_code TEXT,
    PRIMARY KEY (provider, event_id)
);

CREATE INDEX IF NOT EXISTS payment_webhook_events_state_idx
    ON payment_webhook_events(state, received_at);

CREATE TABLE IF NOT EXISTS fabushi_payment_refunds (
    refund_id TEXT PRIMARY KEY,
    payment_id TEXT NOT NULL REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    idempotency_key TEXT NOT NULL UNIQUE,
    provider_refund_id TEXT,
    currency TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK (amount > 0),
    status TEXT NOT NULL CHECK (status IN ('requested', 'processing', 'succeeded', 'failed')),
    reason TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS fabushi_payment_refunds_provider_idx
    ON fabushi_payment_refunds(provider_refund_id)
    WHERE provider_refund_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS payment_disputes (
    dispute_id TEXT PRIMARY KEY,
    payment_id TEXT NOT NULL REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    provider TEXT NOT NULL,
    provider_reference TEXT NOT NULL,
    currency TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK (amount > 0),
    status TEXT NOT NULL CHECK (status IN ('open', 'won', 'lost', 'closed')),
    opened_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (provider, provider_reference)
);

CREATE TABLE IF NOT EXISTS developer_payout_accounts (
    payout_account_id TEXT PRIMARY KEY,
    developer_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    external_account_reference TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'active', 'restricted', 'disabled')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (developer_id, provider, external_account_reference)
);

-- Releases are intentionally not unique by payment: lowering a reserve later
-- can release the remaining held amount without rewriting payment history.
CREATE TABLE IF NOT EXISTS developer_settlement_releases (
    release_id TEXT PRIMARY KEY,
    payment_id TEXT NOT NULL REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    idempotency_key TEXT NOT NULL UNIQUE,
    developer_id TEXT NOT NULL,
    currency TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK (amount > 0),
    released_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS developer_settlement_releases_payment_idx
    ON developer_settlement_releases(payment_id, released_at);

CREATE TABLE IF NOT EXISTS developer_payouts (
    payout_id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    developer_id TEXT NOT NULL,
    payout_account_id TEXT NOT NULL REFERENCES developer_payout_accounts(payout_account_id) ON DELETE RESTRICT,
    currency TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK (amount > 0),
    status TEXT NOT NULL CHECK (status IN (
        'created', 'requires_identity', 'pending', 'processing', 'paid', 'failed', 'cancelled'
    )),
    provider_reference TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS developer_payouts_developer_idx
    ON developer_payouts(developer_id, status, created_at DESC);

-- fabushi_pay_balance_enforced_by_worker_batch: every successful payment,
-- refund, settlement release, dispute loss and payout is represented by a
-- balanced set of integer journal lines and posted atomically by the Rust
-- Worker. No public route writes journal tables directly.
