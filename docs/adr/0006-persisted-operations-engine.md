# 0006. A persisted operations engine

Status: Accepted

## Context

Installs, downloads, backups, restores, content changes and Java installs outlive an HTTP request, can fail halfway, and must not conflict (a backup during a content install). Users want progress, logs and cancel; operators want to know what was in flight when Moss restarted.

## Decision

Every long-running job is an `Operation` (`src/ops.rs`) with a stable id, `kind`, optional `server_id`, a state (`queued`, `running`, `completed`, `failed`, `cancelled`, plus `interrupted`), a stage, progress that is `null` when unmeasurable, and a bounded log. Transitions are persisted in the `operations` table; in-flight state lives in an in-memory registry with a `CancellationToken` per operation. Updates are emitted on the event bus (throttled to about one per 200 ms except forced stage changes). A per-server lock prevents conflicting operations. On startup, operations that were running are marked `interrupted`. History is pruned to the latest 1000 finished rows.

## Consequences

- One mechanism gives progress, logs, cancellation and an audit trail to everything slow.
- Cancellation is cooperative; code must check the token, and downloads and archive loops do.
- Operations are not resumed after a restart; they are reported as interrupted and the user decides. Atomic file placement and staging directories keep interrupted work from corrupting servers.
- Fine-grained progress is not persisted, only stage changes; after a restart only the last stage is known.
- The engine is in-process and single-node.
