-- FDN-001 target-authorized official MCP controller grant.
-- The stable request id enables recovery of a derived client credential without
-- persisting any client/device secret plaintext.
PRAGMA foreign_keys = ON;

ALTER TABLE remote_computer_clients ADD COLUMN target_grant_request_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_clients_target_grant_request_idx
    ON remote_computer_clients(user_id, device_id, service_connection_id, target_grant_request_id)
    WHERE service_connection_id IS NOT NULL
      AND target_grant_request_id IS NOT NULL
      AND revoked_at IS NULL;
