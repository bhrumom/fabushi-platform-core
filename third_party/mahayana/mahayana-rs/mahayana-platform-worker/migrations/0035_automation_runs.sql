-- fabushi.automation.v1: ACK retains terminal idempotency tombstones.
CREATE TABLE IF NOT EXISTS automation_runs (
  user_id TEXT NOT NULL,
  run_uuid TEXT NOT NULL,
  event_json TEXT NOT NULL,
  event_digest TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  completion_json TEXT,
  completion_digest TEXT,
  completed_at INTEGER,
  acknowledged_at INTEGER,
  PRIMARY KEY (user_id, run_uuid),
  CHECK (json_valid(event_json)),
  CHECK ((completion_json IS NULL AND completion_digest IS NULL AND completed_at IS NULL)
      OR (completion_json IS NOT NULL AND json_valid(completion_json) AND completion_digest IS NOT NULL AND completed_at IS NOT NULL)),
  CHECK (acknowledged_at IS NULL OR completed_at IS NOT NULL)
);
CREATE INDEX IF NOT EXISTS automation_runs_delivery ON automation_runs(user_id, acknowledged_at, created_at, run_uuid);
