# Threat model

Audience: people deploying or reviewing Moss. This document describes what Moss defends, against whom, and what it does not defend. It describes the code as it is; where a mitigation does not exist it says so. See also the *Security model* section of the [README](../README.md) and [architecture.md](architecture.md).

## Summary

Moss is a controller for game servers on a host you own. It authenticates people (through Supabase), authorises them per server, and manages files, processes, downloads and backups on that host. It is **not** a sandbox: game servers, plugins, mods and the Forge/NeoForge/Quilt installers run as the same OS user as Moss. Anyone who can run a game server on Moss, or place a jar in one, can act as the Moss OS user. TLS is not terminated by Moss.

## Assets

| Asset | Where | Why it matters |
|---|---|---|
| Server directories, worlds, configs | `<data>/servers/` | Player data; arbitrary code via jars |
| Backups | `<data>/backups/` | Full copies of the above |
| SQLite database | `<data>/` (WAL) | Users, grants, API token digests, webhook URLs, audit log, settings |
| Setup token | `<data>/setup-token` (0600) | Proves host control to claim ownership of a fresh instance |
| API tokens (`moss_pat_...`) | Held by users; SHA-256 digest in DB | Programmatic access with the owner's role |
| Supabase access tokens | Browser; transient in `User.token` | Identity; forwarded to the CurseForge edge function |
| Webhook URLs | Database | Treated as secrets (Discord URLs embed a token) |
| Optional env secrets | `SUPABASE_JWT_SECRET` (legacy HS256), `CURSEFORGE_API_KEY` (Option B) | Token forgery; CurseForge quota |
| The Moss OS user's privileges | Host | Everything above, plus whatever else that user can reach |

## Actors

| Actor | Trust |
|---|---|
| Unauthenticated network client | None |
| Authenticated user with a per-server grant (viewer, operator, manager) | Limited to granted servers and permissions (`src/auth/mod.rs`, `bundle_perms`) |
| Admin / owner | Can create servers, manage users, use the Custom runtime. Effectively able to run commands as the Moss user |
| API token holder | Owner's current role, optionally read-only; cannot manage tokens or users |
| Server operator with file access | Can place jars and scripts that run as the Moss user once the server starts |
| Plugin / mod / game server process | Untrusted by intent, but not contained (same OS user) |
| Upstream providers (Mojang, PaperMC, Modrinth, CurseForge, Adoptium, maven hosts) | Trusted for metadata; payloads checked by hash where upstream publishes one |
| Supabase | Trusted for identity only |
| Reverse proxy | Trusted to terminate TLS and, if `trust_proxy` is set, to append the client address |

## Trust boundaries

1. Internet or LAN to the HTTP listener (through a TLS reverse proxy you provide).
2. Moss to Supabase (JWKS fetch, edge function calls).
3. Moss to upstream download hosts and provider APIs (outbound).
4. Moss to webhook receivers (outbound, user-supplied URLs).
5. Moss process to game server processes (same user; a weak boundary by design).
6. Server directory to the rest of the host (enforced only by Moss's own path checks, not by the OS).

## Entry points

| Entry point | Credential | Code |
|---|---|---|
| REST under `/api/v1` | Bearer Supabase JWT or `moss_pat_` token | `src/api/mod.rs`, `src/auth/extract.rs` |
| `/api/v1/events` (SSE) and `/api/v1/servers/{id}/console` (WebSocket) | Single-use ticket from `POST /ticket` | `take_ticket` in `src/api/mod.rs`, `src/api/session.rs` |
| `/api/v1/setup/claim` | Verified JWT plus setup token | `src/auth/setup.rs`, `Claimant` extractor |
| Static UI and `/healthz` | None | `src/api/assets.rs` |
| File manager and uploads | `files.read` / `files.write` | `src/files.rs` |
| Backup download and restore | `backup.manage` | `src/backups/` |
| Modpack and archive import | `content.manage` | `src/content/modpack/`, `src/backups/archive.rs` |
| Outbound downloads | Server-side, triggered by users | `src/download.rs` |
| Outbound webhooks | Admin-configured URL | `src/notify/webhook.rs` |
| CurseForge edge function | User's Supabase token | `supabase/functions/curseforge/index.ts`, `src/providers/curseforge.rs` |
| Game server processes and FIFO console | Local OS only | `src/lifecycle/start.rs` |

## Threats and mitigations

STRIDE categories: S spoofing, T tampering, R repudiation, I information disclosure, D denial of service, E elevation of privilege.

### HTTP API and authentication

| Threat | Cat. | Mitigation in code | Residual risk |
|---|---|---|---|
| Forged or replayed JWT | S | `src/auth/jwt.rs`: audience `authenticated` and issuer pinned to the Supabase URL; `exp`, `sub`, `aud`, `iss` required; 30 s leeway; asymmetric keys from JWKS (cached 10 min, forced refresh limited to once per 30 s, stale keys used up to 24 h during outages); HS256 only if `SUPABASE_JWT_SECRET` is set | Stale JWKS means a revoked signing key stays trusted for up to 24 h during a Supabase outage. A leaked bearer token is valid until `exp` |
| Valid Supabase user who was never invited | E | Provisioning requires owner or invite and a verified email (`src/auth/provision.rs`, `require_verified_email`) | Depends on Supabase email verification being configured correctly |
| Brute-force of tokens | S | Failed credentials counted per client address, 30 per 60 s, then 429 (`src/auth/throttle.rs`) | In-memory only, resets on restart. Behind a proxy the address is the right-most `X-Forwarded-For` entry only when `trust_proxy` is on; wrong proxy configuration lets an attacker spoof or share an address |
| Stolen API token | S, E | Stored as SHA-256 digest with display prefix; shown once; expiry (fail closed on bad dates); read or full scope; cannot call token or user management; role is the owner's current role; per-user cap (`src/auth/tokens.rs`, `src/auth/extract.rs`) | A full-scope token is as powerful as its owner minus user/token management. No IP binding |
| First-comer claims ownership of a fresh instance | E | When no owner emails are configured, a random token is written to `<data>/setup-token` (0600); claim needs a verified JWT, the token (constant-time compare), is rate-limited (5 per 60 s), and one transaction ensures a single winner (`src/auth/setup.rs`, `src/auth/secret.rs`) | Anyone who can read that file (same host, wrong permissions) can claim. Token file is kept until used |
| Missing second factor | S | `MOSS_REQUIRE_MFA` (off, admins, all) checks the `aal2` claim (`src/auth/mfa.rs`) | Default is off. API tokens bypass MFA by design |
| Dev auth mode exposed | E | `Config::validate` refuses dev mode on any non-loopback bind (`src/config.rs`); a startup warning is logged | Any local process or user on the host, or a reverse proxy on the same host forwarding to loopback, gets full owner access. Never put a proxy in front of dev mode |
| Cross-server access (IDOR) | E | Every handler goes through `server_for` / `authz` with a named permission; invisible servers return not found (`src/api/mod.rs`, `src/auth/mod.rs`) | Authorisation is per handler; a new route that forgets it would be open. Covered by review and tests, not by structure |
| Stream tokens in URLs leaking via logs or history | I | WebSocket and SSE use single-use tickets valid 30 s, capped at 2000 outstanding; the ticket carries the user, and the event stream filters by server visibility (`src/api/session.rs`, `src/api/activity.rs`, `src/console.rs`) | A ticket is a bearer credential for 30 s; an observer of the URL within that window and before the legitimate client connects can use it. No Origin check on the WebSocket upgrade was found in `src/console.rs` |
| XSS or clickjacking in the UI | T, I | CSP (`default-src 'self'`, `script-src 'self'`, `frame-ancestors 'none'`), `nosniff`, `no-referrer`, `X-Frame-Options: DENY` (`src/api/assets.rs`, `src/api/mod.rs`); console and file content rendered as text | `style-src` allows `'unsafe-inline'`; `connect-src` allows any `ws:`/`wss:` host |
| Repudiation of actions | R | Audit log for console commands and file uploads, among others (`st.audit`); sensitive console commands redacted by default (`src/console.rs`) | Audit rows live in the same database an admin can reach; no tamper evidence. Game-side actions by players are not audited |
| Request flooding | D | 512 MiB body limit (`src/api/mod.rs`); auth-failure throttle; ticket cap | No general per-user rate limit, no request concurrency limit. Rely on the reverse proxy |

### File manager, uploads and archives

| Threat | Cat. | Mitigation | Residual risk |
|---|---|---|---|
| Path traversal and symlink escape | T, I | `safe_join` rejects `..`, absolute paths, NUL, and any symlink component (lstat on each) (`src/files.rs`) | Check-then-use is not atomic: a process that can create symlinks inside the server directory (the game server itself, a plugin) can race the check. The game server is not contained, so this boundary does not protect against it |
| Access to controller metadata | T, I | `.moss/` is hidden from listings and refused for read, write, rename, delete, upload and download | None known beyond the previous row |
| Stored XSS via uploaded files | T | Downloads send `Content-Disposition: attachment`; `nosniff`; uploads are never served inline | None known |
| Malicious archive (zip slip, links, devices, bombs) | T, D | Backup archives: plain relative regular files and directories only, checksum verified first, extraction to staging, 2 TiB expansion cap (`src/backups/archive.rs`). Modpacks use the same rules | Expansion cap is large; disk exhaustion is possible. Only `tar.zst` entry types are checked; content of jars is not scanned |
| Malicious jar or script uploaded by a manager | E | None. By design a manager with `files.write` and `server.power` can run code as the Moss user | Not protected. Treat the `manager` bundle as code-execution on the host |

### Outbound downloads

| Threat | Cat. | Mitigation | Residual risk |
|---|---|---|---|
| SSRF or download from an arbitrary host | T, I | HTTPS only; exact-domain or `.domain` suffix allow-list, re-checked on every redirect hop, 5 redirects max (`src/download.rs`); modpack file hosts limited to a second small list | Allow-list includes `github.com` and `githubusercontent.com`, which serve user content. A malicious provider response can point at any file there |
| Tampered or substituted file | T | Streamed SHA-1/256/512 verification when upstream provides a hash; temp file then atomic rename (`src/download.rs`) | Where no hash is published (some upstream metadata, CurseForge files without usable hash) integrity rests on TLS only. A compromised upstream publishes a matching hash |
| Oversized download | D | 4 GiB cap, connect and read timeouts | One server can still fill the disk with several downloads |
| Installer jars run third-party code | E | Forge, NeoForge and Quilt installers run with `env_clear` and a time limit (`src/installer/modded.rs`) | Not sandboxed; they run as the Moss user |

### Webhooks

| Threat | Cat. | Mitigation | Residual risk |
|---|---|---|---|
| SSRF to internal services | I, E | HTTPS only, no redirects, 5 s timeouts; destination resolved once, rejected unless globally routable (private, loopback, link-local, CGNAT, ULA, mapped, NAT64, 6to4 handled) and connected to by that IP (`src/notify/webhook.rs`) | `MOSS_ALLOW_PRIVATE_WEBHOOKS=true` disables the check. HTTPS-only means plain-HTTP internal targets are unreachable regardless |
| Webhook URL leakage | I | URL never logged; errors strip it | Stored in the database in clear text; included in whatever backup of the database you take |

### CurseForge edge function

| Threat | Cat. | Mitigation | Residual risk |
|---|---|---|---|
| API key exposure | I | Key lives only in the Supabase secret (or the local env in Option B); function requires a valid Supabase user token, fixed game id, class ids and endpoints, `Cache-Control: no-store` (`supabase/functions/curseforge/index.ts`) | The function's own limits are in memory per worker. Any signed-in Supabase user of the project can use it, not only Moss users |
| Open proxy | T | Allow-listed routes; download URL read from CurseForge's file record, https on `forgecdn.net` hosts only, 512 MiB cap | Terms-of-use questions about shared keys are the key holder's, see [curseforge.md](curseforge.md) |
| Quota exhaustion | D | Per-user and global limits, Moss-side limiter (`src/providers/curseforge.rs`) | A coordinated set of users can still exhaust the shared quota |

### Game server processes

| Threat | Cat. | Mitigation | Residual risk |
|---|---|---|---|
| Server or plugin reads or alters other servers, Moss data, or the database | I, T, E | None in-process. Launch uses an argument array and a cleared environment (`src/lifecycle/start.rs`); the shipped unit sets `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `RestrictAddressFamilies` (`packaging/moss.service`) | Not protected. All servers share one OS user and filesystem view; one malicious plugin can read every other server, every backup, the database (including webhook URLs and token digests) and the setup token |
| Resource exhaustion by one server | D | Linux only: cgroup v2 memory, CPU and pids limits per server when the unit delegates a cgroup (`src/platform/cgroup.rs`) | cgroups are for fairness and containment of runaway usage, not a security boundary. They do not restrict files, network or syscalls. Not enforced on macOS |
| Console command injection | E | Commands go through a FIFO as data; permission `server.console.write` required | A console writer can run any in-game command, including those that execute code if plugins allow it |
| Custom runtime | E | Admin-only | Runs an admin-supplied command as the Moss user by definition |
| PID reuse hijack after restart | S | Re-attach by PID plus process start time (`src/lifecycle/reconcile.rs`, `src/platform/`) | Same-user processes can still signal each other |

## Explicitly not protected

- **Container or user isolation.** No Docker, no per-server OS user, no namespaces, no seccomp. Container isolation is on the roadmap.
- **A compromised Moss OS user.** Everything Moss holds is readable by anything running as that user.
- **TLS.** Moss serves plain HTTP. You must provide a TLS reverse proxy for any non-loopback use, and the proxy must set the headers `trust_proxy` expects.
- **Dev auth mode on a shared host.** It is loopback only, but loopback is reachable by every local user and process.
- **Supabase compromise.** Anyone who can mint valid Supabase tokens for your project and whose email is invited or owner gets in; the project itself is the root of identity.
- **Supply chain of mods and plugins.** Moss reads jars and checks compatibility and hashes, but does not scan for malware.
- **Availability under abuse.** Throttling is limited to failed authentication and tickets.
- **Network egress control.** Game servers can reach any address the host can.

## Recommendations for deployers

1. Run Moss as a dedicated unprivileged user with the shipped systemd unit.
2. Terminate TLS in a reverse proxy; set `trust_proxy` only if the proxy is the sole path to Moss.
3. Never use `MOSS_AUTH=dev` anywhere except a single-user machine.
4. Enable `MOSS_REQUIRE_MFA` for admins at least.
5. Grant `manager` and admin only to people you would trust with a shell as the Moss user.
6. Keep `<data>` permissions at 0700 and back up the database separately from game data.
7. Add host-level controls (firewall egress, separate hosts per trust level) if you need more than the above.
