-- Browser/OAuth callbacks use an explicit `failed` terminal state. Migration
-- 0002 predates that state and its CHECK constraint would reject the callback
-- update. Rebuild only this ephemeral attempt table while preserving any
-- outstanding attempts; provider identities and durable account sessions are
-- untouched.

CREATE TABLE account_oauth_attempts_v2 (
    attempt_id TEXT PRIMARY KEY,
    state_hash TEXT NOT NULL UNIQUE,
    code_verifier TEXT NOT NULL,
    provider TEXT NOT NULL,
    device_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed', 'expired', 'cancelled', 'failed')),
    session_json TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    completed_at INTEGER,
    delivered_at INTEGER
);

INSERT INTO account_oauth_attempts_v2 (
    attempt_id,
    state_hash,
    code_verifier,
    provider,
    device_id,
    status,
    session_json,
    created_at,
    expires_at,
    completed_at,
    delivered_at
)
SELECT
    attempt_id,
    state_hash,
    code_verifier,
    provider,
    device_id,
    status,
    session_json,
    created_at,
    expires_at,
    completed_at,
    delivered_at
FROM account_oauth_attempts;

DROP TABLE account_oauth_attempts;
ALTER TABLE account_oauth_attempts_v2 RENAME TO account_oauth_attempts;

CREATE INDEX IF NOT EXISTS idx_account_oauth_attempts_expiry
    ON account_oauth_attempts(status, expires_at);
