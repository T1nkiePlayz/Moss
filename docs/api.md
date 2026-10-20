# Moss HTTP API (v1)

Base path `/api/v1`. JSON everywhere. Times are RFC 3339 UTC strings. IDs are UUIDv7 strings.
Auth: `Authorization: Bearer <Supabase access token>` (or no auth in `dev` mode, loopback only).
Errors: HTTP status + `{"error":{"code":"server.locked","message":"…","request_id":"…","fields":[{"path":"memory_mb","message":"…"}]}}`.
Streams (SSE events, WebSocket console) cannot send headers, so first `POST /ticket` → `{"ticket":"…"}` (single use, 30 s) and pass `?ticket=`.
The server may add fields to any object; clients must ignore unknown fields.

## Public
- `GET /auth/config` → `{mode:"supabase"|"dev", supabase_url, supabase_anon_key, owner_claimed:boolean}`
- `GET /healthz` (outside `/api/v1`) → `ok`

## Session
- `GET /me` → `{id,email,role:"owner"|"admin"|"user",permissions:string[]}`; 401 unauthenticated; 403 `auth.not_provisioned` if signed in but not invited.
- `POST /ticket` → `{ticket}`
- `GET /status` → `{version,uptime_s,host:{cpu_pct,mem_total,mem_used,disk_total,disk_free,load1,hostname},counts:{servers,running,players},auth_mode}`

## Runtimes & Java
- `GET /runtimes` → `[{id,name,category:"official"|"plugins"|"modded"|"proxy"|"custom",tier:"stable"|"experimental"|"planned",content_kind:"plugins"|"mods"|"proxy_plugins"|"datapacks"|"none",description,installable:boolean}]`
  ids: `vanilla paper purpur folia fabric velocity custom neoforge(planned) forge(planned)`
- `GET /runtimes/{id}/versions` → `{versions:[{id,stable:boolean}],recommended:string}`
- `GET /java` → `{installs:[{major,path,managed:boolean,version}]}`
- `POST /java/install {major}` → 202 `{operation}`

## Servers
Server object:
```
{id,name,runtime,game_version,loader_version|null,java_major,port,memory_mb,flags_profile:"balanced"|"minimal",
 autostart,restart_policy:{enabled,max_per_10m},eula_accepted,desired:"running"|"stopped",
 state:"installing"|"stopped"|"starting"|"running"|"stopping"|"crashed"|"failed"|"needs_attention",
 state_detail|null,state_since,uptime_s|null,pid|null,players:{online,max,sample:string[]}|null,motd|null,
 cpu_pct|null,mem_bytes|null,content_kind,tags:string[],notes,created_at,
 capabilities:string[] /* console,files,content,backups,players,properties,schedules */}
```
- `GET /servers` → `Server[]`
- `POST /servers {name,runtime,game_version,loader_version?,memory_mb,port?,accept_eula:boolean,flags_profile?,autostart?,custom?:{command:string[]}}` → 202 `{server,operation}`. `accept_eula` must be true for Minecraft runtimes (recorded + audited).
- `GET /servers/{id}` · `PATCH /servers/{id} {name?,memory_mb?,port?,flags_profile?,autostart?,restart_policy?,tags?,notes?,game_version?}` · `DELETE /servers/{id}?delete_data=false`
- `POST /servers/{id}/actions/{start|stop|restart|kill}` → `{server}` (start/stop are asynchronous; watch `state`)
- `POST /servers/{id}/command {command}` → 204
- `GET /servers/{id}/stats` → `{samples:[{t:number(unix ms),cpu:number,mem:number,players:number|null}]}` (≈1 h at 5 s)
- `GET /servers/{id}/timeline?limit=` → `[{time,kind:"start"|"ready"|"stop"|"crash"|"restart"|"backup"|"install"|"update"|"undo",detail}]`
- `GET /servers/{id}/health` → `Finding[]` (see Health)
- `GET /servers/{id}/properties` → `{raw,entries:[{key,value}]}` · `PUT /servers/{id}/properties {raw}` → `{undo_id}` (auto snapshot)

### Console
- `GET /servers/{id}/console/backlog?lines=500` → `{lines:string[]}`
- WS `/servers/{id}/console?ticket=` server→client `{"t":"line","d":"…"}`, `{"t":"state","d":"running"}`; client→server `{"t":"cmd","d":"say hi"}`. Lines may contain ANSI escapes.

### Files (all paths are relative to the server root; `..`/symlink escapes → 400 `path.escape`)
- `GET /servers/{id}/files?path=` → `{path,entries:[{name,kind:"dir"|"file"|"symlink",size,modified}]}` (dirs first)
- `GET /servers/{id}/files/content?path=` → `{content,size,truncated,binary}` (text ≤ 2 MiB)
- `PUT /servers/{id}/files/content {path,content}` → `{undo_id}` · `POST …/files/mkdir {path}` · `POST …/files/rename {from,to}` · `DELETE …/files?path=` · `POST …/files/upload?path=` (multipart field `file`, repeatable) · `GET …/files/download?path=`

## Content (Modrinth and CurseForge; typed by runtime)
- `GET /content/providers?server_id=` → `{providers:[{id:"modrinth"|"curseforge",name,available,supported,reason|null}]}`. `available` = configured on this instance, `supported` = has content for the server's runtime. See [curseforge.md](curseforge.md).
- `search`, `projects/{id}`, `plan` and install take an optional `provider` (`modrinth` default, `curseforge`). CurseForge hits have null `client_side`/`server_side`; versions may carry `downloadable:false` and a `warning`. A CurseForge rate limit answers 429 `provider.rate_limited`.
- `GET /content/search?server_id=&q=&offset=0&limit=20&sort=relevance|downloads|updated|follows&category=` → `{hits:[{project_id,slug,title,description,author,icon_url,downloads,follows,categories:string[],updated,client_side,server_side}],total,compat:{kind,loader,game_version}}` (client-only projects are filtered out; `compat.kind` names the tab: plugins/mods/proxy_plugins/datapacks)
- `GET /content/projects/{project_id}?server_id=` → `{project_id,title,description,body,icon_url,links:{source,issues,wiki,page},license,gallery:string[],versions:[{id,name,version_number,channel:"release"|"beta"|"alpha",date,downloads,files:[{filename,size,primary}],game_versions,loaders,dependencies:[{project_id,kind:"required"|"optional"|"incompatible"}]}]}` (versions already filtered to the server's loader + game version)
- `GET /servers/{id}/content` → `{items:[{id,provider:"modrinth"|"manual",project_id|null,version_id|null,title,version_number|null,filename,kind,sha512|null,installed_at,pinned,source:"moss"|"manual"}]}` (manually-dropped jars appear as `manual`)
- `POST /servers/{id}/content/plan {project_id,version_id?}` → change plan `{items:[{action:"install"|"replace"|"remove",title,filename,size,reason}],warnings:string[],requires_restart:boolean}`
- `POST /servers/{id}/content {project_id,version_id?}` → 202 `{operation}` (backs up touched files for Undo; installs required dependencies)
- `DELETE /servers/{id}/content/{item_id}` → 202 `{operation}` (plugin/mod data dirs are kept)
- `POST /servers/{id}/content/check-updates` → `{updates:[{item_id,title,current,latest,version_id}]}` · `POST /servers/{id}/content/{item_id}/update` → 202 `{operation}` · `PATCH …/content/{item_id} {pinned}`

## Backups
Backup: `{id,server_id,created_at,size,sha256,status:"running"|"ok"|"failed"|"verifying",verified_at|null,note,kind:"manual"|"scheduled"|"pre-change",consistent:boolean,files}`
- `GET /servers/{id}/backups` · `POST /servers/{id}/backups {note?}` → 202 `{operation}` · `POST /backups/{id}/verify` → 202 · `POST /backups/{id}/restore {mode:"new_server"|"in_place"}` → 202 (`in_place` requires the server stopped; a pre-restore backup is taken) · `DELETE /backups/{id}` · `GET /backups/{id}/download`
- `GET /backups` → all backups (cross-server)

## Schedules
Schedule: `{id,server_id,kind:"backup"|"restart"|"start"|"stop"|"command",command|null,every_minutes|null,daily_at|null("HH:MM"),tz_offset_min,enabled,keep|null,last_run|null,next_run|null,last_status|null}`
- `GET /servers/{id}/schedules` · `POST /servers/{id}/schedules {…}` · `PATCH /schedules/{id}` · `DELETE /schedules/{id}` · `POST /schedules/{id}/run`

## Operations, Undo, Activity
Operation: `{id,kind,title,server_id|null,state:"queued"|"running"|"completed"|"failed"|"cancelled"|"interrupted",stage,progress:number|null /*0..1*/,log:string[] /*last 50*/,error|null,created_at,started_at|null,ended_at|null,cancellable}`
- `GET /operations?limit=50&server_id=` · `GET /operations/{id}` · `POST /operations/{id}/cancel`
- `GET /servers/{id}/undo` → `[{id,title,created_at,files:string[]}]` · `POST /undo/{id}/revert` → `{ok:true}`
- `GET /activity?limit=100&server_id=` → `[{id,time,actor,action,target,detail}]`

## Health
Finding: `{id,severity:"info"|"warn"|"critical",server_id|null,title,detail,evidence:string[],action:{label,kind:"start"|"backup"|"open_tab"|"accept_eula"|"install_java",tab?}|null}`
- `GET /health` → `Finding[]` (host + all servers). Rules: crash loop, no recent backup, low disk, missing Java, EULA not accepted, port conflict, readiness timeout, orphaned process.

## Access (owner/admin)
- `GET /users` · `POST /users {email,role}` · `PATCH /users/{id} {role}` · `DELETE /users/{id}`
- `GET /servers/{id}/access` → `[{user_id,email,bundle:"manager"|"operator"|"viewer"}]` · `PUT /servers/{id}/access {grants:[{user_id,bundle}]}`
- Permissions: `server.view server.power server.console.read server.console.write files.read files.write content.manage backup.manage config.write server.delete admin.users admin.settings`

## Settings
- `GET /settings` → `{default_memory_mb,backup_keep,backup_before_change:boolean,modrinth_contact,public_base_url,data_dir,bind,version}` · `PUT /settings` (admin)

## Events (SSE) `GET /events?ticket=`
`event: op` data=Operation · `event: server` data=Server · `event: activity` data=Activity · `event: hello` data `{}`. Stream resumes with `Last-Event-ID` (best effort) and sends `:` keep-alives every 15 s.

## Additions
- `PATCH /servers/{id} {accept_eula:true}` records Minecraft EULA acceptance (audited; writes `eula.txt`).
- Health action `install_java` carries `major`; call `POST /java/install {major}`.

## 0.2 additions: mods and content

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

## 0.2 additions: platform, limits, release

### Resource limits
Server object gains `limits:{memory_mb:number|null, cpu_pct:number|null, pids:number|null, enforced:boolean, backend:"cgroup2"|"none", detail:string|null}`. `POST /servers` and `PATCH /servers/{id}` accept `memory_limit_mb`, `cpu_limit_pct` (10–`100*cores`), `pids_limit` (64–32768); `null` clears. Default memory hard cap when unset: heap + 25% + 512 MiB (only applied when enforceable). Enforced with cgroup v2 (a delegated subtree: the shipped unit sets `Delegate=yes`): Moss creates a child cgroup per server, writes `memory.max`, `cpu.max`, `pids.max`, moves the launched process group in. OOM kills are detected (`memory.events`) and surfaced as crash detail "killed by the memory limit". On macOS and where cgroups are unavailable: `backend:"none"`, `detail` explains, and a health finding of severity info appears when limits were requested. Memory metrics come from the cgroup when present.

### Actions
- `POST /servers/{id}/actions/reinstall {keep_data:boolean}` → 202 `{operation}`. Allowed when the install failed or the server is stopped; `keep_data:true` only re-downloads the server jar/loader files.
- `DELETE /java/{major}` → 204. Managed Java only; 409 `java.in_use` naming servers that need it. `POST /java/install` returns 409 `java.in_use` if any running server uses that major (install into a temp dir, atomic swap only when none run).
- `GET /status` host gains `os: "Linux"|"macOS"`.

### Release
Tags `v*` build and attach: `moss-<tag>-linux-x86_64.tar.gz`, `moss-<tag>-macos-aarch64.tar.gz`, `moss-<tag>-macos-x86_64.tar.gz`, `SHA256SUMS.txt`, `install.sh`, `moss.rb` (Homebrew formula with the real checksums); build attestations for each tarball.

## 0.2 additions: schedules, backups, notifications, metrics

### Schedules with time zones
Schedule gains `tz: string` (IANA name, e.g. `"Europe/Berlin"`, default `"UTC"`). `tz_offset_min` stays in responses (current offset) and is still accepted on create/patch for old clients (stored as a fixed-offset zone `"UTC+HH:MM"`). `daily_at` follows local wall-clock across DST changes (a skipped hour runs at the next valid instant, a repeated hour runs once). `GET /timezones` → `{zones:string[]}` (IANA names).

### Backups
- Setting `backup_before_change` (existing) is now honoured: content install/update/update-all, modpack import, version upgrade, and in-place restore (always) take a `pre-change` backup first when true and the server has data. Plain file and `server.properties` edits use per-file undo snapshots instead, because a full backup per save would be absurd; pre-change backups are pruned to the newest 3 per server.
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

## 0.2 additions: auth and access

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

## Container isolation

Optional per-server mode. Server object gains `isolation:"process"|"container"` (default `process`, unchanged behaviour) and `container:{image, image_override|null, running_in_container:boolean, restart_needed:boolean}`. `image` is the effective image (`docker.io/library/eclipse-temurin:<java>-jre` unless overridden). `restart_needed` is true while a running server was launched in the other mode than the one now configured.

- `GET /container/status` (any signed-in user) → `{available:boolean, engine:"docker"|"podman"|null, version|null, rootless:boolean|null, message|null, outbound_restricted:false, game_only_supported:boolean, default_image}`. Never an error: when neither Docker nor Podman is usable `available` is false and `message` says what to do. Calling it forces a fresh detection.
- `PATCH /servers/{id}` accepts `isolation:"process"|"container"` and `container_image:string|null` (null = default image). **Admin only** (403 otherwise). 400 `validation` for an unknown mode or an image outside the allow-list (`docker.io/library/eclipse-temurin`, `docker.io/azul/zulu-openjdk`, `docker.io/library/amazoncorretto`, anything under `ghcr.io/`; a tag or sha256 digest is required; references are normalised to a fully qualified form); 400 `container.unsupported` for custom servers; 409 `container.engine_missing` when enabling without a usable engine. The change applies on the next start. Enabling it (or changing the image) starts a `container.pull` operation so the image is ready.
- `PATCH /servers/{id}` also accepts `network:"open"|"game-only"` and `extra_ports:[{port,proto:"tcp"|"udp"}]` (admin only; the server object's `container` gains `network` and `extra_ports`). `game-only` runs the container on Moss's own `moss-gameonly` bridge network (created on demand, IP masquerading off) with no usable DNS: the published game port keeps working, but the server cannot open outbound connections to the internet. It needs container isolation (409 `container.not_enabled`), a usable engine and a network that can be created (409 `container.network_failed`; Podman is refused with an explanation, there is never a silent fallback to `open`). `extra_ports` publishes additional ports (plugins such as voice chat or Geyser): 1024-65535, at most 8, no duplicates, not another Moss server's game port or extra port; 400 `validation` otherwise. Both apply on the next start. Starting a `game-only` server whose network cannot be set up fails with 409 `container.network_failed`.
- `POST /servers/{id}/container/pull` (admin) → 202 `{operation}` (kind `container.pull`, cancellable, log shows pull progress). 409 `container.not_enabled` / `container.engine_missing`.
- `POST /servers/{id}/actions/start` in container mode can return 409 `container.image_missing` (download it first), `container.engine_missing` or `container.start_failed` (message carries the engine's last log line).
- Health: finding `<server>:network` (critical) while a server is set to `game-only` and the network cannot be created. Finding `<server>:container` (critical, "Container engine not available") while a server is set to container mode and no engine is reachable. `moss doctor` prints the detected engine.
- There is no parameter anywhere for flags, bind mounts, devices, capabilities, networks or privileged mode.
### Staged updates ("try on a copy first")
A test copy is a hidden server on a free port (`MOSS_STAGING_PORT_RANGE`, default `30000-30999`) cloned from the origin: files are copied (never linked), the world only with `include_world`, rcon/query are switched off, autostart and auto-restart are off. Copies are not in `GET /servers` unless `?include_staging=true` (each then has `staging_of`), cannot be patched, started or deleted directly, and are removed after 24 hours, on discard, when the origin is deleted and on controller restart. All routes need `content.manage` on the origin (`upgrade` also needs `config.write`).
- `GET /servers/{id}/staging/estimate` → `{files, copy_bytes, world_bytes, free_bytes, port_range:[lo,hi], defaults:{ready_timeout_s,watch_s}, kinds}`
- `POST /servers/{id}/staging {kind:"none"|"update_all"|"upgrade"|"pack_update", include_world?, ready_timeout_s? (10-900, 180), watch_s? (0-300, 30), upgrade?:{...as the upgrade assistant target}, pack?:{install_id, token}}` → 202 `{run, operation}`. `pack.token` is a token from `POST …/modpacks/{install}/check-update`; its file is kept for the run. One active run per server (409 `staging.busy`); 409 `disk.low`, `staging.no_port`, `modpack.pinned`.
- `GET /servers/{id}/staging` · `GET …/staging/{run}` → `{id, server_id, kind, state:"building"|"testing"|"healthy"|"unhealthy"|"failed", include_world, request, report, port, operation_id, created_at, expires_at, expired, can_promote}`. `report`: `{healthy, reasons[], ready_secs, errors, warnings, samples[], fatal, crashed, changes[{title,action,to}], signature, notes[], copy:{files,bytes,world}, port, tested_at}`. Events: SSE `staging`.
- `GET …/staging/{run}/log?lines=` → `{lines[], truncated}` · `DELETE …/staging/{run}` discards (202 `cancelling` while building).
- `POST …/staging/{run}/promote` → 202 `{operation}`: applies the same plan to the real server with the pre-change backup and rollback, then discards the copy. 409 `staging.not_healthy`, `staging.expired`, `staging.nothing_to_promote` (kind `none`), `server.running` (upgrade needs the server stopped). The plan is re-checked against its tested signature and refused if anything moved.
- Health: ready within the timeout, still running after the watch period, no crash and no fatal log line (OutOfMemoryError, missing class, mod load failure, ...). Error and warning counts are reported but only fatal lines fail a copy.

### Modpack versions and pinning
- `GET /servers/{id}/modpacks` → `{packs:[{id,name,version,format,game_version,loader,mode,installed_at,files,pinned,items,override_files,source:{provider,project_id,version_id}|null,can_check}]}` (`server.view`).
- `PATCH /servers/{id}/modpacks/{install} {pinned}` keeps a pack at its version: it is not checked or updated, and update-all skips its items (items also carry `pack_pinned`). Individual items are pinned with `PATCH …/content/{item} {pinned}`.
- `POST /servers/{id}/modpacks/{install}/check-update {token?}` → `{pinned, up_to_date, current, latest:{version_id,version}, token, diff}`; without `token` the provider is asked for the newest version (409 `modpack.no_source` for file imports; send the newer pack's inspect `token` instead). `diff`: `{versions:{from,to}, files:{added,removed,updated,held,unchanged}, overrides:{added,updated,user_modified:[{path,reason}],removed_upstream,unchanged}, blockers[], warnings[], can_apply, changes, signature}`. Files are matched by provider project; pinned or disabled items are `held`. Override files are compared with the checksum recorded at install: files the person edited are listed in `user_modified` and never overwritten. A different runtime, game version or loader version is a blocker (use the upgrade assistant).
- `POST /servers/{id}/modpacks/{install}/update {token}` → 202 `{operation}`: pre-change backup (if enabled), downloads verified, one undo snapshot, journaled file moves that are rolled back on failure.
- `check-updates` (`POST …/content/check-updates`) also returns `warnings:string[]` (a provider that could not be reached does not hide the other's results).
- Also: `MOSS_MODRINTH_API_URL` overrides the Modrinth API base (tests and mirrors).

## 0.2 additions: configuration editors, personal data, blueprints, import, search

### Configuration files
Structured editors for `paper-global.yml`, `paper-world-defaults.yml`, per-world `paper-world.yml`, `spigot.yml`, `bukkit.yml`, `purpur.yml` (Purpur only) and `velocity.toml`. Edits change single lines of the text, so comments and unknown keys survive. Every change takes an undo snapshot (revert with `POST /undo/{id}/revert`).
- `GET /servers/{id}/config` -> `{files:[{id,title,path,format:"yaml"|"toml",exists}]}` (`files.read`). Ids: `paper-global`, `paper-world-defaults`, `world--<name>`, `spigot`, `bukkit`, `purpur`, `velocity`.
- `GET /servers/{id}/config/{file}` -> `{id,title,path,format,exists,raw,parse_error|null,fields:[{key,type:"bool"|"int"|"float"|"string"|"enum",default,group,description,min,max,options,restart,secret,danger,mixed,present,value}],history:[{id,title,created_at}]}`. Fields are the curated catalogue for the server's Minecraft version; `secret` values are never returned.
- `PUT /servers/{id}/config/{file}` (`config.write`) with exactly one of `{raw}` (whole text, must parse) or `{set:{"dotted.key":value},unset:["dotted.key"]}` (catalogue keys only, validated for type, range and options) -> `{undo_id|null,changed}`. Errors: 400 `config.invalid` / `config.unknown_key`, 404 unknown file id, 409 `config.missing` (file not created yet).
- `GET /servers/{id}/config/{file}/diff?snapshot=` -> `{current,previous|null,snapshot:{id,title,created_at}|null}`; defaults to the latest snapshot; only snapshots of that file are accepted.

### Preferences and saved views (per user)
- `GET /me/preferences` -> `{data:object,updated_at|null}` · `PUT /me/preferences {data:object}` (at most 32 KB). Used to sync the theme between browsers.
- `GET /views?scope=servers|content` -> `[{id,scope,name,filters:object,sort,created_at}]` · `POST /views {scope,name,filters?,sort?}` -> 201 (same name replaces; 50 per list, else 409 `views.limit`) · `DELETE /views/{id}` -> 204. Always limited to the caller's own views.

### Blueprints (admin)
A blueprint holds runtime, versions, memory, flags, `server.properties` (without port, world and secret-looking keys), the content list (provider and project id only) and schedules. Never worlds, player data or secrets.
- `GET /blueprints` -> `[{id,name,runtime,game_version,loader_version,memory_mb,properties:n,content:n,schedules:n,created_by,created_at}]`
- `POST /blueprints {server_id,name}` -> 201 summary · `POST /blueprints/import` (body: the exported JSON, at most 256 KB) -> 201 summary · `GET /blueprints/{id}/export` -> the JSON document (`{format:"moss-blueprint",version:1,...}`) · `DELETE /blueprints/{id}` -> 204
- `POST /blueprints/{id}/create {name,port?,accept_eula}` -> 202 `{server,operation,blueprint:{id,skipped_schedules}}`; the install operation then applies the properties and installs the content.

### Import an existing server (admin)
- `POST /servers/import/inspect` with JSON `{path}` (absolute host path; never inside system folders or Moss's data folder) or multipart field `file` (`.zip` or `.tar.zst`, up to the 512 MiB request limit) -> `{runtime|null,game_version|null,loader_version|null,port|null,jar|null,has_world,java_major|null,java_installed,name,size,notes:string[],token?}`. `token` (uploads only) is valid for 6 hours and can be used once.
- `POST /servers/import {name,path|token,runtime?,game_version?,loader_version?,memory_mb?,port?,accept_eula}` -> 202 `{server,operation}`. Detected values are used unless overridden. A host path is copied (symbolic links are skipped, the original is untouched); an upload is moved into place. A failed import removes everything it created. Errors: 400 `import.path`, `import.archive`, `import.token`, `eula.required`, 409 `port.taken`.

### Search
- `GET /search?q=&limit=6` -> `{q,servers:[{id,name,runtime,game_version}],backups:[{id,server_id,server_name,created_at,note,kind}],content:[{id,server_id,server_name,title,filename,kind}],files:[{server_id,server_name,path}]}`. Needs at least 2 characters (shorter returns empty groups). Results only include servers the caller can see; file names only where they have `files.read`.

## Implementation notes and deviations
- **Mods:** `/content/check` judges installed files by their own metadata; provider dependency data is enforced when a plan is built. Datapack `pack_format` is not checked. `meta` string fields are `""` when unknown. Update-all skips disabled items. `check-updates` entries also carry `project_id` and `provider`. Modpack `unavailable` entries may carry a `url` to the project page. `GET …/modpacks/export` sets `x-moss-skipped` (items left out). `POST /modpacks/inspect` needs an admin or a `manager` grant. Replace-mode modpack import returns 409 while the server runs. Items from a pack report `source:"modpack"` through the `modpack_items` table.
- **Platform:** the wrapper shell moves itself into the server's cgroup before the game starts, so there is no unconfined window. Limits take effect at the next start. Under systemd they need version 254 or newer (`DelegateSubgroup`); otherwise they are reported as not enforced. `reinstall` defaults to `keep_data:true` (keeps `server.properties` and `eula.txt`); `false` wipes everything except `.moss`.
- **Data and ops:** `notify_events` selects webhook deliveries only; in-app notifications are always created. `POST /notifications/test` makes one attempt. Metrics `t` is epoch milliseconds. Webhooks are strictly HTTPS, even with `MOSS_ALLOW_PRIVATE_WEBHOOKS=true`. Selective restore keeps replaced entries under `data/removed/<server>-pre-restore-<time>/`.
- **Auth:** `auth.mfa_required` carries `aal` and `required` next to `code` in `error`. Extra codes: `setup.not_pending` (409), `setup.bad_token` (403), `setup.no_token` (409), `setup.rate_limited` (429), `token.limit` (409), `auth.token_scope`, `auth.token_forbidden`. Tokens are also blocked on `/setup/*` and `PUT /servers/{id}/access`, and a read-scope token cannot request a stream ticket. `GET /tokens?all=1` adds an `owner` email. Claiming ownership also needs an `aal2` session when MFA would apply to an owner.

## Remote backups (0.3)
- Destinations (administrators): `GET /backup-destinations` · `POST /backup-destinations {name, kind:"folder"|"s3", path | endpoint,region,bucket,prefix?,path_style?, access_key, secret_key, is_default?, keep_local?, keep_remote?}` → 201 · `PATCH /backup-destinations/{id}` (omitted credentials are kept) · `DELETE /backup-destinations/{id}[?force=true]` (409 `destination.in_use` while backups are stored there; files already on the destination are not deleted) · `POST /backup-destinations/{id}/test` and `POST /backup-destinations/test` (unsaved settings) → `{ok, steps:[{step,ok,detail}]}`.
- Destination: `{id,name,kind,is_default,keep_local,keep_remote,created_at,stored_backups?, path | endpoint,region,bucket,prefix,path_style,access_key:"set"|"not set",secret_key:"set"|"not set"}`. Credentials are write-only and live in a separate 0600 file under the data directory.
- Folder: absolute path, must exist, be writable and not lie inside the data directory or a server directory. Objects are `<path>/moss-backups/<server_id>/<backup_id>.tar.zst`. S3: HTTPS only (plain http for loopback when `MOSS_ALLOW_PRIVATE_BACKUP_HOSTS=true`), path-style or virtual-hosted, multipart streamed from disk, SHA-256 stored as object metadata, size/ETag/SHA-256 verified after upload.
- `GET|PUT /servers/{id}/backup-destination {destination_id:"default"|"none"|<id>}` → `{destination_id, effective:{id,name,kind}|null}` (PUT: administrators).
- Backup gains `remote:"none"|"uploading"|"stored"|"failed"`, `remote_error`, `remote_at`, `remote_destination` (name) and `local_present`. After a successful non-pre-change backup the copy runs as operation `backup.upload` (3 attempts with backoff); failures raise a `backup_failed` notification. `POST /backups/{id}/upload` retries or starts a copy → 202; `POST /backups/{id}/fetch` downloads a remote-only backup and verifies its checksum → 202. A remote-only backup answers 409 `backup.remote_only` on preview and download; verify and restore fetch it first and refuse a copy whose SHA-256 differs.
- Retention: `keep_remote` newest remote copies are kept (min 1); `keep_local` (0 = all) trims local archives once their remote copy is verified. Locked backups and the newest good copy are never removed. Deleting a backup removes its remote copy first (502 `backup.remote_delete_failed`). Pre-change backups are not uploaded.
- Staged restore: `POST /backups/{id}/restore {mode:"in_place", staged:true, only?}` → 202 `{operation, staged_id}` (nothing is changed yet). `GET /servers/{id}/staged-restores` · `GET /staged-restores/{id}` → `{id,backup_id,server_id,only,status:"preparing"|"ready"|"failed"|"promoting"|"promoted",error,can_promote,checks:[{id,ok,blocking,message}],diff:{added,removed,changed,unchanged,size_delta,samples,...},warnings}` · `POST /staged-restores/{id}/promote` → 202 operation (re-checks, safety backup, atomic swap, the old directory is kept in `data/removed/`; the server must be stopped) · `DELETE /staged-restores/{id}` discards. Without `staged` the restore behaves as before.
- Environment: `MOSS_ALLOW_PRIVATE_BACKUP_HOSTS` (see README).
