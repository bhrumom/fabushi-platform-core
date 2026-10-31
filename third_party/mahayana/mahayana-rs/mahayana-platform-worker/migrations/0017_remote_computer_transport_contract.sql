-- RDF-002/RDF-003 boundary: durable provider-neutral transport negotiation metadata.
-- Fabushi remains the account/session authority. A provider implementation may later
-- realize these choices through WebRTC or an isolated RustDesk-compatible sidecar.
ALTER TABLE remote_computer_sessions
    ADD COLUMN route_policy TEXT NOT NULL DEFAULT 'direct-first'
    CHECK (route_policy IN ('direct-first', 'relay-only'));

ALTER TABLE remote_computer_sessions
    ADD COLUMN selected_route TEXT
    CHECK (selected_route IS NULL OR selected_route IN ('direct', 'relay'));

ALTER TABLE remote_computer_sessions
    ADD COLUMN relay_region TEXT;

ALTER TABLE remote_computer_sessions
    ADD COLUMN transport_updated_at INTEGER;

-- A controlling client must not be able to rewrite the provider selected from the
-- registered device. Route choice is intentionally separate from provider identity.
CREATE TRIGGER IF NOT EXISTS remote_computer_session_route_requires_live_session
BEFORE UPDATE OF selected_route ON remote_computer_sessions
FOR EACH ROW
WHEN NEW.selected_route IS NOT NULL
 AND (OLD.state = 'closed' OR OLD.expires_at <= CAST(strftime('%s','now') AS INTEGER))
BEGIN
    SELECT RAISE(ABORT, 'cannot select transport route for closed session');
END;

-- Direct-first sessions may settle on direct or relay. Relay-only sessions cannot be
-- rewritten to direct, which provides the persistence invariant needed by policy gates.
CREATE TRIGGER IF NOT EXISTS remote_computer_session_route_policy_guard
BEFORE UPDATE OF selected_route ON remote_computer_sessions
FOR EACH ROW
WHEN NEW.selected_route = 'direct' AND OLD.route_policy = 'relay-only'
BEGIN
    SELECT RAISE(ABORT, 'relay-only session cannot select direct route');
END;

CREATE INDEX IF NOT EXISTS remote_computer_sessions_transport_idx
    ON remote_computer_sessions(user_id, device_id, provider, route_policy, selected_route, state, expires_at);
