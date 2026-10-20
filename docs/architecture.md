# Architecture

Moss is one Rust binary (`axum` + `tokio`) that embeds a Svelte single-page UI. State lives in SQLite (WAL); high-rate data (console output, metrics) never touches the database.

```text
Browser ── HTTPS ─▶ reverse proxy ─▶ moss serve ─┬─ Supabase JWT verification (JWKS, local) + RBAC
                                                 ├─ SQLite: servers, users, grants, operations, backups, content, schedules, audit
                                                 ├─ Operation engine: persisted state machine, locks, cancellation, SSE feed
                                                 ├─ Lifecycle: detached processes, monitor loop, restart policy, reconcile on boot
                                                 ├─ Console hub: one log tailer per server → N WebSocket viewers
                                                 ├─ Providers: Mojang · PaperMC Fill · Purpur · Fabric/Quilt Meta · Forge/NeoForge maven · Modrinth · CurseForge · Adoptium
                                                 ├─ Platform layer: Linux (/proc, cgroup v2) or macOS (libproc, sysctl)
                                                 ├─ Content: jar inspection, compatibility checks, change plans, modpacks, version upgrades
                                                 └─ Backups (tar.zst) · Undo snapshots · Scheduler · Notifications · Metrics history · Health findings
```

## Servers outlive the controller

A game server is **not** a child of Moss. It is started as:

```sh
setsid sh -c 'echo $$ > pid; java … <>console.fifo >>console.log 2>&1; echo $? > exit'
```

- The wrapper shell opens the FIFO read-write and gives it to the game as stdin, so the FIFO always has a writer and the game never sees EOF. Moss sends console input by opening the FIFO and writing a line.
- Output is appended to a log file that a tailer streams to browsers.
- Moss stores `pid` + the process **start time** (to defeat PID reuse). On startup it re-attaches to anything still running, resumes tailing, and handles processes that died while it was down (clean stop vs crash, with the restart policy).
- Under systemd the unit uses `KillMode=process` so stopping the service does not touch game servers.

Verified: `SIGKILL` the controller, keep playing, start the controller again, the console and commands work.

## Operations

Anything that can outlive an HTTP request (install, download, backup, restore, content change, Java install) is an **operation**: a persisted record (`queued → running → completed | failed | cancelled`, plus `interrupted` for work in flight during a restart), a bounded log, progress that is `null` when it cannot be measured, cooperative cancellation, and an event on `/api/v1/events`. A per-server lock prevents conflicting operations (for example a backup during a content install).

## Safety rules implemented

- All file access goes through `safe_join`: no `..`, no absolute paths, **no symlink components**; `.moss/` metadata is unreachable.
- Downloads: HTTPS only, host allow-list re-checked on every redirect hop, size cap, streamed SHA verification, temp file then atomic rename.
- Archives (backups, Java): entries must be plain relative regular files/dirs; symlinks, hard links, devices and traversal are rejected; restores verify the checksum first and extract to a staging directory.
- Console and file content are served as data and rendered as text by the UI (ANSI is parsed, never injected as HTML); uploads are never served inline.
- The browser gets a strict CSP; WebSocket/SSE use single-use 30-second tickets instead of tokens in URLs.
- Secrets: the only credential Moss holds is the optional legacy JWT secret in the environment. The Supabase anon key is public.

## Undo

Before Moss changes a file (editor save, `server.properties`, plugin install/replace/remove) it copies the previous version into `<server>/.moss/undo/<id>/` and records it. Reverting restores (or removes newly created) files and re-syncs the content inventory.

## Not yet built

Container isolation (Docker/Podman backend), Forge/NeoForge, CurseForge, event workflows, multi-node. See [PLAN.md](PLAN.md).

## Code layout

The backend is organised by responsibility, each module with a `//!` doc at the top:

| Module | Owns |
|---|---|
| `platform/` | Everything OS specific: process identity and usage, host stats, disk, Java discovery, cgroup limits (Linux). Nothing else reads `/proc`. |
| `lifecycle/` | Launch, stop/kill/restart, exit handling and restart policy, the supervision and sampling loops, boot reconciliation. |
| `api/` | The router (one list of routes) and thin handlers per area. |
| `auth/` | JWT verification, provisioning, RBAC, API tokens, setup token, MFA policy, throttling. |
| `content/`, `providers/`, `jars/` | Catalogue search, change plans, install/update/upgrade, modpacks; Modrinth and CurseForge clients; jar metadata and compatibility rules. |
| `backups/` | Archive format, create/prune, safety backups, preview, restore and swap. |
| `backups/remote/` | Backup destinations: folder and hand-written S3 (SigV4, streamed multipart) drivers, validators, the 0600 secrets file, upload/fetch/retention jobs, destination self-test. Credentials live only in `<data>/backup-secrets.json`; they are never in SQLite, the support bundle or API responses. Outbound S3 traffic uses the notify SSRF guard (resolve once, pin the address, no redirects). |
| `backups/staged/` | Promote-after-compare restore: extract to `servers/.staged-<id>`, pre-promotion checks, diff against the live server, then an explicit atomic swap with the old directory kept in `removed/`. |
| `notify/`, `history.rs` | Notifications and webhooks; hourly metric roll-ups. |
| `db/` | SQLite access and numbered migrations (`db/migrations/NNNN_name.sql`). |

## Platform differences

Linux reads `/proc` for process and host numbers and, when the service unit delegates a cgroup subtree (`Delegate=yes`), enforces per-server memory, CPU and process limits and detects OOM kills. macOS uses libproc and sysctl; limits are not enforceable there and are reported as such. Process identity is PID plus start time on both, which is what makes re-attaching after a controller restart safe.
