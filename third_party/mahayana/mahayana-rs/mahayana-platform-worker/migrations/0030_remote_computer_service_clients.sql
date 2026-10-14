-- FDN-001 official MCP controller binding on the existing Remote Computer client authority.
-- This migration stores only service identity metadata; client/mobile/device secrets remain hashed elsewhere.
PRAGMA foreign_keys = ON;

ALTER TABLE remote_computer_clients ADD COLUMN service_connection_id TEXT;
ALTER TABLE remote_computer_clients ADD COLUMN service_delegation_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_clients_service_connection_device_idx
    ON remote_computer_clients(user_id, device_id, service_connection_id)
    WHERE service_connection_id IS NOT NULL AND revoked_at IS NULL;

CREATE TRIGGER IF NOT EXISTS remote_computer_clients_service_binding_pair_insert
BEFORE INSERT ON remote_computer_clients
WHEN (NEW.service_connection_id IS NULL) <> (NEW.service_delegation_id IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'service connection and delegation binding must be stored together');
END;

CREATE TRIGGER IF NOT EXISTS remote_computer_clients_service_binding_immutable
BEFORE UPDATE OF service_connection_id, service_delegation_id ON remote_computer_clients
WHEN COALESCE(NEW.service_connection_id, '') <> COALESCE(OLD.service_connection_id, '')
  OR COALESCE(NEW.service_delegation_id, '') <> COALESCE(OLD.service_delegation_id, '')
BEGIN
    SELECT RAISE(ABORT, 'remote service client binding is immutable');
END;
