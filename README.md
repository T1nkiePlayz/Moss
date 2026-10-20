<p align="center">
  <img src="./docs/brand/moss-logo-256.png" alt="Moss logo" width="96">
</p>

<h1 align="center">Moss</h1>

<p align="center">
  <strong>Game servers that keep growing, quietly.</strong><br>
  A fast, local-first controller for Minecraft Java servers on Linux and macOS: one small binary, a polished web panel, full mod support with compatibility checks, and servers that survive everything except you pulling the plug.
</p>

<p align="center">
  <img src="https://github.com/T1nkiePlayz/Moss/actions/workflows/ci.yml/badge.svg" alt="Moss CI">
  <img src="https://img.shields.io/github/last-commit/T1nkiePlayz/Moss?style=flat-square" alt="Last commit">
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-informational?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/license-GPL--3.0-blue?style=flat-square" alt="License">
  <img src="https://img.shields.io/badge/status-early%20development-orange?style=flat-square" alt="Early development">
</p>

<p align="center">
  <img src="./docs/screenshots/overview.png" alt="The Moss overview with host gauges and server cards" width="100%">
</p>

> [!WARNING]
> **Early development.** Moss runs real servers today, but storage formats and APIs may change between versions, and there is **no container isolation yet**: servers run as the same OS user as Moss. On Linux Moss can cap memory, CPU and processes per server with cgroups, which is not a security boundary (see [Security model](#security-model)). Keep your own backups, and do not expose Moss to the internet without a TLS reverse proxy.

## Table of contents

- [Overview](#overview)
- [What makes Moss different](#what-makes-moss-different)
- [Features](#features)
- [Screenshots](#screenshots)
- [Quick start](#quick-start)
- [Sign-in with Supabase](#sign-in-with-supabase)
- [Configuration](#configuration)
- [Runtimes](#runtimes)
- [Mods and compatibility](#mods-and-compatibility)
- [Security model](#security-model)
- [How it works](#how-it-works)
- [Platform support](#platform-support)
- [Current limitations](#current-limitations)
- [Roadmap](#roadmap)
- [Documentation index](#documentation-index)
- [Development](#development)
- [Releasing](#releasing)
- [License](#license)

## Overview

Moss installs, runs, watches, updates and backs up game servers on a machine you own. It is a single Rust binary that embeds its web UI, keeps state in SQLite, needs no Docker and no hosted backend, and uses Supabase **only** to sign people in.

- **Fast and small.** One binary, a Svelte UI with no UI framework (about 12 KB of JavaScript on first load, gzipped), live state over Server-Sent Events and WebSockets, no polling storms. Idle cost is a few tens of MB.
- **Local first.** Server files, worlds, console logs and backups never leave your machine. The controller works offline except for downloads and the content catalogues.
- **Honest about compatibility.** Plugins for Paper-family servers, mods for Fabric, Quilt, Forge and NeoForge, proxy plugins for Velocity, datapacks for Vanilla. Moss reads every jar, checks dependencies, versions and loaders, and tells you what is wrong before the server does.
- **Recoverable.** Every change Moss makes can be undone, risky changes take a safety backup first, backups are checksummed and verifiable, and restores never delete what they replace.

## What makes Moss different

| | |
|---|---|
| **Servers outlive the controller** | Games are launched detached and re-attached by PID + start time. Kill or upgrade Moss mid-game and the server, its console and its players are untouched. An end-to-end test `SIGKILL`s the controller on every CI run. |
| **Compatibility checks on real jars** | Moss reads `fabric.mod.json`, `mods.toml`, `plugin.yml` and friends: missing dependencies, version ranges, duplicates, declared incompatibilities, wrong loader, wrong game version, client-only mods, corrupt files. |
| **Version upgrade assistant** | Pick a target Minecraft version and see, per mod, whether a compatible build exists, before anything changes. Then upgrade with a backup, rollback on failure and one operation log. |
| **Undo and safety backups** | Editor saves, `server.properties` and content changes snapshot the previous files; installs, updates, upgrades, modpack imports and restores take a pre-change backup first (the *Back up before risky changes* setting). |
| **Change plans** | Installing or updating shows exactly which files will be added, replaced or broken, dependencies included, before anything is downloaded. |
| **Backups you can trust** | Flushed (`save-off`, `save-all flush`), checksummed `tar.zst`, verified on demand, locked against pruning, previewed before restore (added, removed, changed), restorable as a new server, in place, or per world. |
| **Pulse timeline and health** | A 24-hour strip of starts, crashes, restarts, backups and upgrades, and evidence-backed findings (Java missing, crash loop, no backup, disk full, content problems), each with a one-click fix. |
| **Zero-setup Java** | A checksum-verified Temurin runtime for the Java version each Minecraft release needs, for Linux and macOS. |
| **Command palette** | <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd> reaches every page, server and action. |

## Features

**Servers** · Vanilla, Paper, Purpur, Folia, Fabric, Quilt, Forge, NeoForge and Velocity from official metadata with checksum verification where upstream publishes one · start / graceful stop / restart / force stop with escalation · crash detection with exponential backoff and a crash-loop breaker · autostart · per-server memory, flags profile, port and (Linux) hard limits for memory, CPU and processes · reinstall and repair (keeping data) · EULA acceptance is an explicit, audited step.

**Console** · live WebSocket console shared by any number of viewers, ANSI colours rendered as text never HTML, command history, bounded buffers, survives controller restarts. Sensitive commands (`/login`, `/register`, `rcon`…) are redacted in the audit log by default.

**Files & config** · browse, upload, download, edit, rename, delete, confined to the server directory (no `..`, no symlink escapes) · form-based `server.properties` editor · atomic writes · undo.

**Content (Modrinth and CurseForge)** · search filtered by runtime, loader and game version · dependency resolution · installed inventory with metadata read from the jars (name, version, authors, loader, environment) · enable/disable files · identify unknown files by hash so they can be updated · compatibility panel · update-all with a reviewed plan, pinning, one undo snapshot · removal keeps plugin data.

**Modpacks** · import `.mrpack` and CurseForge packs (uploaded or by Modrinth/CurseForge project) into a new or existing server, with overrides, server-side filtering, hash-verified downloads and a report of files that cannot be fetched; export a server's content as `.mrpack`.

**Version upgrades** · plan and run a Minecraft/loader upgrade with Java and server-jar checks, per-mod status, update/disable/keep strategy, pre-change backup and rollback.

**Backups & schedules** · manual and scheduled backups, retention that never removes your last good one, locks, world-aware listing, verification, restore preview, restore as a new server / in place / selected worlds · schedules for backup, start, stop, restart and console commands, on an interval or daily at a local time in an IANA time zone (daylight-saving aware) · missed runs execute once.

**Operations & notifications** · every long job is a persisted operation with stage, progress, log tail, cancellation and a live feed; interrupted work is marked, never lost · in-app notification inbox (crash, crash loop, failed backup, low disk, failed operation) and an optional Discord or generic webhook (HTTPS only, SSRF-safe).

**Monitoring** · live CPU, memory and players, plus long-term history (1 h, 24 h, 7 d, 30 d) per server and for the host · support bundle with secrets redacted.

**Access** · Supabase sign-in, roles (`owner`, `admin`, `user`) and per-server grants (`manager`, `operator`, `viewer`) enforced on every request, WebSocket and event stream · invite-only · one-time setup token to claim ownership · optional MFA requirement for admins · personal API tokens for scripts · login throttling · full activity log.

**Interface** · dark, light, system and high-contrast themes, accent and density options, reduced motion, keyboard-first, responsive down to a phone, axe-checked in CI.

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

Moss runs on **Linux (x86-64)** and **macOS 13+ (Apple silicon and Intel)**. Windows hosts are not supported.

### From a release

Releases carry `moss-<tag>-linux-x86_64`, `moss-<tag>-macos-aarch64` and `moss-<tag>-macos-x86_64` tarballs, `SHA256SUMS.txt`, `install.sh` and a Homebrew formula (`moss.rb`). Verify before running:

```bash
sha256sum -c SHA256SUMS.txt                      # macOS: shasum -a 256 -c SHA256SUMS.txt
gh attestation verify moss-*-linux-x86_64.tar.gz --repo T1nkiePlayz/Moss
tar xzf moss-*-linux-x86_64.tar.gz && cd moss-*-linux-x86_64
```

Or use the installer, which verifies the checksum and installs one file ([`scripts/install.sh`](scripts/install.sh); read it first):

```bash
sh install.sh --prefix "$HOME/.local"
```

### Try it without any account (development mode)

```bash
MOSS_AUTH=dev ./moss serve        # http://127.0.0.1:8420 , no login, refuses non-loopback binds
```

Open the panel, create a server, accept the EULA, and (if you have no Java installed) use **Settings → Java → Install** to let Moss fetch a Temurin runtime.

### Linux: production-style install

```bash
sudo useradd --system --home /var/lib/moss --create-home --shell /usr/sbin/nologin moss
sudo install -m 0755 moss /usr/local/bin/moss
sudo install -d -m 0750 -o root -g moss /etc/moss
sudo install -m 0640 -o root -g moss moss.env.example /etc/moss/moss.env   # edit: Supabase + owner email
sudo install -m 0644 moss.service /etc/systemd/system/moss.service
sudo systemctl enable --now moss
```

The unit sets `Delegate=yes` so Moss can enforce per-server resource limits (needs systemd 254+). An Arch `PKGBUILD` lives in [`packaging/aur`](packaging/aur).

### macOS

Install the tarball (or the Homebrew formula from the release), run it as a LaunchAgent/LaunchDaemon with [`packaging/macos/io.moss.controller.plist`](packaging/macos/io.moss.controller.plist), and read [docs/macos.md](docs/macos.md) for Gatekeeper, Java and what differs from Linux.

### Reverse proxy

Put a TLS reverse proxy in front before exposing Moss (Caddy example):

```caddyfile
moss.example.com {
    reverse_proxy 127.0.0.1:8420
}
```

WebSockets and Server-Sent Events work through Caddy as is. For nginx, disable response buffering on `/api/v1/events` (`proxy_buffering off;`) and forward `Upgrade`/`Connection` headers. Set `MOSS_TRUST_PROXY=true` only behind a proxy that sets `X-Forwarded-For`. Game ports are separate and never proxied.

`moss doctor` checks the data directory, disk, ports, Java, auth and CurseForge configuration.

### From source

Requires Rust (stable) and Node.js (LTS).

```bash
git clone https://github.com/T1nkiePlayz/Moss && cd Moss
(cd web && npm ci && npm run build)       # the binary embeds web/dist
cargo build --release                      # → target/release/moss
```

## Sign-in with Supabase

Moss uses [Supabase Auth](https://supabase.com/docs/guides/auth) to prove who someone is; Moss itself decides what they may do. The browser only ever receives the public project URL and publishable key; tokens are verified locally against your project's signing keys (JWKS). Step-by-step setup (providers, redirect URLs, owners, invites, setup token, MFA, API tokens) is in **[docs/supabase.md](docs/supabase.md)**.

```bash
export SUPABASE_URL=https://YOUR-PROJECT.supabase.co
export SUPABASE_ANON_KEY=YOUR-PUBLISHABLE-KEY
export MOSS_OWNER_EMAILS=you@example.com
./moss serve
```

Nobody gets in without being the owner or being invited, even if sign-ups are open on your Supabase project. Without `MOSS_OWNER_EMAILS`, Moss writes a one-time token to `<data dir>/setup-token` and the first person to enter it on the setup screen becomes owner.

The repository's `.env` already holds this project's public `SUPABASE_URL` and publishable key, and Moss loads `.env` then `.env.local` from its working directory (real environment variables win). Fork it for your own instance by replacing those two values; keep secrets and `MOSS_OWNER_EMAILS` in the gitignored `.env.local`.

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
| `MOSS_REQUIRE_MFA` | `off` | `off`, `admins` or `all`: require an MFA (aal2) session. |
| `MOSS_TRUST_PROXY` | `false` | Read the client address from `X-Forwarded-For` (only behind a proxy). |
| `MOSS_ALLOW_PRIVATE_WEBHOOKS` | `false` | Let notification webhooks resolve to private addresses (still HTTPS only). |
| `CURSEFORGE_API_KEY` | – | Your own CurseForge key for direct access (secret; `.env.local`). Without it Moss uses the Supabase edge function, see [docs/curseforge.md](docs/curseforge.md). |
| `MOSS_CURSEFORGE_PROXY` | auto | Edge function URL, or `off` to disable CurseForge. |
| `MOSS_CURSEFORGE_API_URL` | `https://api.curseforge.com` | API base for direct mode (https, or loopback for tests). |
| `MOSS_DEV_RUNTIMES` | `false` | Expose the simulated server runtime (demos, tests). |
| `RUST_LOG` | `moss=info` | Log filter. |

Data layout: `moss.sqlite`, `servers/<id>/` (game files; Moss metadata in `.moss/`), `backups/<server-id>/<backup-id>.tar.zst`, `java/<major>/`, `removed/` (directories kept on delete/restore), `tmp/` (cleared on start), `setup-token` (until ownership is claimed).

## Runtimes

| Runtime | Tier | Content tab | Notes |
|---|---|---|---|
| Vanilla | Stable | Datapacks | Mojang manifest, SHA-1 verified |
| Paper | Stable | Plugins | PaperMC Fill API, SHA-256 verified |
| Purpur | Stable | Plugins | Upstream publishes no checksum; TLS only (logged) |
| Folia | Experimental | Plugins | Only plugins tagged for Folia are offered |
| Fabric | Stable | Mods | Server-side and both-side mods only |
| Quilt | Stable | Mods | Official installer, Fabric mods work too |
| NeoForge | Stable | Mods | Official installer; Minecraft 1.20.2+ |
| Forge | Experimental | Mods | Official installer; Minecraft 1.17+ (older versions use a different launch scheme) |
| Velocity | Stable | Proxy plugins | |
| Custom command | Experimental | – | Admin-only; explicit argv, reduced management |
| Simulated | Dev only | – | Fake server for demos and tests |

A runtime is only offered as installable when it works end to end. Spigot (BuildTools) is intentionally not offered: it cannot be redistributed, use Paper. The Forge, NeoForge and Quilt installers run the official installer jar in the server directory with a cleared environment and a time limit; they are not sandboxed (see [Security model](#security-model)).

## Mods and compatibility

What Moss does for mods and plugins, so you do not have to learn it from a crash log:

- **Right content for the right server.** Loader, game version and (Modrinth) server side are filtered before you see a result; CurseForge files tagged client-only are skipped.
- **Reads the jars.** Name, version, authors, loader and environment come from the files themselves, so manually dropped jars are first-class.
- **Checks the whole set.** The compatibility panel lists missing required dependencies, dependency version ranges (Fabric/Quilt semver, Maven ranges), duplicate ids, declared incompatibilities, loader and game-version mismatches, client-only mods and unreadable files, each with a fix button. The same findings are shown in change plans before you install.
- **Identifies unknown files** by SHA-512 (Modrinth) and CurseForge fingerprint so they can be updated like anything else.
- **Enable or disable** a file without deleting it.
- **Update everything** from one reviewed plan, with pinning, one undo snapshot and rollback of already-replaced files if a download fails.
- **Upgrade Minecraft** with a per-mod verdict (ok, update available, no build for that version, unknown) before touching the server.
- **Modpacks** in and out, with a report of anything that could not be fetched (usually CurseForge authors who disabled third-party downloads; Moss never works around that).

Provider notes: CurseForge needs an API key and is rate limited and uncached by design, see [docs/curseforge.md](docs/curseforge.md).

## Security model

Moss is designed so that signing in is not the only line of defence. Read this section before exposing it.

- **Loopback by default.** Dev auth mode refuses any other bind. A TLS reverse proxy is required for remote access.
- **Authorisation on every call.** REST, WebSocket console, SSE stream, downloads and undo/restore go through the same per-server permission check. Servers you cannot see return *not found*.
- **Invite-only, setup token, MFA.** A valid Supabase token is not enough; the account must be the owner or invited, with a verified email. Ownership of a fresh instance is claimed with a one-time token, not by being first. `MOSS_REQUIRE_MFA` can demand an MFA session for admins or everyone.
- **API tokens.** Personal tokens (`moss_pat_…`) are stored as SHA-256, shown once, scoped read or full, expire, and can never manage tokens or users.
- **Throttling.** Repeated invalid credentials from one address are rejected with 429.
- **Files confined to the server.** Path normalisation plus a symlink check on every component; `.moss/` is unreachable; uploads are never served inline.
- **Hardened downloads and archives.** HTTPS-only host allow-list re-checked on redirects, size caps, checksum verification, atomic placement; archives and modpacks reject traversal, links and devices; project links from authors are only passed on as http(s).
- **Webhooks are SSRF-safe.** HTTPS only, no redirects, the resolved address must be public.
- **CurseForge key never leaves the server side.** The edge function confirms a real signed-in user with Supabase Auth before using it.
- **Single-use stream tickets** instead of tokens in URLs; strict CSP; `nosniff`, no-referrer, no framing.
- **No shell exposure.** The UI cannot run commands on the host. Game processes are launched from an argument array. The one exception is the **admin-only Custom runtime**, which by definition runs an administrator-supplied command as the Moss user; grant admin accordingly.
- **No isolation yet (important).** Game servers, and any plugin or mod they load, run as the same OS user as Moss with that user's file access. The Forge/NeoForge/Quilt installers are third-party code run the same way. Run Moss as a dedicated unprivileged user (the shipped systemd unit does, with sandboxing), and treat plugins and mods as code you are trusting. Linux cgroup limits cap memory, CPU and process counts per server but do not isolate files or the network. Container isolation is on the roadmap.

Report security issues privately through GitHub's *Report a vulnerability* on this repository.

## How it works

One axum process: SQLite for state (numbered migrations), a persisted operation engine with per-server locks, a lifecycle supervisor that launches games detached and re-attaches on boot, one log tailer per server fanning out to WebSocket viewers, a small platform layer for Linux and macOS, and thin adapters for Mojang, PaperMC, Purpur, Fabric, Quilt, Forge/NeoForge maven, Modrinth, CurseForge and Adoptium. The full picture, including how servers survive controller restarts, is in **[docs/architecture.md](docs/architecture.md)**; the HTTP API is in [docs/api.md](docs/api.md) and [docs/openapi.json](docs/openapi.json); the original product plan is in [docs/PLAN.md](docs/PLAN.md).

## Platform support

| | Linux x86-64 | macOS (Apple silicon, Intel) |
|---|---|---|
| Servers, console, files, backups, schedules, content | ✅ | ✅ |
| Survive controller restart | ✅ | ✅ |
| Managed Temurin Java | ✅ | ✅ |
| Per-server memory/CPU/process limits | ✅ cgroup v2 (delegated) | ➖ JVM heap only |
| Host CPU and memory gauges | `/proc` | `libproc`, `sysctl`, `vm_stat`, `ps` |
| Service files | systemd unit, AUR `PKGBUILD` | launchd plist, Homebrew formula |

macOS binaries are built in CI and are ad-hoc signed, not notarised; see [docs/macos.md](docs/macos.md).

## Current limitations

- No container backend: no file or network isolation between servers and the host (resource limits are Linux cgroups only).
- Content comes from Modrinth and CurseForge only. CurseForge needs a key, cannot say whether a mod is server-safe, and never downloads files whose authors disabled third-party distribution.
- Forge before Minecraft 1.17 is not supported; non-Minecraft games are not supported.
- One node. Backups are local; no remote storage.
- Long-term metrics are hourly roll-ups kept 90 days.
- If the host reboots, running servers stop (as always); Moss restarts the ones that were running when it comes back, or that have *autostart* set.
- No staging-server health gate for updates yet.

## Roadmap

Milestones follow [docs/PLAN.md §27](docs/PLAN.md). They are gates, not dates. ✅ done · 🟡 partly done · ⬜ not started.

| Milestone | Status | What is left |
|---|---|---|
| **M0 Foundations and spikes** | 🟡 | ADRs and `docs/threat-model.md`; written spike results (kill -9 mid-operation reconciliation, console flood/backpressure); a written Modrinth terms review (CurseForge's is in [docs/curseforge.md](docs/curseforge.md)). |
| **M1 "Hello, server"** | ✅ | Recovery CLI (re-link the owner, reset a stuck operation); the simulated runtime as a shared conformance suite. |
| **M2 "Plugins"** | ✅ | Paper config editors (`paper-global.yml`, `spigot.yml`) with history; provider-outage tests. |
| **M3 "Don't lose data"** | ✅ | An upgrade test from the previous release; promote-after-compare restore flow. |
| **M4 "Mods and packs"** | 🟡 | Pack diff and pack-version pinning, staged updates with a health-gated promotion, sandboxed installers, pre-1.17 Forge. Fabric, Quilt, Forge, NeoForge, Velocity, `.mrpack`/CurseForge packs, CurseForge and the upgrade assistant are in. |
| **M5 "Polish"** | 🟡 | Theme editor and import/export, saved views, cross-product search, visual-regression suite, blueprints and collections, directory import, config diffs, desktop shell (Tauri), performance budgets. Four themes, the command palette and an axe page suite are in. |
| **M6 "Open it up" and 1.0** | 🟡 | OpenAPI freeze and compatibility policy, event webhooks beyond notifications, sandboxed UI extensions and safe mode, external security review and dependency audit, soak tests (upgrade, restore, crash, provider outage), a published supported-runtime matrix, signed checksums. The OpenAPI document, MFA policy and API tokens are in. |

**Next:** container isolation (Docker/Podman) with real resource and network limits, then staged updates with health-gated promotion, remote backup destinations, and the polish items above.

**After 1.0:** event workflows, multi-node, remote backup destinations, backend extensions and SDK, player analytics, other games and importers, public status pages.

## Documentation index

| Doc | What it covers |
|---|---|
| [docs/supabase.md](docs/supabase.md) | Auth setup, owners, setup token, MFA, API tokens, roles |
| [docs/curseforge.md](docs/curseforge.md) | CurseForge setup, edge function, limits and terms |
| [docs/macos.md](docs/macos.md) | Running Moss on macOS |
| [docs/architecture.md](docs/architecture.md) | Process model, operations, safety rules |
| [docs/api.md](docs/api.md) · [docs/openapi.json](docs/openapi.json) | HTTP API reference |
| [docs/PLAN.md](docs/PLAN.md) | Product and engineering plan |
| [web/README.md](web/README.md) | Frontend structure and mock mode |

## Development

```bash
# backend (needs web/dist to exist; an empty directory is enough for unit tests)
mkdir -p web/dist
cargo clippy --all-targets -- -D warnings && cargo test   # includes the end-to-end and OpenAPI coverage tests

# run against a simulated server, no login, no Java needed
MOSS_AUTH=dev MOSS_DEV_RUNTIMES=true MOSS_DATA_DIR=/tmp/moss-dev cargo run -- serve

# frontend with hot reload (proxies /api to :8420), or fully mocked
cd web && npm ci
npm run dev
npm run dev:mock
npm run check && npm test && npm run build
npx playwright install chromium && npm run test:ui          # every page renders cleanly and passes axe

# the CurseForge edge function (Node 22.18+)
node supabase/functions/curseforge/index.test.mts
```

Layout: `src/` (Rust: `api/`, `auth/`, `lifecycle/`, `content/`, `backups/`, `platform/`, `runtimes/`, `installer/`, `providers/`, `notify/`, `jars/`, `db/` with numbered migrations, plus `console`, `ops`, `files`, `undo`, `sched`, `health`, `java`, `download`, `history`), `web/` (Svelte UI), `supabase/` (edge function), `docs/`, `packaging/`, `scripts/`, `tests/`.

CI (`.github/workflows/ci.yml`) runs the UI type-check, tests, build and page checks, the edge function test, and backend clippy (`-D warnings`) and tests on Linux, plus clippy and tests on macOS.

## Releasing

Push a tag such as `v0.2.0`. `.github/workflows/release.yml` syncs the version into the manifests, builds the UI once, builds the Linux x86-64, macOS Apple silicon and macOS Intel binaries, packages `moss-<tag>-<os>-<arch>.tar.gz` (binary, service files, `.env`, example config), writes `SHA256SUMS.txt`, creates a GitHub **build attestation** for each tarball, generates a Homebrew formula with the real checksums, attaches `install.sh`, and opens a **draft** release whose notes are the tagged commit's message. Tags with a `-` (for example `v0.2.0-rc1`) become pre-releases.

## License

GPL-3.0-only. See [LICENSE](LICENSE). Moss is not affiliated with Mojang or Microsoft; "Minecraft" is a trademark of Mojang Synergies AB.
