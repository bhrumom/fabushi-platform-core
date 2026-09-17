-- RDF-003: monotonic route fallback and auditable remote-session transport decisions.
CREATE TABLE IF NOT EXISTS remote_computer_transport_audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    previous_route TEXT,
    selected_route TEXT NOT NULL CHECK (selected_route IN ('direct', 'relay')),
    relay_region TEXT,
    created_at INTEGER NOT NULL
);

CREATE TRIGGER IF NOT EXISTS remote_computer_transport_no_relay_to_direct
BEFORE UPDATE OF selected_route ON remote_computer_sessions
FOR EACH ROW
WHEN OLD.selected_route = 'relay' AND NEW.selected_route = 'direct'
BEGIN
    SELECT RAISE(ABORT, 'relay transport cannot be upgraded back to direct');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_transport_audit_update
AFTER UPDATE OF selected_route ON remote_computer_sessions
FOR EACH ROW
WHEN NEW.selected_route IS NOT NULL
 AND (OLD.selected_route IS NOT NEW.selected_route OR OLD.relay_region IS NOT NEW.relay_region)
BEGIN
    INSERT INTO remote_computer_transport_audit
        (session_id, user_id, device_id, provider, previous_route, selected_route, relay_region, created_at)
    VALUES
        (NEW.session_id, NEW.user_id, NEW.device_id, NEW.provider, OLD.selected_route, NEW.selected_route, NEW.relay_region, NEW.transport_updated_at);
END;

CREATE INDEX IF NOT EXISTS remote_computer_transport_audit_session_idx
    ON remote_computer_transport_audit(user_id, device_id, session_id, created_at);
