-- FDN-001 account/cell-scoped private ingress peer projection.
-- Only the dedicated network authority may write this table. It contains no
-- account tokens, device secrets, service credentials, Headscale auth keys,
-- private keys, or target network proof material.
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS remote_computer_network_ingress_bindings (
    user_id TEXT NOT NULL CHECK (length(user_id) BETWEEN 1 AND 160),
    cell_id TEXT NOT NULL CHECK (length(cell_id) BETWEEN 8 AND 128),
    service_id TEXT NOT NULL CHECK (length(service_id) BETWEEN 3 AND 64),
    node_id TEXT NOT NULL CHECK (length(node_id) BETWEEN 8 AND 160),
    generation INTEGER NOT NULL CHECK (generation >= 1),
    bound_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    PRIMARY KEY (user_id, cell_id, service_id),
    CHECK (expires_at > bound_at)
);

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_network_ingress_live_node_idx
    ON remote_computer_network_ingress_bindings(node_id)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS remote_computer_network_ingress_account_cell_idx
    ON remote_computer_network_ingress_bindings(user_id, cell_id, service_id, generation, revoked_at, expires_at);

CREATE TABLE IF NOT EXISTS remote_computer_network_ingress_audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    cell_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('bound','rotated','revoked')),
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS remote_computer_network_ingress_audit_account_idx
    ON remote_computer_network_ingress_audit(user_id, cell_id, service_id, generation, created_at DESC);

CREATE TRIGGER IF NOT EXISTS remote_computer_network_ingress_identity_immutable
BEFORE UPDATE ON remote_computer_network_ingress_bindings
FOR EACH ROW
WHEN NEW.user_id <> OLD.user_id
  OR NEW.cell_id <> OLD.cell_id
  OR NEW.service_id <> OLD.service_id
  OR NEW.generation < OLD.generation
BEGIN
    SELECT RAISE(ABORT, 'remote network ingress identity or generation cannot move backwards');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_ingress_created_audit
AFTER INSERT ON remote_computer_network_ingress_bindings
FOR EACH ROW
BEGIN
    INSERT INTO remote_computer_network_ingress_audit
      (user_id,cell_id,service_id,node_id,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.cell_id,NEW.service_id,NEW.node_id,NEW.generation,'bound',NEW.bound_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_ingress_rotated_audit
AFTER UPDATE OF generation ON remote_computer_network_ingress_bindings
FOR EACH ROW WHEN NEW.generation > OLD.generation
BEGIN
    INSERT INTO remote_computer_network_ingress_audit
      (user_id,cell_id,service_id,node_id,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.cell_id,NEW.service_id,NEW.node_id,NEW.generation,'rotated',NEW.bound_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_network_ingress_revoke_audit
AFTER UPDATE OF revoked_at ON remote_computer_network_ingress_bindings
FOR EACH ROW WHEN OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL
BEGIN
    INSERT INTO remote_computer_network_ingress_audit
      (user_id,cell_id,service_id,node_id,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.cell_id,NEW.service_id,NEW.node_id,NEW.generation,'revoked',NEW.revoked_at);
END;
