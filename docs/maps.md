# Live maps

When a Minecraft Java server has a web map plugin, Moss shows it in a **Map** tab on the server page and, if you opt in,
on the server's public status page. The map is the plugin's own web server, proxied through Moss.

## Detection

Moss looks in `plugins/` for a directory or jar named `BlueMap`, `squaremap`, `Pl3xMap` or `dynmap` (case-insensitive;
cached for 30 seconds) and reads the web port from the plugin's config:

| Plugin | File | Setting | Default |
|---|---|---|---|
| BlueMap | `plugins/BlueMap/webserver.conf` | `port` | 8100 |
| squaremap | `plugins/squaremap/config.yml` | `settings.internal-webserver.port` | 8080 |
| Pl3xMap | `plugins/Pl3xMap/config.yml` | `settings.internal-webserver.port` | 8080 |
| dynmap | `plugins/dynmap/configuration.txt` | `webserver-port` | 8123 |

A plugin whose built-in web server is switched off (`enabled: false`, `disable-webserver: true`) is not shown. Ports
below 1024 are ignored. The server then reports the capability `map`.

## How it is proxied, and why it is safe

Map pages are plugin-controlled HTML and JavaScript, so they must never run with Moss's privileges.

- The proxy only ever connects to `http://127.0.0.1:<detected port>`. Nothing in a request chooses the host.
- Every proxied response carries `Content-Security-Policy: sandbox allow-scripts allow-popups` (no `allow-same-origin`),
  so the page has an opaque origin even if opened in its own tab, plus `X-Content-Type-Options: nosniff` and
  `Access-Control-Allow-Origin: *` (the sandboxed page can read its own tiles). The plugin's `Set-Cookie`, CSP, CORS and
  framing headers are removed. The UI frame uses `sandbox="allow-scripts allow-popups"` as well.
- A sandboxed page sends no Moss cookies, so access is a capability link: `/map/{server_id}/{token}/...`, where the token
  is `expiry.p.HMAC` for public links or `expiry.u.<hex user id>.HMAC` for user links (HMAC-SHA256 under the key over
  server id, expiry, scope and holder). The key is `map.key` in the data directory (created on first use, owner-only).
  `GET /api/v1/servers/{id}/map` issues a 12 hour link to anyone who may view the server. Tokens are bound to one
  server and fail once expired or altered. A user link also names its holder, whose access is re-checked (cached for
  5 seconds): deleting the user or removing their grant ends their links at once.
- Only `GET`, `HEAD` (and a locally answered `OPTIONS`) are accepted. Only `Accept`, `Accept-Encoding`, `Range`,
  `If-None-Match` and `If-Modified-Since` are forwarded. Paths are normalised (`..`, encoded separators and control
  characters are refused), redirects are kept inside the proxy, requests time out after 60 seconds and responses are
  streamed with a 128 MiB cap.

Because the page runs in an opaque origin, plugin features that need `localStorage` or cookies may not persist settings.

## Status pages

Turn on **Show the live map** for a status page (`show_map`). Visitors see a frame for each listed server that is running
and has a map. The page mints 2 hour public links at render time; they go through a per-client rate limit, and stop
working immediately when `show_map` is turned off or the page is disabled. The page reloads every 25 minutes instead
of every 30 seconds so the map is not reset.

If the server is stopped or the port does not answer, the Map tab shows an empty state with a "Check again" button.
