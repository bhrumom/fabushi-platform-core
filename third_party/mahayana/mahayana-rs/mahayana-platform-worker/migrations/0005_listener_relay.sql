PRAGMA foreign_keys = ON;

-- Account-scoped listener registrations are the durable handshake between a
-- desktop Host and provider-specific webhook adapters. Providers may deliver
-- at least once; the desktop acknowledges only events it actually processed.
CREATE TABLE IF NOT EXISTS listener_registrations (
    user_id TEXT NOT NULL,
    platform TEXT NOT NULL CHECK (platform IN ('slack', 'github', 'git', 'teams', 'linear', 'sentry', 'pagerduty')),
    subscriptions_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, platform)
);

CREATE TABLE IF NOT EXISTS listener_events (
    event_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    platform TEXT NOT NULL CHECK (platform IN ('slack', 'github', 'git', 'teams', 'linear', 'sentry', 'pagerduty')),
    event_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    acknowledged_at INTEGER
);

CREATE INDEX IF NOT EXISTS listener_events_pending_idx
    ON listener_events(user_id, acknowledged_at, created_at);
