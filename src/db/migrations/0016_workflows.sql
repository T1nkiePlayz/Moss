-- Event workflows: "when <event> happens (on these servers, if <condition>), do <actions>". Actions and the condition are
-- JSON; a workflow runs with the permissions of `created_by`. workflow_runs keeps the last 50 runs per workflow.
CREATE TABLE IF NOT EXISTS workflows (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
  "trigger" TEXT NOT NULL,                -- an event type from the webhooks event list
  server_ids TEXT NOT NULL DEFAULT '[]',  -- JSON list; empty = every server the creator can manage
  condition TEXT,                         -- JSON {field, op, value} or NULL
  actions TEXT NOT NULL DEFAULT '[]',     -- JSON ordered list
  created_by TEXT NOT NULL, created_at TEXT NOT NULL, last_run_at TEXT, last_result TEXT, disabled_reason TEXT);
CREATE TABLE IF NOT EXISTS workflow_runs (
  id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
  at TEXT NOT NULL, event TEXT NOT NULL, ok INTEGER NOT NULL, log TEXT NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS workflow_runs_by_workflow ON workflow_runs(workflow_id, at);
