PRAGMA foreign_keys = ON;

-- The control plane stores only authenticated device/client/session metadata and
-- short-lived WebRTC signaling. Desktop frames and input events travel over the
-- peer-to-peer data channel and are never persisted in D1.
CREATE TABLE IF NOT EXISTS remote_computers (
    device_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    label TEXT NOT NULL,
    device_secret_hash TEXT NOT NULL,
    pairing_code_hash TEXT,
    pairing_expires_at INTEGER,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE INDEX IF NOT EXISTS remote_computers_user_idx
    ON remote_computers(user_id, revoked_at, last_seen_at);
CREATE INDEX IF NOT EXISTS remote_computers_pairing_idx
    ON remote_computers(user_id, pairing_code_hash, pairing_expires_at);

CREATE TABLE IF NOT EXISTS remote_computer_clients (
    client_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES remote_computers(device_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    label TEXT NOT NULL,
    paired_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE INDEX IF NOT EXISTS remote_computer_clients_device_idx
    ON remote_computer_clients(user_id, device_id, revoked_at, paired_at);

CREATE TABLE IF NOT EXISTS remote_computer_sessions (
    session_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES remote_computers(device_id) ON DELETE CASCADE,
    client_id TEXT NOT NULL REFERENCES remote_computer_clients(client_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    mobile_token_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'active', 'closed')),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    closed_at INTEGER
);

CREATE INDEX IF NOT EXISTS remote_computer_sessions_device_idx
    ON remote_computer_sessions(user_id, device_id, state, expires_at);

CREATE TABLE IF NOT EXISTS remote_computer_signals (
    signal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES remote_computer_sessions(session_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    sender_role TEXT NOT NULL CHECK (sender_role IN ('desktop', 'mobile')),
    kind TEXT NOT NULL CHECK (kind IN ('offer', 'answer', 'ice', 'ready', 'close')),
    payload_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS remote_computer_signals_pending_idx
    ON remote_computer_signals(user_id, session_id, sender_role, signal_id, expires_at);
