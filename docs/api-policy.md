# API stability policy

This is the contract for anything that talks to Moss over HTTP: scripts, dashboards, bots, your own tools, and the web UI itself. The API lives under `/api/v1`.

## What is stable

Everything documented in [openapi.json](openapi.json) and [api.md](api.md) under `/api/v1`: paths, methods, parameters, request bodies, response fields, enum values, status codes and error codes (`error.code`). A snapshot of the documented API is committed as `openapi.frozen.json`, and the test suite fails if the current API breaks it.

Not covered: the web UI's own behaviour, log output, the layout of the data directory and database (use the API, backups and `moss recover`), the contents of `X-Moss-*` webhook payload `data` fields beyond what [webhooks.md](webhooks.md) lists, undocumented routes, and anything marked experimental in the docs. The `/healthz` route is stable but outside `/api/v1`.

## What counts as breaking

Within `v1` we do not:

- remove or rename a path, method, query or path parameter, request field, or response field;
- change the type of a field in a way that would surprise an existing client (a response field gaining `null` or changing from number to string; a request field that used to accept a type refusing it);
- remove a value from an enum, or stop accepting it;
- make an optional request field or parameter required, or add a new required one;
- remove a success status code or change its meaning;
- change the meaning of an existing field, error code or permission so that a previously working call does something else;
- make an existing call need a broader permission than before.

## What is allowed in `v1` (additive)

- new routes, methods, optional parameters and optional request fields;
- new response fields: **clients must ignore fields they do not know**;
- new enum values in responses (clients must tolerate values they do not know, for example a new operation `state` or notification `kind`);
- new error codes on new failure modes, and new event types for webhooks (subscribe by name, so existing subscriptions are unaffected);
- stricter validation of values that never worked (a bug fix), documented in the release notes;
- performance, ordering of unordered lists, wording of human-readable `message` text (match on `error.code`, never on `message`).

## Deprecation

A feature that has to go is first marked deprecated in `api.md` and `openapi.json` (`"deprecated": true`) and in the release notes, and keeps working for at least **two minor releases and 90 days**, whichever is longer. Where practical, responses to deprecated calls carry a `Deprecation: true` header. Removal only ever happens in a new major API version.

## Versioning

The version is the path prefix. A breaking change means a new prefix (`/api/v2`) that is served **alongside** `/api/v1` for at least the deprecation window above. The Moss release number is separate from the API version.

## How it is enforced (for contributors)

- `tests/openapi.rs` checks that every route is documented.
- `tests/api_compat.rs` compares `docs/openapi.json` with `docs/openapi.frozen.json` and fails on any removed path or method, removed or retyped response field, newly required request field or parameter, removed enum value or removed success status.
- After an **additive** change the test still passes. When you want the new routes and fields protected too, refresh the snapshot with `node scripts/api-freeze.mjs` and commit it with your change. Refreshing is also how a reviewed, intended break is recorded (`--allow-breaking`); do that only together with a new API version or a documented pre-release exception.
