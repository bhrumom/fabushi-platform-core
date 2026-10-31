PRAGMA foreign_keys = ON;

-- M9-PAY-003 extends the existing Fabushi Pay payout primitives. It does not
-- create a second wallet or journal. Provider state is fail-closed until KYC /
-- KYB and payout capabilities are verified by the external platform.
ALTER TABLE developer_payout_accounts ADD COLUMN country_code TEXT NOT NULL DEFAULT 'ZZ';
ALTER TABLE developer_payout_accounts ADD COLUMN legal_entity_type TEXT NOT NULL DEFAULT 'unknown'
    CHECK (legal_entity_type IN ('unknown', 'individual', 'individual_business', 'company', 'nonprofit'));
ALTER TABLE developer_payout_accounts ADD COLUMN onboarding_state TEXT NOT NULL DEFAULT 'not_started'
    CHECK (onboarding_state IN ('not_started', 'pending', 'requirements_due', 'verified', 'rejected'));
ALTER TABLE developer_payout_accounts ADD COLUMN kyc_status TEXT NOT NULL DEFAULT 'unverified'
    CHECK (kyc_status IN ('unverified', 'pending', 'verified', 'restricted', 'rejected'));
ALTER TABLE developer_payout_accounts ADD COLUMN payouts_enabled INTEGER NOT NULL DEFAULT 0
    CHECK (payouts_enabled IN (0, 1));
ALTER TABLE developer_payout_accounts ADD COLUMN currencies_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE developer_payout_accounts ADD COLUMN purposes_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE developer_payout_accounts ADD COLUMN provider_metadata_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE developer_payout_accounts ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0
    CHECK (is_default IN (0, 1));
ALTER TABLE developer_payout_accounts ADD COLUMN last_capability_sync_at INTEGER;

CREATE INDEX IF NOT EXISTS developer_payout_accounts_capability_idx
    ON developer_payout_accounts(developer_id, state, kyc_status, payouts_enabled, country_code);

CREATE TABLE IF NOT EXISTS developer_payout_profiles (
    developer_id TEXT PRIMARY KEY,
    country_code TEXT NOT NULL,
    legal_entity_type TEXT NOT NULL CHECK (legal_entity_type IN (
        'individual', 'individual_business', 'company', 'nonprofit'
    )),
    preferred_currency TEXT NOT NULL,
    payout_schedule TEXT NOT NULL DEFAULT 'manual' CHECK (payout_schedule IN (
        'manual', 'daily', 'weekly', 'monthly'
    )),
    minimum_payout_amount INTEGER NOT NULL DEFAULT 0 CHECK (minimum_payout_amount >= 0),
    compliance_state TEXT NOT NULL DEFAULT 'pending' CHECK (compliance_state IN (
        'pending', 'eligible', 'restricted', 'disabled'
    )),
    last_scheduled_payout_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- This table is operational policy only; credentials remain Worker secrets.
-- A route in pending_configuration is intentionally unusable.
CREATE TABLE IF NOT EXISTS payout_provider_routes (
    route_id TEXT PRIMARY KEY,
    region_code TEXT NOT NULL CHECK (region_code IN ('CN', 'GLOBAL')),
    purpose TEXT NOT NULL CHECK (purpose IN (
        'original_order_split', 'external_proceeds_payout', 'marketplace_payout'
    )),
    provider TEXT NOT NULL CHECK (provider IN (
        'stripe_connect', 'adyen_platform', 'paypal_multiparty', 'paypal_payouts',
        'wechat_platform', 'alipay_platform', 'lianlian_account_plus', 'huifu_dougong'
    )),
    priority INTEGER NOT NULL CHECK (priority >= 0),
    state TEXT NOT NULL DEFAULT 'pending_configuration' CHECK (state IN (
        'pending_configuration', 'active', 'suspended', 'disabled'
    )),
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (region_code, purpose, provider)
);

CREATE INDEX IF NOT EXISTS payout_provider_routes_selection_idx
    ON payout_provider_routes(region_code, purpose, state, priority);

-- Reconciliation is the authority for moving developer revenue from pending to
-- available. All values are integer minor units.
CREATE TABLE IF NOT EXISTS developer_settlement_reconciliations (
    reconciliation_id TEXT PRIMARY KEY,
    payment_id TEXT NOT NULL REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    idempotency_key TEXT NOT NULL UNIQUE,
    developer_id TEXT NOT NULL,
    region_code TEXT NOT NULL CHECK (region_code IN ('CN', 'GLOBAL')),
    settlement_source TEXT NOT NULL CHECK (settlement_source IN (
        'wechat_order', 'alipay_order', 'apple_store_proceeds', 'google_store_proceeds',
        'web_marketplace', 'other_external_proceeds'
    )),
    currency TEXT NOT NULL,
    gross_amount INTEGER NOT NULL CHECK (gross_amount >= 0),
    tax_amount INTEGER NOT NULL DEFAULT 0 CHECK (tax_amount >= 0),
    provider_fee_amount INTEGER NOT NULL DEFAULT 0 CHECK (provider_fee_amount >= 0),
    refund_amount INTEGER NOT NULL DEFAULT 0 CHECK (refund_amount >= 0),
    chargeback_amount INTEGER NOT NULL DEFAULT 0 CHECK (chargeback_amount >= 0),
    net_receipts INTEGER NOT NULL CHECK (net_receipts >= 0),
    platform_fee_bps INTEGER NOT NULL CHECK (platform_fee_bps BETWEEN 0 AND 10000),
    platform_fee_amount INTEGER NOT NULL CHECK (platform_fee_amount >= 0),
    reserve_bps INTEGER NOT NULL CHECK (reserve_bps BETWEEN 0 AND 10000),
    reserve_amount INTEGER NOT NULL CHECK (reserve_amount >= 0),
    reserve_release_after INTEGER NOT NULL,
    reserve_released_at INTEGER,
    developer_payable_amount INTEGER NOT NULL CHECK (developer_payable_amount >= 0),
    provider_settlement_reference TEXT,
    status TEXT NOT NULL CHECK (status IN ('reconciled', 'released', 'reversed')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (payment_id, provider_settlement_reference)
);

CREATE INDEX IF NOT EXISTS settlement_reconciliations_developer_idx
    ON developer_settlement_reconciliations(developer_id, currency, status, created_at DESC);
CREATE INDEX IF NOT EXISTS settlement_reconciliations_reserve_idx
    ON developer_settlement_reconciliations(status, reserve_release_after, reserve_released_at);

CREATE TABLE IF NOT EXISTS store_settlement_batches (
    settlement_batch_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL CHECK (provider IN (
        'apple', 'google', 'wechat', 'alipay', 'web', 'merchant'
    )),
    external_reference TEXT NOT NULL,
    currency TEXT NOT NULL,
    reported_gross_amount INTEGER NOT NULL DEFAULT 0,
    reported_fee_amount INTEGER NOT NULL DEFAULT 0,
    reported_tax_amount INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL CHECK (state IN ('imported', 'reconciling', 'reconciled', 'error')),
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (provider, external_reference, currency)
);

CREATE TABLE IF NOT EXISTS developer_payout_onboarding_sessions (
    session_id TEXT PRIMARY KEY,
    developer_id TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN (
        'stripe_connect', 'adyen_platform', 'paypal_multiparty', 'paypal_payouts',
        'wechat_platform', 'alipay_platform', 'lianlian_account_plus', 'huifu_dougong'
    )),
    country_code TEXT NOT NULL,
    payout_account_id TEXT,
    provider_session_reference TEXT,
    state TEXT NOT NULL CHECK (state IN (
        'created', 'pending', 'completed', 'expired', 'failed'
    )),
    expires_at INTEGER,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS payout_onboarding_sessions_developer_idx
    ON developer_payout_onboarding_sessions(developer_id, provider, state, created_at DESC);

CREATE TABLE IF NOT EXISTS developer_payout_attempts (
    attempt_id TEXT PRIMARY KEY,
    payout_id TEXT NOT NULL REFERENCES developer_payouts(payout_id) ON DELETE RESTRICT,
    provider TEXT NOT NULL CHECK (provider IN (
        'stripe_connect', 'adyen_platform', 'paypal_multiparty', 'paypal_payouts',
        'wechat_platform', 'alipay_platform', 'lianlian_account_plus', 'huifu_dougong'
    )),
    idempotency_key TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN (
        'created', 'configuration_required', 'submitted', 'processing', 'paid', 'failed', 'cancelled'
    )),
    provider_reference TEXT,
    request_fingerprint TEXT NOT NULL,
    response_code TEXT,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS developer_payout_attempts_payout_idx
    ON developer_payout_attempts(payout_id, created_at DESC);

-- Explicit reserve account movement is represented in the existing wallet and
-- journal tables; no separate mutable balance table is introduced here.

-- Provider policy defaults are visible but disabled. Operations may activate a
-- route only after the corresponding contract, credentials and product grant.
INSERT OR IGNORE INTO payout_provider_routes
(route_id, region_code, purpose, provider, priority, state, created_at, updated_at)
VALUES
('route.cn.wechat.split', 'CN', 'original_order_split', 'wechat_platform', 10, 'pending_configuration', 0, 0),
('route.cn.alipay.split', 'CN', 'original_order_split', 'alipay_platform', 20, 'pending_configuration', 0, 0),
('route.cn.lianlian.external', 'CN', 'external_proceeds_payout', 'lianlian_account_plus', 10, 'pending_configuration', 0, 0),
('route.cn.huifu.external', 'CN', 'external_proceeds_payout', 'huifu_dougong', 20, 'pending_configuration', 0, 0),
('route.cn.lianlian.marketplace', 'CN', 'marketplace_payout', 'lianlian_account_plus', 10, 'pending_configuration', 0, 0),
('route.cn.huifu.marketplace', 'CN', 'marketplace_payout', 'huifu_dougong', 20, 'pending_configuration', 0, 0),
('route.global.stripe.marketplace', 'GLOBAL', 'marketplace_payout', 'stripe_connect', 10, 'pending_configuration', 0, 0),
('route.global.adyen.marketplace', 'GLOBAL', 'marketplace_payout', 'adyen_platform', 20, 'pending_configuration', 0, 0),
('route.global.paypal.marketplace', 'GLOBAL', 'marketplace_payout', 'paypal_multiparty', 30, 'pending_configuration', 0, 0),
('route.global.paypal-payouts.marketplace', 'GLOBAL', 'marketplace_payout', 'paypal_payouts', 40, 'pending_configuration', 0, 0),
('route.global.stripe.external', 'GLOBAL', 'external_proceeds_payout', 'stripe_connect', 10, 'pending_configuration', 0, 0),
('route.global.adyen.external', 'GLOBAL', 'external_proceeds_payout', 'adyen_platform', 20, 'pending_configuration', 0, 0),
('route.global.paypal-payouts.external', 'GLOBAL', 'external_proceeds_payout', 'paypal_payouts', 30, 'pending_configuration', 0, 0);
