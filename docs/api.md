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
