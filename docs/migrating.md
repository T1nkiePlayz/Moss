# Moving to Moss

Moss can take over a Minecraft Java server that already exists. It copies the server's files, works out what kind of server it is, and manages it from then on. This page covers Pterodactyl and Pelican, AMP, and a hand-written systemd setup with `screen` or `tmux`.

## What Moss imports

Moss imports **Minecraft Java servers**: Vanilla, Paper, Purpur, Folia, Fabric, Quilt, Forge, NeoForge and Velocity. It reads the server's files to find the type, the Minecraft version and the port, and it identifies the plugins and mods already in the folder so they can be updated like anything else.

- **Other games do not import.** Detection is built around Minecraft server jars and asks for a Minecraft version. For Bedrock, Terraria, Valheim and the rest, create a new server with **New server** and copy your world or config files in afterwards from the server's **Files** tab.
- **Spigot and CraftBukkit are not run by Moss.** Moss reports this when it sees one. Paper runs Spigot plugins, so choose Paper as the runtime.
- **Settings from your old panel do not carry over.** Memory, ports, startup commands, schedules and backups are set up again in Moss. Your server's own files do carry over: worlds, `server.properties`, `whitelist.json`, `ops.json`, bans, plugins, mods and plugin configs.
- **Old backups do not import as Moss backups.** Keep them where they are. If you want a backup in Moss, take a new one after the import.
- Only administrators can import.

## How importing works in Moss

Open **Servers** and choose **Import** (the page is titled **Import a server**). There are two sources:

- **Upload an archive**: a `.zip` or `.tar.zst`. The upload limit is 512 MiB, so this suits small servers. If the archive holds a single top-level folder, Moss uses that folder.
- **Folder on this machine**: the full path of a folder on the machine Moss runs on, for example `/srv/migrate/survival`. Moss copies it and never changes the original. Symbolic links are skipped. Folders under `/var/lib`, `/etc`, `/usr` and other system locations are refused, and so is Moss's own data folder.

Choose **Look at it**. Moss shows **What Moss found**: runtime, version, port, Java, and whether a world is included. If it could not tell something, a note says so and you fill it in. Then set the name, **Runtime**, **Minecraft version**, **Memory (MB)** and **Port**. Leaving **Port** empty picks a free one; Moss refuses a port another Moss server uses.

The EULA is an explicit choice on this page. Either accept the Minecraft EULA, which Moss records in the activity log and writes to `eula.txt`, or choose *skip for now*. A skipped server will not start until you accept it from the server page.

Choose **Import server**. The progress and the log stay on the server's **Setup** tab, and the log ends with how many plugin and mod files were recognised. If the import fails, Moss removes the half-made server and says why.

> [!NOTE]
> If you run Moss from the shipped systemd unit, it runs as the `moss` user with `ProtectHome=yes`. It cannot read `/home`, and a folder it can read must be readable by `moss`. Copy the server to a neutral folder such as `/srv/migrate/<name>` first, and give that folder to Moss for the import:
>
> ```sh
> sudo mkdir -p /srv/migrate
> sudo cp -a /path/to/old/server /srv/migrate/survival
> sudo chown -R moss:moss /srv/migrate/survival
> ```
>
> Delete the copy after the import has finished and you have checked the server.

## From Pterodactyl or Pelican

Wings usually keeps each server's files in `/var/lib/pterodactyl/volumes/<uuid>`. Check your Wings configuration if yours differs. Pelican's Wings works the same way but may use a different data folder, so look at its configuration.

1. In the old panel, **stop** the server and wait for it to say it is offline. This makes sure the world is saved and nothing writes to the files while you copy.
2. Get the files somewhere Moss can read them. Moss refuses folders under `/var/lib`, so you cannot point it at the Wings volume directly. Pick one:

   - **Copy the folder** and import it by path:

     ```sh
     sudo cp -a /var/lib/pterodactyl/volumes/<uuid> /srv/migrate/survival
     sudo chown -R moss:moss /srv/migrate/survival
     ```

   - **Make an archive** and upload it (up to 512 MiB), or copy the archive to the machine and import its folder after unpacking:

     ```sh
     sudo tar --zstd -cf survival.tar.zst -C /var/lib/pterodactyl/volumes/<uuid> .
     # or: cd /var/lib/pterodactyl/volumes/<uuid> && sudo zip -r ~/survival.zip .
     ```

3. **Fix the ownership.** The files belong to the Wings user (often `pterodactyl`), not to the user that runs Moss. A copy made with `cp -a` keeps that owner. The `chown` above gives the files to `moss`, which is what you want for a server Moss runs. When Moss itself makes the copy (a folder import) the new files belong to the user that runs Moss, so the `chown` is mainly for letting Moss read the folder you hand it.
4. Import it as described above. Check the detected runtime and version.
5. **Enter what the egg did for you.** Pterodactyl's startup command and egg variables do not carry over. In Moss set:
   - **Memory (MB)** on the import form, or later in the server's **Settings** tab. Moss caps it at this machine's memory.
   - **Port**: use the old allocation's port if players already connect to it, or change it and tell your players. Stop the old server first so the two do not compete for the port.
   - **JVM flags profile** in the server's **Settings**: *Balanced (Aikar-style tuned flags)* or *Minimal (just -Xms/-Xmx)*. Custom startup flags are not imported.
   - Server version, jar and any variables such as the game version or a modpack: Moss reads them from the files. If it could not tell, enter them on the import form.
6. Re-create scheduled tasks, subusers and Discord hooks in Moss; see the checklist.

## From AMP (CubeCoders)

AMP keeps each instance in its own folder, typically under `~/.ampdata/instances/<name>/`, and the Minecraft files are typically inside a subfolder such as `Minecraft/`. Check your own AMP instance folder to be sure.

1. Stop the instance in AMP.
2. Find the folder that holds `server.properties` and the server jar directly. That is the folder to import, not the AMP instance folder around it. Moss looks at the top of the folder it is given and says it does not look like a Minecraft server if it finds no `server.properties` and no `.jar` there.
3. Copy it out to a neutral folder and give it to the Moss user (see the note above). AMP runs under its own user, and `/home` is not readable by the shipped Moss unit.

   ```sh
   sudo cp -a /home/amp/.ampdata/instances/<name>/Minecraft /srv/migrate/<name>
   sudo chown -R moss:moss /srv/migrate/<name>
   ```

   Adjust the path to where your instance really is.
4. Import the copy, then check the runtime, version, port and memory. Memory, ports, schedules, AMP users and any AMP-specific settings do not carry over.

## From systemd with screen or tmux

If you run the server yourself with a unit file or script that starts it in `screen` or `tmux`, Moss replaces both the unit and the session.

1. Stop the server cleanly from its own console (`stop`), and wait for the process to exit. Then stop and turn off the old unit so it does not start again on boot, and so it does not race Moss for the port:

   ```sh
   sudo systemctl disable --now minecraft.service    # use your unit's name
   ```

   Also remove any `@reboot` cron line or `.service` that restarts it.
2. Note what the old setup did: the port in `server.properties`, the `-Xmx` and `-Xms` values, and any extra Java flags.
3. Import the folder. Moss copies it, so your original stays as a fallback. Use a folder Moss can read; `/home` is not readable from the shipped unit, so copy the server to `/srv/migrate/<name>` first if it lives there.
4. Carry the old command line over:
   - `-Xmx`: set it as **Memory (MB)**.
   - Aikar-style flags: choose *Balanced (Aikar-style tuned flags)* as the **JVM flags profile**. A plain `-Xms/-Xmx` start is *Minimal*. Other custom flags are not carried over.
   - The Java version: Moss picks the Java version the Minecraft release needs. If it is not installed, use **Settings** and the **Java installs** card.
5. Replace cron backup scripts with Moss **Schedules** (below). A backup script that also copied files off the machine can be replaced by a remote destination (a folder such as a NAS mount, or S3-compatible storage) in the Backups tab; see the README's Backups section.
6. `whitelist.json`, `ops.json`, `banned-players.json` and `banned-ips.json` come along with the folder, so your player lists keep working. For several servers sharing one list, see [player-lists.md](player-lists.md).
7. Once you are happy, delete the old unit file and `systemctl daemon-reload`.

## Replacing scheduled tasks and backups

In the server's **Backups** tab, the **Schedules** card has **Add schedule**. Choose an **Action** (Backup, Restart, Run command, Start or Stop), whether it repeats **Daily at a time** or **Every N minutes**, a time zone, and for backups **Keep last**. This covers the usual cron backup, nightly restart and "say restarting soon" command. Run one with **Run now** to check it. Moss backups are checksummed `tar.zst` files, flushed with the game's own save commands where the game supports it.

## Checklist

Work through this before you retire the old setup.

- [ ] **Ports.** The server uses the port you expect, the old server is stopped, and the firewall or router forwards that port. Game ports are never proxied by Moss.
- [ ] **Memory.** **Memory (MB)** matches what the old server had, and the machine has room for it.
- [ ] **Java and flags.** The right Java is installed (**Settings**, **Java installs**) and the **JVM flags profile** is chosen.
- [ ] **Starts and works.** Start it, watch the console for the world loading, and join it. Check the **Content** tab for plugin and mod problems.
- [ ] **EULA.** Accepted, or accepted later from the server page.
- [ ] **Backups.** Take a first backup, use **Verify**, and add a schedule. Optionally add a remote destination.
- [ ] **Schedules.** Restarts, commands and other cron jobs are re-created.
- [ ] **Autostart.** Turn on **Start automatically with Moss** in the server's **Settings** if the server should come back after a reboot, and enable Moss itself (`sudo systemctl enable --now moss`).
- [ ] **Player lists.** `whitelist.json`, `ops.json` and the ban lists are there, and your players can join.
- [ ] **People.** Invite the other administrators and give each server's people the right access in the server's **Access** tab.
- [ ] **Reverse proxy.** Put Moss's panel behind a TLS reverse proxy if it is reachable from outside; see the README's Reverse proxy section and [troubleshooting.md](troubleshooting.md) if the console does not connect.
- [ ] **Remove the old panel.** Only after a few days of normal play and a verified Moss backup, delete the old unit, the old panel and its copy of the files. Keep the original files until then.

If something does not work, start with [troubleshooting.md](troubleshooting.md).
