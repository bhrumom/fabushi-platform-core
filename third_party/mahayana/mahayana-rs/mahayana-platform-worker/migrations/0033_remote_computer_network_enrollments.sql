-- FDN-001 short-lived one-time network enrollment tickets.
-- Raw tickets, device secrets, account tokens, Headscale preauth keys and network keys are never persisted.
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS remote_computer_network_enrollments (
    token_hash TEXT PRIMARY KEY CHECK (length(token_hash) BETWEEN 40 AND 96),
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES remote_computers(device_id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK (generation >= 1),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER
);

CREATE INDEX IF NOT EXISTS remote_computer_network_enrollment_device_idx
    ON remote_computer_network_enrollments(user_id, device_id, expires_at, consumed_at);

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_network_enrollment_live_device_idx
    ON remote_computer_network_enrollments(user_id, device_id)
    WHERE consumed_at IS NULL;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_enrollment_identity_immutable
BEFORE UPDATE ON remote_computer_network_enrollments
FOR EACH ROW
WHEN NEW.token_hash <> OLD.token_hash
  OR NEW.user_id <> OLD.user_id
  OR NEW.device_id <> OLD.device_id
  OR NEW.generation <> OLD.generation
  OR NEW.created_at <> OLD.created_at
  OR NEW.expires_at <> OLD.expires_at
  OR (OLD.consumed_at IS NOT NULL AND NEW.consumed_at <> OLD.consumed_at)
BEGIN
    SELECT RAISE(ABORT, 'remote network enrollment identity is immutable');
END;
