<p align="center">
  <img src="./docs/brand/moss-logo-256.png" alt="Moss logo" width="96">
</p>

<h1 align="center">Moss</h1>

<p align="center">
  <strong>Game servers that keep growing, quietly.</strong><br>
  A fast, local-first controller for Minecraft Java servers: one small binary, a polished web panel, and servers that survive everything except you pulling the plug.
</p>

<p align="center">
  <img src="https://github.com/T1nkiePlayz/Moss/actions/workflows/ci.yml/badge.svg" alt="Moss CI">
  <img src="https://img.shields.io/github/last-commit/T1nkiePlayz/Moss?style=flat-square" alt="Last commit">
  <img src="https://img.shields.io/badge/platform-Linux%20x86--64-informational?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/license-GPL--3.0-blue?style=flat-square" alt="License">
  <img src="https://img.shields.io/badge/status-early%20development-orange?style=flat-square" alt="Early development">
</p>

<p align="center">
  <img src="./docs/screenshots/overview.png" alt="The Moss overview with host gauges and server cards" width="100%">
</p>

> [!WARNING]
> **Early development.** Moss runs real servers today, but storage formats and APIs may change between versions, and there is **no container isolation yet** (servers run as the same OS user as Moss; see [Security model](#security-model)). Keep your own backups of anything you care about, and do not expose Moss to the internet without a TLS reverse proxy.

## Table of contents

- [Overview](#overview)
- [What makes Moss different](#what-makes-moss-different)
- [Features](#features)
- [Screenshots](#screenshots)
- [Quick start](#quick-start)
- [Sign-in with Supabase](#sign-in-with-supabase)
- [Configuration](#configuration)
- [Runtimes](#runtimes)
- [Security model](#security-model)
- [How it works](#how-it-works)
- [Current limitations](#current-limitations)
- [Roadmap](#roadmap)
- [Documentation index](#documentation-index)
- [Development](#development)
- [Releasing](#releasing)
- [License](#license)

## Overview

Moss installs, runs, watches, updates and backs up game servers on a machine you own. It is a single Rust binary that embeds its web UI, keeps state in SQLite, needs no Docker and no hosted backend, and uses Supabase **only** to sign people in.

- **Fast and small.** One static-ish binary, a Svelte UI with no UI framework, no polling storms: live state comes over Server-Sent Events and WebSockets. Idle cost is a few tens of MB.
- **Local first.** Server files, worlds, console logs and backups never leave your machine. The controller works offline except for downloads and the plugin catalogue.
- **Honest about compatibility.** Plugins are shown for Paper-family servers, mods for Fabric, proxy plugins for Velocity, datapacks for Vanilla. Client-only mods are filtered out of the catalogue before you can click them.
- **Recoverable.** Every change Moss makes to a file can be undone with one click, backups are checksummed and verified, and restores never delete what they replace.

## What makes Moss different

| | |
|---|---|
| **Servers outlive the controller** | Games are launched detached and re-attached by PID + start time. Kill or upgrade Moss mid-game and the server, its console and its players are untouched. (Tested by `SIGKILL`ing the controller.) |
| **Undo for everything it touches** | Editor saves, `server.properties`, plugin installs, replacements and removals snapshot the previous files first. The Undo history is per server. |
| **Change plans** | Installing a plugin shows exactly which files will be added or replaced (including required dependencies) before anything is downloaded. |
| **Pulse timeline** | Each server shows a 24-hour strip of starts, ready marks, crashes, restarts, backups and updates, so "what happened last night?" is one glance. |
| **Evidence-backed health** | No opaque score: findings like *Java missing*, *crash loop*, *no backup yet*, *disk almost full* each carry their evidence and a one-click fix (install Java, back up now, accept EULA). |
| **Zero-setup Java** | Moss can download a checksum-verified Temurin runtime for the Java major each Minecraft version needs, read from Mojang's own metadata. |
| **Live, consistent backups** | `save-off` → `save-all flush` → archive → `save-on`, confirmed from the console, as a checksummed `tar.zst` that you can verify and restore as a new server. |
| **Command palette** | <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd> reaches every page, server and action. |

## Features

**Servers** · Create Vanilla, Paper, Purpur, Folia, Fabric and Velocity servers from official metadata, with checksum verification where upstream publishes one · start / graceful stop / restart / force stop with escalation (`stop` → SIGTERM → SIGKILL) · crash detection with exponential backoff and a crash-loop breaker · autostart · per-server memory, flags profile and port · EULA acceptance is an explicit, audited step.

**Console** · live WebSocket console shared by any number of viewers (one log tailer per server), ANSI colours rendered as text never HTML, command history, bounded buffers, survives controller restarts.

**Files & config** · browse, upload, download, edit, rename, delete, all confined to the server directory (no `..`, no symlink escapes) · form-based `server.properties` editor with a raw mode · atomic writes · undo.

**Content** · Modrinth search filtered by runtime, loader, game version and server side · detail pages with compatible versions · dependency resolution · installed inventory with provenance (`Modrinth` vs manually added files) · update checks and pinning · removal keeps plugin data.

**Backups & schedules** · manual and scheduled backups with retention that never removes your last good one · verify any backup on demand · restore as a new server or in place (a safety backup is taken and the old directory is kept) · interval or daily-at schedules with an explicit UTC offset for backup, start, stop, restart and console commands · missed runs execute once, never as a backlog.

**Operations** · every long job (install, download, backup, restore, Java) is a persisted operation with stage, progress, a log tail, cancellation and a live feed; work in flight during a restart is marked *interrupted*, never silently lost.

**Access** · Supabase sign-in, roles (`owner`, `admin`, `user`) and per-server grants (`manager`, `operator`, `viewer`) enforced by the backend on every request, WebSocket and event stream · invite-only by default · full activity log.

**Interface** · dark, light, system and high-contrast themes, accent and density options, reduced-motion support, keyboard-first, responsive down to a phone.

## Screenshots

<p align="center">
  <img src="./docs/screenshots/server.png" alt="A server's overview with live CPU and memory, players and the pulse timeline" width="49%">
  <img src="./docs/screenshots/console.png" alt="The live console" width="49%">
</p>
<p align="center">
  <img src="./docs/screenshots/plugins.png" alt="Plugin catalogue with compatibility filtering" width="49%">
  <img src="./docs/screenshots/backups.png" alt="Backups and schedules" width="49%">
</p>

## Quick start

Moss targets **Linux x86-64**. Windows and macOS hosts are not supported.

### From a release

Download the tarball from [Releases](https://github.com/T1nkiePlayz/Moss/releases) and verify it:

```bash
sha256sum -c SHA256SUMS.txt
gh attestation verify moss-*-linux-x86_64.tar.gz --repo T1nkiePlayz/Moss
tar xzf moss-*-linux-x86_64.tar.gz && cd moss-*-linux-x86_64
```

### Try it without any account (development mode)

```bash
MOSS_AUTH=dev ./moss serve        # http://127.0.0.1:8420 , no login, refuses non-loopback binds
```

Open the panel, create a server, accept the EULA, and (if you have no Java installed) use **Settings → Java → Install** to let Moss fetch a Temurin runtime.

### Production-style install

```bash
sudo useradd --system --home /var/lib/moss --create-home --shell /usr/sbin/nologin moss
sudo install -m 0755 moss /usr/local/bin/moss
sudo install -d -m 0750 -o root -g moss /etc/moss
sudo install -m 0640 -o root -g moss moss.env.example /etc/moss/moss.env   # edit: Supabase + owner email
sudo install -m 0644 moss.service /etc/systemd/system/moss.service
sudo systemctl enable --now moss
```

Then put a TLS reverse proxy in front (Caddy example):

```caddyfile
moss.example.com {
    reverse_proxy 127.0.0.1:8420
}
```

WebSockets and Server-Sent Events work through Caddy as is. For nginx, disable response buffering on `/api/v1/events` (`proxy_buffering off;`) and forward `Upgrade`/`Connection` headers. Game ports are separate and are never proxied.

`moss doctor` checks the data directory, disk, ports, Java and auth configuration.

### From source

Requires Rust (stable) and Node.js (LTS).

```bash
git clone https://github.com/T1nkiePlayz/Moss && cd Moss
(cd web && npm ci && npm run build)       # the binary embeds web/dist
cargo build --release                      # → target/release/moss
```

## Sign-in with Supabase

Moss uses [Supabase Auth](https://supabase.com/docs/guides/auth) to prove who someone is; Moss itself decides what they may do. The browser only ever receives the public project URL and publishable key; tokens are verified locally against your project's signing keys (JWKS). Step-by-step setup (providers, redirect URLs, owner pinning, invites) is in **[docs/supabase.md](docs/supabase.md)**.

```bash
export SUPABASE_URL=https://YOUR-PROJECT.supabase.co
export SUPABASE_ANON_KEY=YOUR-PUBLISHABLE-KEY
export MOSS_OWNER_EMAILS=you@example.com
./moss serve
```

Nobody gets in without being the owner or being invited, even if sign-ups are open on your Supabase project.

## Configuration

Flags and environment variables (flags win):

| Variable | Default | Meaning |
|---|---|---|
| `MOSS_BIND` / `MOSS_PORT` | `127.0.0.1` / `8420` | Listener. Keep loopback unless a TLS proxy fronts it. |
| `MOSS_DATA_DIR` | `./moss-data` | Database, servers, backups, managed Java, cache. |
| `MOSS_AUTH` | `supabase` | `supabase` or `dev` (no login, loopback only). |
| `SUPABASE_URL` / `SUPABASE_ANON_KEY` | – | Your project URL and publishable key. |
| `SUPABASE_JWT_SECRET` | – | Only for legacy HS256-signed projects. |
| `MOSS_OWNER_EMAILS` | – | Comma-separated emails that become owner on first verified sign-in. |
| `MOSS_REQUIRE_VERIFIED_EMAIL` | `true` | Require a verified email before matching owner/invite. |
| `MOSS_DEV_RUNTIMES` | `false` | Expose the simulated server runtime (demos, tests). |
| `RUST_LOG` | `moss=info` | Log filter. |

Data layout: `moss.sqlite`, `servers/<id>/` (game files; Moss metadata in `.moss/`), `backups/<server-id>/<backup-id>.tar.zst`, `java/<major>/`, `removed/` (directories kept on delete/restore), `tmp/` (cleared on start).

## Runtimes

| Runtime | Tier | Content tab | Notes |
|---|---|---|---|
| Vanilla | Stable | Datapacks | Mojang manifest, SHA-1 verified |
| Paper | Stable | Plugins | PaperMC Fill API, SHA-256 verified |
| Purpur | Stable | Plugins | Upstream publishes no checksum; TLS only (logged) |
| Folia | Experimental | Plugins | Only plugins tagged for Folia are offered |
| Fabric | Stable | Mods | Server-side and both-side mods only |
| Velocity | Stable | Proxy plugins | |
| Custom command | Experimental | – | Admin-only; explicit argv, reduced management |
| Forge, NeoForge | Planned | – | Waiting for sandboxed installers |
| Simulated | Dev only | – | Fake server for demos and tests |

A runtime is only offered as installable when it works end to end. Spigot (BuildTools) is intentionally not offered: it cannot be redistributed, use Paper.

## Security model

Moss is designed so that signing in is not the only line of defence. Read this section before exposing it.

- **Loopback by default.** Dev auth mode refuses any other bind. A TLS reverse proxy is required for remote access.
- **Authorisation on every call.** REST, WebSocket console, SSE stream, downloads and undo/restore go through the same per-server permission check. Servers you cannot see return *not found*.
- **Invite-only.** A valid Supabase token is not enough; the account must be the owner or invited, and its email verified.
- **Files confined to the server.** Path normalisation plus a symlink check on every component; `.moss/` is unreachable; uploads are never served inline.
- **Hardened downloads and archives.** HTTPS-only host allow-list re-checked on redirects, size caps, checksum verification, atomic placement; archives reject traversal, links and devices.
- **Single-use stream tickets** instead of tokens in URLs; strict CSP; `nosniff`, no-referrer, no framing.
- **No shell exposure.** The UI cannot run commands on the host. Game processes are launched from an argument array. The one exception is the **admin-only Custom runtime**, which by definition runs an administrator-supplied command as the Moss user; grant admin accordingly.
- **No isolation yet (important).** Game servers, and any plugin or mod they load, run as the same OS user as Moss with that user's file access. Run Moss as a dedicated unprivileged user (the shipped systemd unit does, with sandboxing), and treat plugins as code you are trusting. Container isolation is on the roadmap.

Report security issues privately through GitHub's *Report a vulnerability* on this repository.

## How it works

One axum process: SQLite for state, a persisted operation engine with per-server locks, a lifecycle supervisor that launches games detached and re-attaches on boot, one log tailer per server fanning out to WebSocket viewers, and thin adapters for Mojang, PaperMC, Purpur, Fabric, Modrinth and Adoptium. The full picture, including how servers survive controller restarts, is in **[docs/architecture.md](docs/architecture.md)**; the original product plan is in [docs/PLAN.md](docs/PLAN.md) and the HTTP API in [docs/api.md](docs/api.md).

## Current limitations

- Linux x86-64 only; no container backend, so no per-server resource caps beyond the JVM heap.
- Metrics are in memory (about one hour); there is no long-term history yet.
- Modrinth only; CurseForge is planned behind the same provider interface.
- Forge/NeoForge and non-Minecraft games are not supported.
- One node. Backups are local; no remote storage.
- If the host reboots, running servers stop (as always); Moss restarts the ones that were running when it comes back, or that have *autostart* set.
- Content updates are applied through a reviewed plan, but there is no staging-server health gate yet.

## Roadmap

Container runtime backend (Docker/Podman) with resource limits · CurseForge provider · Forge/NeoForge · event-driven workflows · remote backup destinations · staged updates with a readiness gate · multi-node · public status pages · long-term metrics. The detailed plan lives in [docs/PLAN.md](docs/PLAN.md).

## Documentation index

| Doc | What it covers |
|---|---|
| [docs/supabase.md](docs/supabase.md) | Auth setup, owners, invites, roles |
| [docs/architecture.md](docs/architecture.md) | Process model, operations, safety rules |
| [docs/api.md](docs/api.md) | HTTP API reference |
| [docs/PLAN.md](docs/PLAN.md) | Product and engineering plan |
| [web/README.md](web/README.md) | Frontend structure and mock mode |

## Development

```bash
# backend (needs web/dist to exist; an empty directory is enough for tests)
mkdir -p web/dist
cargo clippy --all-targets -- -D warnings && cargo test

# run against a simulated server, no login, no Java needed
MOSS_AUTH=dev MOSS_DEV_RUNTIMES=true MOSS_DATA_DIR=/tmp/moss-dev cargo run -- serve

# frontend with hot reload (proxies /api to :8420), or fully mocked
cd web && npm ci
npm run dev
npm run dev:mock
npm run check && npm test && npm run build
```

Layout: `src/` (Rust: `api`, `auth`, `lifecycle`, `console`, `ops`, `content`, `backups`, `files`, `undo`, `sched`, `health`, `runtimes`, `java`, `download`, `metrics`, `mcping`), `web/` (Svelte UI), `docs/`, `packaging/` (systemd unit, env example), `scripts/`.

CI (`.github/workflows/ci.yml`) runs UI type-check/tests/build and backend clippy (`-D warnings`) and tests on every push and pull request.

## Releasing

Push a tag such as `v0.1.0`. `.github/workflows/release.yml` syncs the version into the manifests, builds the UI, tests, builds the release binary, packages `moss-<tag>-linux-x86_64.tar.gz` with the systemd unit and env example, writes `SHA256SUMS.txt`, creates a GitHub **build attestation**, and opens a **draft** release whose notes are the tagged commit's message (write it as the announcement). Tags with a `-` (for example `v0.2.0-rc1`) become pre-releases. Re-run the workflow by hand to rebuild an existing tag.

## License

GPL-3.0-only. See [LICENSE](LICENSE). Moss is not affiliated with Mojang or Microsoft; "Minecraft" is a trademark of Mojang Synergies AB.
