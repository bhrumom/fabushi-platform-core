-- Execution arbitration extends the existing run journal; running never expires.
ALTER TABLE automation_runs ADD COLUMN execution_state TEXT
 CHECK(execution_state IS NULL OR execution_state IN ('reserved','running','outcome_unknown'));
ALTER TABLE automation_runs ADD COLUMN claim_id TEXT;
ALTER TABLE automation_runs ADD COLUMN claim_token_digest TEXT;
ALTER TABLE automation_runs ADD COLUMN claim_device_id TEXT;
ALTER TABLE automation_runs ADD COLUMN claim_session_id TEXT;
ALTER TABLE automation_runs ADD COLUMN claim_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE automation_runs ADD COLUMN claim_expires_at INTEGER;
ALTER TABLE automation_runs ADD COLUMN execution_started_at INTEGER;
