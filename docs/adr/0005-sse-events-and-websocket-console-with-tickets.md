# 0005. SSE for events, WebSocket for the console, single-use tickets

Status: Accepted

## Context

The UI needs live state (operations, server status, notifications) and an interactive console. Browsers cannot set an `Authorization` header on `EventSource` or `WebSocket`, and putting a long-lived token in a URL leaks it into logs and history.

## Decision

- State changes are pushed over one Server-Sent Events stream, `/api/v1/events` (`src/api/activity.rs`). It is one-directional, reconnects by itself, and sends `resync` when a subscriber lags.
- The console is a WebSocket per viewer, `/api/v1/servers/{id}/console` (`src/console.rs`), because it needs input. One log tailer per server fans out to any number of viewers.
- Both authenticate with a ticket from `POST /api/v1/ticket`, a random value (two UUIDv4 concatenated) held in memory, valid 30 seconds, removed on first use (`take_ticket` in `src/api/mod.rs`), with at most 2000 outstanding. The ticket maps to the user, whose server visibility and console permissions are then applied.
- The event stream filters each event by the servers the user can see and by admin-only notification kinds.

## Consequences

- No bearer token ever appears in a URL; a leaked URL is useless after use or after 30 seconds.
- Tickets are process-local; a Moss restart invalidates them, and clients simply request a new one.
- Because the permission set is fixed when the ticket is used, a connection stays valid after a grant is revoked until it ends.
- Unlike REST calls, stream connections are not covered by the failed-auth throttle in the same way; the ticket cap bounds abuse.
- SSE gives one-way, proxy-friendly delivery; the reverse proxy must not buffer it.
