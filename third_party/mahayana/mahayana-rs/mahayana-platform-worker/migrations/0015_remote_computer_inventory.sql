PRAGMA foreign_keys = ON;

-- Additive inventory metadata for account-scoped device discovery. Existing
-- devices remain compatible and are identified as the current Fabushi WebRTC
-- provider until they refresh their registration.
ALTER TABLE remote_computers
    ADD COLUMN provider TEXT NOT NULL DEFAULT 'fabushi-webrtc'
    CHECK (provider IN ('fabushi-webrtc', 'rustdesk-sidecar'));

ALTER TABLE remote_computers
    ADD COLUMN platform TEXT NOT NULL DEFAULT 'unknown'
    CHECK (platform IN ('windows', 'macos', 'linux', 'android', 'ios', 'web', 'unknown'));

ALTER TABLE remote_computers
    ADD COLUMN app_version TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE remote_computers
    ADD COLUMN capabilities_json TEXT NOT NULL DEFAULT '[]';

CREATE INDEX IF NOT EXISTS remote_computers_inventory_idx
    ON remote_computers(user_id, revoked_at, provider, platform, last_seen_at);
