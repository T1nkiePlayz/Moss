# Architecture

Moss is one Rust binary (`axum` + `tokio`) that embeds a Svelte single-page UI. State lives in SQLite (WAL); high-rate data (console output, metrics) never touches the database.

```text
Browser ── HTTPS ─▶ reverse proxy ─▶ moss serve ─┬─ Supabase JWT verification (JWKS, local) + RBAC
                                                 ├─ SQLite: servers, users, grants, operations, backups, content, schedules, audit
                                                 ├─ Operation engine: persisted state machine, locks, cancellation, SSE feed
                                                 ├─ Lifecycle: detached processes, monitor loop, restart policy, reconcile on boot
                                                 ├─ Console hub: one log tailer per server → N WebSocket viewers
                                                 ├─ Providers: Mojang · PaperMC Fill · Purpur · Fabric Meta · Modrinth · Adoptium
                                                 └─ Backups (tar.zst) · Undo snapshots · Scheduler · Health findings
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
