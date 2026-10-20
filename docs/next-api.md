# Moss 0.2 API additions (contract)

Source of truth for the work packages below. Backend and frontend are built in parallel against this file; when it is merged into `docs/api.md` this file is deleted. Conventions are those of `docs/api.md`: JSON, RFC 3339 UTC times, UUIDv7 ids, errors `{"error":{"code","message","request_id","fields"}}`, long jobs return `202 {"operation": Operation}`. Unknown fields must be ignored by clients. Permissions use the existing names (`server.view`, `server.power`, `files.write`, `content.manage`, `backup.manage`, `config.write`, `admin.settings`, `admin.users`).

## WP-A Mods and content (backend: src/content, src/providers, src/runtimes*, src/installer.rs, src/jars)

### Runtimes
`GET /runtimes` gains installable runtimes `neoforge`, `forge`, `quilt` (`content_kind:"mods"`, tier `stable` for neoforge/quilt, `experimental` for forge). Forge/NeoForge: modern installers only (Minecraft 1.17+ for Forge, 1.20.2+ for NeoForge); `/runtimes/{id}/versions` returns Minecraft versions each loader supports and `loader_versions:{ "<mc>": [{"id","recommended":bool}] }`. `POST /servers` accepts `loader_version` for these. Installers run the official installer jar in the server directory (env cleared, HOME = server dir, 15 min timeout, output into the operation log). Launch uses `@user_jvm_args.txt @libraries/.../unix_args.txt` for Forge/NeoForge; Moss owns and rewrites `user_jvm_args.txt` from memory/flags on every start.

### Inventory v2 — `GET /servers/{id}/content`
Each item gains:
```
enabled: boolean                        // false when the file is "<name>.jar.disabled"
identified: boolean                     // provenance known (installed by Moss, or matched by hash)
meta: { id, name, version, authors:string[], description, loader:"fabric"|"quilt"|"forge"|"neoforge"|"paper"|"spigot"|"bukkit"|"velocity"|"bungee"|"datapack"|"unknown",
        environment:"server"|"client"|"both"|"unknown", depends:[{id,required:boolean,range:string|null}], breaks:string[] } | null
issues: Finding[]                       // per-item findings from the check below (may be empty)
```
`source` may now be `"moss"|"manual"|"identified"|"modpack"`; `provider` stays `modrinth|curseforge|manual`.

### Actions
- `POST /servers/{id}/content/{item}/toggle {enabled}` → `{item}` (renames `.jar` ↔ `.jar.disabled`; refuses while the server is running only if the server needs the file locked: never on Linux/macOS, so allowed; response carries `requires_restart:true`). `item` may be `manual:<filename>`.
- `POST /servers/{id}/content/identify` → 202 operation. Hashes every jar/zip not yet known (SHA-512 against Modrinth `POST /version_files`, CurseForge fingerprints when available) and links matches as provider items (`source:"identified"`) so they can be updated. Result summary in the operation log.
- `GET /servers/{id}/content/check` → `{findings: Finding[], checked_at}` where `Finding = {id, severity:"info"|"warn"|"critical", item:string|null, title, detail, evidence:string[], action:{label,kind:"toggle"|"install"|"remove"|"update"|"open",item?,project_id?}|null}`. Rules: missing required dependency (jar metadata and provider dependency data; known built-ins `minecraft`, `java`, `fabricloader`, `fabric-api` satisfied appropriately), dependency version out of range (best-effort semver ranges for Fabric/Quilt, Maven ranges for Forge), duplicate mod/plugin id, declared incompatibility/`breaks` present, loader mismatch (a Fabric mod in a Forge server, `plugin.yml` in `mods/`), game-version mismatch against the jar's declared `minecraft`/`api-version`, client-only mod, unreadable or corrupt jar, duplicate file names differing only by case. Also run automatically: findings of severity warn/critical for a plan's files are returned in the plan's `warnings`, and `GET /servers/{id}/health` includes a `content` finding summary (`{id:"<server>:content", severity, title:"N content problems", action:{kind:"open_tab",tab:"content"}}`) when any warn/critical exist.
- `POST /servers/{id}/content/update-all/plan` → ChangePlan (all non-pinned updates, including identified manual items). `POST /servers/{id}/content/update-all` → 202 operation (one undo snapshot, one backup if *Back up before risky changes* is on, dependency-aware, rolls back files already replaced if a later download fails).
- Change plans (`/content/plan`) gain `conflicts: Finding[]` (items the plan would break) and per item `{...,"version":string,"from_version":string|null,"size":number}`; `warnings` stay strings.

### Version upgrade assistant
- `POST /servers/{id}/upgrade/plan {game_version, loader_version?}` → 
```
{ from:{game_version,loader_version}, to:{game_version,loader_version}, java:{from,to,installed:boolean},
  server_jar:{available:boolean, note:string|null},
  content:[{item,title,status:"ok"|"update"|"missing"|"unknown", current:string|null, target:string|null, version_id:string|null, reason:string|null}],
  blockers:string[], warnings:string[], can_proceed:boolean }
```
`missing` = no build for the target version; `unknown` = manual jar not identifiable.
- `POST /servers/{id}/upgrade {game_version, loader_version?, content:"update"|"disable_incompatible"|"keep", backup:true}` → 202 operation: stops nothing (server must be stopped, else 409), pre-change backup, replaces `server.jar`/loader files, updates `java_major`, applies the content choice, records `game_version`, timeline kind `upgrade`. The operation log names the backup id for rollback. Downgrades require `confirm_downgrade:true`.

### Modpacks
- `POST /modpacks/inspect` multipart (`file` = `.mrpack` or CurseForge pack `.zip`, ≤ 512 MiB) **or** JSON `{provider:"modrinth"|"curseforge", project_id, version_id?}` → `{token, format:"mrpack"|"curseforge", name, version, summary|null, game_version, loader:{id,version|null}, files:{total, server, client_only, unavailable}, unavailable:[{name,reason}], overrides:{files, size}, size:number, warnings:string[]}`. `token` is valid 30 minutes and names a temp file under `tmp/`. Unavailable = CurseForge files whose authors disabled third-party downloads (listed with their project page for manual download).
- `POST /servers {..., "modpack_token": "<token>"}` creates the server with the pack's runtime, game version and loader, then imports it (same operation chain). `runtime` must match or be omitted.
- `POST /servers/{id}/modpacks {token, mode:"add"|"replace"}` → 202 operation: downloads hash-verified files, extracts `overrides/` and `server-overrides/` (path-safe), installs server-side mods only, records provenance (`source:"modpack"`), pre-change backup if the setting is on.
- `GET /servers/{id}/modpacks/export` → `.mrpack` download of this server's Modrinth-sourced content (with hashes) plus config overrides (`config/`, `defaultconfigs/`, `kubejs/` when present; never `world`, `logs`, `.moss`).

## WP-B Platform, lifecycle, release (backend: src/platform, src/lifecycle, src/java.rs, packaging, workflows, scripts)

### Resource limits
Server object gains `limits:{memory_mb:number|null, cpu_pct:number|null, pids:number|null, enforced:boolean, backend:"cgroup2"|"none", detail:string|null}`. `POST /servers` and `PATCH /servers/{id}` accept `memory_limit_mb`, `cpu_limit_pct` (10–`100*cores`), `pids_limit` (64–32768); `null` clears. Default memory hard cap when unset: heap + 25% + 512 MiB (only applied when enforceable). Enforced with cgroup v2 (a delegated subtree: the shipped unit sets `Delegate=yes`): Moss creates a child cgroup per server, writes `memory.max`, `cpu.max`, `pids.max`, moves the launched process group in. OOM kills are detected (`memory.events`) and surfaced as crash detail "killed by the memory limit". On macOS and where cgroups are unavailable: `backend:"none"`, `detail` explains, and a health finding of severity info appears when limits were requested. Memory metrics come from the cgroup when present.

### Actions
- `POST /servers/{id}/actions/reinstall {keep_data:boolean}` → 202 `{operation}`. Allowed when the install failed or the server is stopped; `keep_data:true` only re-downloads the server jar/loader files.
- `DELETE /java/{major}` → 204. Managed Java only; 409 `java.in_use` naming servers that need it. `POST /java/install` returns 409 `java.in_use` if any running server uses that major (install into a temp dir, atomic swap only when none run).
- `GET /status` host gains `os: "Linux"|"macOS"`.

### Release
Tags `v*` build and attach: `moss-<tag>-linux-x86_64.tar.gz`, `moss-<tag>-macos-aarch64.tar.gz`, `moss-<tag>-macos-x86_64.tar.gz`, `SHA256SUMS.txt`, `install.sh`, `moss.rb` (Homebrew formula with the real checksums); build attestations for each tarball.

## WP-C Data and operations (backend: src/sched.rs, src/backups, src/notify, src/history.rs, src/ops.rs, src/console.rs, src/api/activity.rs, src/state.rs audit)

### Schedules with time zones
Schedule gains `tz: string` (IANA name, e.g. `"Europe/Berlin"`, default `"UTC"`). `tz_offset_min` stays in responses (current offset) and is still accepted on create/patch for old clients (stored as a fixed-offset zone `"UTC+HH:MM"`). `daily_at` follows local wall-clock across DST changes (a skipped hour runs at the next valid instant, a repeated hour runs once). `GET /timezones` → `{zones:string[]}` (IANA names).

### Backups
- Setting `backup_before_change` (existing) is now honoured: content install/update/update-all, modpack import, version upgrade, in-place restore and world file edits via Moss take a `pre-change` backup first when true and the server has data; pre-change backups are pruned to the newest 3 per server.
- `PATCH /backups/{id} {locked?:boolean, note?:string}` → Backup. Locked backups are never pruned or deleted (`DELETE` → 409 `backup.locked`).
- Backup objects gain `worlds:string[]` (directories containing `level.dat`), `locked:boolean`.
- `POST /backups/{id}/preview {mode:"new_server"|"in_place", only?:string[]}` → `{mode, archive:{files,size,created_at}, current:{files,size}|null, added:number, removed:number, changed:number, unchanged:number, size_delta:number, samples:{added:string[],removed:string[],changed:string[]} /* ≤50 each */, worlds:string[], warnings:string[]}`. "changed" = size or modification time differs. Streams the archive once; never extracts.
- `POST /backups/{id}/restore` body gains `only?:string[]` (top-level names such as `["world","world_nether"]`; in-place only): restores just those entries, taking the safety backup first.

### Notifications and webhooks
Notification: `{id, time, kind:"crash"|"crash_loop"|"backup_failed"|"disk_low"|"operation_failed"|"restart_failed"|"update_available"|"security", severity:"info"|"warn"|"critical", title, body, server_id:string|null, read:boolean}`.
- `GET /notifications?unread=1&limit=50` → `{items:Notification[], unread:number}` (scoped to servers the caller can see; admin-only kinds hidden from others).
- `POST /notifications/read {ids?:string[], all?:boolean}` → 204.
- SSE event `notification` (data = Notification) on `/events`.
- Settings gain `notify_webhook_url:string`, `notify_events:string[]` (kinds to deliver; default crash, crash_loop, backup_failed, disk_low, restart_failed), `audit_console_commands:"redacted"|"all"|"off"` (default `redacted`), `access_message:string` (≤500 chars, shown on the not-invited screen).
- `POST /notifications/test` (admin) → `{delivered:boolean, detail:string}` sends a test message to the webhook. Delivery: HTTPS only, no redirects, DNS-resolved address must be public unless `MOSS_ALLOW_PRIVATE_WEBHOOKS=true`, 3 attempts with backoff, Discord webhooks (`discord.com/api/webhooks/…`) get an embed payload, anything else gets `{"event","severity","title","body","server","time","moss":"<version>"}`.

### Long-term metrics
- `GET /servers/{id}/metrics?range=1h|24h|7d|30d` → `{range, step_s, samples:[{t, cpu_avg, cpu_max, mem_avg, mem_max, players_max}]}` (1 h = raw 5 s samples from memory; longer ranges from hourly roll-ups kept 90 days in SQLite).
- `GET /metrics/host?range=…` (admin) → same shape with `cpu_avg/cpu_max/mem_avg/mem_max/disk_free`.

### Events
SSE sends `event: resync` (data `{}`) when a subscriber lagged and events were dropped; clients must refetch lists. `hello` data becomes `{server_time}`.

### Support bundle
`GET /support-bundle?preview=1` (admin) → `{files:[{name,size}], redactions:string[]}`; without `preview` → `application/zip` with `moss.txt` (version, OS, doctor output), `config.json` (secrets redacted), `servers.json`, `health.json`, `activity.json` (last 500), `operations.json`, and the last 300 console lines per server. Tokens, JWTs, API keys and webhook URLs are replaced by `[redacted]`; e-mail addresses are hashed.

## WP-D Auth and access (backend: src/auth, src/api/session.rs, src/api/access.rs)

### Owner claim with a setup token
There is no more "first verified account becomes owner". When `MOSS_OWNER_EMAILS` is empty and no owner exists, Moss writes a random token to `<data>/setup-token` (mode 0600) and logs where it is. `GET /auth/config` gains `setup_required:boolean`. A verified user who is not the owner or invited gets 403 `auth.setup_required` while setup is pending. `POST /setup/claim {token}` (bearer required) → `{role:"owner"}`; constant-time comparison, 5 attempts per minute per user, the file is deleted on success. Owner emails in `MOSS_OWNER_EMAILS` still become owner on first verified sign-in.

### MFA policy
Env `MOSS_REQUIRE_MFA=off|admins|all` (default `off`). When required and the token's `aal` claim is not `aal2`, protected endpoints answer 403 `auth.mfa_required` (all except `GET /auth/config` and `GET /me`). `GET /me` gains `mfa:{required:boolean, satisfied:boolean}`. Enrolment and verification run against Supabase (`/auth/v1/factors`, `…/challenge`, `…/verify`) from the browser; Moss only enforces.

### API tokens
- `POST /tokens {name, scope:"read"|"full", expires_in_days?:number}` → `{id, token:"moss_pat_…", name, scope, expires_at}` (token shown once; requires an interactive session, not another token).
- `GET /tokens` → `[{id, name, scope, prefix, created_at, last_used, expires_at}]` (own tokens; admins may pass `?all=1`). `DELETE /tokens/{id}`.
- `Authorization: Bearer moss_pat_…` authenticates as the owning user with their current role and grants. `read` allows `GET` only. Tokens cannot manage tokens or users. Stored as SHA-256; `last_used` updated at most once a minute. Audit entries read `<email> (token <name>)`.

### Throttling
Repeated invalid credentials from one address (30 a minute) answer 429 `auth.throttled`. The client address comes from `X-Forwarded-For` only when `MOSS_TRUST_PROXY=true`.

### Not-invited experience
`GET /auth/config` gains `access_message:string` (from the setting) so the login screen can tell people whom to ask.

## Shared database changes
Each package adds its own numbered migration file (`src/db/migrations/NNNN_name.sql`) and one line in `MIGRATIONS`: 0002 limits+tz+locks+notifications+metrics (WP-B/WP-C, split into 0002_limits and 0003_ops), 0004 content (WP-A), 0005 tokens (WP-D). Pick the next free number when you merge.
