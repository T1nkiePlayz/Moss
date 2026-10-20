# Contributing to Moss

Thanks for helping. Moss is a Rust backend (`src/`) with an embedded Svelte UI (`web/`).

## Build and test

Requires Rust (stable) and Node.js (LTS).

```bash
(cd web && npm ci && npm run build)        # the binary embeds web/dist
cargo build --release                       # target/release/moss

# backend checks (an empty web/dist is enough for unit tests)
mkdir -p web/dist
cargo clippy --all-targets -- -D warnings && cargo test   # includes end-to-end, OpenAPI coverage and container-stub tests

# run against a simulated server, no login, no Java needed
MOSS_AUTH=dev MOSS_DEV_RUNTIMES=true MOSS_DATA_DIR=/tmp/moss-dev cargo run -- serve

# frontend
cd web && npm ci
npm run dev            # proxies /api to :8420
npm run dev:mock       # fully mocked backend
npm run check && npm test && npm run build
npx playwright install chromium && npm run test:ui   # every page renders cleanly and passes axe
node ../scripts/bundle-budget.mjs                   # first-load JavaScript budget
```

The API is covered by a stability policy ([docs/api-policy.md](docs/api-policy.md)): `tests/api_compat.rs` fails if you remove or break anything in `docs/openapi.frozen.json`. After adding routes or fields, run `node scripts/api-freeze.mjs` to extend the snapshot and commit it.

The soak test (`tests/soak.rs`, ignored by default) runs the real binary with the simulated game server through random start/stop/restart, console spam, backups, restores, schedules, controller SIGKILLs and provider outages, checking invariants as it goes. Run it with `MOSS_SOAK_MINUTES=5 cargo test --release --test soak -- --ignored --nocapture`; a failure prints its seed (`MOSS_SOAK_SEED`) and keeps the data and controller log under `MOSS_SOAK_DIR`. It also runs nightly (`.github/workflows/soak.yml`).

Every new HTTP route goes in `src/api/mod.rs` (the single route list), `docs/openapi.json` and `docs/api.md`; `tests/openapi.rs` enforces this. Database changes are a new numbered file in `src/db/migrations/` appended to `MIGRATIONS`. Read [docs/architecture.md](docs/architecture.md), [docs/threat-model.md](docs/threat-model.md) and the notes in [docs/adr](docs/adr) before large changes.

Layout: `src/` (`api/`, `auth/`, `lifecycle/`, `container/`, `content/`, `backups/`, `platform/`, `runtimes/`, `installer/`, `providers/`, `notify/`, `jars/`, `cfgfiles/`, `cli/`, `db/` plus single-purpose modules), `web/`, `supabase/` (the CurseForge relay function and its test: `node supabase/functions/curseforge/index.test.mts`, Node 22.18+), `docs/`, `packaging/`, `scripts/`, `tests/`.

CI (`.github/workflows/ci.yml`) runs the UI checks, the relay test and the backend on Linux and macOS.

## Releasing

Push a tag such as `v0.3.0`. `.github/workflows/release.yml` syncs the version, builds the UI once, builds Linux x86-64 and macOS (Apple silicon and Intel) binaries, packages `moss-<tag>-<os>-<arch>.tar.gz`, writes `SHA256SUMS.txt`, attests the tarballs (public repositories only), generates a Homebrew formula, attaches `install.sh` and opens a **draft** release whose notes are the tagged commit's message. Tags with a `-` become pre-releases.
