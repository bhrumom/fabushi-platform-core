PRAGMA foreign_keys = ON;

-- Unified commercial control plane layered on the canonical Fabushi Pay ledger.
-- Money movement remains journal_entries + journal_lines. These tables provide
-- revenue-event normalization, effective-dated split policy, subscriptions,
-- advertising, developer compliance and payout-request orchestration.

CREATE TABLE IF NOT EXISTS monetization_developer_profiles (
    developer_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    compliance_state TEXT NOT NULL DEFAULT 'pending' CHECK (compliance_state IN (
        'pending', 'reviewing', 'verified', 'restricted', 'rejected'
    )),
    payout_enabled INTEGER NOT NULL DEFAULT 0 CHECK (payout_enabled IN (0, 1)),
    external_kyc_reference TEXT,
    tax_profile_reference TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS monetization_developer_compliance_idx
    ON monetization_developer_profiles(compliance_state, payout_enabled);

CREATE TABLE IF NOT EXISTS monetization_split_rules (
    rule_id TEXT PRIMARY KEY,
    scope_type TEXT NOT NULL CHECK (scope_type IN (
        'platform', 'miniapp', 'product', 'ad_placement'
    )),
    scope_id TEXT NOT NULL,
    revenue_source TEXT NOT NULL CHECK (revenue_source IN (
        'purchase', 'subscription', 'ad_impression', 'ad_click', 'ad_conversion',
        'ad_rewarded', 'tip', 'api_usage', 'adjustment'
    )),
    version INTEGER NOT NULL CHECK (version > 0),
    effective_from INTEGER NOT NULL,
    effective_to INTEGER,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'retired')),
    rule_json TEXT NOT NULL,
    created_by_user_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (scope_type, scope_id, revenue_source, version),
    CHECK (effective_to IS NULL OR effective_to > effective_from)
);

CREATE INDEX IF NOT EXISTS monetization_split_rule_resolution_idx
    ON monetization_split_rules(scope_type, scope_id, revenue_source, status, effective_from DESC);

CREATE TABLE IF NOT EXISTS monetization_revenue_events (
    revenue_event_id TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL CHECK (source_kind IN (
        'payment', 'subscription', 'advertising', 'refund', 'chargeback',
        'payout', 'tip', 'api_usage', 'adjustment'
    )),
    source_id TEXT NOT NULL,
    payment_id TEXT REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    mini_app_id TEXT,
    developer_id TEXT,
    customer_user_id TEXT,
    gross_amount INTEGER NOT NULL CHECK (gross_amount >= 0),
    platform_amount INTEGER NOT NULL DEFAULT 0 CHECK (platform_amount >= 0),
    developer_amount INTEGER NOT NULL DEFAULT 0 CHECK (developer_amount >= 0),
    currency TEXT NOT NULL,
    split_rule_id TEXT REFERENCES monetization_split_rules(rule_id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'posted', 'reversed', 'rejected')),
    metadata_json TEXT NOT NULL DEFAULT '{}',
    occurred_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (source_kind, source_id),
    CHECK (platform_amount + developer_amount <= gross_amount)
);

CREATE INDEX IF NOT EXISTS monetization_revenue_developer_idx
    ON monetization_revenue_events(developer_id, currency, occurred_at DESC);
CREATE INDEX IF NOT EXISTS monetization_revenue_customer_idx
    ON monetization_revenue_events(customer_user_id, occurred_at DESC);

CREATE TABLE IF NOT EXISTS monetization_subscriptions (
    subscription_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    mini_app_id TEXT NOT NULL,
    developer_id TEXT NOT NULL,
    product_id TEXT NOT NULL REFERENCES products(product_id) ON DELETE RESTRICT,
    price_id TEXT NOT NULL REFERENCES prices(price_id) ON DELETE RESTRICT,
    payment_id TEXT REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    provider TEXT NOT NULL,
    provider_subscription_reference TEXT,
    entitlement_capability TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'trialing', 'active', 'past_due', 'paused', 'cancelled', 'expired', 'refunded'
    )),
    current_period_start INTEGER,
    current_period_end INTEGER,
    cancel_at_period_end INTEGER NOT NULL DEFAULT 0 CHECK (cancel_at_period_end IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (provider, provider_subscription_reference),
    CHECK (current_period_end IS NULL OR current_period_start IS NULL OR current_period_end > current_period_start)
);

CREATE INDEX IF NOT EXISTS monetization_subscriptions_user_idx
    ON monetization_subscriptions(user_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS monetization_subscriptions_developer_idx
    ON monetization_subscriptions(developer_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS monetization_provider_events (
    provider TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('received', 'processed', 'rejected')),
    subject_id TEXT,
    occurred_at INTEGER NOT NULL,
    processed_at INTEGER,
    error_code TEXT,
    PRIMARY KEY (provider, event_id)
);

CREATE TABLE IF NOT EXISTS monetization_ad_campaigns (
    campaign_id TEXT PRIMARY KEY,
    advertiser_id TEXT NOT NULL,
    billing_model TEXT NOT NULL CHECK (billing_model IN ('cpm', 'cpc', 'cpa', 'rewarded')),
    currency TEXT NOT NULL,
    bid_amount INTEGER NOT NULL CHECK (bid_amount > 0),
    daily_budget INTEGER NOT NULL CHECK (daily_budget > 0),
    total_budget INTEGER NOT NULL CHECK (total_budget > 0),
    spent_amount INTEGER NOT NULL DEFAULT 0 CHECK (spent_amount >= 0),
    status TEXT NOT NULL CHECK (status IN ('draft', 'active', 'paused', 'ended')),
    starts_at INTEGER NOT NULL,
    ends_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (spent_amount <= total_budget),
    CHECK (ends_at IS NULL OR ends_at > starts_at)
);

CREATE INDEX IF NOT EXISTS monetization_ad_campaign_active_idx
    ON monetization_ad_campaigns(status, starts_at, ends_at);

CREATE TABLE IF NOT EXISTS monetization_ad_placements (
    placement_id TEXT PRIMARY KEY,
    mini_app_id TEXT NOT NULL,
    developer_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('banner', 'native', 'interstitial', 'rewarded')),
    developer_share_bps INTEGER NOT NULL DEFAULT 7000 CHECK (developer_share_bps BETWEEN 0 AND 10000),
    status TEXT NOT NULL CHECK (status IN ('active', 'paused', 'disabled')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS monetization_ad_placement_developer_idx
    ON monetization_ad_placements(developer_id, status);

CREATE TABLE IF NOT EXISTS monetization_ad_events (
    ad_event_id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    campaign_id TEXT NOT NULL REFERENCES monetization_ad_campaigns(campaign_id) ON DELETE RESTRICT,
    placement_id TEXT NOT NULL REFERENCES monetization_ad_placements(placement_id) ON DELETE RESTRICT,
    mini_app_id TEXT NOT NULL,
    developer_id TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('impression', 'click', 'conversion', 'rewarded')),
    quantity INTEGER NOT NULL DEFAULT 1 CHECK (quantity > 0 AND quantity <= 1000),
    billable_amount INTEGER NOT NULL DEFAULT 0 CHECK (billable_amount >= 0),
    currency TEXT NOT NULL,
    session_hash TEXT,
    actor_hash TEXT,
    verification_state TEXT NOT NULL CHECK (verification_state IN ('pending', 'verified', 'rejected')),
    verification_reason TEXT,
    occurred_at INTEGER NOT NULL,
    verified_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS monetization_ad_events_campaign_idx
    ON monetization_ad_events(campaign_id, verification_state, occurred_at DESC);
CREATE INDEX IF NOT EXISTS monetization_ad_events_developer_idx
    ON monetization_ad_events(developer_id, verification_state, occurred_at DESC);

CREATE TABLE IF NOT EXISTS monetization_payout_requests (
    request_id TEXT PRIMARY KEY,
    developer_id TEXT NOT NULL REFERENCES monetization_developer_profiles(developer_id) ON DELETE RESTRICT,
    requester_user_id TEXT NOT NULL,
    payout_account_id TEXT NOT NULL REFERENCES developer_payout_accounts(payout_account_id) ON DELETE RESTRICT,
    currency TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK (amount > 0),
    idempotency_key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN (
        'requested', 'reviewing', 'approved', 'submitted', 'paid', 'failed', 'cancelled'
    )),
    canonical_payout_id TEXT REFERENCES developer_payouts(payout_id) ON DELETE RESTRICT,
    failure_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS monetization_payout_requests_developer_idx
    ON monetization_payout_requests(developer_id, status, created_at DESC);

-- Unified developer balance projection. Pending/available stay ledger-derived;
-- no mutable balance column is introduced here.
CREATE VIEW IF NOT EXISTS monetization_developer_balances AS
SELECT
    profile.developer_id,
    wa.currency,
    COALESCE(MAX(CASE WHEN wa.account_id = 'developer-pending:' || profile.developer_id || ':' || wa.currency
                      THEN wb.balance END), 0) AS pending,
    COALESCE(MAX(CASE WHEN wa.account_id = 'developer-available:' || profile.developer_id || ':' || wa.currency
                      THEN wb.balance END), 0) AS available
FROM monetization_developer_profiles profile
LEFT JOIN wallet_accounts wa
  ON wa.owner_type = 'developer'
 AND wa.owner_id IN (profile.developer_id || ':pending', profile.developer_id || ':available')
LEFT JOIN wallet_balances wb ON wb.account_id = wa.account_id
GROUP BY profile.developer_id, wa.currency;
