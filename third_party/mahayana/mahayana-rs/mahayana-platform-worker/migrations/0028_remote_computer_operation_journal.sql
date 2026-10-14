-- FDN-001 OPJ-01..OPJ-09: durable Remote Computer operation identity and reconciliation.
-- Request arguments and sensitive-input plaintext are deliberately not persisted.
CREATE TABLE IF NOT EXISTS remote_computer_operations (
    operation_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES remote_computer_sessions(session_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    request_sha256 TEXT NOT NULL CHECK (length(request_sha256) = 64),
    sensitive_result INTEGER NOT NULL DEFAULT 0 CHECK (sensitive_result IN (0, 1)),
    state TEXT NOT NULL CHECK (state IN ('pending','running','completed','failed','outcome_unknown','cancelled')),
    result_sha256 TEXT,
    result_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (result_sha256 IS NULL OR length(result_sha256) = 64),
    CHECK (sensitive_result = 0 OR result_json IS NULL),
    UNIQUE(user_id, device_id, session_id, request_id)
);

CREATE INDEX IF NOT EXISTS remote_computer_operations_reconcile_idx
    ON remote_computer_operations(user_id, device_id, session_id, request_id, state, updated_at);

CREATE TABLE IF NOT EXISTS remote_computer_operation_audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT NOT NULL REFERENCES remote_computer_operations(operation_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    previous_state TEXT,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS remote_computer_operation_audit_account_idx
    ON remote_computer_operation_audit(user_id, device_id, session_id, created_at DESC);

CREATE TRIGGER IF NOT EXISTS remote_computer_operation_terminal_monotonic
BEFORE UPDATE OF state ON remote_computer_operations
FOR EACH ROW
WHEN OLD.state IN ('completed','failed','cancelled') AND NEW.state <> OLD.state
BEGIN
    SELECT RAISE(ABORT, 'terminal remote operation state is immutable');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_operation_transition_guard
BEFORE UPDATE OF state ON remote_computer_operations
FOR EACH ROW
WHEN NEW.state <> OLD.state AND NOT (
       (OLD.state = 'pending' AND NEW.state IN ('running','outcome_unknown','cancelled'))
    OR (OLD.state = 'running' AND NEW.state IN ('completed','failed','outcome_unknown'))
    OR (OLD.state = 'outcome_unknown' AND NEW.state IN ('completed','failed'))
)
BEGIN
    SELECT RAISE(ABORT, 'invalid remote operation state transition');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_operation_created_audit
AFTER INSERT ON remote_computer_operations
FOR EACH ROW
BEGIN
    INSERT INTO remote_computer_operation_audit
      (operation_id,user_id,device_id,client_id,session_id,request_id,tool_name,previous_state,state,created_at)
    VALUES
      (NEW.operation_id,NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,NEW.request_id,NEW.tool_name,NULL,NEW.state,NEW.created_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_operation_state_audit
AFTER UPDATE OF state ON remote_computer_operations
FOR EACH ROW WHEN NEW.state <> OLD.state
BEGIN
    INSERT INTO remote_computer_operation_audit
      (operation_id,user_id,device_id,client_id,session_id,request_id,tool_name,previous_state,state,created_at)
    VALUES
      (NEW.operation_id,NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,NEW.request_id,NEW.tool_name,OLD.state,NEW.state,NEW.updated_at);
END;
