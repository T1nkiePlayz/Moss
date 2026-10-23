# 0002. SQLite with one WAL connection for state

Status: Accepted

## Context

Moss stores servers, users, grants, operations, backups, content inventory, schedules, notifications and the audit log. It is single-node. High-rate data (console output, metrics samples) is the only thing that could stress a database, and it does not need to be in one.

## Decision

Use SQLite through `rusqlite`. `Db` (`src/db/mod.rs`) wraps one writer connection in a mutex and runs closures on `spawn_blocking` (`Db::call`). The connection is opened with WAL, `synchronous=NORMAL`, foreign keys on and a 5 s busy timeout. A second, `query_only` connection (`Db::read`) serves hot lookups, and a third, in a background thread, runs WAL checkpoints. Schema changes are numbered SQL files in `src/db/migrations/`, applied at startup. Console output lives in per-server log files; metrics are rolled up hourly (`src/history.rs`). History tables are pruned (`prune_history`).

## Consequences

- No external service to run, back up or secure; the database is one file plus WAL.
- Writers are serialised by the mutex. This is fine at the intended scale and makes transactions such as the owner claim (`src/auth/setup.rs`) simple to reason about.
- A slow query holds the writer and delays every other write. Handlers must keep queries short and indexed.
- Reader, writer and checkpointer (2026-10-23). With one connection, SQLite's automatic checkpoint ran inside whichever commit crossed 1000 WAL pages, under the mutex, and fsynced the WAL and the database file. On a busy rotational btrfs disk that fsync took over 10 s, and every database call waited, so 16 concurrent console senders timed out (each command wrote an audit row). Now the writer never checkpoints (`wal_autocheckpoint=0`); a background connection runs a passive checkpoint every second. btrfs also blocks WAL appends for as long as a WAL fsync runs (it holds the inode lock), so a write can still wait on a slow disk. Hot read paths (the server lookup on every request, settings, grants, token and user lookups) therefore use `Db::read`, which WAL lets run alongside the writer, and console-command audit entries are batched by a background task (`state::AuditQueue`) rather than written before the response. `Db::read` sees every committed write but not one in progress, so code that reads what it is about to write stays on `Db::call`. Queued console-command entries are written a moment after the response, so they can sort after a later synchronous entry, and the few still queued when the process exits are lost; a failed batch is logged.
- Multi-node operation is out of scope; moving to a client/server database would need a new `Db` implementation and a data migration.
- `synchronous=NORMAL` (WAL): the database cannot be corrupted and a crash loses nothing; a power cut can lose the last few commits. Changed from `FULL` (2026-10-21) to drop the fsync on every commit.
- The database holds sensitive values (API token digests, webhook URLs); its file permissions matter (see the threat model).
