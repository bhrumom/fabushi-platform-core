-- Require a possession-bound credential for every paired remote-control client.
-- Existing pairings predate this credential and are revoked deliberately; users
-- must pair once more so account login alone can never impersonate a phone.
ALTER TABLE remote_computer_clients ADD COLUMN client_token_hash TEXT;

UPDATE remote_computer_sessions
SET state = 'closed',
    closed_at = COALESCE(closed_at, CAST(strftime('%s', 'now') AS INTEGER))
WHERE client_id IN (
    SELECT client_id FROM remote_computer_clients WHERE client_token_hash IS NULL
)
  AND state <> 'closed';

UPDATE remote_computer_clients
SET revoked_at = COALESCE(revoked_at, CAST(strftime('%s', 'now') AS INTEGER))
WHERE client_token_hash IS NULL;

CREATE INDEX IF NOT EXISTS remote_computer_clients_token_idx
    ON remote_computer_clients(user_id, device_id, client_id, revoked_at, client_token_hash);
