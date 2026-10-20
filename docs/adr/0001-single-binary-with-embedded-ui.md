# 0001. Single binary with an embedded UI

Status: Accepted

## Context

Moss is installed by people who run game servers on a spare machine or a small VPS. They should not need Node, a database server, Docker or a separate web server to try it. The UI is a Svelte single-page app with no UI framework.

## Decision

Build one Rust binary (axum on tokio). The compiled UI (`web/dist`) is embedded with `rust-embed` and served by the same router as the API (`src/api/assets.rs`). The same binary provides the CLI (`moss serve`, `moss doctor`, ...). Static files carry cache headers and a strict CSP; unknown paths fall back to the SPA index.

## Consequences

- Install, upgrade and rollback are a single file replacement. The shipped systemd unit runs it directly.
- API and UI versions can never drift apart.
- The UI must be built before a release build of the backend; development uses a separate dev server.
- The CSP can be strict (`script-src 'self'`) because every script is first-party and served from the same origin.
- Binary size grows with the UI; compression is applied at response time (`tower_http` compression layer).
- Serving the UI from the same process means TLS, caching and rate limiting at the edge must come from a reverse proxy; Moss does not terminate TLS.
