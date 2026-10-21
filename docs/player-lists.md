# Shared player lists

Create whitelists ("allow lists") and ban lists once under Settings, Player lists, then tick them on a server's Players tab. Moss applies them when you add or remove a name, when a server's subscriptions change, when a server starts (files, before launch) and when it becomes ready.

| Server | Allow list | Ban list |
|---|---|---|
| Minecraft Java | `whitelist.json` when stopped, `whitelist add/remove` + `whitelist reload` when running | `banned-players.json` when stopped, `ban` / `pardon` when running |
| Bedrock | `allowlist.json` / `allowlist add/remove` + `allowlist reload` | not supported (Bedrock has no ban command) |
| Velocity proxy | not supported: it has no vanilla whitelist. Subscribe the backend servers behind the proxy. | not supported |
| Games with an admin channel (Source, Rust, Factorio, Palworld, Zomboid, Terraria) | not supported | bans only, sent while the server runs; Source and Palworld can only ban connected players; removing a name does not unban |

## How it works
- Moss remembers which names it put on each server and compares them with what the subscribed lists want, so applying is idempotent and only the difference is sent. Entries you add by hand to the JSON files or with console commands are kept; Moss only removes names it added.
- Java UUIDs: with `online-mode=true` Moss looks the name up at `api.mojang.com` (5 s timeout) and caches the UUID on the entry; names Mojang does not know stay "pending" and are not written. With `online-mode=false` the offline UUID is used (UUIDv3 of `OfflinePlayer:<name>`).
- Moss never turns the whitelist on. If `white-list` (Java) or `allow-list` (Bedrock) is false in `server.properties`, the Players tab says so.
- A file that is not a valid JSON list is left untouched and a warning appears in the server console.
- Names may not contain quotes or control characters (the same rules as kick and ban).
