# Game mods

Servers for Valheim, Factorio, Garry's Mod and CS2 get a **Mods** tab. It is separate from the Minecraft plugin and mod catalogue (Modrinth, CurseForge, Hangar) and never touches it.

| Game | Source | What Moss does |
|---|---|---|
| Valheim | [Thunderstore](https://thunderstore.io/c/valheim/) | Installs BepInEx and mods with their dependencies; starts the server through BepInEx |
| Factorio | [Mod portal](https://mods.factorio.com) | Downloads mods into `mods/`, keeps `mods/mod-list.json` in sync |
| Garry's Mod | Steam Workshop | Mounts a Workshop collection at launch |
| CS2 | Steam Workshop | Starts on a Workshop map instead of de_dust2 |

Changes need a **stopped** server (otherwise 409 `server.running`) and take effect at the next start. Installing needs the `content.manage` permission, reading `server.view`, and saving the Factorio login `config.write`.

## State

Everything lives in the server directory, not in the database:

- `moss-mods.json`: what Moss installed (id, version, enabled, the files it wrote, what it requires). Moss only removes files it listed here. If you delete the file, installed mods stay on disk but Moss forgets them.
- `moss-game.cfg`: settings (`workshop_collection`, `workshop_map`, `steam_api_key`, `factorio_username`, `factorio_token`).
- Factorio: `mods/<name>_<version>.zip` and `mods/mod-list.json`.

Back-ups include these files like any other.

## Valheim (Thunderstore)

- Search uses the community listing endpoint that the Thunderstore site uses itself (`/api/cyberstorm/listing/valheim/?q=`). It is not part of Thunderstore's documented API, so search could break if they change it; installing uses the documented package API (`/api/experimental/package/{namespace}/{name}/[{version}/]`).
- Dependencies (`Namespace-Name-1.2.3`) are resolved transitively, each package once (cycles are tolerated). Where two mods ask for different versions of a dependency, the newer one is installed. A dependency that is already installed at the same or a newer version is left alone.
- **BepInEx** (`denikson-BepInExPack_Valheim`) is installed first if it is missing. Its `BepInExPack_Valheim/` folder is unpacked into the server root. Once `BepInEx/core/BepInEx.Preloader.dll` exists, Moss starts Valheim with the same environment as the pack's `start_server_bepinex.sh`: `DOORSTOP_ENABLED=1`, `DOORSTOP_TARGET_ASSEMBLY=./BepInEx/core/BepInEx.Preloader.dll`, `LD_LIBRARY_PATH=./doorstop_libs:./linux64`, `LD_PRELOAD=libdoorstop_x64.so`.
- Other mods go to `BepInEx/plugins/<Namespace-Name>/` (`patchers/`, `monomod/` and `core/` folders map to the matching BepInEx folder, `config/` files go to `BepInEx/config/` and never overwrite an existing config). Manifest, icon, README, changelog and licence files are not copied.
- **Disable** renames a mod's `.dll` files to `.dll.old`; enabling renames them back. BepInEx itself cannot be disabled. A mod that another installed mod needs cannot be removed (409 `game_mods.required`).
- Thunderstore publishes no checksums, so these downloads are **not hash-verified**. They are restricted to `thunderstore.io` and its subdomains, including on redirects.

## Factorio (mod portal)

- The portal has no text search API. Moss lists the most downloaded mods (cached for an hour) and filters them, and also looks up a query that is an exact mod name.
- Downloads need your factorio.com **username and token** (token from your [profile](https://factorio.com/profile)). Enter them in the Mods tab; they are written to `moss-game.cfg`, the token is never returned by the API and never logged. Without them, installs fail with 409 `game_mods.credentials`.
- Required dependencies are installed (newest release that satisfies `>= / > / = / < / <=`). `base`, `space-age`, `quality` and `elevated-rails` are part of the game and skipped. `?` and `(?)` optional dependencies are not installed. `~` (no load order) counts as required. `!` marks an incompatible mod: the install fails if a mod in the plan, or one already installed, is incompatible with another. Unknown prefixes (such as `+`) are treated as optional.
- Each download is verified against the SHA-1 the portal publishes. Enabling or disabling edits `mods/mod-list.json` and keeps the other entries.
- Downloads are restricted to `factorio.com` and its subdomains.

## Steam Workshop (Garry's Mod, CS2)

Steam downloads the content itself when the server starts, so Moss only stores an id and adds launch arguments.

- Garry's Mod: `workshop_collection=<id>` adds `+host_workshop_collection <id>`.
- CS2: `workshop_map=<id>` replaces `+map de_dust2` with `+host_workshop_map <id>`.
- IDs must be digits. An invalid value found in the file is ignored at launch.
- `steam_api_key` (optional) is passed as `-authkey <key>`; Garry's Mod needs one to mount collections on some setups. The key is visible in the process list on the host.

## Safety

- Downloads only come from the provider's own hosts (re-checked on every redirect), are capped at 512 MiB each, and time out.
- Archives are vetted before anything is written: entries with `..` or symlinks, more than 20 000 entries, or more than 2 GiB unpacked are refused. Extraction goes through the same path check as the file manager (no writing through symlinks).
- API errors and logs never include the Factorio token.

## API

See [api.md](api.md#game-mods): `GET/POST /servers/{id}/game-mods`, `GET .../search?q=`, `GET .../updates`, `PATCH/DELETE .../{mod}`, `PUT .../workshop`, `PUT .../credentials`.
