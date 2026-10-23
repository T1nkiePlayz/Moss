# Troubleshooting

Start with `moss doctor`. It checks the machine Moss runs on and prints one line per check. Then look at the findings in the panel, which look at your servers. The last section explains how to collect a support bundle for a bug report.

## Running `moss doctor`

```sh
moss doctor
moss doctor --data-dir /var/lib/moss     # check a specific data folder
```

Run it as the user that runs Moss (for the systemd unit, `sudo -u moss moss doctor --data-dir /var/lib/moss`), or the data folder and Java checks will describe a different setup. Doctor does not change your data. When no data folder is named and none exists yet, it works in a throwaway folder and removes it again.

Each line starts with a tag: `[ok  ]`, `[FAIL]`, `[warn]` or `[info]`. `moss doctor` exits with status 0 even when a line says `FAIL`, so read the output; do not rely on the exit status. The first line shows the Moss version, the operating system and the CPU architecture.

## What each `moss doctor` line means

### `[ok  ] config: valid` / `[FAIL] config: <reason>`

Moss read its settings and they make sense, or one of them does not. The reason is one of these:

- `--bind must be an IP address`, or `dev auth mode (no login) may only bind to loopback; refusing to bind <address>`: `MOSS_AUTH=dev` only works on `127.0.0.1` or `::1`. Remove `MOSS_AUTH=dev` or set `MOSS_BIND` back to a loopback address.
- `SUPABASE_URL is required in supabase auth mode (or run with MOSS_AUTH=dev on loopback)`: sign-in is built in, so you only see this after setting `SUPABASE_ANON_KEY` (or `SUPABASE_URL`) yourself. Setting either one turns the built-in pair off, so set both, or remove both.
- `MOSS_REQUIRE_MFA must be off, admins or all`: fix the value.
- `MOSS_CURSEFORGE_API_URL`, `MOSS_MODRINTH_API_URL` or `MOSS_HANGAR_API_URL must be https (or loopback http for tests)`: remove the override or use an `https://` address.
- `MOSS_STAGING_PORT_RANGE must look like 30000-30999 (1024 or higher, first <= last)`: fix the range.

Settings live in the environment, flags, `.env` and, for the systemd unit, `/etc/moss/moss.env`. The full list is in the README under Configuration.

### `[ok  ] data directory <path> writable` / `[FAIL] data directory <path> writable`

The `FAIL` form means Moss cannot create files in its data folder. Usually the folder belongs to another user, for example after you ran Moss once as root or with `sudo`.

```sh
sudo chown -R moss:moss /var/lib/moss
```

Use your own folder and user if you run Moss another way. With the shipped systemd unit the data folder is `/var/lib/moss`, and the unit only allows writing there (`ReadWritePaths`). If you moved the data folder, edit the unit's `--data-dir` and the `ReadWritePaths` line to match.

When the folder does not exist yet, the line reads `data directory <path> (not created yet; Moss creates it on first start) writable`, and doctor tests the nearest folder that exists. That is normal on a fresh machine.

### `[ok  ] disk: <free> GiB free of <total> GiB` / `[FAIL] disk: ...`

`FAIL` means less than 2 GiB is free on the volume that holds the data folder. Backups, world saves and downloads can fail on a full disk. Free some space, delete backups you no longer need, or move the data folder to a bigger disk (stop Moss and move the folder; see the README's note on the data folder). Game servers and backups both live under the data folder.

### `[ok  ] port <n> available` / `[FAIL] port <n> available`

Doctor tries to listen on the configured address and port (default `127.0.0.1:8420`). `FAIL` means something else is using it. If Moss itself is running, that is the cause, so stop Moss first or ignore this line. See [The port is already in use](#the-port-is-already-in-use).

### `[ok  ] resource limits: cgroup v2 (memory, CPU and process limits are enforced)`

Moss found a writable cgroup v2 tree, so per-server memory, CPU and process limits work.

### `[info] resource limits are not enforced: <reason>`

Per-server limits are saved but not applied. This is not an error. The reason says why:

- `resource limits need cgroup v2, which only Linux has; on macOS Moss cannot cap memory or CPU`: on macOS only the Java heap setting limits memory. Container mode can also limit resources.
- `cgroup v2 is not mounted (the host uses cgroup v1 or none)`: the host needs cgroup v2.
- `no delegated cgroup is writable (...); run Moss from a systemd unit with Delegate=yes and DelegateSubgroup=supervisor (systemd 254+), see packaging/moss.service`: run Moss from the shipped unit, which sets both lines. On systemd older than 254, limits stay unenforced. Running Moss from a terminal usually shows this line.
- `the memory controller is not delegated to <path>`: the delegated cgroup does not include the memory controller.

The panel shows the same thing per server as **Resource limits are not enforced** (see below).

### `[ok  ] container engine: <docker|podman> <version>, rootless|rootful (servers can use container isolation)`

Docker or Podman works, so a server can use container isolation (the **Isolation** card on the server's **Settings** tab).

### `[info] container engine: <reason>`

No usable engine. This only matters if you want container isolation. The usual reason is `Neither Docker nor Podman was found. Install one of them (rootless Podman is the safest choice), make sure the user running Moss can use it, then check again. Servers keep running without isolation until then.` Another is `the configured container engine <name> was not found`, which means `MOSS_CONTAINER_ENGINE` names something that is not installed. See [Container mode says the engine is not found](#container-mode-says-the-engine-is-not-found).

### `[ok  ] java <major> (<version>) at <path>`

Doctor found a Java runtime. `[managed]` after the path means Moss downloaded it (Temurin). Moss needs the Java version a Minecraft release requires, which differs between releases, so one line is not always enough.

### `[warn] no Java found — Moss can download Temurin from Settings → Java`

Nothing usable was found. Open **Settings** and use the **Java installs** card: pick a version and install it. Moss downloads a checksum-verified Temurin runtime into the data folder. Alternatively install Java with your package manager.

### `[ok  ] auth: supabase` / `[ok  ] auth: dev mode (no login, loopback only)` / `[FAIL] auth: ...`

Which sign-in mode is active. `FAIL` means supabase mode with no `SUPABASE_URL`, which the config line explains above. Dev mode is for trying Moss on your own machine. Do not use it on a server other people can reach.

### `[info] curseforge: ...`

Informational, never a failure:

- `direct, with CURSEFORGE_API_KEY`: you set your own key.
- `through the edge function at <url>`: the built-in relay is used (see [curseforge.md](curseforge.md)).
- `off (deploy the Supabase edge function or set CURSEFORGE_API_KEY)`: CurseForge search is off. Set `CURSEFORGE_API_KEY` or remove `MOSS_CURSEFORGE_PROXY=off`. Modrinth and Hangar are not affected.

## Panel health findings

The panel's **Overview** page shows a **Needs attention** count and cards for each finding. Each server's Overview shows its own. A finding has a severity (critical, warn or info), evidence and, where one exists, a one-click fix. The titles below are the ones Moss shows.

| Title | Severity | What it means and what to do |
|---|---|---|
| **EULA not accepted** | critical | The server cannot start until the Minecraft EULA is accepted. Use **Accept EULA**, or set `eula=true` in the server's `eula.txt` (Minecraft Java only). Applies to Java servers and Bedrock. |
| **Java <n> is missing** | critical | This server needs Java <n> and Moss cannot find it. **Install Java** downloads a verified Temurin runtime. |
| **Server crashed** | critical | The server process exited unexpectedly. **Open console** and read the end of the log. Moss restarts with a backoff and stops after repeated crashes (a crash-loop breaker). |
| **Install failed** | critical | The install did not finish. Open the **Setup** tab for the install log, then fix the cause and install again. |
| **Container engine not available** | critical | The server is set to container isolation but no usable Docker or Podman is reachable. Install one, or set the **Isolation** card on the server's **Settings** tab back to *Normal (runs as the Moss user)*. |
| **Game-only network not available** | critical | The server uses the game-only container network and the engine cannot provide it, so it will not start. The detail gives the engine's reason. Podman cannot provide this network (Moss refuses rather than weaken it); use Docker, or choose *Open (can reach the internet)* in the Isolation card. |
| **Port shared with another server** | warn | Two Moss servers have the same port, and only one can run at a time. Change the port in the server's **Settings**; it applies on the next restart. |
| **Slow to become ready** | warn | The server has been starting for more than 3 minutes without reporting readiness. Check the console. Large modpacks can take that long on first start. |
| **No backup yet** | warn | The server is more than a day old and has no successful backup. Use **Back up now**, then add a schedule in the **Backups** tab. |
| **Backup is getting old** | info | The last successful backup is 7 days old or more. |
| **<n> content problem(s)** | warn or critical | Installed mods or plugins have missing dependencies, conflicts or other compatibility problems. **Review content** opens the Content tab, which lists each one with a fix. |
| **Resource limits are not enforced** | info | You set memory, CPU or process limits but this host cannot apply them. See the doctor line above. |
| **Disk almost full** | critical | Less than 5% or less than 2 GiB free on the data volume. Backups and world saves may fail. Shown to administrators. |
| **Disk space is getting low** | warn | Less than 15% free. Shown to administrators. |

## Common problems

### I cannot sign in, or Moss asks for a setup token

Sign-in is built in, so no config file is needed. A valid account is still not enough: Moss only lets in the owner and people the owner invited.

On a fresh instance Moss has no owner. Sign in, then enter the one-time setup token. Moss wrote it to `setup-token` in the data folder when it first started and logged the path. Read it on the host:

```sh
sudo cat /var/lib/moss/setup-token
```

Moss deletes the file once an owner exists. If you lost access or the token is gone, see [recovery.md](recovery.md): `moss recover setup-token new --unclaim` writes a new one. If you changed to your own Supabase project and nobody is owner any more, `moss recover owner set you@example.com`. You can also pin the owner with `MOSS_OWNER_EMAILS`.

Moss throttles repeated bad credentials from one address and answers `429`. Wait and try again. Behind a reverse proxy that throttle sees the proxy's address unless `MOSS_TRUST_PROXY=true` is set, and you should set that only when the proxy sets `X-Forwarded-For`.

### The port is already in use

`moss serve` stops with `cannot bind <address>:<port>: <error>` when another program has the port. Either stop the other program, find what it is (`ss -ltnp | grep 8420`), or use another port with `--port` or `MOSS_PORT`. Another Moss may be running already (`systemctl status moss`).

For a game server, Moss will not give two of its own servers the same port at import time, and the **Port shared with another server** finding covers the rest. A game port taken by a program that is not Moss makes the game itself fail to start; its console shows why.

### The server will not start because the EULA is not accepted

See **EULA not accepted** above. Minecraft Java servers and Bedrock need the acceptance recorded. Accept it from the server's page, or when you create or import the server. Moss writes `eula=true` to `eula.txt` for you at that point. If you chose *skip for now* when importing, the server stays stopped until you accept.

### The console does not connect through my reverse proxy

The console is a WebSocket, and live updates use Server-Sent Events. Caddy handles both as is. For nginx:

- Forward the upgrade headers (`Upgrade` and `Connection`) for the WebSocket.
- Set `proxy_buffering off;` on `/api/v1/events`.

Moss also refuses a stream when the browser's `Origin` does not match the `Host` header the proxy passes on. Make the proxy pass the original `Host` (nginx: `proxy_set_header Host $host;`). Only if the proxy changes `Host` and sets `X-Forwarded-Host`, set `MOSS_TRUST_PROXY=true`. The refusal reads `cross-origin streams are not allowed`. Game ports are separate and are never proxied. Open them in the firewall directly.

### Container mode says the engine is not found

Doctor prints `[info] container engine: ...` and the panel shows **Container engine not available** for servers set to container isolation. The user that runs Moss must be able to run `docker` or `podman`. Check as that user:

```sh
sudo -u moss docker info        # or: sudo -u moss podman info
```

If you have both installed and Moss picks the wrong one, set `MOSS_CONTAINER_ENGINE=docker` or `podman`. The shipped systemd unit uses `ProtectHome=yes` and restricts writes to `/var/lib/moss`, which can get in the way of rootless engines that keep data in a home folder. Container isolation is new and has been tested against a stand-in for the engine, as the README says.

### Moss was updated and an older Moss will not start

An older Moss refuses a database that a newer Moss has upgraded, rather than changing it. Run the newer Moss (`moss update`). See [recovery.md](recovery.md).

### An operation says "running" forever, or the database looks damaged

See [recovery.md](recovery.md) for `moss recover operations reset` and `moss recover db check`.

## Making a support bundle

A support bundle is a zip with the version, `moss doctor`-style output, settings, server list, health findings, the last 500 activity entries, operations and the last 300 console lines per server. Tokens, JWTs, API keys, and webhook URLs are replaced by `[redacted]`, and e-mail addresses are hashed.

In the panel (administrators only): **Settings → Support bundle**. **Preview contents** lists every file with its size and what is redacted, then **Download bundle** saves `moss-support-bundle.zip`.

Without the panel, with a personal API token (create one in **Settings**; it must belong to an administrator):

```sh
curl -H "Authorization: Bearer moss_pat_..." -o moss-support.zip http://127.0.0.1:8420/api/v1/support-bundle
```

Add `?preview=1` to get a JSON list of the files instead. The endpoint needs an administrator, see [api.md](api.md). Moss records each download in the activity log. Look through the zip before you attach it to a public issue; console lines can contain player names and IP addresses.

## Reporting a bug

Open an issue at <https://github.com/T1nkiePlayz/Moss/issues/new/choose> and pick **Bug report**. The form asks for the Moss version (`moss --version`), your OS and install method, and the output of `moss doctor` plus relevant `journalctl -u moss` lines. Remove tokens and e-mail addresses first. Attach the support bundle if you can.

Report security problems privately, not in a public issue. Use GitHub's **Report a vulnerability** on the repository's **Security** tab, as [SECURITY.md](../SECURITY.md) describes.
