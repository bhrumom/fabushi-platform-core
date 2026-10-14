-- FDN-001 trusted private-network node projection on the existing Remote Computer authority.
-- Raw node proof material and all account/device/session/network secrets remain outside D1.
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS remote_computer_network_bindings (
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES remote_computers(device_id) ON DELETE CASCADE,
    cell_id TEXT NOT NULL CHECK (length(cell_id) BETWEEN 8 AND 128),
    node_id TEXT NOT NULL CHECK (length(node_id) BETWEEN 8 AND 160),
    proof_sha256 TEXT NOT NULL CHECK (length(proof_sha256) = 64),
    generation INTEGER NOT NULL CHECK (generation >= 1),
    bound_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    PRIMARY KEY (user_id, device_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_network_live_node_idx
    ON remote_computer_network_bindings(node_id)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS remote_computer_network_account_idx
    ON remote_computer_network_bindings(user_id, device_id, generation, revoked_at, expires_at);

CREATE TABLE IF NOT EXISTS remote_computer_network_binding_audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    cell_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    proof_sha256 TEXT NOT NULL,
    generation INTEGER NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('bound','rebound','revoked')),
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS remote_computer_network_binding_audit_account_idx
    ON remote_computer_network_binding_audit(user_id, device_id, generation, created_at DESC);

CREATE TRIGGER IF NOT EXISTS remote_computer_network_binding_identity_immutable
BEFORE UPDATE ON remote_computer_network_bindings
FOR EACH ROW
WHEN NEW.user_id <> OLD.user_id
  OR NEW.device_id <> OLD.device_id
  OR NEW.generation < OLD.generation
BEGIN
    SELECT RAISE(ABORT, 'remote network binding identity or generation cannot move backwards');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_binding_created_audit
AFTER INSERT ON remote_computer_network_bindings
FOR EACH ROW
BEGIN
    INSERT INTO remote_computer_network_binding_audit
      (user_id,device_id,cell_id,node_id,proof_sha256,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.device_id,NEW.cell_id,NEW.node_id,NEW.proof_sha256,NEW.generation,'bound',NEW.bound_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_binding_rebound_audit
AFTER UPDATE OF generation ON remote_computer_network_bindings
FOR EACH ROW WHEN NEW.generation > OLD.generation
BEGIN
    INSERT INTO remote_computer_network_binding_audit
      (user_id,device_id,cell_id,node_id,proof_sha256,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.device_id,NEW.cell_id,NEW.node_id,NEW.proof_sha256,NEW.generation,'rebound',NEW.bound_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_binding_revoke_audit
AFTER UPDATE OF revoked_at ON remote_computer_network_bindings
FOR EACH ROW WHEN OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL
BEGIN
    INSERT INTO remote_computer_network_binding_audit
      (user_id,device_id,cell_id,node_id,proof_sha256,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.device_id,NEW.cell_id,NEW.node_id,NEW.proof_sha256,NEW.generation,'revoked',NEW.revoked_at);
END;
