-- RCS-01..RCS-07: durable mobile session-create identity and lost-response reconciliation.
--
-- Legacy sessions remain valid through their existing lifetime but have NULL request identity
-- and therefore cannot be recovered through the mobile reconcile endpoint.
ALTER TABLE remote_computer_sessions ADD COLUMN mobile_request_id TEXT;
ALTER TABLE remote_computer_sessions ADD COLUMN mobile_request_hash TEXT;
ALTER TABLE remote_computer_sessions ADD COLUMN client_token_hash_at_create TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_sessions_mobile_request_identity_idx
    ON remote_computer_sessions(user_id, device_id, client_id, mobile_request_id)
    WHERE mobile_request_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS remote_computer_sessions_mobile_reconcile_idx
    ON remote_computer_sessions(user_id, device_id, client_id, mobile_request_id, state, expires_at);
