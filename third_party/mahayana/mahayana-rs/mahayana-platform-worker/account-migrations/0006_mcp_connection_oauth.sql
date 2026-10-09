-- Service consent attempts are separate from account sign-in attempts.
CREATE TABLE IF NOT EXISTS account_mcp_oauth_attempts (
    attempt_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    state_hash TEXT NOT NULL UNIQUE,
    ticket_hash TEXT NOT NULL,
    verifier TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','exchanging','ready','failed','expired','cancelled','consumed')),
    payload_ciphertext TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS account_mcp_oauth_expiry_idx ON account_mcp_oauth_attempts(expires_at);
CREATE INDEX IF NOT EXISTS account_mcp_oauth_session_idx ON account_mcp_oauth_attempts(user_id,session_id,status);

-- Binding hashes are not usable tokens. Raw credentials live only in encrypted
-- delivery ciphertext (briefly) and the account-scoped native credential vault.
CREATE TABLE IF NOT EXISTS account_mcp_native_credentials (
    connection_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    access_digest TEXT NOT NULL,
    refresh_digest TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY(connection_id) REFERENCES account_connections(connection_id) ON DELETE CASCADE
);
