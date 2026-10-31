-- FDN-001 short-lived one-time network enrollment grants.
-- Raw grant secrets, device secrets, account/session credentials, network private
-- keys and Headscale credentials must never be persisted in this table.
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS remote_computer_network_enrollment_grants (
    grant_id TEXT PRIMARY KEY CHECK (length(grant_id) BETWEEN 24 AND 96),
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES remote_computers(device_id) ON DELETE CASCADE,
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 16 AND 160),
    cell_id TEXT NOT NULL CHECK (length(cell_id) BETWEEN 8 AND 128),
    generation INTEGER NOT NULL CHECK (generation >= 1),
    secret_hash TEXT NOT NULL CHECK (length(secret_hash) = 43),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    redeemed_at INTEGER,
    revoked_at INTEGER,
    CHECK (expires_at > created_at),
    CHECK (expires_at - created_at <= 300),
    CHECK (redeemed_at IS NULL OR redeemed_at >= created_at),
    CHECK (revoked_at IS NULL OR revoked_at >= created_at)
);

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_network_enrollment_request_idx
    ON remote_computer_network_enrollment_grants(user_id, device_id, request_id);

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_network_enrollment_live_device_idx
    ON remote_computer_network_enrollment_grants(user_id, device_id)
    WHERE redeemed_at IS NULL AND revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS remote_computer_network_enrollment_expiry_idx
    ON remote_computer_network_enrollment_grants(expires_at, redeemed_at, revoked_at);

CREATE TRIGGER IF NOT EXISTS remote_computer_network_enrollment_identity_immutable
BEFORE UPDATE ON remote_computer_network_enrollment_grants
FOR EACH ROW
WHEN NEW.grant_id <> OLD.grant_id
  OR NEW.user_id <> OLD.user_id
  OR NEW.device_id <> OLD.device_id
  OR NEW.request_id <> OLD.request_id
  OR NEW.cell_id <> OLD.cell_id
  OR NEW.generation <> OLD.generation
  OR NEW.secret_hash <> OLD.secret_hash
  OR NEW.created_at <> OLD.created_at
  OR NEW.expires_at <> OLD.expires_at
BEGIN
    SELECT RAISE(ABORT, 'remote network enrollment grant identity is immutable');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_enrollment_terminal_once
BEFORE UPDATE ON remote_computer_network_enrollment_grants
FOR EACH ROW
WHEN (OLD.redeemed_at IS NOT NULL AND NEW.redeemed_at <> OLD.redeemed_at)
  OR (OLD.revoked_at IS NOT NULL AND NEW.revoked_at <> OLD.revoked_at)
  OR (OLD.redeemed_at IS NOT NULL AND NEW.revoked_at IS NOT OLD.revoked_at)
  OR (OLD.revoked_at IS NOT NULL AND NEW.redeemed_at IS NOT OLD.redeemed_at)
BEGIN
    SELECT RAISE(ABORT, 'remote network enrollment grant terminal state is immutable');
END;
