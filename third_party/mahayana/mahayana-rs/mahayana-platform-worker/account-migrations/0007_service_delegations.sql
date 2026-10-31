-- FDN-001 official MCP service delegation.
-- Ordinary Fabushi access/refresh credentials are never persisted here.
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS account_service_delegations (
    delegation_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    parent_session_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    external_client_id_hash TEXT NOT NULL CHECK(length(external_client_id_hash) = 64),
    resource TEXT NOT NULL,
    scopes_json TEXT NOT NULL,
    current_refresh_token_hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    revoked_reason TEXT,
    FOREIGN KEY(parent_session_id) REFERENCES account_sessions(session_id) ON DELETE CASCADE,
    UNIQUE(user_id, parent_session_id, service_id, connection_id)
);

CREATE INDEX IF NOT EXISTS idx_account_service_delegations_session
    ON account_service_delegations(user_id, parent_session_id, service_id, revoked_at, expires_at);

CREATE TABLE IF NOT EXISTS account_service_refresh_tokens (
    token_hash TEXT PRIMARY KEY,
    delegation_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('active','used','revoked')),
    issued_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    replaced_by_hash TEXT,
    UNIQUE(delegation_id, generation),
    FOREIGN KEY(delegation_id) REFERENCES account_service_delegations(delegation_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_account_service_refresh_delegation
    ON account_service_refresh_tokens(delegation_id, state);
