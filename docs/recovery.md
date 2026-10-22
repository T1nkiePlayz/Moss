# Recovering a Moss installation

`moss recover` fixes the common "I can't get in" and "something is stuck" situations without the web panel. It works directly on Moss's data directory, so it needs shell access to the machine Moss runs on.

Moss refuses to run these commands while it is running, because two programs changing the same database can lose changes. Stop Moss first (for example `sudo systemctl stop moss`). Your game servers keep running while Moss is stopped. If you are sure no controller is running but you still see the refusal, add `--force` (put it before the subcommand: `moss recover --force db check`).

Point the tool at your data directory with `--data-dir /path/to/data` or the `MOSS_DATA_DIR` variable. It uses the same default as `moss serve` (`~/.local/share/moss` on Linux, `~/Library/Application Support/Moss/data` on macOS, or an existing `./moss-data`, `~/moss/moss-data` or `~/moss-data` that holds a database).

Every command prints what it did and exits with status 0. A problem is reported in plain words and the exit status is 1, so the commands are safe to use in scripts.

## I changed Supabase project and now nobody is the owner

Sign-in accounts belong to a Supabase project. A new project means new accounts, so the owner row in Moss no longer matches anyone.

```sh
moss recover owner set you@example.com
```

Start Moss and sign in with that e-mail address (verified in the new project). Moss links your new account to the owner row, and any server access that was granted to that e-mail is kept. If other owners exist, the command lists them; `moss recover owner clear` demotes all owners to administrators first.

## I lost access and nobody can claim ownership

```sh
moss recover owner clear
moss recover setup-token new
```

or in one step: `moss recover setup-token new --unclaim`. This writes a fresh token to `setup-token` in the data directory (readable only by you). Start Moss, sign in, and paste the token when asked. Without `--unclaim`, the token is only useful when no owner exists; Moss deletes it on start otherwise.

## An operation says "running" forever

```sh
moss recover operations reset
```

Marks every queued or running operation, and every backup that was in progress, as failed. Nothing is deleted. Moss already marks work interrupted by a restart; use this when something is stuck without a restart. Then look at the server's files and backups before retrying.

## Check the database

```sh
moss recover db check
```

Runs SQLite's integrity check and compares the schema version with what this Moss expects. If the file is damaged, restore `moss.sqlite` from a copy (stop Moss first, and delete `moss.sqlite-wal` and `moss.sqlite-shm` when you replace the file). A database from a newer Moss is refused rather than modified: upgrade Moss (`moss update`) instead of downgrading.

## Check backups

```sh
moss recover backup verify <backup-id>
moss recover backup verify all
```

Reads the archive, compares its SHA-256 with the one recorded when the backup was made, and checks that every path inside is safe. `all` checks every completed backup and fails if any is damaged. This is the same check as **Verify** in the web panel, usable when the panel is not.

## List servers

```sh
moss recover servers list
```

Shows each server's id, runtime, version, port, whether it should be running, and whether its directory exists (`MISSING` means the files are gone from `servers/<id>` under the data directory).

## Revoke every API token

```sh
moss recover tokens revoke-all
```

Deletes all personal API tokens at once, for example after a token leaked. People can create new ones in **Settings**.

## Export the configuration

```sh
moss recover export-config
moss recover export-config --out moss-config.json
```

Prints (or writes) a JSON summary of settings, users and their roles, servers, access grants and schedules. It contains no secrets: no token hashes, no notification webhook URL, no Supabase keys. It is meant for documenting an installation or moving to a new machine; it does not include world data (use backups for that).

## What these commands never do

They never delete servers, worlds or backups, and they never start or stop game servers.
