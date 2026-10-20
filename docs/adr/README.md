# Architecture decision records

Short records of decisions that shape Moss. Each has Status, Context, Decision and Consequences. A decision that changes gets a new record that supersedes the old one; old records are not rewritten.

| # | Title | Status |
|---|---|---|
| [0001](0001-single-binary-with-embedded-ui.md) | Single binary with an embedded UI | Accepted |
| [0002](0002-sqlite-single-wal-connection.md) | SQLite with one WAL connection for state | Accepted |
| [0003](0003-detached-game-servers-reattach-by-pid-and-start-time.md) | Game servers are detached and re-attached by PID and start time | Accepted |
| [0004](0004-supabase-for-authentication-only.md) | Supabase is used for authentication only | Accepted |
| [0005](0005-sse-events-and-websocket-console-with-tickets.md) | SSE for events, WebSocket for the console, single-use tickets | Accepted |
| [0006](0006-persisted-operations-engine.md) | A persisted operations engine | Accepted |
| [0007](0007-no-caching-of-curseforge-content.md) | No caching of CurseForge content, and the edge-function proxy | Accepted |
| [0008](0008-platform-layer-linux-proc-cgroup-macos-libproc.md) | A platform layer: Linux /proc and cgroup v2, macOS libproc | Accepted |

Related: [architecture overview](../architecture.md), [threat model](../threat-model.md).
