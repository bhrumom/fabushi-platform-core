-- FDN-001 exact-tool capability grants.
-- Tool authority is bound to the existing Remote Computer session and exact tool_name.
-- No tool arguments, credentials, or sensitive-input plaintext are persisted.
CREATE TABLE IF NOT EXISTS remote_computer_session_tool_grants (
    session_id TEXT NOT NULL REFERENCES remote_computer_sessions(session_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    tool_name TEXT NOT NULL CHECK (length(tool_name) BETWEEN 1 AND 128),
    generation INTEGER NOT NULL DEFAULT 1 CHECK (generation >= 1),
    granted_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    PRIMARY KEY (session_id, tool_name)
);

CREATE INDEX IF NOT EXISTS remote_computer_session_tool_grants_account_idx
    ON remote_computer_session_tool_grants(
        user_id, device_id, client_id, session_id, tool_name, revoked_at, expires_at
    );

ALTER TABLE remote_computer_operations ADD COLUMN grant_generation INTEGER;

CREATE TABLE IF NOT EXISTS remote_computer_tool_grant_audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    generation INTEGER NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('granted','regranted','revoked')),
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS remote_computer_tool_grant_audit_account_idx
    ON remote_computer_tool_grant_audit(
        user_id, device_id, client_id, session_id, created_at DESC
    );

CREATE TRIGGER IF NOT EXISTS remote_computer_tool_grant_binding_immutable
BEFORE UPDATE ON remote_computer_session_tool_grants
FOR EACH ROW
WHEN NEW.session_id <> OLD.session_id
  OR NEW.user_id <> OLD.user_id
  OR NEW.device_id <> OLD.device_id
  OR NEW.client_id <> OLD.client_id
  OR NEW.tool_name <> OLD.tool_name
  OR NEW.generation < OLD.generation
BEGIN
    SELECT RAISE(ABORT, 'remote exact-tool grant binding or generation cannot move backwards');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_tool_grant_created_audit
AFTER INSERT ON remote_computer_session_tool_grants
FOR EACH ROW
BEGIN
    INSERT INTO remote_computer_tool_grant_audit
      (user_id,device_id,client_id,session_id,tool_name,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,NEW.tool_name,NEW.generation,'granted',NEW.granted_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_tool_grant_regrant_audit
AFTER UPDATE OF generation ON remote_computer_session_tool_grants
FOR EACH ROW WHEN NEW.generation > OLD.generation
BEGIN
    INSERT INTO remote_computer_tool_grant_audit
      (user_id,device_id,client_id,session_id,tool_name,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,NEW.tool_name,NEW.generation,'regranted',NEW.granted_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_tool_grant_revoke_audit
AFTER UPDATE OF revoked_at ON remote_computer_session_tool_grants
FOR EACH ROW WHEN OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL
BEGIN
    INSERT INTO remote_computer_tool_grant_audit
      (user_id,device_id,client_id,session_id,tool_name,generation,event_type,created_at)
    VALUES
      (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,NEW.tool_name,NEW.generation,'revoked',NEW.revoked_at);
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_session_close_revoke_tool_grants
AFTER UPDATE OF state ON remote_computer_sessions
FOR EACH ROW WHEN OLD.state <> 'closed' AND NEW.state = 'closed'
BEGIN
    UPDATE remote_computer_session_tool_grants
       SET revoked_at = COALESCE(
           revoked_at,
           COALESCE(NEW.closed_at, CAST(strftime('%s','now') AS INTEGER))
       )
     WHERE session_id = NEW.session_id AND revoked_at IS NULL;
END;
