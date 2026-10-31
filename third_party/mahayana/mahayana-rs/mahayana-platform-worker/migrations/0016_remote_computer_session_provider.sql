-- RDF-002: bind every control session to the provider declared by the registered device.
-- This remains additive: existing fabushi-webrtc sessions keep working, while a future
-- rustdesk-sidecar transport can share the same Fabushi identity/authorization/session rows.
ALTER TABLE remote_computer_sessions
    ADD COLUMN provider TEXT
    CHECK (provider IN ('fabushi-webrtc', 'rustdesk-sidecar'));

-- Backfill from the authoritative account-scoped device inventory.
UPDATE remote_computer_sessions
SET provider = COALESCE(
    (SELECT computer.provider
     FROM remote_computers AS computer
     WHERE computer.device_id = remote_computer_sessions.device_id
       AND computer.user_id = remote_computer_sessions.user_id),
    'fabushi-webrtc'
)
WHERE provider IS NULL;

-- Existing Worker code does not yet pass a provider in INSERT. New rows begin with NULL
-- and are bound during the same INSERT statement's trigger execution from the registered
-- computer, so the controlling client never chooses the transport provider.
CREATE TRIGGER IF NOT EXISTS remote_computer_session_provider_bind_after_insert
AFTER INSERT ON remote_computer_sessions
FOR EACH ROW
WHEN NEW.provider IS NULL
BEGIN
    UPDATE remote_computer_sessions
    SET provider = COALESCE(
        (SELECT computer.provider
         FROM remote_computers AS computer
         WHERE computer.device_id = NEW.device_id
           AND computer.user_id = NEW.user_id),
        'fabushi-webrtc'
    )
    WHERE session_id = NEW.session_id;
END;

-- The one NULL -> bound-provider transition above is initialization. Any later provider
-- mutation is rejected, preserving the provider chosen from the device registration for
-- the lifetime of the session.
CREATE TRIGGER IF NOT EXISTS remote_computer_session_provider_immutable
BEFORE UPDATE OF provider ON remote_computer_sessions
FOR EACH ROW
WHEN OLD.provider IS NOT NULL AND NEW.provider IS NOT OLD.provider
BEGIN
    SELECT RAISE(ABORT, 'remote session provider is immutable');
END;

CREATE INDEX IF NOT EXISTS remote_computer_sessions_provider_idx
    ON remote_computer_sessions(user_id, device_id, provider, state, expires_at);
