# Hangar

Moss can browse and install plugins from [Hangar](https://hangar.papermc.io), PaperMC's official plugin repository, next to Modrinth and CurseForge. The content catalogue shows Hangar in the provider switch for Paper, Purpur, Folia and Velocity servers. Installed items remember where they came from, and updates are checked against Hangar.

You do not need an account or a key.

## What to expect

- **Compatibility.** Searches and version lists are filtered by your server's platform (Paper, Purpur and Folia use Hangar's Paper platform; Velocity uses its own) and by the server's Minecraft version. Hangar does not tag Folia support, so Moss warns you to check the project page.
- **Hosted files only.** Some versions only link to a download elsewhere. Moss shows them with the link but never installs them or downloads from an external address. Download the file yourself and upload it in **Files**.
- **Checksums.** Every Hangar download is verified against the SHA-256 checksum Hangar publishes, and goes through the usual change plan and undo snapshot.
- **Dependencies.** Required dependencies that live on Hangar are added to the change plan. Required dependencies that are not on Hangar are listed as "install manually".
- **Not covered.** Hangar has no modpacks, and it cannot identify unknown files by hash, so Identify does not match against it. Many well-known plugins (LuckPerms, for example) are not on Hangar; use Modrinth or CurseForge for those.

## Outages

If Hangar is unavailable (outage, rate limit), Modrinth and CurseForge keep working and the Hangar tab explains what happened. Update checks report which provider failed instead of saying everything is up to date.

## Settings

`MOSS_HANGAR_API_URL` changes the API address (default `https://hangar.papermc.io/api/v1`; must be https, or loopback http for tests).
