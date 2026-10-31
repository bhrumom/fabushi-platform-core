-- Provider identities are deliberately separate from application users.
-- Email is profile data and must never replace the stable (issuer, subject)
-- identity key.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS account_identities (
    identity_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    email TEXT,
    email_verified INTEGER NOT NULL DEFAULT 0,
    display_name TEXT,
    avatar_url TEXT,
    created_at INTEGER NOT NULL,
    last_login_at INTEGER NOT NULL,
    UNIQUE(issuer, subject)
);

CREATE INDEX IF NOT EXISTS idx_account_identities_user
    ON account_identities(user_id, provider);

CREATE TABLE IF NOT EXISTS account_oauth_attempts (
    attempt_id TEXT PRIMARY KEY,
    state_hash TEXT NOT NULL UNIQUE,
    code_verifier TEXT NOT NULL,
    provider TEXT NOT NULL,
    device_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed', 'expired', 'cancelled')),
    session_json TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    completed_at INTEGER,
    delivered_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_account_oauth_attempts_expiry
    ON account_oauth_attempts(status, expires_at);
