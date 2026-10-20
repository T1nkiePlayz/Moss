-- Outbound event webhooks. Signing secrets never live here: they are kept in a 0600 file in the data directory.
CREATE TABLE IF NOT EXISTS webhook_endpoints (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, url TEXT NOT NULL,
  events TEXT NOT NULL DEFAULT '[]',      -- JSON list of event types
  server_ids TEXT NOT NULL DEFAULT '[]',  -- JSON list; empty = every server
  enabled INTEGER NOT NULL DEFAULT 1, failures INTEGER NOT NULL DEFAULT 0, disabled_reason TEXT, created_at TEXT NOT NULL);
-- The delivery queue and the delivery log in one table: pending rows are the queue (survives restarts), the rest is history.
CREATE TABLE IF NOT EXISTS webhook_deliveries (
  id TEXT PRIMARY KEY, endpoint_id TEXT NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
  event_id TEXT NOT NULL, event_type TEXT NOT NULL, body TEXT NOT NULL, test INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL CHECK (state IN ('pending','delivered','dead')), attempts INTEGER NOT NULL DEFAULT 0, next_at INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, finished_at TEXT, status_code INTEGER, latency_ms INTEGER, response TEXT, error TEXT);
CREATE INDEX IF NOT EXISTS webhook_deliveries_due ON webhook_deliveries(state, next_at);
CREATE INDEX IF NOT EXISTS webhook_deliveries_endpoint ON webhook_deliveries(endpoint_id, created_at);
