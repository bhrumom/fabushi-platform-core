-- FAB-P0001 / M9 Developer Commerce dynamic-fiat catalog.
-- Monetary values remain integer minor units. Provider credentials never live in D1.

CREATE TABLE IF NOT EXISTS developer_commerce_profiles (
    developer_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','suspended','closed')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mini_app_commerce_owners (
    mini_app_id TEXT PRIMARY KEY,
    developer_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','suspended','archived')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(developer_id) REFERENCES developer_commerce_profiles(developer_id)
);

CREATE TABLE IF NOT EXISTS mini_app_commerce_members (
    mini_app_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('owner','admin','catalog_manager','viewer')),
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(mini_app_id, user_id),
    FOREIGN KEY(mini_app_id) REFERENCES mini_app_commerce_owners(mini_app_id)
);

CREATE TABLE IF NOT EXISTS payment_product_catalog (
    product_id TEXT PRIMARY KEY,
    mini_app_id TEXT NOT NULL,
    developer_id TEXT NOT NULL,
    sku TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    product_kind TEXT NOT NULL CHECK(product_kind IN ('digital_consumable','digital_durable','subscription','physical','service')),
    entitlement_capability TEXT NOT NULL,
    tax_code TEXT,
    subscription_period_seconds INTEGER,
    catalog_status TEXT NOT NULL DEFAULT 'draft' CHECK(catalog_status IN ('draft','pending_sync','active','archived')),
    created_by_user_id TEXT NOT NULL,
    updated_by_user_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(mini_app_id, sku),
    FOREIGN KEY(mini_app_id) REFERENCES mini_app_commerce_owners(mini_app_id)
);

CREATE TABLE IF NOT EXISTS payment_price_revisions (
    revision_id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL,
    price_id TEXT NOT NULL UNIQUE,
    currency TEXT NOT NULL CHECK(length(currency) = 3),
    amount INTEGER NOT NULL CHECK(amount > 0),
    actor_user_id TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT 'developer_update',
    created_at INTEGER NOT NULL,
    FOREIGN KEY(product_id) REFERENCES payment_product_catalog(product_id)
);
CREATE INDEX IF NOT EXISTS idx_payment_price_revisions_product
    ON payment_price_revisions(product_id, created_at DESC);

CREATE TABLE IF NOT EXISTS payment_provider_bindings (
    product_id TEXT NOT NULL,
    provider TEXT NOT NULL CHECK(provider IN ('apple_advanced_commerce','google_play','web_provider','merchant_provider','credits')),
    external_product_ref TEXT,
    generic_product_id TEXT,
    sync_state TEXT NOT NULL DEFAULT 'pending_configuration'
        CHECK(sync_state IN ('pending_configuration','pending_sync','active','error','archived')),
    metadata_json TEXT NOT NULL DEFAULT '{}',
    last_error TEXT,
    last_synced_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(product_id, provider),
    FOREIGN KEY(product_id) REFERENCES payment_product_catalog(product_id)
);
CREATE INDEX IF NOT EXISTS idx_payment_provider_bindings_state
    ON payment_provider_bindings(provider, sync_state);

CREATE TABLE IF NOT EXISTS developer_commerce_audit_events (
    event_id TEXT PRIMARY KEY,
    developer_id TEXT NOT NULL,
    mini_app_id TEXT,
    product_id TEXT,
    actor_user_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO developer_commerce_profiles
    (developer_id, owner_user_id, display_name, status, created_at, updated_at)
VALUES ('official.fabushi', 'system:official', 'Fabushi Official', 'active', 0, 0);

INSERT OR IGNORE INTO mini_app_commerce_owners
    (mini_app_id, developer_id, owner_user_id, display_name, status, created_at, updated_at)
VALUES ('global-dharma', 'official.fabushi', 'system:official', '全球法布施', 'active', 0, 0);

INSERT OR IGNORE INTO mini_app_commerce_members
    (mini_app_id, user_id, role, active, created_at, updated_at)
VALUES ('global-dharma', 'system:official', 'owner', 1, 0, 0);
