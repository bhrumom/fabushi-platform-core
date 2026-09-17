-- RDF-006 security groundwork: durable least-privilege grants and automatic audit events.
-- Fabushi remains the authorization authority; provider implementations consume these
-- grants but cannot widen them. No frame/input/clipboard/file/audio payload is persisted.
CREATE TABLE IF NOT EXISTS remote_computer_session_grants (
    session_id TEXT PRIMARY KEY REFERENCES remote_computer_sessions(session_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    allow_display INTEGER NOT NULL DEFAULT 1 CHECK (allow_display IN (0, 1)),
    allow_input INTEGER NOT NULL DEFAULT 1 CHECK (allow_input IN (0, 1)),
    allow_clipboard INTEGER NOT NULL DEFAULT 0 CHECK (allow_clipboard IN (0, 1)),
    allow_file_transfer INTEGER NOT NULL DEFAULT 0 CHECK (allow_file_transfer IN (0, 1)),
    allow_audio INTEGER NOT NULL DEFAULT 0 CHECK (allow_audio IN (0, 1)),
    granted_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE INDEX IF NOT EXISTS remote_computer_session_grants_account_idx
    ON remote_computer_session_grants(user_id, device_id, client_id, revoked_at, expires_at);
CREATE TABLE IF NOT EXISTS remote_computer_audit_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    client_id TEXT,
    session_id TEXT,
    event_type TEXT NOT NULL CHECK (event_type IN ('session-created','session-activated','session-closed','route-selected','grant-revoked')),
    provider TEXT,
    route TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS remote_computer_audit_events_account_idx
    ON remote_computer_audit_events(user_id, device_id, created_at DESC);
CREATE TRIGGER IF NOT EXISTS remote_computer_session_default_grant
AFTER INSERT ON remote_computer_sessions
FOR EACH ROW
BEGIN
    INSERT INTO remote_computer_session_grants
      (session_id,user_id,device_id,client_id,allow_display,allow_input,allow_clipboard,allow_file_transfer,allow_audio,granted_at,expires_at,revoked_at)
    VALUES (NEW.session_id,NEW.user_id,NEW.device_id,NEW.client_id,1,1,0,0,0,NEW.created_at,NEW.expires_at,NULL);
    INSERT INTO remote_computer_audit_events
      (user_id,device_id,client_id,session_id,event_type,provider,route,created_at)
    VALUES (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,'session-created',NEW.provider,NULL,NEW.created_at);
END;
CREATE TRIGGER IF NOT EXISTS remote_computer_session_activation_audit
AFTER UPDATE OF state ON remote_computer_sessions
FOR EACH ROW WHEN OLD.state='pending' AND NEW.state='active'
BEGIN
    INSERT INTO remote_computer_audit_events
      (user_id,device_id,client_id,session_id,event_type,provider,route,created_at)
    VALUES (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,'session-activated',NEW.provider,NEW.selected_route,CAST(strftime('%s','now') AS INTEGER));
END;
CREATE TRIGGER IF NOT EXISTS remote_computer_session_route_audit
AFTER UPDATE OF selected_route ON remote_computer_sessions
FOR EACH ROW WHEN NEW.selected_route IS NOT OLD.selected_route AND NEW.selected_route IS NOT NULL
BEGIN
    INSERT INTO remote_computer_audit_events
      (user_id,device_id,client_id,session_id,event_type,provider,route,created_at)
    VALUES (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,'route-selected',NEW.provider,NEW.selected_route,CAST(strftime('%s','now') AS INTEGER));
END;
CREATE TRIGGER IF NOT EXISTS remote_computer_session_close_revoke_grant
AFTER UPDATE OF state ON remote_computer_sessions
FOR EACH ROW WHEN OLD.state<>'closed' AND NEW.state='closed'
BEGIN
    UPDATE remote_computer_session_grants SET revoked_at=COALESCE(revoked_at,COALESCE(NEW.closed_at,CAST(strftime('%s','now') AS INTEGER))) WHERE session_id=NEW.session_id AND revoked_at IS NULL;
    INSERT INTO remote_computer_audit_events (user_id,device_id,client_id,session_id,event_type,provider,route,created_at)
    VALUES (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,'session-closed',NEW.provider,NEW.selected_route,COALESCE(NEW.closed_at,CAST(strftime('%s','now') AS INTEGER)));
    INSERT INTO remote_computer_audit_events (user_id,device_id,client_id,session_id,event_type,provider,route,created_at)
    VALUES (NEW.user_id,NEW.device_id,NEW.client_id,NEW.session_id,'grant-revoked',NEW.provider,NEW.selected_route,COALESCE(NEW.closed_at,CAST(strftime('%s','now') AS INTEGER)));
END;
CREATE TRIGGER IF NOT EXISTS remote_computer_session_grant_no_escalation
BEFORE UPDATE ON remote_computer_session_grants
FOR EACH ROW
WHEN NEW.allow_display>OLD.allow_display OR NEW.allow_input>OLD.allow_input OR NEW.allow_clipboard>OLD.allow_clipboard OR NEW.allow_file_transfer>OLD.allow_file_transfer OR NEW.allow_audio>OLD.allow_audio
BEGIN
    SELECT RAISE(ABORT,'remote session grants cannot be escalated in place');
END;
