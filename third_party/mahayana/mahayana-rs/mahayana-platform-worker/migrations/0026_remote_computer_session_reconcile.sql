-- FDN-001: stable Remote Computer session request identity and reconcile tombstone.
-- Legacy sessions remain valid with NULL request_id. New clients may opt into stable
-- request identity without changing the authorization authority or paired-client secret.
ALTER TABLE remote_computer_sessions
    ADD COLUMN request_id TEXT;

-- A stable request identity belongs to exactly one session inside one authenticated
-- Fabushi account. Including device/client in the row lets the Worker reject payload
-- drift explicitly instead of silently creating a second session.
CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_sessions_request_id_idx
    ON remote_computer_sessions(user_id, request_id)
    WHERE request_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS remote_computer_sessions_request_lookup_idx
    ON remote_computer_sessions(user_id, request_id, device_id, client_id, state, expires_at);
