-- Personal API tokens (WP-D). Only the SHA-256 digest and a short display prefix are stored.
CREATE TABLE IF NOT EXISTS api_tokens (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  scope TEXT NOT NULL CHECK (scope IN ('read','full')),
  hash TEXT NOT NULL UNIQUE,
  prefix TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_used TEXT,
  expires_at TEXT
);
CREATE INDEX IF NOT EXISTS api_tokens_user ON api_tokens(user_id);
