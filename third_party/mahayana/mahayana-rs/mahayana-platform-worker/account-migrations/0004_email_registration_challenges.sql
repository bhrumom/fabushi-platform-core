-- Browser-first Fabushi registration challenges.
-- Verification codes are never persisted directly: the Worker stores a server-peppered SHA-256
-- digest and ties each challenge to the browser login attempt that requested it.
CREATE TABLE IF NOT EXISTS account_email_challenges (
    challenge_id TEXT PRIMARY KEY,
    attempt_id TEXT NOT NULL,
    email TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('register')),
    code_hash TEXT NOT NULL,
    sent_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    consumed_at INTEGER,
    FOREIGN KEY(attempt_id) REFERENCES account_oauth_attempts(attempt_id) ON DELETE CASCADE,
    UNIQUE(email, purpose)
);

CREATE INDEX IF NOT EXISTS account_email_challenges_attempt_idx
    ON account_email_challenges(attempt_id, purpose, expires_at);
CREATE INDEX IF NOT EXISTS account_email_challenges_expiry_idx
    ON account_email_challenges(expires_at, consumed_at);
