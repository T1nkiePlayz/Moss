# Moss — Master Product & Engineering Plan

> **Project:** Moss — a self-hosted game-server controller (Minecraft Java first)
> **Binary / CLI:** `moss` · **Service user:** `moss` · **Crate prefix:** `moss-*`
> **Document type:** Product requirements + architecture + decisions + delivery plan
> **Status:** Living plan. Nothing here is "implemented" until it is verified in code (see [§28 Definition of done](#28-priorities-and-definition-of-done)).
> **Primary target:** Linux x86-64 with systemd; self-hosted Minecraft Java servers
> **Revision:** v2 (restructured from the original `plan.md`; see [Appendix C](#appendix-c-what-changed-from-the-original-plan))

**Moss in one sentence:** a fast, local-first control panel that installs, runs, updates, backs up and recovers game servers on machines you own — with a polished web UI, an optional desktop shell, a headless CLI/API, runtime-aware plugin/mod management, and safe, permissioned extensibility.

---

## How to read this document

| If you are… | Read |
|---|---|
| Deciding *what* Moss is and isn't | §1–2 |
| Reviewing the technical bets | §3 (decisions), §4 (architecture), §5 (security) |
| Planning product work | §6–§17 |
| Planning engineering/ops work | §18–§26 |
| Planning delivery | §27 (milestones), §28 (priorities), §29 (stable gate), §30 (risks) |

**Contents**

**Part A — Foundations**
1. [Vision, principles and non-goals](#1-vision-principles-and-non-goals)
2. [Scope, terminology and requirement language](#2-scope-terminology-and-requirement-language)
3. [Decision register (ADRs)](#3-decision-register-adrs)
4. [Architecture](#4-architecture)
5. [Security model](#5-security-model)

**Part B — Product**
6. [Runtimes and server lifecycle](#6-runtimes-and-server-lifecycle)
7. [Content: plugins, mods, modpacks and providers](#7-content-plugins-mods-modpacks-and-providers)
8. [Console, files, configuration and players](#8-console-files-configuration-and-players)
9. [Backups, worlds, change plans and recovery](#9-backups-worlds-change-plans-and-recovery)
10. [Schedules, events, workflows and notifications](#10-schedules-events-workflows-and-notifications)
11. [Overview, monitoring, health and diagnostics](#11-overview-monitoring-health-and-diagnostics)
12. [Blueprints, collections, import and migration](#12-blueprints-collections-import-and-migration)
13. [Web panel UX, themes, search and command palette](#13-web-panel-ux-themes-search-and-command-palette)
14. [Desktop shell](#14-desktop-shell)
15. [Controller extensions](#15-controller-extensions)
16. [CLI and headless administration](#16-cli-and-headless-administration)
17. [Public API, events and webhooks](#17-public-api-events-and-webhooks)

**Part C — Engineering**
18. [Configuration, filesystem layout and data model](#18-configuration-filesystem-layout-and-data-model)
19. [Performance budgets](#19-performance-budgets)
20. [Deployment and reverse proxy](#20-deployment-and-reverse-proxy)
21. [Logging, diagnostics and audit](#21-logging-diagnostics-and-audit)
22. [Reliability and failure contracts](#22-reliability-and-failure-contracts)
23. [Testing strategy](#23-testing-strategy)
24. [Repository and engineering workflow](#24-repository-and-engineering-workflow)
25. [Support policy, compatibility and releases](#25-support-policy-compatibility-and-releases)
26. [Operational runbooks](#26-operational-runbooks)

**Part D — Delivery**
27. [Roadmap: milestones with demos](#27-roadmap-milestones-with-demos)
28. [Priorities and definition of done](#28-priorities-and-definition-of-done)
29. [Stable-release (1.0) acceptance criteria](#29-stable-release-10-acceptance-criteria)
30. [Risks and open questions](#30-risks-and-open-questions)
31. [Deferred expansion](#31-deferred-expansion)

**Appendices:** [A. Glossary](#appendix-a-glossary) · [B. References](#appendix-b-references) · [C. Changes from original plan](#appendix-c-what-changed-from-the-original-plan)

---

# Part A — Foundations

## 1. Vision, principles and non-goals

### 1.1 Vision

Moss lets an owner install, configure, operate, monitor, customise, update, back up and recover game servers from one coherent interface, on hardware they control. It takes the focused management depth of Pterodactyl and pairs it with a faster, more cohesive interface and a stronger safety story (previewed changes, verified backups, recoverable operations).

Moss is a **server controller**, not a hosting business. It needs no vendor cloud, subscription, account or telemetry for any core function.

### 1.2 Three first-class operating modes

1. **Local web panel** — the service serves the UI on a configurable port, bound to `127.0.0.1` by default.
2. **Desktop shell (optional)** — a thin native window with tray and notifications over the same API and frontend.
3. **Headless** — service + CLI + API with no display. The web panel can be disabled by configuration.

**Invariant:** closing a tab or window, crashing the UI, or restarting/upgrading the controller must **not** stop healthy game servers unless an administrator explicitly asks for that.

### 1.3 Product promises

- Pterodactyl-class depth with a more polished, cohesive UI.
- A hard distinction between **server software**, **plugins**, **mods**, **modpacks** and **datapacks**; the UI only offers what the selected server can actually run.
- Modrinth and CurseForge behind provider adapters; correct compatibility filtering.
- Previewed, recoverable changes; backups that are verified and restore-tested.
- Works fully offline for management; provider outages never block lifecycle operations.
- Low idle overhead, bounded memory/disk, measured performance.
- Safe defaults: loopback bind, no default credentials, no host shell exposed, explicit consent for risky actions.

### 1.4 Working principles (tie-breakers when requirements conflict)

1. **Reliability before breadth.** Few fully supported runtimes beat many half-working installers.
2. **The controller is not the server.** Game processes outlive UI and controller.
3. **Typed content.** Plugins, mods, proxy plugins and modpacks are different things.
4. **Local-first.** No mandatory cloud, account or paid service.
5. **Security is architectural.** Auth, permissions, isolation and secret handling precede UI.
6. **Customisation without arbitrary code.** Themes are data; extensions get explicit capabilities.
7. **Fast by measurement.** Budgets and benchmarks from the start; no unmeasured claims.
8. **Recoverable operations.** Preview, log, and recovery path for every destructive action.
9. **One API.** Web, desktop, CLI and automation use identical capabilities and permissions.
10. **Honest compatibility.** Supported / experimental / planned / unsupported are visible and truthful.
11. **Never destroy user data on a heuristic.** Drift, diagnostics and AI-style suggestions propose; humans approve.

### 1.5 Non-goals for 1.0

- Billing, reseller, or customer-subscription features; a hosted/managed Moss.
- Requiring Supabase/remote DB/vendor cloud/GitHub Pages.
- Supporting every game or every server fork.
- Editing the user's Nginx/Caddy/Apache config, DNS, router or firewall without explicit consent.
- Running third-party scripts as root.
- Windows/macOS *hosts* (the desktop shell may later run there as a client only).
- A public extension marketplace; multi-node orchestration; public status pages; AI features (see [§31](#31-deferred-expansion)).
- Redistributing provider files outside the provider's own distribution rules.

### 1.6 Naming and legal notes

- "Moss" is the working product name. Before the first public release: check trademark, domain, GitHub org and crates.io availability, and settle on a distinctive package name if `moss` collides.
- Moss is **not affiliated with Mojang/Microsoft**. Do not use Minecraft logos/branding; use nominative references only ("Minecraft Java Edition servers"). Include a notice in the About page and docs.
- Licence is an open decision ([§30.2](#302-open-questions)); it must be chosen before the first public commit.

---

## 2. Scope, terminology and requirement language

### 2.1 Content terminology (used identically in UI, API and code)

| Term | Meaning |
|---|---|
| **Server software / runtime** | The executable platform: Vanilla, Paper, Purpur, Folia, Fabric, Forge, NeoForge, Quilt, Velocity, … |
| **Plugin** | Extension for a plugin-based server (Paper-family). |
| **Proxy plugin** | Extension for a proxy (Velocity, BungeeCord, Waterfall). |
| **Mod** | Modification for a mod loader. Never presented as a Paper plugin. |
| **Modpack** | Versioned bundle of mods/config for a game+loader. |
| **Datapack / resource pack / override** | Supporting content that is not a plugin or mod. |
| **Provider** | External catalogue: Modrinth, CurseForge. |
| **Controller Extension** | An extension to Moss itself (unrelated to game plugins). |
| **Node** | A machine running game servers. 1.0 has exactly one implicit `local` node. |
| **Operation** | A durable, observable unit of work (install, backup, restore, update…). |
| **Change plan** | A reviewable description of file/config mutations before approval. |

The backend uses **typed content models** (`Plugin`, `ProxyPlugin`, `Mod`, `Modpack`, `Datapack`, …), never a single generic `mod`.

### 2.2 Initial target

Minecraft Java Edition on Linux x86-64. The architecture accommodates other games, but Moss does not claim support for another game until its installer, lifecycle, readiness/query, backup and content integrations are implemented and tested.

### 2.3 Requirement language

**must / must not / required** = release-blocking unless scoped to a later milestone. **should** = strong default; deviation needs a recorded rationale. **may** = optional. Feature status is tracked separately in `docs/status.md` as *planned → in progress → implemented → tested → deprecated*; this document never proves implementation.

### 2.4 Requirement IDs

Cross-cutting, test-linked requirements carry stable IDs (`SEC-xx`, `OPS-xx`, `CNT-xx`, `BKP-xx`, `API-xx`, `PERF-xx`). Tests and milestone exit criteria reference these IDs so coverage can be audited.

---

## 3. Decision register (ADRs)

Each row is a **proposed default**. Changing one requires an entry here with date, rationale, migration impact and affected sections. Items marked ⚑ are validated by a Phase-0 spike ([§27](#27-roadmap-milestones-with-demos)) before being treated as final.

| # | Decision | Default | Why / revisit when |
|---|---|---|---|
| D1 | Product model | Self-hosted controller, not a host | Keeps ownership local |
| D2 | First game / host | Minecraft Java; Linux x86-64 + systemd | Smallest real surface |
| D3 | Core language | Rust (Tokio, Axum, Serde, tracing) | Supervision, concurrency, single static-ish binary |
| D4 | DB | SQLite (WAL, `synchronous=FULL` for op/audit tables) via `sqlx` | Zero ops; PostgreSQL only when multi-node coordination needs it |
| D5 | Frontend | TypeScript, Svelte 5 + Vite, static SPA embedded in the binary (`rust-embed`) | One artifact; no separate web server |
| D6 | UI libraries | xterm.js (console), CodeMirror 6 (editor), uPlot (charts) | Small, fast, accessible enough; avoid Monaco/heavy chart libs |
| D7 | API typing | `utoipa` OpenAPI → generated TS client (`openapi-typescript`) | Contract-tested, single source of truth |
| D8 | Streaming | **SSE** for events/operation progress; **WebSocket** for console (bidirectional) | SSE is proxy-friendly and resumable (`Last-Event-ID`) |
| D9 | Desktop | Tauri 2, thin shell, minimal capabilities | Native bits only; no business logic |
| D10 ⚑ | Process survival | Game processes are **not children of the controller** (see [§4.3](#43-process-supervision-and-controller-restart-survival)) | Core promise; must be proven by spike S1 |
| D11 ⚑ | Isolation default | Restricted container (Docker Engine API via `bollard`) behind a `RuntimeBackend` trait; rootless Podman as hardening path; native mode via `moss-shim` + systemd transient scope, marked *less isolated* | Docker is pragmatic; socket access is root-equivalent and must be documented ([§5.4](#54-isolation-and-host-safety)) |
| D12 | Java provisioning | One pinned image per Java major (Temurin LTS); Java version chosen from game version | Avoids host Java drift |
| D13 | Backup format v1 | `tar` + `zstd` + `manifest.json` + SHA-256; content-addressed/dedupe backends (e.g. restic) later behind a trait | Simple, restorable with standard tools |
| D14 | Secrets | Envelope encryption (XChaCha20-Poly1305) of secret rows with a root-owned `0600` master key; optional `systemd-creds` | No secrets in settings JSON, exports or logs |
| D15 | IDs | UUIDv7 for all entities; server directories keyed by ID, never display name | Sortable, collision-free, path-safe |
| D16 | Default listener | `127.0.0.1:8420` (placeholder port; confirm free of well-known conflicts) | No accidental exposure |
| D17 | Remote access | User-managed reverse proxy first; no relay | Clear trust boundary |
| D18 | Providers | Modrinth first; CurseForge optional and policy-reviewed | Isolates terms/access risk |
| D19 | Extensions | Data-only themes → sandboxed-iframe UI extensions → trusted backend adapters (later) | Privilege staging |
| D20 | Automatic changes | Preview + approval for destructive/high-impact | Prevents surprise loss |
| D21 | Telemetry | None in 1.0. Any future telemetry is opt-in and content-free | Local-first |
| D22 | RBAC shape | Global roles (Owner, Admin) + **per-server grants** of permission bundles (Manager, Operator, Viewer) | Original conflated global and per-server roles |
| D23 | Node model | Every server carries a `node_id` from day 1; only `local` exists in 1.0 | Avoids a schema rewrite later |
| D24 | Metrics | cgroup v2 first; bounded ring buffers + rollups | Accurate per-server attribution on Linux |
| D25 | Licence | **Open** — see §30.2 | Must close before first public commit |

---

## 4. Architecture

### 4.1 Components

| Component | Responsibilities | Needed headless? |
|---|---|---|
| **Controller service** (`moss serve`) | State, RBAC, API, lifecycle, jobs, config, events, embedded web UI | Yes |
| **Operation engine** | Durable jobs, locks, recovery | Yes |
| **Runtime backends** | Start/stop/attach/limits for a game process (container, shim) | Yes |
| **Game adapters** | Installers, startup rules, readiness, ports, data catalogue, capabilities | Per game |
| **Provider adapters** | Search, versions, dependencies, downloads, update checks | Per enabled provider |
| **Metrics/health collectors** | Bounded sampling, findings | Optional, isolated |
| **Web panel** | Svelte SPA served by the service | Optional (on by default) |
| **Desktop shell** | Window, tray, notifications, deep links | No |
| **CLI** (`moss …`) | Admin, recovery, automation; talks to the API (local socket or HTTP) | Yes |
| **Extension host** | Validated UI extensions; later backend adapters | Core works if disabled |

```text
Browser ───────┐
Desktop shell ─┼─ HTTP/SSE/WS ─▶ Controller API ─┬─ AuthN / RBAC / audit
External tools ┘                                 ├─ SQLite (+ migrations)
CLI ── unix socket / HTTP ──────────────────────▶├─ Operation engine ─ locks, checkpoints
                                                 ├─ Event bus (outbox + cursors)
                                                 ├─ Game adapters ─▶ RuntimeBackend ─▶ [container | moss-shim] ─▶ game process
                                                 ├─ Provider adapters ─▶ Modrinth / CurseForge
                                                 └─ Collectors, scheduler, backup workers
```

The UI never builds shell commands or controls processes. It calls validated API operations; adapters execute a narrow set of audited actions.

### 4.2 Local-first rules

- Core management works offline (providers and downloads excepted).
- No vendor login, license server, hosted relay or required telemetry.
- Server data and state stay on the user's machine unless the user configures another destination.
- A provider outage must not prevent start, stop, backup or management.

### 4.3 Process supervision and controller-restart survival

This is the hardest promise in the plan and the original left it implicit. Decision D10/D11:

- **Container mode (recommended):** the container runtime owns the process; Moss reconnects to the container on startup and re-attaches console I/O. Console history comes from the runtime's log driver *and* Moss's own rotating log file.
- **Native mode:** each server is launched under a tiny `moss-shim` supervisor started in a **systemd transient scope/unit** (so cgroup v2 limits, `KillMode=process` semantics and survival across controller restarts come from systemd). The shim owns the PTY/stdin pipe, exposes a **Unix socket** for console attach/command injection/stop requests, writes an exit-status file, and forwards logs to disk. The controller is a *client* of the shim.
- On controller start: enumerate expected servers → reattach to live shims/containers → compare to desired state → reconcile (never blindly start/kill all).
- Shim crash handling: if the shim dies the game process is treated as *orphaned* — surfaced as a health finding with explicit "adopt" / "stop" actions; never silently killed.
- **Spike S1 gate:** kill `moss serve` with `SIGKILL` mid-run; servers keep running; restart reattaches console with no lost lines beyond the documented buffer; stop/restart work afterwards.

### 4.4 Durable operation engine

All work that may outlive an HTTP request — install, content download, update, backup, restore, import, clone, cleanup — is an **Operation**.

**State machine**

```text
queued → preparing → running → verifying → completed
            │           │          │
            └───────────┴──────────┴──▶ failed | cancelled
controller crash while running/verifying ─▶ interrupted ─▶ (resume | reconcile | needs-attention)
```

Each operation records: ID, actor, target (server/node/resource), type, idempotency key, created/started/ended times, stage, progress (`measured | estimated | indeterminate`), cancellation policy, retry policy, checkpoint, result, and a bounded event stream + readable log.

Rules:
- **OPS-01** State transitions are persisted *before* they are acknowledged or acted on.
- **OPS-02** The browser, desktop shell or API client disconnecting never cancels an operation.
- **OPS-03** On restart, in-flight operations are reconciled against actual filesystem/process state. A destructive step is **never** blindly replayed; if safe resumption is impossible the operation becomes `interrupted/needs-attention` with evidence and a next step.
- **OPS-04** Cancellation stops only at a declared safe boundary and cleans controller-owned temp artifacts; it never leaves a half-written live directory.
- **OPS-05** Per-resource locks have an owner, a lease with heartbeat, and recovery metadata; stale locks are resolvable safely after crashes. Lock scope: `server:<id>` (exclusive for install/restore/delete/update; shared for backup-read), plus `cache:<key>` for downloads.
- **OPS-06** API-triggered operations accept an `Idempotency-Key`; a replay returns the original operation.
- **OPS-07** Job history, logs and events have configurable retention and size caps.
- **OPS-08** UI, CLI, API and webhooks observe identical operation IDs and states.
- **OPS-09** Every operation type has a documented **failure contract** ([§22.2](#222-failure-contracts-per-operation)).

### 4.5 Desired state vs observed state

Keep declared configuration (**desired**) separate from what exists on disk and in the process table (**observed**). Desired state: runtime/version, startup settings, limits, declared content, ports, autostart policy, controller-managed config.

- Detect drift (removed plugin, unexpected JAR, changed runtime, edited managed config, server running when policy says stopped).
- Show expected vs actual and the **source** of each expected value (blueprint, user change, import, default).
- **Never overwrite user files because they differ from a manifest.** Produce a change plan; destructive reconciliation needs approval.
- Paths can be marked `controller-managed`, `user-managed` or `excluded` from drift checks.
- Persist last successful reconciliation; expose a machine-readable drift report.
- Reconcile after import, update, restore, controller restart.

**Server lifecycle states** (observed) and transitions:

```text
uninstalled → installing → stopped ⇄ starting → running → stopping → stopped
                              │          │         │
                              │          └─ failed-start (readiness timeout / exit)
                              └──────────────── crashed (unexpected exit) ─▶ restart policy → starting | stopped(crash-loop)
suspended (admin-disabled, cannot start) · needs-attention (orphaned, drift, interrupted op)
```

Desired states: `running | stopped | suspended`. A mismatch is a *finding*, not an automatic action, except where an explicit restart policy applies (with backoff and crash-loop breaker).

### 4.6 Capability-driven adapters

Each runtime/game adapter publishes a **versioned capability manifest**: install, start/stop/restart, graceful stop, console in/out, player query, metrics, file/config support, plugin/mod/proxy-plugin management, backups (online/offline), staged updates, health checks, clone, readiness detection, data catalogue.

- The backend validates every request against capabilities; the frontend renders from the same response. **No runtime-name conditionals scattered through the UI.**
- Unknown capability ⇒ unavailable (never guessed from a similar runtime).
- Capabilities are documented, testable, and tied to supported version ranges.

### 4.7 Event bus

Events are written to an **outbox table** with a monotonically increasing sequence, then fanned out to SSE/webhooks/in-app notifications. Clients resume with a cursor; consumers must tolerate duplicates and gaps (see [§17.4](#174-event-contract)). Console output is *not* stored in the relational DB.

---

## 5. Security model

### 5.1 Threat model (summary — expand in `docs/threat-model.md` during Phase 0)

| Asset | Main actors | Key threats | Primary mitigations |
|---|---|---|---|
| Host integrity | Malicious plugin/mod/modpack, compromised provider response, hostile server admin | Container/host escape, arbitrary code via installer, privileged socket abuse | Unprivileged identities, restricted containers, no docker.sock in guests, installers run **in** the sandbox, no host shell API |
| Controller credentials & secrets | Remote attacker, local unprivileged user, XSS | Session theft, token leak, secret in logs/exports | Argon2id, hashed tokens, `HttpOnly`/`Secure`/`SameSite` cookies, CSRF, redaction, `0600` secrets, CSP |
| World/save data | Operator error, bad restore, buggy update | Silent loss/corruption | Verified backups, restore-to-new-dir default, change plans, retention locks |
| Panel users' isolation | Low-privilege user | IDOR, WS subscription bypass | Per-request, per-resource authz on REST/WS/SSE/downloads |
| Network surface | Internet scanners | Exposed admin port, Host-header/proxy spoofing, SSRF via webhooks/downloads | Loopback default, allowed-hosts, trusted-proxy list, SSRF-safe fetcher |
| Supply chain | Malicious archive/package | Zip-slip, zip bombs, symlink escape, TOCTOU | Hardened extractor, size/ratio limits, re-check at use, no execution of downloaded files |
| UI integrity | Hostile console/file content, themes, extensions | XSS, spoofed warnings, hidden security notices | Text-only rendering, ANSI parsed to spans, sanitised previews, theme validation, locked "security chrome" |

**Security-sensitive review list** (extra review required in PRs): authentication, session/token code, process execution, path handling, archive extraction, provider downloads, webhooks/fetchers, extensions, reverse-proxy trust, backup/restore.

### 5.2 Authentication and sessions

- **SEC-01** No default credentials. First admin created in setup; setup route closes permanently once an Owner exists.
- **SEC-02** First-run takeover protection: setup is loopback-only **or** requires a short-lived one-time secret printed to the local console/journal.
- **SEC-03** Argon2id hashing (starting parameters ≈ 64 MiB, t=3, p=1; configurable; re-hash on login if parameters strengthen). Reject common/compromised passwords using a **local** list; configurable minimum length; no arbitrary composition rules.
- **SEC-04** Sessions: 256-bit random IDs, only a hash stored server-side; `HttpOnly`, `Secure` (when HTTPS), `SameSite=Lax` (or `Strict`), `__Host-` prefix where possible; rotate on login and privilege change; idle + absolute timeouts; list/revoke sessions.
- **SEC-05** CSRF protection for state-changing browser requests (double-submit or origin-bound token) plus `Origin`/`Host` validation.
- **SEC-06** Login/recovery throttling with **progressive delay** (not hard lockout) so an attacker cannot lock the owner out; local CLI recovery always available.
- **SEC-07** Optional TOTP MFA (1.0 *should*, not a blocker); recovery codes.
- **SEC-08** Browser sessions and API tokens are different credential types; neither appears in logs or URLs.
- **SEC-09** CLI password reset invalidates all sessions and is an audited security event (password never logged).
- **SEC-10** Role downgrade or account deletion revokes sessions/tokens immediately.

### 5.3 Authorisation (RBAC)

Two layers (D22):

- **Global roles:** `Owner` (everything, incl. security/recovery), `Admin` (users, nodes, runtimes, templates, global settings per grants), `User` (no global power).
- **Per-server grants** of permission bundles: `Manager`, `Operator`, `Viewer` (and custom bundles). API service accounts receive scoped permissions only.

**Permission vocabulary** (checked in the service layer on *every* REST request, SSE/WS subscription, download, restore and extension action):

| Namespace | Permissions (examples) |
|---|---|
| `server.*` | `view`, `power` (start/stop/restart), `kill`, `console.read`, `console.write`, `settings.write`, `delete` |
| `files.*` | `read`, `write`, `delete`, `archive` |
| `config.*` | `read`, `write`, `read-secrets` |
| `content.*` | `view`, `install`, `update`, `remove` |
| `backup.*` | `view`, `create`, `restore`, `delete`, `download` |
| `schedule.*` | `view`, `manage` |
| `players.*` | `view`, `moderate` |
| `access.*` | `view`, `manage` (per-server sharing) |
| `admin.*` | `users`, `roles`, `tokens`, `runtimes`, `settings`, `extensions`, `audit`, `nodes` |

Hiding a button is never security. Cross-resource ID guessing must return not-found, not forbidden, for resources the caller cannot see.

### 5.4 Isolation and host safety

- Each server runs under an unprivileged identity, preferably in a restricted container (no extra capabilities, `no-new-privileges`, read-only root where practical, seccomp/AppArmor default profile).
- CPU, memory, PID and storage limits applied where supported (cgroup v2).
- Mount only the server's own directory (and read-only runtime assets). **Never** mount host root, controller secrets, SSH keys, other servers' data, or a container socket.
- No generic shell/process API. Installers for runtimes (e.g. Forge/NeoForge installer JARs) execute **inside the sandbox** as an install-stage operation, never on the host.
- Document honestly: giving the controller access to the Docker socket is root-equivalent; Moss's privileged surface is minimised, enumerated in `docs/privileged-operations.md`, and rootless Podman support is a tracked hardening goal with a tested matrix.
- Native mode: dedicated unprivileged OS user per server where possible; labelled *less isolated* in UI and docs.

### 5.5 Secrets and tokens

- **SEC-11** API tokens: format `moss_<id>_<secret>_<checksum>` (prefix enables secret scanning); shown once; only a hash stored; scopes, optional expiry, last-used, owner, instant revocation.
- **SEC-12** Provider keys (CurseForge) stored server-side in the secret store only; never in JS bundles, HTML, desktop assets, logs, error messages, exports, support bundles or API responses.
- **SEC-13** Secrets never appear in settings JSON, audit metadata, blueprint/collection exports, content manifests, backups' manifests, or release artifacts (CI secret scan + tests).
- **SEC-14** Redaction is *structured* (typed secret fields + pattern rules) and tested — not just regex replacement of strings.

### 5.6 Public-exposure safeguards

Before a remote-domain setup is reported "ready" the UI/`moss doctor` validates or warns on: completed setup + strong admin credentials; HTTPS at proxy/service; public base URL and allowed hostnames; trusted-proxy list (never trust arbitrary `X-Forwarded-*`); Host/Origin validation and explicit CORS; bounded upload sizes; controller port not directly internet-reachable; a safe backup-before-upgrade path.

### 5.7 UI-integrity rules

- Console and file previews render as **text**; ANSI is parsed to safe styled spans; uploaded HTML/SVG is never rendered as active content in the authenticated origin (serve downloads with `Content-Disposition: attachment`, `X-Content-Type-Options: nosniff`, sandboxed CSP).
- Strict CSP for the panel (no inline script; hashed/nonce'd where unavoidable).
- Themes/extensions cannot hide critical security warnings, change security wording, or remove visible focus indicators ([§13.5](#135-themes-layout-and-branding), [§15](#15-controller-extensions)).

---

# Part B — Product

## 6. Runtimes and server lifecycle

### 6.1 Runtime catalogue (Minecraft Java)

| Category | Runtimes | Notes |
|---|---|---|
| Official | **Vanilla** | Mojang version manifest (with published SHA-1) |
| Plugin servers | **Paper**, **Purpur**, **Folia**, Spigot | Folia: warn about regionised threading; Spigot needs a local BuildTools build (cannot be redistributed) ⇒ *Experimental* at best |
| Modded | **Fabric**, **NeoForge**, **Forge**, Quilt | Installers run inside the sandbox; Quilt only if maintained/tested |
| Proxy | **Velocity**, Waterfall/BungeeCord | Velocity first; others only with a maintained adapter |
| Custom | Manual/custom profile | Advanced, clearly marked, reduced guarantees |

Use **official metadata APIs** (Mojang manifest, PaperMC downloads API, Fabric meta, loader installers) rather than scraping; pin installer source metadata; verify checksums/signatures when upstream provides them. Verify current API shapes at implementation time — upstream APIs have versioned and deprecated before.

### 6.2 Support tiers and matrix

A checked-in `runtime-definitions/support-matrix.json` renders into docs and the runtime picker. Each entry: runtime ID, upstream, supported game versions, OS/arch, installer source, content types, minimums, tier, last-verified date, known limitations.

| Tier | Meaning | UI |
|---|---|---|
| **Stable** | Automated + end-to-end tests pass for the declared range | Normal install |
| **Experimental** | Usable, known gaps/limited coverage | Warning + limitations before install |
| **Community/custom** | User/extension-supplied | No automatic compatibility promise |
| **Planned** | Not implemented | Informational; no install button |
| **Deprecated** | Manage existing, discourage new | Migration guidance + end date |

A runtime is **not** Stable because it launched once. Promotion requires tests for installer, version resolution, start/stop, console, ports, backup/restore, error reporting, upgrade and content filtering, and re-testing when upstream changes metadata/installers/Java requirements/config formats. Each runtime has a compatibility fixture and smoke-test procedure.

### 6.3 Minecraft-specific rules (explicit, because they are easy to get wrong)

- **EULA:** Moss must never silently accept the Minecraft EULA. Creating a server requires an explicit user action that records acceptance (who/when) and writes `eula.txt` accordingly; this is audited.
- **Java selection:** map game version → required Java major (e.g. recent versions need newer LTS than 1.16-era ones) via runtime definitions; run in the matching pinned image; warn on user overrides.
- **JVM flags:** ship sane, versioned default flag sets per runtime (heap sizing with `-Xms`=`-Xmx` default, GC choice documented); user-editable with validation and a preview of the final command line. Never allow free-form shell strings; arguments are an array.
- **Graceful stop:** send the console `stop` command → wait up to a per-runtime timeout → escalate (SIGTERM → confirmed force-kill). Force-kill needs explicit confirmation in UI.
- **Readiness:** log-pattern detection (e.g. the "Done" line) plus a Server List Ping/query probe when enabled; a readiness timeout produces a `failed-start` finding with the log excerpt.
- **Console channel:** use stdin/PTY via the shim/container attach. RCON is optional and, if used, its password is a managed secret and never exposed.
- **Ports:** Minecraft uses TCP (and optionally UDP for query); allocate and track per protocol.
- **Proxy forwarding:** Velocity forwarding mode and backend firewall guidance are documented; never silently choose an insecure mode (e.g. disabled forwarding with online-mode off).

### 6.4 Server creation wizard

1. Game → 2. Category → 3. Software → 4. Version (only tested combinations get install buttons) → 5. Java/runtime (auto-selected, overridable) → 6. Port allocation + advertised address → 7. Memory/limits + directory → 8. Accept EULA → 9. Optional content/blueprint/modpack → 10. **Review plan** (downloads, disk estimate, dependencies, warnings) → 11. Install with live operation progress → 12. Start and show readiness health + actionable failure.

The wizard uses the **same operation pipeline** as every other install (no special shortcut path).

### 6.5 Runtime templates

Versioned, data-driven definitions (executable install scripts are **not** accepted as trusted template data). Each defines: ID/name/category/upstream links; version constraints; download source + verification; Java/system requirements; startup arguments + validated variables; ports/protocols; readiness/query; graceful-stop procedure + timeout; config files/schemas + restart requirements; supported content types; backup consistency behaviour; world/data catalogue; known incompatibilities; maintenance status.

### 6.6 Lifecycle features

Install, start, graceful stop, force stop (confirm), restart, suspend, clone, reinstall, delete (typed-name confirmation; **never** cascades to world data without an explicit separate choice); autostart with ordering/delays; crash detection, restart policy with exponential backoff and a crash-loop breaker; per-server locks; recovery after host/service restart without blindly starting or killing everything.

### 6.7 Networking

Controller port separate from game ports; collision-free per-protocol allocation; configurable bind addresses (IPv4/IPv6 where supported); show **internal bind vs public advertised address** distinctly; firewall/NAT guidance without claiming to configure routers; warn when a running server is not externally reachable *or* reachability cannot be verified.

### 6.8 First-run host setup and diagnostics wizard

1. Detect OS, arch, CPU, memory, disk, service manager, container runtime, cgroup v2, Java where relevant. 2. Explain dependencies, show pass/fail/skipped; **never install packages without approval**. 3. Choose data/server/backup/cache/log directories and verify permissions + space. 4. Create the Owner securely; close setup. 5. Explain local vs reverse-proxy deployment. 6. Create a first server through the normal pipeline. 7. Offer a backup schedule and a quick post-start health check. 8. Offer an optional **previewed, redacted** support bundle.

---

## 7. Content: plugins, mods, modpacks and providers

The UI must never mix incompatible content because two projects share a search term.

### 7.1 Provider selection and fallback

Users choose **Modrinth** or **CurseForge** (global default + per-user preference) with optional **Search both**.

1. Search the selected provider first.
2. Optionally (user setting, or when nothing useful is found) search the other provider.
3. Show alternatives in a distinct **Found on the other provider** section with branding and direct links.
4. **Never** treat two projects as the same because names match; equivalence requires known links, author identity, metadata and explicit user confirmation.
5. **Never** silently substitute one provider's file for the other's.
6. Deduplicate requests, cache, respect limits, and show fallback status.

For non-Minecraft games on CurseForge, show that game's catalogue and only the actions its adapter supports.

### 7.2 Provider adapters

Separate adapters behind a common internal trait; keep provider IDs and rules intact.

- **Modrinth:** search, project, versions, loaders, game versions, dependencies, files, hashes, downloads. Identify Moss with a descriptive `User-Agent` including contact info as the API asks; honour documented rate limits and `Retry-After`. Docs: https://docs.modrinth.com/api/.
- **CurseForge:** search/files/categories/dependencies/downloads where permitted. Key is **user-supplied or build-supplied but always server-side** (SEC-12); explain where it's stored and how to rotate it. Handle missing `downloadUrl` (distribution disabled by the author) as a normal "manual download required" state — never work around it. Verify current API terms before release. Docs: https://docs.curseforge.com/rest-api/.
- Treat key rejection, rate limits, missing downloads and outages as ordinary states; failures never affect server lifecycle.

### 7.3 Mandatory content-kind rules (CNT-01)

| Runtime family | Primary content | Never shown as installable |
|---|---|---|
| Paper / Purpur / Folia / Spigot | **Plugins** (+ datapacks where supported) | Fabric/Forge mods |
| Velocity / BungeeCord / Waterfall | **Proxy plugins** | Paper plugins, mods |
| Fabric / Forge / NeoForge / Quilt | **Server mods**, modpacks | Plugins; warn on client-only |
| Vanilla | Datapacks and server-side additions | Plugins/mods |
| Other games | Types from that adapter + provider metadata | Minecraft-only filters |
| Custom | User-declared types, or auto-install disabled | — |

Filter by game, software, game version, loader, environment/side, class/category, dependencies. Incompatible results may be *viewable*, but install is disabled or requires an explicit, informed override. Ambiguous classification ⇒ **unknown**, requiring inspection.

### 7.4 Content UI

Search + filters + provider-valid sorts; icons, descriptions, authors, links, version history, changelogs, licences, dependencies; compatibility badges with *reasons*; installed state, pinned version, available update, provider, file/version ID, hash status; **review screen** (files, dependency changes) before install/update/uninstall; pin/unpin; favourites and collections; provider switch + alternatives.

- Plugin servers: default tab **Plugins**; warn on version/implementation mismatch; detect duplicate projects and obvious duplicate jar names; update plan with backup first; **removal preserves plugin data/config** unless separately chosen; state plainly that provider listing ≠ safe/compatible.
- Mod servers: **Mods** tab; recursive required-dependency resolution with cycle/conflict reporting; client-only/unknown-side warnings; manifest of provider/project/version/file/hash/source/time/pin policy.

### 7.5 Modpacks

- Support Modrinth `.mrpack`; CurseForge packs where terms/API permit.
- Validate manifests *before* extraction; apply overrides only inside the server directory; reject traversal and symlink escapes; respect side/environment metadata.
- Show runtime, game version, loader, content list, dependencies, disk estimate and warnings **before** install.
- Report missing/restricted/unavailable files explicitly (never silently skip).
- Produce reproducible install manifests; compare pack versions (added/removed/changed/unavailable); prefer staged-copy updates with a readiness check before promotion; **never silently change game major version or loader**.

### 7.6 Installed-content inventory and provenance

Canonical inventory (not just a directory listing). Per item: typed kind, runtime target, provider, project ID, version/file ID, resolved version, source reference, hash + algorithm, install path/time, pin/update policy, dependency links, verification status.

Four distinct states: **Declared** (wanted) · **Observed** (found on disk) · **Resolved** (exact provider file chosen) · **Installed** (placed and verified). Unknown/manual items are labelled as such — never invent a provider match; users may attach identity manually (audited).

### 7.7 Collections

Named, portable lists (items, pinned versions, targets, optional items, notes) created from the catalogue or a server inventory. Compare a collection to a server (missing/extra/mismatched/pinned/unknown/incompatible), apply via a **reviewed change plan** with backup/rollback, export/import (no credentials or file payloads), reuse in blueprints and API, revision history.

### 7.8 Updates and compatibility confidence

Show installed vs latest vs compatible candidates, changelog, dependency changes, pinned state, last-checked time. Policies: manual · notify-only · scheduled-review · explicitly approved automatic (low-risk only).

Compatibility confidence is explicit: **confirmed by metadata · likely · unknown · incompatible**. Missing metadata is never shown as green. Bulk update plans list every file/dependency change, download/storage estimate, and whether a staging test is possible.

### 7.9 Download correctness and caching (CNT-02…)

- Use API-provided IDs/URLs/hashes; never fabricate URLs.
- **SSRF-safe fetcher** shared by providers and webhooks: allow-listed hosts for provider downloads, redirect re-validation, block private/loopback/link-local/metadata ranges, DNS re-resolution checks, size/time limits.
- Verify hashes when supplied and report when verification is unavailable.
- Download to a temp area → validate type/size/hash → **atomic rename**; never truncate a live file before the replacement is verified.
- Bounded concurrency, timeouts, cancellation; resume where practical; exponential backoff with jitter honouring `Retry-After`.
- TTL'd, size-bounded metadata cache with visible freshness; no mirroring of whole catalogues; manual refresh.

### 7.10 Content operation lifecycle

Validate state/capabilities/path/space/permissions → resolve exact project+file and store the decision metadata → resolve dependencies/conflicts (stop with an actionable report if mandatory ones fail) → build a **change plan** → download to temp (bounded) → backup or staging copy when a working server could be affected → apply atomically → verify and record each item `installed | failed | skipped | unknown` → start/validate only per user policy (no automatic restart without consent) → audit + durable result including rollback limits. The controller **never executes** downloaded JARs/scripts during install.

Bulk operations require a preview; partial failure yields precise per-item results, never a blanket "success".

---

## 8. Console, files, configuration and players

### 8.1 Console

- Live stdout/stderr over an authenticated WebSocket (resumable by cursor); command input only for supported game-console commands — never the host shell.
- Search/filter, pause-scroll, copy, bounded range download; timestamps and severity/source when known; start/stop markers; crash exit codes and restart history; optional error-pattern highlighting.
- **Bounded** in-memory buffer (ring) + rotating on-disk logs; backpressure to slow clients (drop with an explicit "N lines skipped" marker, never unbounded growth).
- Rendering: xterm.js or equivalent, text-only, ANSI → safe spans.

### 8.2 File manager

Breadcrumbs, search, sort/filter; upload/download (streamed), mkdir, rename, copy, move, delete; multi-file ops with progress and size limits (as operations when large); text editor (CodeMirror) with syntax highlighting/validation; atomic writes; unsaved-change guard; optional backup-on-overwrite; **concurrent external edit detection** (mtime/hash check at save).

Safety: strict path canonicalisation inside the server root; safe symlink policy (refuse escapes; `openat2`/`O_NOFOLLOW` style re-check at use to defeat TOCTOU); hardened archive extraction (zip-slip, absolute paths, device files, symlink tricks, ratio/size/entry-count limits); never render HTML/SVG actively; recursive delete requires confirmation.

### 8.3 Configuration editor

Schema-driven forms for common settings **alongside** raw editing. Forms declare exactly which file/keys they manage; unknown keys survive; validate before save; indicate *reload / restart / reinstall* requirements; preview changes before applying to a running server; show the final startup command (argument array) with explanations. Preserve comments/formatting when the parser round-trips reliably, otherwise warn or edit as text. Secrets are redacted fields and never enter diffs, history, search indexes or audit logs. Per-key help only from versioned docs or tested metadata.

### 8.4 Config history and rollback

Snapshots/diffs for supported text configs and controller-managed settings: who/when/through what/which operation; readable diffs; restore as a **new operation** with its own backup/plan (history is never rewritten); user-excludable sensitive paths; rollback to a known-good copy only when Moss can establish what it changed. State rollback limits (e.g. plugin migrated its own data).

### 8.5 Players and administration

Where the adapter supports it: online players and counts; allowlist, ops, bans; moderation commands; join/leave history (optional, retention-configurable, can be disabled while keeping live counts). Distinguish **unsupported** from **empty**. Player identifiers and moderation require explicit permissions; never exposed unauthenticated.

---

## 9. Backups, worlds, change plans and recovery

A backup that has never been restored is not a backup. Backups are a P0 feature (milestone M3), not a late extra.

### 9.1 Backup types and records

Manual/scheduled **full-server**; **world-only** (via the runtime's data catalogue); **config + content manifest**; **controller state** (DB + config, separate admin operation); incremental/deduplicated only if measurements justify it (behind a `BackupBackend` trait).

Every backup record states precisely what was included/excluded/verified: server ID, time, size, SHA-256, included paths/logical data, source runtime/version, controller version, status, consistency method.

### 9.2 Behaviour

- **BKP-01** Online consistency via adapter commands (Minecraft: `save-off` → `save-all flush` → copy → `save-on`, always restoring `save-on` even on failure); otherwise stop the server or show an explicit consistency warning.
- **BKP-02** Format v1: `tar.zst` + `manifest.json` (file list, sizes, hashes) + sidecar SHA-256; written to a temp name and atomically renamed only after verification.
- **BKP-03** Verify after creation (re-read/checksum); periodically test-restore into a scratch directory.
- **BKP-04** Pre-flight free-space check; destination outside the live server dir; later: SFTP/object storage via adapters.
- **BKP-05** Retention by count/age/size with **retention locks**; never delete the last known-good backup; cleanup touches only controller-owned artifacts and is audited.
- **BKP-06** A failed backup never replaces a previous valid one.
- Filesystem snapshots/CoW (btrfs/ZFS/LVM) only where detected and tested; otherwise documented file-copy.

### 9.3 Restore

Preview contents and target; **restore into a new directory by default** when overwrite risk is significant; in-place restore needs a preview, a fresh recovery point where feasible, and explicit confirmation; require a safe server state (unless the adapter supports online restore); reject traversal/unsafe symlinks/device files; preserve the original until the restored copy is verified; world-only restore leaves plugins/mods/settings alone and warns on version mismatch; progress + recovery plan on failure. CI includes restore tests (BKP-07).

### 9.4 World/save data catalogue

Adapters expose a typed catalogue (world/save locations, categories, exclusions, whether a clean shutdown is required). Minecraft: derive the **active world from server config**, not from a familiar folder name; show level name, game version metadata, size, mtime. Unknown games get a generic file view with no guessed semantics.

World import/export: guided, with path/space/archive/symlink checks; never extract untrusted archives straight into a live server directory; basic integrity checks (truncated archives, missing expected files); deep region-file diagnostics are a later, separately tested capability.

### 9.5 Change plans

Any controller-managed operation that may replace or delete files produces a **change plan**: targets, adds/replaces/removes, dependencies, config impact, storage estimate, downtime, backup strategy, rollback feasibility. Users inspect before approval; high-risk plans need explicit confirmation. Change plans are persisted, linked to their operation, and available via API/CLI.

### 9.6 Staged updates and health-gated promotion

Where supported and space permits: resolve versions → download to cache → verify hashes → apply to a **staging copy** → run readiness checks → collect logs → show a diff → user approves promotion across a clear maintenance boundary → rollback restores prior files/config. Be honest: a staging boot doesn't prove gameplay correctness — show exactly which checks ran. Without staging, fall back to backup-first maintenance.

### 9.7 Storage pressure

Show consumption by server/backups/logs/cache/controller data where attribution is reliable; warn **before** cleanup would remove the last good backup; early disk/inode warnings; never treat a world or unknown file as disposable cache.

---

## 10. Schedules, events, workflows and notifications

### 10.1 Scheduler

One-off and recurring: start/stop/restart; backup + verify + prune; update **discovery**; supported console commands; controller-owned cache/log cleanup; resource summaries.

- Timezone-aware with explicit DST rules; show the next N run times before saving.
- Prevent overlapping runs; maintenance windows/quiet hours with visible "deferred because…".
- **Discovery ≠ installation:** discovery can be scheduled freely; applying disruptive updates needs an approval policy.
- Pause/resume/run-now/edit/disable with permissions and audit.
- Durable across restarts. **Missed-run policy** per schedule: `skip | run-once | replay-bounded` — never replay a backlog of destructive jobs without explicit opt-in.
- Every run links to its operation record.

### 10.2 Event-triggered workflows (post-1.0, P2)

A workflow is a constrained graph of typed triggers, conditions, delays and actions — **no arbitrary shell**. Requirements: small safe action set first; loop prevention, rate limits, max depth, per-workflow concurrency; idempotence (repeated crash events must not become an infinite restart loop); approval for destructive/disruptive actions unless a narrowly scoped admin policy exists; dry-run preview; per-step execution history; exportable versioned definitions without secrets.

### 10.3 Notifications

In-panel **inbox** first (unread/read/acknowledged/snoozed/resolved; grouped by finding/resource ID; links to evidence), then optional desktop notifications, then adapters (Discord webhook, email, others). No integration is required for core operation.

- Distinguish **event** (happened), **finding** (problem persists) and **completed operation**; reading never resolves a finding.
- Per-user/per-server routing, severity thresholds, digests, quiet hours — never suppressing security audit events.
- Delivery queue with bounded retry/backoff/dedup; a failing channel never blocks operations; show last error without revealing secret URLs/headers.
- Payloads exclude credentials, full console dumps, world data and private file contents; no leakage to users lacking access to the server.
- Same controller-side event identity across desktop and web (no duplicate alerts); bounded retention with export.

---

## 11. Overview, monitoring, health and diagnostics

### 11.1 Overview

A live operational summary useful for one server and navigable for dozens: status, uptime, runtime/version, players (if supported), last operation, latest backup, update availability, alerts; host CPU/memory/storage/IO/network (host-wide vs per-server vs estimated clearly distinguished); pin/sort/group/tag/filter/save views (e.g. "failed", "needs update", "unhealthy backups"); contextual actions with identical permission checks; timestamps and **stale/unavailable** indicators; incremental updates over bounded subscriptions (no per-server polling loops); a broken collector never breaks the dashboard or lifecycle API.

### 11.2 Metrics

- Sampling: configurable (default live 5 s while viewed; stored 60 s), rolled up to 15 min / 1 h aggregates; default retention short→long (initial proposal 24 h / 14 d / 180 d — tune after measuring); disable or reduce on constrained hosts.
- Source of truth: cgroup v2 counters per server; host `/proc`. Label anything shared, inferred or sampled.
- Minecraft TPS/MSPT only via a supported integration/query source — **never inferred from CPU**.
- Avoid personal data when aggregates suffice. Time-series stored in a bounded append-oriented store (rolling files or a dedicated SQLite file), **not** the core operational tables.

### 11.3 Health centre and findings

Evidence-backed findings, **not** an opaque score. Each finding: stable ID, severity, resource, observed-at, evidence, confidence/limitations (deterministic vs heuristic), impact, suggested actions, links, ack/snooze state.

Initial rules: crash/restart loops and failed readiness; sustained memory pressure, limit violations, disk/inode pressure, log growth; port conflicts, invalid startup config, missing/incompatible Java; missing/stale/failed/unverified backups; content conflicts and unexpected inventory change; migration failures, stale locks, interrupted jobs, permission errors; provider outages/rate limits/invalid credentials; extension compatibility failures; orphaned processes.

Rules are versioned, testable, configurable (thresholds, hysteresis, cooldowns, dedup, ack). Users can disable noisy non-critical rules; **security-critical findings cannot be silenced by ordinary preferences**. Correlations (e.g. plugin install → restart → resource spike) are presented as evidence, never proof; Moss never auto-removes plugins or edits configs from a heuristic.

### 11.4 Trends and forecasts

Disk-exhaustion and recurring-pressure estimates show assumptions and uncertainty and are advisory only (never auto-delete or resize). Attribute resources to a server only where measurable.

### 11.5 Diagnostics and support bundles

- `moss doctor` and the Help page report: version/build, OS/runtime facts, dependencies, listeners, disk, DB/migration status, failed jobs, redacted logs, provider status — works offline.
- **Support bundle:** user selects scope → generated locally → **redaction applied before preview** → user inspects/removes items → export. Contains manifest, redaction summary, file hashes, size cap. Excludes world data, server directories, player DBs, arbitrary files, secrets. **Moss never uploads a bundle automatically.**
- Every recommendation cites its evidence and is labelled deterministic or heuristic. Remediation is an explicit operation (preview, permission, audit, rollback) — never a silent shell command.

### 11.6 Player activity insights (optional)

Current players, recent joins/leaves, aggregate activity only where supported and enabled; configurable retention; disable history while keeping live counts; permission-gated; never in public/unauthenticated endpoints.

---

## 12. Blueprints, collections, import and migration

### 12.1 Blueprints

A blueprint is a portable, versioned **recipe**, not a backup. Contains: runtime/version policy, loader/proxy type, Java requirements, startup arguments, env declarations with **secret references (never values)**, limits, ports, a **pinned content manifest** (provider, project, version/file ID, hash, target, update policy), controller-managed config patches with conflict policy and exclusions, backup/schedule defaults, health checks, restart policy, tags, docs, schema version and provenance/history.

Workflows: create from a running server (preview), export/import (schema + runtime + path + limit + provider validation), duplicate, recreate on another node. Secrets are prompted or resolved from the secret store. Blueprint vs actual content flows through the drift/change-plan workflow.

### 12.2 Import existing server directories (read-only first)

1. Select a source directory and a destination strategy (copy · move-after-verified-copy · reference where safe).
2. **Read-only scan** (never executes server files): detect likely runtime/version, startup args, Java needs, ports, world paths, content, config, symlinks, ownership, unsupported items — each with a *confidence/unknown* state.
3. Dry-run report: detected / unknown / conflicting / unsupported; collisions; space; inaccessible files; absolute symlinks and paths escaping the source.
4. User maps identity/runtime/ports/storage/world/content; unknown content stays in place and is reported.
5. Copy-first with checksum verification; source untouched until finalisation.
6. Start only when asked; verify the imported server launches before suggesting the original be retired.

Never import host credentials, privileged scripts, service definitions or unknown executable hooks as trusted config. Record all assumptions/omissions.

### 12.3 Panel-to-panel migration (later)

Pterodactyl and other adapters only after documented export/API support and a tested compatibility matrix listing preserved/transformed/ignored/unsupported fields. Staged: inspect → report → resolve conflicts → backup source → import → validate → isolated start test → user acceptance. Never request the source panel's admin password when a read-only export or limited token suffices.

### 12.4 Portable bundles and exports

Server definitions, blueprints, schedules and supported config metadata export separately from world files. Versioned, schema-validated, documented. **Never** include secrets, sessions, password hashes or provider credentials. Include a manifest (files, hashes, runtime needs, content provenance, omissions); optional world/content payload with size estimates and licence/provider caveats; import validates paths, expansion limits and manifest consistency; round-trip tests across supported versions.

---

## 13. Web panel UX, themes, search and command palette

### 13.1 Visual principles

Premium developer/hosting-product feel: deliberate typography, hierarchy and spacing; subtle surfaces; fast feedback. Distinctive identity — don't copy another product's branding or layouts.

- State and next safe action are always obvious; destructive actions are visually separated.
- Useful density; colour is semantic **and** never the only signal.
- Restrained, short motion with reduced-motion support; no animated graphs competing with the console.
- No layout shift: reserve space for graphs/cards/tables/status; skeletons only when they help; no flash for cached data.
- Consistent skeleton/empty/error/retry/confirmation patterns.
- Keyboard-operable, screen-reader friendly, visible focus, responsive, high-contrast capable.
- Strong dark theme **and** an equally considered light theme (not an inverted dark).
- Avoid blanket blur/glass, oversized cards, decorative charts.

**Before full UI build** produce design references: desktop+mobile, component states, type/spacing/elevation scales, icons, charts, forms, tables, toasts, modals, command palette, empty/error/loading states, theme tokens. Visual quality is an acceptance criterion, not final polish.

### 13.2 Information architecture

Global: **Overview · Servers · Content · Backups · Schedules · Host/Nodes · Users & Access · Activity · Extensions · Settings · Help**. Disabled features stay discoverable via settings/docs.

Server list: compact list + cards; search, tags, saved filters, favourites, sort; status/runtime/version/uptime/CPU/mem/players/alerts; permissioned quick actions.

Server detail tabs: Overview · Console · Files · Configuration/Startup · Content (named for runtime: Plugins / Mods / Proxy Plugins / Datapacks) · Players · Backups · Schedules · Access · Activity/Logs · Settings.

### 13.3 Long-running operation UX

Persistent jobs visible across navigation/reload; stage, measurable progress, elapsed time, log excerpt; actionable errors with safe retry/recovery; distinct queued/running/verifying/completed/failed/cancelled/interrupted states; success only after verification; cancel only where safe. An **Operations centre** drawer is available everywhere.

### 13.4 Command palette and power-user navigation

Keyboard-first palette (fuzzy search for pages, servers, actions, settings, help, recent operations) built from the **same capability and permission data** as the UI, filtered server-side. Shows target and consequence; destructive actions go through the normal confirmation (never one keystroke); discoverable shortcuts with conflict detection and customisation; respects text-entry focus, screen readers and reduced motion; invalid/colliding extension commands fall back safely. Also: quick-switcher, favourites, recents, saved filters, compact/comfortable density.

### 13.5 Themes, layout and branding

- **Built-in:** Dark, Light, Follow-system, High-contrast; controlled accent, density, radius, motion — genuinely designed variants.
- **User themes:** theme editor with live preview; documented tokens (colour, type, spacing, border, radius, shadow, surface) within validated limits; import/export versioned theme files; reset-to-default (also via URL/CLI escape hatch); per-user choice with admin default; shareable without a marketplace.
- **Data-only:** schema-validated, versioned; **no JS**; automated checks reject themes that fail minimum contrast, hide warnings, alter security wording, make destructive actions ambiguous, or remove focus indicators. "Security chrome" (warnings, destructive confirmations) uses locked tokens.
- **Layout:** reorder/hide dashboard widgets, pin servers, card/list, density, sidebar state, units/time display, bounded refresh settings. Preferences are stored separately from server config.
- **Branding:** admin-configurable title/logo/favicon/accent/help links; must not impersonate another company or hide security notices; user themes remain separate from admin branding.

### 13.6 Cross-product search

Permission-aware, bounded search across server names, tags, runtime metadata, content inventory, operation history, config keys and indexed log windows. Excludes secrets/credential files/excluded paths; incremental, time-bounded log search (no multi-GB loads); shows source, timestamp, context, deep links; filters by server/kind/status/date; indexing optional with disclosed disk footprint.

### 13.7 UI quality gates

Shared design-token system for web and desktop; visual-regression screenshots for first-run, login, overview, server detail, content search, job progress, backup/restore, theme editor and error states at supported viewports/themes; automated contrast + keyboard checks (axe); manual screen-reader pass; reduced-motion and high-contrast verification.

---

## 14. Desktop shell

Tauri 2, same frontend/API as the web panel; native-only features live in a narrow bridge.

- **Features:** native window; open panel in the system browser; optional tray with status/basic actions; native notifications (crash, backup done, job failed, resource warning); open server dir in file manager; copy address/logs/path; launch-at-login and minimise-to-tray; connection/recovery screen when the service is unreachable; per-user desktop preferences.
- **Lifecycle:** distinguish *close window* ≠ *quit GUI* ≠ *stop controller* ≠ *stop game server*. Closing the GUI never stops the service or servers. A systemd-managed service is never killed or adopted unexpectedly; stopping the controller is a distinct action with a warning if managed processes depend on it. Desktop updates never touch server data.
- **Capabilities:** minimal Tauri permission set; no arbitrary shell or unrestricted filesystem from the webview; the webview talks to the controller API like any other client (token/session handling reviewed).
- **Tests:** window close, renderer crash and sign-out leave controller and servers running.
- Desktop is delivered late (M5); the web panel is the primary surface.

---

## 15. Controller extensions

Called **Controller Extensions** in the UI (distinct from game plugins). Core must work with extensions disabled; extensions are **disabled by default** until trusted.

### 15.1 Trust classes

| Class | Examples | Risk | Stage |
|---|---|---|---|
| **Themes** | Token files | Low (data only, no code) | M5 |
| **UI extensions** | Widgets, nav entries, panels, commands, settings pages | Medium | M6 |
| **Backend adapters** | Game/provider/notification/backup adapters | High | Post-1.0 |
| **Catalogue/marketplace** | Signed distribution | Needs mature infra | Deferred |

### 15.2 UI extensions (stage 1)

Run in a **sandboxed iframe** (unique origin, restrictive CSP) communicating through a `postMessage` bridge to **explicitly granted** controller APIs; no direct DOM access to the host panel, no raw DB/filesystem/process/arbitrary network access. Manifest declares: ID, name, version, author, controller-API compatibility range, contribution points, requested capabilities (each with a reason), integrity/signature metadata, install/update/removal and data-migration rules, resource limits where possible.

### 15.3 Backend adapters (stage 2, post-1.0)

Installed only from an explicit **local** package chosen by an admin; admin trust decision with an exact capability list; prefer a WASM or separate-process boundary; native dynamic libraries are treated as fully trusted code with explicit consent; never load from a remote URL at runtime.

### 15.4 Permissions, recovery and compatibility

- Backend enforces extension permissions; manifest-declared permissions are *requests*, not grants. Update that requests more permission requires **renewed consent**; no silent expansion.
- Incompatible extensions are disabled with an explanation (no optimistic loading).
- **Safe mode** (`moss serve --safe-mode` / config flag / UI banner) starts the base UI with third-party extensions disabled; CLI can disable a specific extension; one broken extension can't block login or server control.
- Records of versions/hashes enabling rollback; extension error log (redacted); audit of install/update/enable/disable/permission change/removal.
- An extension cannot alter another extension's permissions, disable audit, or bypass backend authz.
- Be explicit: an iframe is **not** a complete security boundary against a compromised controller; do not market it as such.

### 15.5 Extension SDK (after the core contract stabilises)

Versioned SDK: manifest schema, capability declarations, typed API client, event contracts, UI guidance, local dev mode with a **mock controller**, validation CLI, test fixtures, compatibility checks. Dev mode cannot silently grant production permissions. Document lifecycle hooks, timeouts, cancellation, error reporting, data migration, uninstall cleanup, and the deprecation process before encouraging third-party adoption.

---

## 16. CLI and headless administration

`moss` is a single binary; the CLI talks to the running service over a **local Unix socket** (root-/owner-authenticated via filesystem permissions) or HTTP with a token, so it works over SSH and in scripts. A small set of **offline recovery commands** operate directly on the data directory when the service is down.

```text
moss install                         # guided system install (unit file, user, dirs) — explicit consent for each step
moss serve [--bind 127.0.0.1] [--port 8420] [--safe-mode]
moss status | doctor | config validate | config dump --redacted
moss user create | list | reset-password | revoke-sessions        # recovery-capable offline
moss token create | list | revoke
moss server list|create|status|start|stop|restart|kill|delete <id>
moss server console <id>             # attach (read, optionally send commands)
moss backup create|list|verify|restore|delete <id>
moss content search|install|check-updates|update <id>
moss op list|show|cancel|logs <op-id>
moss blueprint export|import|apply
moss extension list|disable <id>     # recovery path
moss support-bundle create --preview
moss dev sim-server                  # simulated game server for UI/dev/testing
```

Requirements: human output by default, `--json` for automation; meaningful exit codes (non-zero on failure; never report success early); `--dry-run` for destructive/broad commands; secrets redacted by default; safe prompts plus explicit non-interactive flags (`--yes`); shell completions; `doctor` covers config, DB, storage, ports, runtime, permissions, reverse proxy, providers and backups; API/CLI parity (deliberate, documented exceptions only).

---

## 17. Public API, events and webhooks

### 17.1 Conventions

- Versioned under `/api/v1`; OpenAPI generated from, or contract-tested against, the implementation; schemas are public interfaces once documented. Additive changes preferred; breaking changes need a new version or a deprecation window.
- UUIDv7 IDs; **cursor pagination**; input validation; bounded payloads/uploads; rate limits; `ETag`/conditional requests for static runtime/provider metadata (never for security-sensitive state).
- `Idempotency-Key` for retried mutations; long work returns **202 + operation resource**.

**Error shape** (stable codes; no stack traces, secret paths or env vars):

```json
{
  "error": {
    "code": "server.locked",
    "message": "Another operation is changing this server.",
    "request_id": "01J…",
    "retryable": true,
    "retry_after_s": 5,
    "fields": [{ "path": "memory_mb", "code": "range", "message": "Must be ≥ 512" }]
  }
}
```

### 17.2 Endpoint families

| Area | Examples |
|---|---|
| Health | `/health`, `/api/v1/status` |
| Auth/sessions | `/api/v1/auth/*`, `/api/v1/sessions/*` |
| Users/roles/tokens | `/api/v1/users`, `/roles`, `/tokens` |
| Servers | `/api/v1/servers`, `/servers/{id}` |
| Actions | `/servers/{id}/actions/{start,stop,restart,kill}` |
| Console | WS `/servers/{id}/console` |
| Events | SSE `/api/v1/events` (cursor) |
| Files/config | `/servers/{id}/files/*`, `/configuration/*`, `/config-revisions` |
| Runtimes | `/games`, `/runtimes`, `/runtimes/{id}/versions` |
| Content | `/content/search`, `/projects/*`, `/versions/*`, `/servers/{id}/content/*` |
| Operations | `/operations`, `/operations/{id}`, `/operations/{id}/events`, `/cancel` |
| Plans & drift | `/change-plans/*`, `/servers/{id}/drift` |
| Backups | `/servers/{id}/backups/*` |
| Schedules/workflows | `/schedules/*` |
| Blueprints/collections | `/blueprints/*`, `/collections/*` |
| Health | `/findings`, `/servers/{id}/health` |
| Webhooks | `/webhooks/*`, `/webhooks/{id}/deliveries` |
| Extensions/themes/prefs | `/extensions/*`, `/themes/*`, `/preferences/*` |
| Activity | `/activity` |

### 17.3 Webhooks

HMAC-signed per-endpoint secret with timestamp + event ID for replay protection; bounded exponential backoff with jitter and capped attempts; delivery history; secret rotation; event filtering; test delivery with a redacted sample. **HTTPS by default**; the shared SSRF-safe fetcher rejects loopback/link-local/private/metadata addresses (revalidating DNS and redirects) unless an admin-only local-integration setting is deliberately enabled. Payloads never contain passwords, tokens, API keys or full file contents. Inbound webhook receivers (if any) verify signatures, enforce age limits and payload sizes, and never treat inbound fields as instructions.

### 17.4 Event contract

Stable event types for server state, operation progress/completion, backups, content updates, health findings, user/security events and node connectivity. Envelope: `id`, `seq`, `time`, `schema_version`, `actor|system`, `target`, redacted `payload`. Ordering per operation where practical; consumers tolerate reconnects, duplicates and gaps via cursors/replay. Per-subscriber authorisation on every event.

---

# Part C — Engineering

## 18. Configuration, filesystem layout and data model

### 18.1 Precedence

Compiled safe defaults → `moss.toml` → environment variables (`MOSS_*`) → CLI flags. UI-changed settings are validated and written **atomically**; `config dump --redacted` is always available; secrets live in the secret store or restrictive-permission files, never in ordinary settings.

### 18.2 Settings groups

`http` (bind, port, public URL, allowed hosts, trusted proxies, upload limits) · `security` (sessions, passwords, throttling, CSRF/origins) · `storage` (data root, servers, cache, logs, backups, tmp) · `runtime` (backend, isolation defaults, limits, allowed roots) · `providers` (enabled, key references, cache TTLs, concurrency, contact metadata) · `content` (default provider, fallback, allowed types, overrides) · `backups` (destination, retention, compression, verification) · `scheduler` · `logging` · `metrics` (sampling/retention) · `desktop` · `themes` · `extensions`.

### 18.3 Filesystem layout (system install)

```text
/etc/moss/moss.toml                 # config (0640 root:moss)
/var/lib/moss/
  db/moss.sqlite (+wal, shm)        # core state
  db/metrics/                       # bounded time-series store
  secrets/master.key                # 0600 moss:moss
  servers/<server-uuid>/            # game data (per-server ownership)
  backups/<server-uuid>/…           # tar.zst + manifest
  cache/{providers,downloads,runtimes}/
  tmp/                              # controller-owned temp (only place recursive cleanup is allowed)
/var/log/moss/ or journald          # controller logs; per-server console logs under servers/<id>/.moss/logs
/run/moss/moss.sock                 # CLI control socket
```

Developer/desktop mode uses XDG paths. All paths resolved and re-validated at point of use; cleanup only deletes under `tmp/` and verified controller-owned artifact directories.

### 18.4 Conceptual entities

`Server` (owner/group, `node_id`, runtime/version, desired + observed state, paths, limits, network, policy) · `RuntimeDefinition`/`RuntimeCapability` · `Operation`/`OperationEvent`/`Lock` · `ContentProject`/`ContentVersion`/`InstalledContent`/`ContentDependency` · `Blueprint(+Revision)`/`Collection(+Revision)` · `ChangePlan`/`ChangePlanItem`/`ConfigRevision` · `Backup`/`BackupArtifact`/`RestoreOperation` · `HealthFinding`/`MetricSample`/`Notification` · `User`/`Role`/`PermissionGrant`/`Session`/`ApiToken`/`NodeIdentity` · `Theme`/`UserPreference`/`Extension`/`ExtensionPermission`/`ExtensionState` · `Schedule`/`Webhook`/`WebhookDelivery`/`AuditEvent`/`OutboxEvent`.

### 18.5 Data rules

- Migration-managed schema; explicit FKs; unique constraints preventing cross-user/server mistakes; migrations tested from fresh and from the previous released schema; **verified backup before risky migrations**.
- Console output and high-frequency metrics stay **out** of core relational tables.
- Transactions for multi-row transitions; optimistic concurrency (version columns) for edited entities.
- **Deletion semantics defined per entity** (cascade / soft-delete / retention / cleanup job). Deleting a UI record never cascades to world data.
- Indexes for server ownership, operation state/time, event cursor, schedule next-run, installed-content lookup; query plans reviewed as data grows.
- Single-writer discipline for SQLite (writer task/queue) with WAL; documented behaviour on `SQLITE_BUSY`.

---

## 19. Performance budgets

Performance is a requirement. The numbers below are **initial engineering hypotheses, not claims**: measure on documented reference hardware (e.g. 4 vCPU / 8 GB / NVMe, 50 simulated servers), then revise with a recorded rationale.

| ID | Metric | Initial budget (hypothesis) |
|---|---|---|
| PERF-01 | Controller cold start → login usable (excl. long migrations) | ≤ 1.5 s |
| PERF-02 | Idle RSS, 0 servers / 20 servers | ≤ 60 MB / ≤ 120 MB |
| PERF-03 | Idle CPU, 20 servers | ≤ 1 % of one core |
| PERF-04 | `GET /servers` p95 at 100 servers (local) | ≤ 25 ms |
| PERF-05 | Console line → browser (p95) | ≤ 150 ms |
| PERF-06 | Initial UI payload (gz, shell route) | ≤ 250 KB JS |
| PERF-07 | Command palette open → first results | ≤ 100 ms |
| PERF-08 | Backup/restore throughput; peak memory | Record; peak RSS bounded (streaming) |
| PERF-09 | Large modpack install incl. cancel + cleanup | Record; no unbounded memory |

Startup path: load config → init logging → check/migrate DB → bind listener → start bounded workers → **ready**. Providers, extension checks, metrics and indexing never block the critical path; subsystems initialise lazily; no blocking filesystem work on async threads (bounded blocking pool); stream large files/archives; hashing async + cancellable.

**Bounded everything:** each background subsystem defines memory/disk bounds, concurrency, queue capacity, retention and behaviour at limits (apply backpressure; drop or aggregate low-priority telemetry with a visible counter; **never** drop operation transitions or audit events). Constrained-host configuration may reduce analytics but never disables safety-critical checks.

Benchmarks (repeatable scripts + scheduled CI runs): cold/warm start; idle CPU/RSS; list/status latency (p50/p95/p99); UI load/navigation; console under flood; large upload/download; modpack install/cancel; backup/restore; hundreds of servers and large catalogues. Each records hardware, OS, workload, sample size and method.

---

## 20. Deployment and reverse proxy

- **Local:** the controller serves the built UI itself; Settings shows the local URL with a copy action.
- **Own domain:** documented, tested configs for **Nginx, Caddy, Apache** covering: reverse proxy to the loopback listener; WebSocket + SSE forwarding (disable buffering for SSE); public base URL; TLS termination and secure cookies; trusted-proxy + allowed-host config; upload/timeouts; firewall guidance keeping the app port private; diagnostic checks (`moss doctor --proxy`).
- **Packaging:** `.deb`/`.rpm`/tarball + systemd unit (hardened: `NoNewPrivileges`, `ProtectSystem`, `PrivateTmp`, etc., within what container/shim management allows) + container image for the controller only if the docker-socket implications are documented.
- No dependency on GitHub Pages, Supabase or any hosted panel. A public marketing/docs site may be hosted separately; it is never the management backend.

---

## 21. Logging, diagnostics and audit

- Structured logs: timestamp, level, component, request ID, operation ID. Separate streams: controller logs, per-server console logs, audit events, job history. Rotation and retention are configurable.
- **Redaction** of credentials, tokens, keys, sensitive env vars (SEC-14).
- **Audit** (append-oriented, tamper-evident chaining as a *should*): logins, permission/token/session changes, lifecycle operations, file deletions, restores, content updates, extension changes, security settings, EULA acceptance, support-bundle exports. Distinguishes actor-initiated from automated actions; stores no secrets or unnecessary personal data; read access gated by `admin.audit`.
- **Operation timeline:** follow one install/restore from request → logs → transitions → verification → result.
- Support bundles per [§11.5](#115-diagnostics-and-support-bundles).

---

## 22. Reliability and failure contracts

### 22.1 General rules

Game servers survive UI/controller failure; controller restart reconciles (never blindly kills/starts); provider outages don't block lifecycle; failed backups never replace good ones; failed updates retain prior files or a staging recovery point; full disks warn early and fail safely (no unbounded logs/temp, no DB corruption, no half-replaced live file); DB corruption or failed migration produces diagnostics and a documented recovery path (never auto-overwrite a possibly recoverable DB); handle occupied ports, invalid config, missing runtimes, dead downloads, hash mismatch, failed extraction, hung processes, interrupted jobs; retry only operations known idempotent/safe to resume. **Never claim a rollback succeeded unless the restored state was verified.**

### 22.2 Failure contracts per operation

Every operation type ships a table like the one below (in `docs/operations/<type>.md`), reviewed with the code and exercised by failure-injection tests:

| Field | Example: `content.install` |
|---|---|
| Stages | validate → resolve → plan → download → verify → apply → verify-installed |
| Invariants | Live file is never truncated before replacement is verified; inventory matches disk or is marked unknown |
| Checkpoints | After download+verify (temp artifacts retained until apply) |
| Rollback boundary | Before apply: discard temp; during apply: restore from backup/staging; after: revert plan |
| Retryable? | Yes (idempotent by content hash) |
| Cancel safe-points | Between stages and between files |
| Inject-failure tests | Network drop mid-download · hash mismatch · disk full on apply · kill controller during apply · permission denied |
| Recovery text | What the user sees on `interrupted/needs-attention` |

Failure injection covers: download completion, staging, file application, DB update, process start, final verification.

---

## 23. Testing strategy

**Backend**
- Unit: config, RBAC, path validation, port allocation, state machines, provider→compatibility mapping, job recovery, update planning, redaction.
- Integration: migrations (fresh + previous release), sessions, API authz, file ops, backup/restore, schedules, lock/lease recovery.
- **`FakeRuntime`/simulated game server** adapters make lifecycle, console and failure tests fast and deterministic; **disposable real-server smoke tests** (nightly) per Stable runtime/version.
- Providers: recorded fixtures, injected errors/rate-limits, optional conservative live smoke.
- Fuzzing: archive extractor, path canonicaliser, manifest/blueprint parsers, `.mrpack` parser.

**Security & abuse** (SEC-xx traceable): IDOR across servers/logs/backups/jobs/tokens/themes; traversal and symlink races in files/archives/uploads/restore; oversized/malformed provider responses, malicious redirects, hash mismatch, truncated archives, zip bombs; rate limiting, CSRF/Origin, session fixation, cookie flags, trusted-proxy spoofing, Host injection, WebSocket/SSE authz; secrets absent from logs/crash reports/bundles/DB exports/blueprints/backups/release archives; theme markup injection; extensions exceeding capabilities; disk-full, read-only FS, DB locked/corrupt, process killed mid-write, network timeout, controller restart.

**Frontend/desktop:** component tests (login, wizard, content filtering, provider switching/fallback, themes, extension permissions, job progress); API contract tests against OpenAPI; **Playwright E2E**: first-run, login/logout, create/start/stop, console, plugin search/install, modded search/install, CurseForge setup, backup/restore, theme import/export, extension disable, headless mode; desktop window-close/renderer-crash survival; headless service without a display server.

**Performance/visual/accessibility:** benchmark inputs and machine specs recorded; p50/p95/p99; large server list/console/catalogue cases; visual regression across themes/viewports with reviewed (not blind) snapshot updates; keyboard-only, screen-reader labels, reduced motion, contrast, zoom, narrow viewports.

**Release checks:** fmt/lint/static analysis; `cargo deny`/`cargo audit`/npm audit and licence review; secret scanning (e.g. gitleaks) incl. release artifacts; upgrade from each supported prior version; installer + systemd smoke on a clean machine; backup/restore; checksums and signed release metadata where practical; runtime/version matrix tests before labelling Stable.

---

## 24. Repository and engineering workflow

### 24.1 Layout (start small; split when a boundary is real and tested)

```text
/
├── apps/
│   ├── web/                 # Svelte SPA (shared with desktop)
│   └── desktop/             # Thin Tauri shell + native bridge
├── crates/
│   ├── moss-core/           # Domain rules, services, permissions
│   ├── moss-api/            # HTTP routes, auth middleware, SSE/WS, OpenAPI
│   ├── moss-store/          # SQLite repositories + migrations
│   ├── moss-jobs/           # Operation engine, locks, recovery
│   ├── moss-runtime/        # RuntimeBackend + adapter contracts, container backend
│   ├── moss-shim/           # Native-mode supervisor
│   ├── moss-content/        # Typed content model + provider trait + SSRF-safe fetcher
│   ├── moss-provider-modrinth/
│   ├── moss-provider-curseforge/
│   ├── moss-cli/            # `moss` binary entry points
│   └── moss-testkit/        # Fakes, fixtures, failure injection
├── runtime-definitions/     # Reviewed data-driven runtime metadata + support matrix
├── themes/                  # Built-in data-only themes
├── docs/                    # install, ops, api, security, threat model, ADRs, operations/*
├── tests/{integration,e2e,fixtures,performance}/
├── scripts/                 # Reproducible dev/release scripts (justfile targets)
├── PLAN.md                  # this document
└── README.md
```

Avoid circular dependencies; frontend never touches the DB; the web UI uses only the public API.

### 24.2 Workflow

- One-command local setup and test (`just dev`, `just test`, `just e2e`); `moss dev sim-server` for UI work without Docker.
- CI on every PR: Rust fmt/clippy/test, frontend lint/typecheck/test/build, OpenAPI contract validation + generated-client drift check, dependency/licence check, secret scan, migration test (fresh + upgrade).
- Provider fixtures by default; live tests optional, rate-limited, scheduled.
- Generated artefacts are reproducible — never hand-edited.
- Conventional commits; ADR file for every row change in §3; security-sensitive paths require CODEOWNERS review.
- Never commit API keys, tokens, world data, private addresses or unredacted bundles.
- A release candidate must install on a clean supported machine using only the published docs.

### 24.3 Code-quality rules

Domain logic in services (handlers validate + delegate); typed internal errors → stable API codes; **no unbounded channels, tasks, buffers or provider concurrency**; cancellation tokens and deadlines for long async work; filesystem access only through a validated storage-root abstraction with use-time re-checks; every privileged operation documented with why fewer privileges won't do; small testable adapters rather than giant switches; dependency additions weighed for maintenance, licence, startup cost, binary size and security history.

---

## 25. Support policy, compatibility and releases

### 25.1 Platforms

Declare a Linux distro/arch matrix, starting with a mainstream systemd x86-64 distribution. Others may work but aren't "supported" until install, service management, paths, permissions, container behaviour and upgrade tests pass. **ARM64 is tested separately — compiling is not support.** Frontend: current evergreen desktop browsers (mobile layouts supported, not the primary target). Desktop shell: only OS versions tested in CI/release smoke. No Windows/macOS *host-server* promises.

### 25.2 Versions

Publish a matrix of controller version × OS × arch × runtime × game version × Java × provider features. Identify minimum/recommended runtime versions and EOL risks. Existing servers on deprecated runtimes stay manageable (management support ≠ new-install support). Upgrade notes for schema, installer and provider-API changes; deprecation windows (except urgent security fixes). Provider logic is isolated so it can be disabled/replaced without breaking core management.

### 25.3 Channels and updates

Stable channel plus optional preview/nightly, kept distinct and clearly labelled; previews never silently replace stable. Show current version, channel, available version, migration notes. Verify metadata/integrity where signed. Verified backup of controller state before schema-changing upgrades; **no automatic DB or runtime downgrade** — a documented recovery procedure instead. Admins may defer non-security updates; security notices explain urgency and mitigations. Experimental features/runtimes never become enabled by an update.

---

## 26. Operational runbooks

Short, pressure-friendly docs in `docs/runbooks/`:

1. **Controller won't start:** `moss status` / `moss doctor`; classify failure (config, migration, port, permissions, worker); validate config + ownership + free space; if a migration failed, **preserve and back up the DB before any recovery — never delete it first**; safe mode without launching game processes unnecessarily.
2. **Game server fails to start:** show operation ID, runtime/version, exit status, startup time, log excerpt; separate install failure from runtime failure; check prerequisites/ports/memory/disk/recent changes; evidence-based suggestions only; offer to revert a recent controller-managed change when a recovery point exists.
3. **Restore/migration failure:** preserve source and last good backup; mark destination *needs attention*; record completed vs not; safe retry/cleanup plan (recursive delete only inside verified controller-owned temp); explicit choice before retiring source or promoting destination.
4. **Provider outage/rate limit:** report separately from server health; honour `Retry-After`; lifecycle unaffected; cached data shown with freshness; manual retry; redacted diagnostic event.
5. **Suspected credential exposure:** revoke sessions/tokens/provider keys; disable compromised extension/integration; preserve audit logs with restricted access; rotation steps and the list of locations that may hold the secret; never print the credential in reports.
6. **Orphaned server process (shim/container lost):** identify, verify ownership, adopt or stop with explicit confirmation.
7. **Lost owner access:** offline `moss user reset-password` (invalidates sessions, audited).

---

# Part D — Delivery

## 27. Roadmap: milestones with demos

Phases are **gates, not dates**. Each milestone ends with a *demo script* that a reviewer can run, automated tests for its exit criteria, and a buildable repository. Effort is shown as relative size (S/M/L/XL) to help sequencing, not as a promise.

The original ten phases are regrouped into vertical slices that reach **real value early** (a managed server → plugins → safe data → mods → polish → open). Notable reorderings: **backups arrive before mods/modpacks** (they are P0), **Modrinth arrives with the first plugin runtime**, and **EULA/Java/readiness details are explicit**.

### M0 — Foundations and spikes (L)

- Repo, CI, licence decision, name check, ADRs, threat model draft, design tokens + wireframes, support-matrix schema.
- **Spikes (each with a written pass/fail result):**
  - **S1 Survival:** SIGKILL the controller; servers survive; console reattaches (container *and* shim paths).
  - **S2 Embed:** Axum serving an embedded Svelte SPA + OpenAPI-generated client in one binary.
  - **S3 Ops engine:** SQLite WAL job/lock/lease model survives kill -9 mid-operation with correct reconciliation.
  - **S4 Console streaming:** xterm.js over WS with backpressure under a flood.
  - **S5 Cgroup metrics:** per-server CPU/mem from cgroup v2 in container and systemd-scope modes.
  - **S6 Provider terms:** written review of Modrinth and CurseForge API terms/limits and distribution rules.
- **Exit:** service starts, validates config, migrates SQLite, binds loopback, passes CI; S1–S5 pass or the ADRs (D10/D11) are revised.

### M1 — "Hello, server" (XL) → **Moss 0.1 (alpha)**

- First-run setup (token/loopback), login, sessions, CSRF, RBAC core, audit, `doctor`, systemd install, recovery CLI.
- Operation engine v1 + lifecycle for **Vanilla**: EULA step, Java mapping, install, start/graceful stop/restart/kill, readiness, crash detection + restart policy, ports.
- Console (stream + command), safe file manager, basic resource limits, `FakeRuntime` for tests.
- **Demo:** fresh VM → install → create a Vanilla server → join it from a client → `kill -9 moss` → keep playing → restart Moss → console reattaches → stop gracefully.
- **Exit:** OPS-01…06, SEC-01…06 tests green; servers survive browser/controller restarts.

### M2 — "Plugins" (XL) → **0.2**

- Paper (then Purpur/Folia) adapter; runtime-aware wizard and tabs.
- **Modrinth** adapter (cache, rate limits, SSRF-safe fetcher), plugin catalogue, compatibility confidence, content inventory + provenance, basic change plans, install/update/remove with backup-first.
- Config editor v1 (`server.properties`, `paper` configs) and config history.
- **Demo:** create a Paper server; search Modrinth; client-only mods are filtered/disabled; install a plugin with dependency; see the change plan; remove it and confirm data dir is preserved.
- **Exit:** CNT-01/02 tests; provider outage doesn't affect lifecycle.

### M3 — "Don't lose data" (XL) → **0.3 (public beta candidate)**

- Backups (manual + scheduled), verification, retention + locks, restore-to-new-dir and in-place with preview, world-aware catalogue, storage warnings.
- Scheduler (timezone/DST, missed-run policy), in-app notification inbox, health findings v1 (backup/disk/crash-loop), support bundle (preview + redaction), host/server overview v1.
- **Demo:** schedule nightly backup; corrupt a world; restore into a new directory; compare; promote. Run `moss support-bundle create --preview`.
- **Exit:** BKP-01…07 incl. restore tests; failure-injection suite for backup/restore; upgrade test from 0.2.

### M4 — "Mods and packs" (XL) → **0.4**

- Fabric + NeoForge (installers in-sandbox), then Forge/Quilt as tested; Velocity proxy.
- `.mrpack` install, overrides, dependency planning, pinning, pack diff; staged updates with health-gated promotion.
- **CurseForge** adapter (server-side key, missing-download handling) after S6 sign-off.
- **Demo:** import an `.mrpack` onto Fabric, see unavailable-file report, update the pack via a staging copy, roll back.
- **Exit:** modded runtimes show only appropriate content; key never leaks (SEC-12 tests).

### M5 — "Polish" (L) → **0.5**

- Themes (4 built-ins + editor + import/export), layout prefs, command palette, cross-product search, saved views, accessibility and visual-regression suites.
- Health centre rules, blueprints, collections, directory import, config diffs.
- Tauri desktop shell (tray, notifications, reconnect), performance tuning against PERF budgets.
- **Exit:** a11y/visual gates pass; PERF baselines recorded; desktop survival tests pass.

### M6 — "Open it up" and hardening (XL) → **1.0 RC**

- OpenAPI freeze + compatibility policy, CLI/API parity audit, webhooks, TOTP, UI extensions (sandboxed) + safe mode, proxy docs + diagnostics.
- External security review; dependency audit; full upgrade/restore/crash/provider-outage soak tests; supported-runtime matrix published; docs complete; signed checksums.
- **Exit:** §29 satisfied.

### Post-1.0 (gated by §31.2)

Event workflows, multi-node, remote backup destinations, backend extensions + SDK, player analytics, additional games/importers, optional AI diagnostics, public status pages.

---

## 28. Priorities and definition of done

### 28.1 Priorities (scope-conflict tie-breakers)

**P0 — release blockers:** secure setup/auth/RBAC, safe loopback default, host isolation, secret handling; durable operations + locks + recovery + idempotency; capability-driven adapters + desired/observed model; one Vanilla and one plugin runtime end-to-end; structured redacted logging and disk safeguards; **verified backup/restore**; one API shared by web/desktop/CLI; controller/servers survive UI closure.

**P1 — why choose Moss:** actionable overview/diagnostics; world-aware backups; scheduler + notification inbox; safe config diffs/history; Modrinth + authorised CurseForge with provenance and reviewed update plans; blueprints/collections/import; staged updates; command palette; custom themes; polished accessible UI; optional desktop; reverse-proxy guides; setup wizard.

**P2 — after the core is dependable:** event workflows; forecasting; player analytics; remote backup targets; extension SDK; AI-assisted diagnostics; public status pages; extra notification adapters; Pterodactyl importer; multi-node; backend extensions; marketplace; more games.

### 28.2 Definition of done (every major feature)

1. User workflow and failure states documented.
2. Backend enforces authn, ownership and capability checks.
3. Long-running work uses durable operations and survives navigation/restart.
4. Structured logs + audit events emitted without secrets.
5. Cancellation, retry and recovery defined **and tested**.
6. API and CLI provided where operationally meaningful.
7. Tests cover expected, invalid, interrupted and permission-denied paths.
8. UI has loading, empty, error, success and unsupported-capability states.
9. Resource bounds and performance impact measured for background work.
10. Docs state limitations and make no claim for checks that don't exist.
11. Status in `docs/status.md` updated; requirement IDs linked to tests.

---

## 29. Stable-release (1.0) acceptance criteria

For the declared support matrix, **all** of the following hold:

**Security & access**
- Panel binds to loopback by default; secure login; setup route closed after first admin; CLI recovery works.
- Backend authorisation enforced on REST, SSE, WebSocket, downloads and extension actions (IDOR suite green).
- CurseForge key never reaches frontend, logs, bundles, exports or release assets; secret scans pass.
- Extensions disabled by default; safe mode and CLI disable work; updates can't silently expand permissions.

**Reliability**
- Headless service runs under systemd without a desktop session.
- Closing browser/desktop GUI, and restarting or upgrading the controller, doesn't stop game servers.
- Long operations survive navigation and controller restart or clearly report interruption; locks prevent conflicts; schedules survive restart with a documented missed-run policy.
- Desired vs observed state distinguishable; drift never overwrites user files without a reviewed plan.
- Server lifecycle, console, files, permissions, backups and restore have automated tests; backup→restore round-trip verified in CI; restore preserves the original until verified.
- Upgrade/rollback and migration paths tested; failure-injection suites pass for every operation type.

**Product**
- At least one Vanilla and one Paper-family runtime Stable end-to-end; other runtimes labelled honestly by tier.
- Plugin servers show plugins; modded runtimes show mods; proxies show proxy plugins; Modrinth works; CurseForge works where terms/API permit; provider selection and cross-provider suggestions never silently replace projects/files.
- Blueprints export/import with schema validation, no secrets, unavailable-dependency preview; inventories record provenance and mark manual/unknown items; config history and change plans redact secrets and state rollback limits.
- Health centre reports only supported metrics, links findings to evidence, and never auto-fixes destructively; dashboard distinguishes live/stale/estimated/unsupported; monitoring failure never stops lifecycle control.
- Directory import is read-only until approval and source-preserving.
- Command palette respects permissions and confirms destructive actions; search excludes secrets and is bounded.
- Diagnostic bundles local-first, scoped, redacted, reviewable, size-bounded.
- Event workflows (if shipped) have loop prevention, bounds and audit.

**Quality & docs**
- Visual regression and accessibility checks cover primary workflows and supported themes.
- Benchmarks exist with recorded baselines for startup, idle resources, list latency, console output, large installs and backup/restore.
- Reverse-proxy docs include WebSockets/SSE, TLS, trusted proxies and exposure warnings; install, backup, security, API and extension docs complete; support matrix published.
- Release artifacts signed/checksummed where practical and verified free of secrets.

---

## 30. Risks and open questions

### 30.1 Risk register

| # | Risk | L | I | Mitigation | Owner milestone |
|---|---|---|---|---|---|
| R1 | **Process survival** across controller restart is harder than expected (PTY ownership, reattach) | M | H | Spike S1 first; shim + container paths; fall back to container-only support if needed | M0 |
| R2 | **Docker socket is root-equivalent**; compromise of controller ⇒ host | M | H | Minimise/enumerate privileged ops, document, pursue rootless Podman, restrict API surface | M0–M6 |
| R3 | **CurseForge terms/access** change or block distribution workflows | M | M | Isolated optional adapter; "manual download required" state; never bypass restrictions | M0 (S6), M4 |
| R4 | **Runtime maintenance burden** (installer/API drift) | H | M | Official APIs, pinned metadata, nightly smoke matrix, honest tiers | continuous |
| R5 | **Content metadata quality** is inconsistent across providers | H | M | Confidence states, unknown-is-not-green, explicit overrides | M2 |
| R6 | **Scope creep** (original plan had 38 sections of surface) | H | H | P0/P1/P2, milestone demos, §31.2 expansion gate, say no | continuous |
| R7 | **Backup/restore bugs** cause data loss | L | H | Verify-after-write, restore tests in CI, retention locks, restore-to-new-dir default | M3 |
| R8 | **Extension sandbox escape / over-trust** | M | H | UI-only first; honest boundary claims; backend enforcement; safe mode | M6 |
| R9 | **SQLite contention** under heavy console/metrics writes | M | M | Keep high-rate data out of core DB; single-writer queue; benchmarks | M0–M3 |
| R10 | **Performance claims** unmet | M | M | Budgets are hypotheses; measure early (M1) and gate regressions | M1+ |
| R11 | **Trademark/naming** conflicts (Moss; Minecraft references) | M | M | Name check in M0; nominative use only; "not affiliated" notice | M0 |
| R12 | **Rootless/network matrix** (Docker/Podman, IPv6, host networking) | M | M | Test matrix; don't claim universal support | M4+ |
| R13 | **Single maintainer bus-factor / burnout** | M | H | Small P0, strong docs/ADRs, automation, contributor guide | continuous |

### 30.2 Open questions (need an answer; owner and deadline in `docs/decisions/`)

1. **Licence:** AGPL-3.0-or-later for core (protects against closed hosted forks) with Apache-2.0/MIT for SDK/schemas/themes? Or permissive throughout for adoption? *Needs a decision before the first public commit.*
2. **Final product/package name** after trademark, domain and crate checks.
3. **Default port** (8420 placeholder) and whether to expose the control socket path under `/run/moss`.
4. **Native-mode scope:** ship `moss-shim` in 1.0 or container-only first (if S1 shows container-only is dramatically simpler)?
5. **CurseForge key model:** user-supplied only, or an optional project-registered key (and what the terms allow for an open-source tool)?
6. **TOTP/MFA** in 1.0 blocker vs. should.
7. **Spigot:** support via BuildTools at all, or document as "use Paper"?
8. **Metrics store:** dedicated SQLite file vs. rolling files.
9. **Hosted docs/marketing site** and who maintains it.

---

## 31. Deferred expansion

### 31.1 Candidates

Multi-node management (agent pairing, capacity-aware placement, node-to-node migration) · additional games via maintained adapter packages · other-panel importers · community extension catalogue (signing, review, revocation first) · backend extensions and SDK · external backup targets (SFTP/object storage; credentials in the secret store; verified uploads; remote ≠ local completion; never delete the last good local copy because an upload was queued) · public status pages (separate surface, explicit field allowlist, fast disable) · profiler integrations · infrastructure-as-code export and declarative reconciliation · SSO/identity providers (local recovery retained) · maintenance-window-aware scheduling · **optional AI-assisted troubleshooting** · advanced player analytics.

### 31.2 Expansion gate

Before accepting any item, document: the user problem and audience; security implications; dependencies; ongoing maintenance cost; local/free deployment implications; migration path; test strategy; behaviour when its external service is unavailable. Don't add a feature merely because an upstream API makes it possible.

### 31.3 Constraints for the most-requested deferred areas

- **Multi-node:** mutual-TLS (or equivalent) pairing with short-lived single-use tokens, revocable; nodes receive only authorised scope; offline node ≠ stopped server; defined behaviour for partitions, duplicates, clock drift, upgrades; never auto-move a live server without an approved migration plan (source, destination, space, ports, downtime, files, backups, rollback). Capacity values are labelled measured / reserved / estimated / configured. The `node_id` is already in the schema (D23).
- **AI troubleshooting:** optional, proposal-only layer over *structured deterministic diagnostics*; separate facts, hypotheses and actions; external transmission opt-in per report with exact redacted payload preview and disclosed provider; never world data or full logs by default; no shell, no autonomous destructive change, no background upload; fully functional manual workflow when disabled.
- **Public status page:** explicit allowlist of low-sensitivity fields only; no private IPs/ports, console output, player identities or host metrics; independent access controls.
- **Read-only/team roles:** viewer role sees granted status/metrics/selected logs/backup health only; sensitive files, player data, secret-bearing config and audit require separate permissions; resource-scoped roles, invitations, ownership transfer, audit — SSO only after local recovery and account lifecycle are robust.

---

## Appendix A: Glossary

**Adapter** — code implementing a game/runtime or provider contract. **Backend (runtime)** — the mechanism that actually runs a process (container, shim). **Blueprint** — portable recipe to create a server. **Capability** — a declared, testable ability of an adapter. **Change plan** — reviewable set of intended mutations. **Desired/Observed state** — what should be vs what is. **Finding** — a persistent, evidence-backed problem. **Operation** — durable unit of work. **Shim** — `moss-shim`, the native-mode process supervisor. **Safe mode** — start with third-party extensions disabled.

## Appendix B: References

- Modrinth API — https://docs.modrinth.com/api/
- CurseForge REST API — https://docs.curseforge.com/rest-api/
- Mojang version manifest & PaperMC downloads API, Fabric Meta, Velocity — consult each project's current docs at implementation time
- Pterodactyl — https://pterodactyl.io/
- Tauri 2 — https://v2.tauri.app/
- Axum — https://docs.rs/axum/latest/axum/
- SQLite — https://www.sqlite.org/docs.html
- Docker Engine API / `bollard`; systemd transient units (`systemd-run`, `man systemd.scope`); cgroup v2 documentation
- OWASP: ASVS, Password Storage and Session Management cheat sheets

These are implementation aids, **not a substitute** for checking current provider terms, supported endpoints, advisories and compatibility before each release.

## Appendix C: What changed from the original plan

| Area | Change |
|---|---|
| Structure | 38 sections (with sections 31–38 restating §7, §13, §12, §3 etc.) merged into 31 non-overlapping sections; every requirement now has one home. Duplicate acceptance lists, duplicated differentiator descriptions and repeated "not auto-delete" statements consolidated into principles and one gate. |
| Identity | Named **Moss**; binary/crate/path/port/env conventions defined; trademark and "not affiliated" notice added. |
| Hard decisions | Added a numbered **ADR register** with concrete picks (Svelte 5, xterm.js, CodeMirror 6, uPlot, SSE+WS, `sqlx`, `bollard`, tar.zst backups, UUIDv7, envelope-encrypted secrets). |
| Biggest gap closed | Original promised "servers survive controller restarts" but never said how. Added **process-supervision architecture** (container reattach, `moss-shim` + systemd transient scope), orphan handling and a pass/fail **spike S1**. |
| Security | Added a **threat-model table**, concrete auth parameters, token format, permission vocabulary, split global roles vs per-server grants, docker-socket honesty, UI-integrity (XSS) rules, SSRF-safe shared fetcher. |
| Minecraft realism | Added **EULA acceptance flow**, Java-version mapping, JVM-flag policy, graceful-stop/readiness mechanics, Forge installer-in-sandbox, Spigot BuildTools caveat, CurseForge "no download URL" handling, active-world detection. |
| State machines | Added explicit **server lifecycle** and **operation** state machines, lock leases, outbox/cursor event model, failure-contract template and failure-injection list. |
| Roadmap | Ten phases → **seven milestones (M0–M6) with demo scripts**, spikes, version tags and sizes. Reordered: **backups before mods**, **Modrinth with the first plugin runtime**, CurseForge after a terms review, desktop late. |
| Measurability | Performance targets turned into **IDs with initial hypotheses** and required measurement methodology. |
| Scope control | Added P0/P1/P2 table, explicit deferrals (workflows, multi-node, AI, status pages, backend extensions), an expansion gate, and a **risk register** with likelihood/impact/owner. |
| Engineering | Concrete filesystem layout, repo crates, CI gates, fuzz targets, `FakeRuntime` and `moss dev sim-server` for fast development, runbooks including orphaned process and lost-owner access. |
| Open items | Added an **open-questions list** (licence, name, port, shim scope, CF key model, MFA, Spigot, metrics store) that must be resolved with owners. |
| Preserved | All original safety invariants: typed content, desired/observed separation, preview-before-destructive, honest support tiers, bounded resources, redaction, no arbitrary shell, optional cloud-free operation. |
