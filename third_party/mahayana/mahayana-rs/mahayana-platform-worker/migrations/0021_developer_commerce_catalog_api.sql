PRAGMA foreign_keys = ON;

-- The old Global Dharma migration is kept in the migration history so product
-- IDs, prices, orders and entitlements remain addressable. From this point on
-- those rows are ordinary Developer Commerce catalog records, not a privileged
-- built-in product path. New writes must come through the developer API.
ALTER TABLE payment_product_catalog
    ADD COLUMN catalog_source TEXT NOT NULL DEFAULT 'developer_api'
    CHECK (catalog_source IN ('developer_api', 'legacy_migration'));

UPDATE payment_product_catalog
   SET catalog_source = 'developer_api'
 WHERE catalog_source = 'legacy_migration';

-- Record the one-time adoption without changing the immutable product identity
-- or deleting any historical purchase/entitlement relation.
INSERT OR IGNORE INTO developer_commerce_audit_events
    (event_id, developer_id, mini_app_id, product_id, actor_user_id,
     event_type, payload_json, created_at)
SELECT
    'audit.developer-commerce.adoption.' || c.product_id,
    c.developer_id,
    c.mini_app_id,
    c.product_id,
    'system:developer-commerce-adoption',
    'product.adopted_to_developer_api',
    '{"catalogSource":"developer_api","previousSource":"legacy_migration"}',
    c.updated_at
FROM payment_product_catalog c
WHERE c.catalog_source = 'developer_api'
  AND c.created_by_user_id = 'system:official';
