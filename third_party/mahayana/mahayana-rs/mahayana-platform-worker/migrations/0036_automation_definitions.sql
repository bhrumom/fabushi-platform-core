-- Native definitions and clock cursor share the existing automation state owner.
CREATE TABLE IF NOT EXISTS automation_definitions (
  user_id TEXT NOT NULL,
  automation_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
  revision TEXT NOT NULL,
  mutation_id TEXT NOT NULL,
  next_runs_json TEXT NOT NULL CHECK(json_valid(next_runs_json)),
  updated_at INTEGER NOT NULL,
  clock_checked_at INTEGER NOT NULL DEFAULT 0,
  deleted_at INTEGER,
  PRIMARY KEY(user_id, automation_id)
);
CREATE INDEX IF NOT EXISTS automation_definitions_agent
  ON automation_definitions(user_id, agent_id, deleted_at, automation_id);
