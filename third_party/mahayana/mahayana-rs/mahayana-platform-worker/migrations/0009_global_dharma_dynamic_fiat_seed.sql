-- Global Dharma is intentionally a normal Developer Commerce consumer.
-- Store-specific provider states remain fail-closed until production program
-- entitlements / credentials are configured; Web is immediately catalog-ready.

INSERT OR IGNORE INTO products
  (product_id, plugin_id, sku, seller_user_id, entitlement_capability, consumption_mode, active, created_at, updated_at)
VALUES
  ('prod.global-dharma.local-prayer-wheel.monthly', 'global-dharma', 'local-prayer-wheel.monthly', 'official.fabushi', 'local-prayer-wheel', 'durable', 1, 0, 0),
  ('prod.global-dharma.local-prayer-wheel.lifetime', 'global-dharma', 'local-prayer-wheel.lifetime', 'official.fabushi', 'local-prayer-wheel', 'durable', 1, 0, 0);

INSERT OR IGNORE INTO prices
  (price_id, product_id, currency, amount, active, starts_at, ends_at, created_at)
VALUES
  ('price.global-dharma.local-prayer-wheel.monthly.cny.v1', 'prod.global-dharma.local-prayer-wheel.monthly', 'CNY', 3000, 1, 0, NULL, 0),
  ('price.global-dharma.local-prayer-wheel.lifetime.cny.v1', 'prod.global-dharma.local-prayer-wheel.lifetime', 'CNY', 108000, 1, 0, NULL, 0);

INSERT OR IGNORE INTO payment_product_catalog
  (product_id, mini_app_id, developer_id, sku, display_name, description,
   product_kind, entitlement_capability, tax_code, subscription_period_seconds,
   catalog_status, created_by_user_id, updated_by_user_id, created_at, updated_at)
VALUES
  ('prod.global-dharma.local-prayer-wheel.monthly', 'global-dharma', 'official.fabushi', 'local-prayer-wheel.monthly', '本地转经轮月付', '30 天本地转经轮权限', 'subscription', 'local-prayer-wheel', NULL, 2592000, 'active', 'system:official', 'system:official', 0, 0),
  ('prod.global-dharma.local-prayer-wheel.lifetime', 'global-dharma', 'official.fabushi', 'local-prayer-wheel.lifetime', '本地转经轮永久版', '永久本地转经轮权限', 'digital_durable', 'local-prayer-wheel', NULL, NULL, 'active', 'system:official', 'system:official', 0, 0);

INSERT OR IGNORE INTO payment_product_config
  (product_id, developer_id, product_kind, platform_fee_bps, allowed_rails_json,
   provider_product_refs_json, active, created_at, updated_at)
VALUES
  ('prod.global-dharma.local-prayer-wheel.monthly', 'official.fabushi', 'subscription', 1000,
   '["apple_in_app_purchase","google_play_billing","web_provider"]',
   '{"google_play_billing":"global_dharma.local_prayer_wheel.monthly","web_provider":"fabushi.global-dharma.local-prayer-wheel.monthly"}', 1, 0, 0),
  ('prod.global-dharma.local-prayer-wheel.lifetime', 'official.fabushi', 'digital_durable', 1000,
   '["apple_in_app_purchase","google_play_billing","web_provider"]',
   '{"google_play_billing":"global_dharma.local_prayer_wheel.lifetime","web_provider":"fabushi.global-dharma.local-prayer-wheel.lifetime"}', 1, 0, 0);

INSERT OR IGNORE INTO payment_price_revisions
  (revision_id, product_id, price_id, currency, amount, actor_user_id, reason, created_at)
VALUES
  ('rev.global-dharma.local-prayer-wheel.monthly.cny.v1', 'prod.global-dharma.local-prayer-wheel.monthly', 'price.global-dharma.local-prayer-wheel.monthly.cny.v1', 'CNY', 3000, 'system:official', 'initial_seed', 0),
  ('rev.global-dharma.local-prayer-wheel.lifetime.cny.v1', 'prod.global-dharma.local-prayer-wheel.lifetime', 'price.global-dharma.local-prayer-wheel.lifetime.cny.v1', 'CNY', 108000, 'system:official', 'initial_seed', 0);

INSERT OR IGNORE INTO payment_provider_bindings
  (product_id, provider, external_product_ref, generic_product_id, sync_state,
   metadata_json, last_error, last_synced_at, created_at, updated_at)
VALUES
  ('prod.global-dharma.local-prayer-wheel.monthly', 'apple_advanced_commerce', NULL, NULL, 'pending_configuration', '{}', NULL, NULL, 0, 0),
  ('prod.global-dharma.local-prayer-wheel.monthly', 'google_play', 'global_dharma.local_prayer_wheel.monthly', NULL, 'pending_configuration', '{}', NULL, NULL, 0, 0),
  ('prod.global-dharma.local-prayer-wheel.monthly', 'web_provider', 'fabushi.global-dharma.local-prayer-wheel.monthly', NULL, 'active', '{}', NULL, 0, 0, 0),
  ('prod.global-dharma.local-prayer-wheel.lifetime', 'apple_advanced_commerce', NULL, NULL, 'pending_configuration', '{}', NULL, NULL, 0, 0),
  ('prod.global-dharma.local-prayer-wheel.lifetime', 'google_play', 'global_dharma.local_prayer_wheel.lifetime', NULL, 'pending_configuration', '{}', NULL, NULL, 0, 0),
  ('prod.global-dharma.local-prayer-wheel.lifetime', 'web_provider', 'fabushi.global-dharma.local-prayer-wheel.lifetime', NULL, 'active', '{}', NULL, 0, 0, 0);

INSERT OR IGNORE INTO developer_commerce_audit_events
  (event_id, developer_id, mini_app_id, product_id, actor_user_id, event_type, payload_json, created_at)
VALUES
  ('audit.global-dharma.local-prayer-wheel.monthly.seed', 'official.fabushi', 'global-dharma', 'prod.global-dharma.local-prayer-wheel.monthly', 'system:official', 'product.seeded', '{"currency":"CNY","amount":3000}', 0),
  ('audit.global-dharma.local-prayer-wheel.lifetime.seed', 'official.fabushi', 'global-dharma', 'prod.global-dharma.local-prayer-wheel.lifetime', 'system:official', 'product.seeded', '{"currency":"CNY","amount":108000}', 0);
