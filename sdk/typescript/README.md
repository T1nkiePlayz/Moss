# @moss/client (TypeScript)

Typed client for the [Moss](../../README.md) HTTP API, built on [openapi-fetch](https://openapi-ts.dev/openapi-fetch/).
Types in `src/schema.ts` are generated from [docs/openapi.frozen.json](../../docs/openapi.frozen.json). ESM, Node 20+
and browsers.

## Install

```sh
npm install @moss/client
```

From a checkout of the Moss repo: `cd sdk/typescript && npm install && npm run build`.

## Create an API token

In the Moss panel open **Settings** and create a token: give it a name and a scope (`read` allows `GET` only, `full`
allows everything the owning user can do) and an optional expiry. The token (`moss_pat_...`) is shown once. You can
also call `POST /tokens {name, scope, expires_in_days?}` from a signed-in session; a token cannot create tokens.
The client sends it as `Authorization: Bearer moss_pat_...`. All routes live under `/api/v1`, which the client adds.

## Examples

```ts
import { createMossClient } from "@moss/client";

const moss = createMossClient({ baseUrl: "http://localhost:8080", token: "moss_pat_..." });

// 1. List servers (GET /servers)
const { data: servers, error } = await moss.GET("/servers");
if (error) throw new Error(error.error?.message);
for (const s of servers ?? []) console.log(s.id, s.name, s.state);

// 2. Start a server (POST /servers/{id}/actions/{action})
await moss.POST("/servers/{id}/actions/{action}", {
  params: { path: { id: "SERVER_ID", action: "start" } },
});

// 3. Create a backup (POST /servers/{id}/backups, answers 202 with an operation)
const { data: op } = await moss.POST("/servers/{id}/backups", {
  params: { path: { id: "SERVER_ID" } },
  body: {},
});
console.log(op?.id, op?.state);
```

### Live events

`streamEvents` reads `GET /events` (Server-Sent Events) with `fetch`. It first gets a single-use ticket from
`POST /ticket`, which needs a `full` token. Events are `op`, `server`, `activity`, `update` and `hello`; `data` is
loosely typed.

```ts
import { streamEvents } from "@moss/client";

for await (const e of streamEvents({ baseUrl: "http://localhost:8080", token: "moss_pat_..." })) {
  console.log(e.event, e.data);
}
```

## Develop

```sh
npm run generate   # regenerate src/schema.ts (or run scripts/gen-sdk.sh from the repo root)
npm run build      # tsc to dist/
npm test           # typecheck plus vitest against a stubbed fetch
```
