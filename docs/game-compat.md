# Game compatibility

Moss's SteamCMD games have unit-tested commands and log rules, but a unit test cannot tell whether the real server installs, reaches ready and answers a player query. A nightly workflow, [games-e2e.yml](../.github/workflows/games-e2e.yml), runs every one of them for real.

## What the matrix does

One job per SteamCMD game (the list is read from the catalogue in `src/games/mod.rs`, so a new game joins on its own). Each job runs `tests/games_e2e.rs`, which starts the real Moss binary in dev mode and drives it over the HTTP API:

| Step | Passes when |
|---|---|
| install | the create-server operation completes (SteamCMD downloads the game; up to 60 minutes) |
| start | the server reaches `running`, which is Moss's own readiness: the game's ready log rule, or its fallback timer |
| players | the player query (A2S, RCON or log, whatever the game uses) answers with an empty list |
| backup | a backup completes with status `ok` and its verification completes |
| stop | a graceful stop ends in `stopped` |
| no_orphan | no process of the game is left behind |

A failed step ends that game's run; later steps show as not reached. Games too large for a hosted runner (listed in `TOO_LARGE` in the workflow, currently CS2 and ARK) are recorded as skipped. The last job renders the results with `scripts/game-compat-table.mjs` into the run summary and a `game-compat.md` artifact; each game's raw results are the `results-<game>` artifacts (one JSON line per step: `game`, `step`, `ok`, `seconds`, `error`).

Run it by hand from the Actions tab with a comma-separated `games` list to test only some games.

## Running one game locally

You need Linux x86-64, SteamCMD's 32-bit libraries (`lib32gcc-s1` and `lib32stdc++6` on Debian and Ubuntu) and disk space for the game.

```sh
MOSS_E2E_GAME=valheim MOSS_E2E_RESULTS=/tmp/valheim.jsonl \
  cargo nextest run --run-ignored only --test games_e2e --no-capture
node scripts/game-compat-table.mjs /tmp/valheim.jsonl
```

`MOSS_E2E_GAME=all` runs every SteamCMD game one after another. The tests are `#[ignore]`, so a plain `cargo nextest run` never touches them. Optional settings: `MOSS_E2E_DIR` (where the data folder goes, default the temp directory), and time limits in seconds `MOSS_E2E_INSTALL_S` (3600), `MOSS_E2E_START_S` (1200), `MOSS_E2E_PLAYERS_S` (120), `MOSS_E2E_BACKUP_S` (1800), `MOSS_E2E_STOP_S` (300).

## Results

Nothing has been run yet. When a nightly run finishes, paste the rows from its `game-compat.md` artifact over the table below.

| Game | install | start | players | backup | stop | no_orphan |
|---|---|---|---|---|---|---|
| valheim | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| gmod | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| cs2 | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| tf2 | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| l4d2 | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| rust | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| palworld | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| zomboid | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| satisfactory | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| l4d | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| css | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| dods | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| hl2dm | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| nmrih | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| ark | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
| sdtd | not yet run | not yet run | not yet run | not yet run | not yet run | not yet run |
