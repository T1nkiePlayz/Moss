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

## Container isolation (optional)

A server whose `isolation` is `container` runs in a Docker or Podman container instead of as a bare process; `process` stays the default and is unchanged. Engine choice: `MOSS_CONTAINER_ENGINE` (name or path) if set, otherwise rootless Podman, then Docker, then rootful Podman. Everything goes through the CLI with argument arrays; `container::Engine` is a trait so tests use a fake engine, and `tests/stubs/docker` is a shell test double that records the argv.

What the container is: `run --detach --name moss-<serverid>`, one bind mount (the server directory at `/data`, also the working directory), `--cap-drop=ALL`, `--security-opt=no-new-privileges`, read-only root filesystem with a `tmpfs` `/tmp`, `--pids-limit`, `--memory` (= `--memory-swap`) and `--cpus` from the per-server limits (same defaults as process mode), `--restart=no` (Moss supervises), user mapping that keeps file ownership (`--user uid:gid`; `--userns=keep-id` on rootless Podman; `0:0` on rootless Docker), `:Z` on the mount when SELinux is enforcing. The image is `eclipse-temurin:<java major>-jre` unless overridden from the allow-list; Java comes from the image, not the host. There is no code path that adds flags, mounts or privileges from API input.

- **Console input is the same FIFO as process mode**, bind-mounted with the server directory (`/data/.moss/console.fifo`). The entrypoint is `sh -c 'exec "$@" <>console.fifo >>console.log 2>&1'`, so the game is PID 1, its stdin is a FIFO it holds open itself and its output lands in the log file the existing tailer streams. `docker attach` was rejected because its stdin dies with the attaching client, so it cannot survive a controller restart; `docker logs -f` was rejected because the file tailer already gives backlog and rotation.
- **Re-attach is by container name + inspect.** The database records that the current run is in a container (`in_container`), so a mode change only applies on the next start. On boot `reconcile` inspects `moss-<id>`: running means re-attach and resume tailing, exited or missing means the normal exit path (exit code and `OOMKilled` from inspect, then `rm`). If the engine cannot answer, the server is kept as running with an explanatory detail: an unreachable daemon is never read as a crash.
- **Liveness and stats are batched.** The supervision loop makes one `ps --filter label=moss.managed=true` call every two seconds for all container servers, and the sampler one `stats --no-stream` for all of them, feeding the usual metrics pipeline (CPU percent of one core, memory without page cache).
- **Stop** sends `stop`/`end` to the console, waits, then `docker stop --time 15` and finally `kill`; **kill** is `docker kill`. Crash detection, backoff, the crash-loop breaker and notifications are shared with process mode.
- **Network:** the game port is published as `<port>:<port>/tcp` so the port in the UI is the real in-game port (server.properties and velocity.toml need no change). A per-server list of extra ports (TCP or UDP, 1024-65535, at most 8, never another Moss server's port) is published the same way for plugins such as Simple Voice Chat or Geyser (UDP 19132). Two network modes: `open` (default) uses Docker's default bridge, so the server can reach the internet and anything reachable from the bridge; `game-only` runs on `moss-gameonly`, a user-defined bridge Moss creates idempotently with `com.docker.network.bridge.enable_ip_masquerade=false` (an existing network of that name without the option is refused, never reused) and `--dns=192.0.2.1` (unroutable). Published ports still work through DNAT and conntrack, but a new outbound connection leaves with the container's private address, which nothing routes back, so it cannot reach the internet. Limits: containers on that bridge and the host itself (the bridge gateway) remain reachable, so host services bound to all interfaces are still exposed to the server; online-mode servers cannot reach Mojang's session servers (use it behind a proxy that authenticates, or in offline mode); Podman is refused with a clear message because Moss does not know a verified equivalent (an `--internal` network cannot publish ports); if the network cannot be created the server does not start (never a fallback to `open`) and a critical health finding is raised. An unknown stored mode is treated as `game-only`.
- **What this is not:** the server directory (including `.moss/`) is writable by the game, exactly as in process mode; the container shares the host kernel; `DOCKER_HOST` must point at a local engine (container PIDs and bind mounts are meaningless otherwise). Rootful Docker means anyone who can reach the Docker socket is root: prefer rootless Podman.
- **Installers** (Forge, NeoForge, Quilt) for a container-mode server run in a short-lived `--rm` container with the same single mount and hardening (default network, since installers download libraries, regardless of the server's network mode); the installer jar is placed in `.moss/` for the run.

Verified only against the stub engine and a fake: no Docker or Podman was available when this was built.

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

Event workflows, multi-node, extensions. See the roadmap in the README.

## Code layout

The backend is organised by responsibility, each module with a `//!` doc at the top:

| Module | Owns |
|---|---|
| `platform/` | Everything OS specific: process identity and usage, host stats, disk, Java discovery, cgroup limits (Linux). Nothing else reads `/proc`. |
| `container/` | The optional Docker/Podman backend: engine detection, the `Engine` trait and its CLI implementation, the one fixed `run` argv, image allow-list and pulls. |
| `lifecycle/` | Launch, stop/kill/restart, exit handling and restart policy, the supervision and sampling loops, boot reconciliation. |
| `api/` | The router (one list of routes) and thin handlers per area. |
| `auth/` | JWT verification, provisioning, RBAC, API tokens, setup token, MFA policy, throttling. |
| `content/`, `providers/`, `jars/` | Catalogue search, change plans, install/update/upgrade, modpacks; Modrinth and CurseForge clients; jar metadata and compatibility rules. |
| `backups/` | Archive format, create/prune, safety backups, preview, restore and swap. |
| `backups/remote/` | Backup destinations: folder and hand-written S3 (SigV4, streamed multipart) drivers, validators, the 0600 secrets file, upload/fetch/retention jobs, destination self-test. Credentials live only in `<data>/backup-secrets.json`; they are never in SQLite, the support bundle or API responses. Outbound S3 traffic uses the notify SSRF guard (resolve once, pin the address, no redirects). |
| `backups/staged/` | Promote-after-compare restore: extract to `servers/.staged-<id>`, pre-promotion checks, diff against the live server, then an explicit atomic swap with the old directory kept in `removed/`. |
| `notify/`, `history.rs` | Notifications and webhooks; hourly metric roll-ups. |
| `db/` | SQLite access and numbered migrations (`db/migrations/NNNN_name.sql`). |

## Desktop app

`desktop/` is a separate Cargo workspace (not part of the root build) holding a Tauri v2 shell around the unchanged controller.
`desktop/core` is plain Rust with unit tests: it picks a free loopback port, starts the bundled `moss serve --auth supabase`
as a detached process in its own process group (so closing the app never signals it), records `{pid, port}` in `controller.json`,
reuses a healthy controller on the next launch, and stops it with SIGTERM on request. `desktop/src-tauri` adds the window (its
navigation is limited to the controller's own `http://127.0.0.1:<port>` origin; other links open in the system browser), the
tray icon, single-instance handling and the quit prompt. The window gets no Tauri commands (empty capability set). See
[desktop.md](desktop.md).

## Platform differences

Linux reads `/proc` for process and host numbers and, when the service unit delegates a cgroup subtree (`Delegate=yes`), enforces per-server memory, CPU and process limits and detects OOM kills. macOS uses libproc and sysctl; limits are not enforceable there and are reported as such. Process identity is PID plus start time on both, which is what makes re-attaching after a controller restart safe.
