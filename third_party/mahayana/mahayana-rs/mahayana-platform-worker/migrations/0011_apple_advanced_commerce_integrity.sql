PRAGMA foreign_keys = ON;

-- Persist the server-authoritative Advanced Commerce request that was signed
-- for StoreKit. Apple transaction verification must match this exact dynamic
-- SKU/request reference/price rather than trusting the generic App Store
-- Connect product identifier alone. Repeated checkout calls reuse this request.
CREATE TABLE IF NOT EXISTS apple_advanced_commerce_requests (
    payment_id TEXT PRIMARY KEY REFERENCES payment_intents(payment_id) ON DELETE RESTRICT,
    request_reference_id TEXT NOT NULL UNIQUE,
    generic_product_id TEXT NOT NULL,
    dynamic_sku TEXT NOT NULL,
    currency TEXT NOT NULL,
    price_milliunits INTEGER NOT NULL CHECK (price_milliunits > 0),
    tax_code TEXT NOT NULL,
    storefront TEXT NOT NULL,
    request_json TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS apple_advanced_commerce_request_sku_idx
    ON apple_advanced_commerce_requests(dynamic_sku, currency, created_at DESC);
