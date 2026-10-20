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
> **Early development.** Moss runs real servers today, but storage formats and APIs may change between versions, and servers run as the same OS user as Moss unless you switch a server to **container isolation** (Docker or Podman). Container mode was verified in tests against a stand-in for the engine, so treat it as new. Keep backups of anything you care about, and do not expose Moss to the internet without a TLS reverse proxy.

## Table of contents

- [Overview](#overview) · [Features](#features) · [Quick start](#quick-start) · [Sign-in](#sign-in) · [Configuration](#configuration)
- [Runtimes](#runtimes) · [Mods and compatibility](#mods-and-compatibility) · [Security model](#security-model) · [Platform support](#platform-support)
- [Current limitations](#current-limitations) · [Roadmap](#roadmap) · [Documentation](#documentation)

## Overview

Moss installs, runs, watches, updates and backs up game servers on a machine you own. It is a single Rust binary that embeds its web UI, keeps state in SQLite, needs no Docker (container isolation is optional) and no hosted backend, and uses Supabase **only** to sign people in.

- **Fast and small.** One binary, a Svelte UI with no UI framework (about 52 KB of JavaScript on first load, gzipped), live state over Server-Sent Events and WebSockets, no polling storms. Idle cost is a few tens of MB.
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

**Servers** · Vanilla, Paper, Purpur, Folia, Fabric, Quilt, Forge, NeoForge and Velocity from official metadata, checksum-verified where upstream publishes one · start, graceful stop, restart, force stop with escalation · crash detection with backoff and a crash-loop breaker · autostart · per-server memory, flags profile and port · **import an existing server** from a folder, zip or tar.zst · **blueprints** to create new servers from a saved recipe, shareable as a copy-and-paste code · EULA acceptance is an explicit, audited step.

**Isolation and limits** · optional **container mode** (Docker or Podman): one mounted folder, no capabilities, read-only root, memory/CPU/process limits, Java provided by the image · on Linux, cgroup limits without containers · servers survive controller restarts in both modes.

**Console** · live WebSocket console shared by any number of viewers, command history, bounded buffers, survives controller restarts · sensitive commands are redacted in the activity log.

**Files and config** · browse, upload, download, edit, rename, delete, confined to the server folder · form-based `server.properties` · **structured editors with history and diffs for Paper, Spigot, Bukkit, Purpur and Velocity config files** (comments kept, raw editor always available) · undo for every save.

**Content (Modrinth and CurseForge)** · search filtered by runtime, loader and game version · dependency resolution · inventory read from the jars · enable/disable · identify unknown files by hash · compatibility panel · update-all with a reviewed plan and pinning · **staged updates on a test copy** with health-gated promotion.

**Modpacks** · import `.mrpack` and CurseForge packs into a new or existing server, export `.mrpack`, **update to a newer pack version with a diff**, pin a pack version.

**Version upgrades** · plan and run a Minecraft/loader upgrade with per-mod verdicts, backup and rollback; optionally try it on a copy first.

**Backups** · manual and scheduled, checksummed `tar.zst`, verify, lock, world-aware listing, restore as a new server, in place or per world · **restore preview, or restore to a staging folder and promote after comparing** · **remote destinations**: a folder (for example a NAS mount) or any S3-compatible storage (AWS, Backblaze B2, Cloudflare R2, MinIO), with upload retries, checksums and remote retention.

**Schedules, operations, notifications** · backup, start, stop, restart and console-command schedules in an IANA time zone (daylight-saving aware) · every long job is a persisted, cancellable operation · in-app inbox plus optional Discord or generic webhook (HTTPS, SSRF-safe).

**Monitoring** · live CPU, memory and players, long-term history (1 h to 30 d) per server and for the host · support bundle with secrets redacted.

**Interface** · dark, light, system and high-contrast themes with a **theme editor** (live preview, contrast checker, import/export) · accent and density options · saved views for lists · command palette that searches servers, backups, content and files (<kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd>) · keyboard first, responsive, axe-checked in CI.

**Access and recovery** · roles and per-server grants enforced everywhere · personal API tokens · login throttling · optional MFA requirement · activity log · **`moss recover`** command line for locked-out owners, stuck operations and integrity checks.

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
sudo install -m 0640 -o root -g moss .env /etc/moss/moss.env   # sign-in settings; add options from moss.env.example if you like
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

### From source (to contribute, see [CONTRIBUTING.md](CONTRIBUTING.md))

Requires Rust (stable) and Node.js (LTS).

```bash
git clone https://github.com/T1nkiePlayz/Moss && cd Moss
(cd web && npm ci && npm run build)       # the binary embeds web/dist
cargo build --release                      # → target/release/moss
```

## Sign-in

Moss uses [Supabase Auth](https://supabase.com/docs/guides/auth) only to prove who someone is; Moss itself decides what they may do. Sign-in works out of the box: releases include the public sign-in settings in `.env`. Open the panel, create an account or sign in with a provider, and enter the one-time setup token Moss wrote to `<data dir>/setup-token` to become the owner. Anyone else needs an invite from the owner, even though the sign-in service itself is open.

- Roles: `owner`, `admin`, `user`, plus per-server grants (`manager`, `operator`, `viewer`).
- Pin the owner by e-mail with `MOSS_OWNER_EMAILS`, require two-factor sign-in with `MOSS_REQUIRE_MFA`, and create personal API tokens in **Settings**.
- Locked out? [docs/recovery.md](docs/recovery.md) explains `moss recover`.
- To use your own Supabase project, set `SUPABASE_URL` and `SUPABASE_ANON_KEY` in `.env.local` (secrets and `MOSS_OWNER_EMAILS` belong there, never in `.env`).

## Configuration

Flags and environment variables (flags win):

| Variable | Default | Meaning |
|---|---|---|
| `MOSS_BIND` / `MOSS_PORT` | `127.0.0.1` / `8420` | Listener. Keep loopback unless a TLS proxy fronts it. |
| `MOSS_DATA_DIR` | `./moss-data` | Database, servers, backups, managed Java, cache. |
| `MOSS_AUTH` | `supabase` | `supabase` or `dev` (no login, loopback only). |
| `SUPABASE_URL` / `SUPABASE_ANON_KEY` | shipped `.env` | Only to use your own sign-in project instead of the default. |
| `SUPABASE_JWT_SECRET` | – | Only for legacy HS256-signed projects. |
| `MOSS_OWNER_EMAILS` | – | Comma-separated emails that become owner on first verified sign-in. |
| `MOSS_REQUIRE_VERIFIED_EMAIL` | `true` | Require a verified email before matching owner/invite. |
| `MOSS_REQUIRE_MFA` | `off` | `off`, `admins` or `all`: require an MFA (aal2) session. |
| `MOSS_TRUST_PROXY` | `false` | Read the client address from `X-Forwarded-For` (only behind a proxy). |
| `MOSS_ALLOW_PRIVATE_WEBHOOKS` | `false` | Let notification webhooks resolve to private addresses (still HTTPS only). |
| `MOSS_ALLOW_PRIVATE_BACKUP_HOSTS` | `false` | Let an S3 backup destination resolve to a private or loopback address (a MinIO on your LAN). Plain `http` is then accepted for loopback only. |
| `CURSEFORGE_API_KEY` | – | Your own CurseForge key for direct access (secret; `.env.local`). Without it Moss uses the built-in relay, see [docs/curseforge.md](docs/curseforge.md). |
| `MOSS_CURSEFORGE_PROXY` | auto | Relay URL, or `off` to disable CurseForge. |
| `MOSS_CONTAINER_ENGINE` | auto | `docker` or `podman` to force an engine for container isolation. |
| `MOSS_STAGING_PORT_RANGE` | `30000-30999` | Ports used by test copies in staged updates. |
| `MOSS_ALLOW_PRIVATE_BACKUP_HOSTS` | `false` | Let a backup destination resolve to a private address. |
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
| Forge | Experimental | Mods | Official installer; Minecraft 1.12.2 and newer (Java 8 for 1.16.5 and older) |
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

- **Try it on a copy first.** Staged updates clone the server, apply the change on a test port, watch it start, and only offer *Promote* if it ran cleanly.
- **Modpack updates** show a diff (added, removed, updated, config files you edited are never overwritten) and can be pinned to a version.

CurseForge works without any setup and is deliberately rate limited and uncached, see [docs/curseforge.md](docs/curseforge.md).

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
- **The CurseForge key never reaches your machine.** The relay confirms a real signed-in user before using it.
- **Single-use stream tickets** instead of tokens in URLs; strict CSP; `nosniff`, no-referrer, no framing.
- **No shell exposure.** The UI cannot run commands on the host. Game processes are launched from an argument array. The one exception is the **admin-only Custom runtime**, which by definition runs an administrator-supplied command as the Moss user; grant admin accordingly.
- **Isolation is opt-in.** By default, game servers and the plugins and mods they load run as the same OS user as Moss. Switch a server to **container isolation** (Settings → Isolation, needs Docker or Podman) to run it with one mounted folder, no capabilities, a read-only root, and memory, CPU and process limits. It does not restrict outbound network access. On Linux, cgroup limits are available without containers but do not isolate files or the network. Run Moss as a dedicated unprivileged user (the shipped systemd unit does), and treat plugins and mods as code you are trusting. The full analysis is in [docs/threat-model.md](docs/threat-model.md).

Report security issues privately through GitHub's *Report a vulnerability* on this repository.

## How it works

One axum process: SQLite for state (numbered migrations), a persisted operation engine with per-server locks, a lifecycle supervisor that launches games detached and re-attaches on boot, one log tailer per server fanning out to WebSocket viewers, a small platform layer for Linux and macOS, and thin adapters for Mojang, PaperMC, Purpur, Fabric, Quilt, Forge/NeoForge maven, Modrinth, CurseForge and Adoptium. The full picture, including how servers survive controller restarts, is in **[docs/architecture.md](docs/architecture.md)**; the HTTP API is in [docs/api.md](docs/api.md) and [docs/openapi.json](docs/openapi.json); the original product plan is in [docs/PLAN.md](docs/PLAN.md).

## Platform support

| | Linux x86-64 | macOS (Apple silicon, Intel) |
|---|---|---|
| Servers, console, files, backups, schedules, content | ✅ | ✅ |
| Survive controller restart | ✅ | ✅ |
| Managed Temurin Java | ✅ | ✅ |
| Per-server memory/CPU/process limits | ✅ cgroup v2 (delegated), or container mode | ➖ JVM heap only, or container mode |
| Container isolation (Docker/Podman) | ✅ | 🟡 untested with Docker Desktop |
| Host CPU and memory gauges | `/proc` | `libproc`, `sysctl`, `vm_stat`, `ps` |
| Service files | systemd unit, AUR `PKGBUILD` | launchd plist, Homebrew formula |

macOS binaries are built in CI and are ad-hoc signed, not notarised; see [docs/macos.md](docs/macos.md).

## Current limitations

- Container mode does not restrict outbound network access, does not publish UDP plugin ports, and has only been tested against a stand-in for Docker and Podman. It is offered for Minecraft servers only.
- CurseForge cannot say whether a mod is server-safe and never downloads files whose authors disabled third-party distribution.
- Non-Minecraft games are not supported. Forge before 1.12.2 is not supported.
- One node. Safety backups taken before risky changes stay local; scheduled and manual backups can go to a remote destination.
- Long-term metrics are hourly roll-ups kept 90 days.
- Remote destinations were tested against a simulated S3 service; please report quirks with a specific provider.
- If the host reboots, running servers stop; Moss restarts the ones that were running or have *autostart* set.
- The first page load is about 52 KB of JavaScript (gzipped), above the 40 KB goal.

## Roadmap

Milestones are gates, not dates. ✅ done · 🟡 partly done.

| Milestone | Status | Done | Still to do |
|---|---|---|---|
| **M0 Foundations** | ✅ | Threat model, design decisions ([docs/adr](docs/adr)), kill-and-reattach tests | Written console-flood spike results |
| **M1 Hello, server** | ✅ | Lifecycle, console, files, recovery CLI | |
| **M2 Plugins** | ✅ | Modrinth and CurseForge, Paper-family config editors, outage tests | |
| **M3 Don't lose data** | ✅ | Verified backups, restore preview, staged restore, remote destinations, upgrade test | |
| **M4 Mods and packs** | ✅ | Fabric, Quilt, Forge 1.12.2+, NeoForge, packs in and out, pack diff and pin, staged updates, container mode | Network restrictions for containers |
| **M5 Polish** | 🟡 | Four themes plus editor, saved views, blueprints, import, search, axe suite | Visual-regression suite, desktop shell (Tauri), a smaller first load |
| **M6 Open it up and 1.0** | 🟡 | OpenAPI document, MFA policy, API tokens | OpenAPI freeze, external security review, soak tests, signed checksums, event webhooks |

**Next:** container network limits and UDP ports, smaller first load, and feedback from real remote-storage providers.

**After 1.0:** event workflows, multi-node, extensions and SDK, player analytics, other games, public status pages.

## Documentation

| Doc | What it covers |
|---|---|
| [docs/curseforge.md](docs/curseforge.md) | How CurseForge works in Moss, limits, using your own key |
| [docs/macos.md](docs/macos.md) | Running Moss on macOS |
| [docs/recovery.md](docs/recovery.md) | `moss recover`: locked-out owner, stuck jobs, integrity |
| [docs/threat-model.md](docs/threat-model.md) | What Moss defends against and what it does not |
| [docs/architecture.md](docs/architecture.md) · [docs/adr](docs/adr) | How it works and why |
| [docs/api.md](docs/api.md) · [docs/openapi.json](docs/openapi.json) | HTTP API reference |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Building, testing and releasing |

## License

GPL-3.0-only. See [LICENSE](LICENSE). Moss is not affiliated with Mojang or Microsoft; "Minecraft" is a trademark of Mojang Synergies AB.
