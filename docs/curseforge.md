# CurseForge

Moss can browse and install plugins, mods and datapacks from CurseForge next to Modrinth. When CurseForge is available, the content catalogue shows a provider switch. Installed items remember where they came from, and updates are checked against the same provider.

You do not need an account or a key. Moss reaches CurseForge through a small relay service that holds the API key, so the key never ends up on your machine. You only need to be signed in to Moss. `moss doctor` shows whether CurseForge is available.

## What to expect

- **Compatibility.** Plugins for Paper, Purpur and Folia, mods for Fabric, Quilt, Forge and NeoForge, datapacks for Vanilla. Searches and file lists are filtered by your server's game version and loader. CurseForge does not say whether a mod runs on servers, so Moss skips files tagged client-only and warns about the rest.
- **Author opt-outs are respected.** Projects whose authors turned off third-party downloads are hidden from search. If a specific file cannot be downloaded through the API, Moss tells you, links to the project page and never tries to get around it. Download the file from CurseForge yourself and upload it in **Files**.
- **Dependencies.** Required dependencies appear in the change plan before anything is installed, exactly like Modrinth. Every install goes through the same plan, undo snapshot and checksum check.
- **Modpacks.** CurseForge packs can be imported by project or as an uploaded zip. Files the authors did not allow are listed in the report instead of being skipped silently.

## Limits

CurseForge does not publish request limits, so Moss is deliberately gentle:

- Moss makes at most about three CurseForge calls a second, and update checks use one batch request for all installed CurseForge items.
- The relay allows 40 calls a minute and 6 file downloads a minute per person. If CurseForge asks Moss to slow down, the UI says so and retries after the time CurseForge gave.
- Nothing from CurseForge is cached, as its terms require. Moss only remembers what an installed item needs: the project and file ids, title, version label and file name.

If CurseForge is unavailable (outage, rate limit, relay down), Modrinth keeps working and the CurseForge tab explains what happened.

## Using your own key (optional)

If you would rather not use the relay, create a key at the CurseForge for Studios console and put it in a file named `.env.local` next to Moss:

```
CURSEFORGE_API_KEY=your-key
```

Restart Moss. It then talks to CurseForge directly and never uses the relay. Keep the key private and do not share it with other people, because CurseForge keys are personal. Setting `MOSS_CURSEFORGE_PROXY=off` turns CurseForge off completely.
