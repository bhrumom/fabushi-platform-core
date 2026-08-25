-- FAB-P0001 / M9 Global Dharma paid local prayer-wheel capability normalization.
-- 0009 may already be applied in production, so this is intentionally a forward-only repair.
-- The host capability is the exact execution boundary: local.prayer-wheel.start.

UPDATE products
   SET entitlement_capability = 'local.prayer-wheel.start'
 WHERE product_id IN (
   'prod.global-dharma.local-prayer-wheel.monthly',
   'prod.global-dharma.local-prayer-wheel.lifetime'
 )
   AND entitlement_capability = 'local-prayer-wheel';

UPDATE payment_product_catalog
   SET entitlement_capability = 'local.prayer-wheel.start',
       updated_by_user_id = 'system:official'
 WHERE product_id IN (
   'prod.global-dharma.local-prayer-wheel.monthly',
   'prod.global-dharma.local-prayer-wheel.lifetime'
 )
   AND entitlement_capability = 'local-prayer-wheel';

UPDATE payment_intents
   SET entitlement_capability = 'local.prayer-wheel.start'
 WHERE product_id IN (
   'prod.global-dharma.local-prayer-wheel.monthly',
   'prod.global-dharma.local-prayer-wheel.lifetime'
 )
   AND entitlement_capability = 'local-prayer-wheel';

UPDATE monetization_subscriptions
   SET entitlement_capability = 'local.prayer-wheel.start'
 WHERE product_id IN (
   'prod.global-dharma.local-prayer-wheel.monthly',
   'prod.global-dharma.local-prayer-wheel.lifetime'
 )
   AND entitlement_capability = 'local-prayer-wheel';

-- Avoid violating UNIQUE(order_id, capability) if a previous repair already created
-- the canonical capability beside the legacy capability.
DELETE FROM entitlements
 WHERE product_id IN (
   'prod.global-dharma.local-prayer-wheel.monthly',
   'prod.global-dharma.local-prayer-wheel.lifetime'
 )
   AND capability = 'local-prayer-wheel'
   AND EXISTS (
     SELECT 1
       FROM entitlements canonical
      WHERE canonical.order_id = entitlements.order_id
        AND canonical.capability = 'local.prayer-wheel.start'
   );

UPDATE entitlements
   SET capability = 'local.prayer-wheel.start'
 WHERE product_id IN (
   'prod.global-dharma.local-prayer-wheel.monthly',
   'prod.global-dharma.local-prayer-wheel.lifetime'
 )
   AND capability = 'local-prayer-wheel';

-- Backfill already-created monthly entitlements from the immutable server-side
-- product term. New reads also compute this fallback, so delayed provider lifecycle
-- delivery can never turn a 30-day subscription into a permanent entitlement.
UPDATE entitlements
   SET expires_at = granted_at + 2592000
 WHERE product_id = 'prod.global-dharma.local-prayer-wheel.monthly'
   AND status = 'active'
   AND expires_at IS NULL;
