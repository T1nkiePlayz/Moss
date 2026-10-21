# 0002. SQLite with one WAL connection for state

Status: Accepted

## Context

Moss stores servers, users, grants, operations, backups, content inventory, schedules, notifications and the audit log. It is single-node. High-rate data (console output, metrics samples) is the only thing that could stress a database, and it does not need to be in one.

## Decision

Use SQLite through `rusqlite`. `Db` (`src/db/mod.rs`) wraps one connection in a mutex and runs closures on `spawn_blocking`. The connection is opened with WAL, `synchronous=FULL`, foreign keys on and a 5 s busy timeout. Schema changes are numbered SQL files in `src/db/migrations/`, applied at startup. Console output lives in per-server log files; metrics are rolled up hourly (`src/history.rs`). History tables are pruned (`prune_history`).

## Consequences

- No external service to run, back up or secure; the database is one file plus WAL.
- Writers are serialised by the mutex. This is fine at the intended scale and makes transactions such as the owner claim (`src/auth/setup.rs`) simple to reason about.
- A slow query holds the single connection and delays every other database call. Handlers must keep queries short and indexed.
- Multi-node operation is out of scope; moving to a client/server database would need a new `Db` implementation and a data migration.
- `synchronous=NORMAL` (WAL): the database cannot be corrupted and a crash loses nothing; a power cut can lose the last few commits. Changed from `FULL` (2026-10-21) to drop the fsync on every commit.
- The database holds sensitive values (API token digests, webhook URLs); its file permissions matter (see the threat model).
