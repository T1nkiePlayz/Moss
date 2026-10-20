# Security review

Internal review of the Moss code base, written by the people building it. **It is not a substitute for an external audit.** It records what was looked at, what was found, what was fixed (with the test that pins the fix) and what was accepted and why. Read it together with the [threat model](threat-model.md).

Scope: the Rust controller (`src/`), the embedded UI's interaction with it, the container backend, the CurseForge edge function boundary and the CI definitions. Method: reading the code against the gaps listed in the threat model, grepping for risky patterns (dynamic SQL, `unwrap`/`expect`/indexing on request data, file creation modes, header construction, logging of secrets), and exercising the fixes with tests against the real binary. Not done: fuzzing, dynamic analysis, review of third-party crates beyond the automated advisory scan (see [CI audit](../.github/workflows/audit.yml)), review of the Supabase project configuration, and anything that needs a real Docker or Podman (none was available).

## Findings

| # | Area | Finding | Severity | Status | Test |
|---|---|---|---|---|---|
| 1 | WebSocket / SSE | The console WebSocket upgrade and the event stream did not check `Origin`. Tickets are single-use and only obtainable with a bearer token, so exploitation needed a leaked ticket, but a cross-site page could use one it obtained any other way. | Medium | Fixed: `src/api/origin.rs`. A request with an `Origin` header must match `Host` (or `X-Forwarded-Host` when `trust_proxy` is on); no `Origin` (curl, scripts) passes. The check runs before the ticket is consumed. | `tests/security.rs::streams_refuse_cross_origin_requests_and_keep_the_ticket`, unit tests in `origin.rs` |
| 2 | File API | `.moss` protection compared the raw path string, so `./.moss/...` (and `.MOSS` on case-insensitive filesystems such as macOS) reached controller metadata through read, write, rename, upload and download. | High (for anyone with `files.read`/`files.write`) | Fixed: protection is decided on path components, case-insensitively. | `tests/security.rs::file_api_cannot_reach_controller_metadata_...`, `files::tests::controller_metadata_is_protected` |
| 3 | File API | Listing `?path=.moss` was not blocked (only hidden from the root listing). | Medium | Fixed: listing a protected path is refused. | same |
| 4 | File API | `DELETE /files?path=.` (or `./`) resolved to the server root and removed the whole server directory including `.moss`. | High (data loss by a `manager`) | Fixed: any spelling of the root is refused. | same |
| 5 | Import | The zip importer summed the size declared in each entry header, not the bytes written, and had no entry-count cap. | Medium | Fixed: real bytes are counted, 1,000,000 entries max, and uploaded archives (zip and tar.zst) may expand to at most 64 GiB (the upload itself is limited to 512 MiB). Folder imports keep their 200 GiB limit. | `importer::tests::extraction_counts_real_bytes_...`, `archive::tests::extraction_never_writes_controller_metadata` |
| 6 | Import / restore | An archive containing a top-level `.moss` would have placed attacker-chosen controller metadata in a server directory (upload import) or restored it (tampered remote backup that still matched its recorded hash is not possible, but a user-supplied `tar.zst` is). | Low (import is admin only) | Fixed: `.moss` entries are dropped from imported archives and never extracted from `tar.zst`. | same |
| 7 | Data at rest | The data directory was created with the process umask (typically 0755), so on a multi-user host other users could read the database, backups and temporary uploads. The support bundle zip was written to `tmp/` with default permissions. | Medium on shared hosts | Fixed: a data directory Moss creates is 0700; the scratch directory `tmp/` is always 0700; the support bundle is created 0600. An existing data directory keeps the mode its owner chose (the threat model recommends 0700). | `tests/security.rs::a_new_data_directory_and_the_scratch_directory_are_private` |
| 8 | Webhook SSRF | The address filter treated the local-use NAT64 prefix `64:ff9b:1::/48` as public. | Low | Fixed. | `webhook::tests::local_use_nat64_prefix_is_not_public` |
| 9 | Container network | Container servers could always reach the internet and the local network. | Design gap | Mitigated: optional `game-only` network (see [architecture](architecture.md)); Docker only; never silently downgraded. | `container::stub_tests`, `lifecycle::ctr::tests`, `tests/container.rs` |

## Reviewed and found sound

- **SQL.** All values are bound parameters. The only formatted SQL is column lists and fixed table names from constants (`COLS`, `SEL`, migrations tests); no request data reaches a SQL string.
- **Header injection.** Download file names are either generated from validated ids or reduced to printable ASCII without quotes; no request header values are echoed.
- **Error messages.** `ApiError` from an internal failure returns `internal error` and logs the detail server-side; I/O `NotFound`/`PermissionDenied` map to generic messages. Import and webhook messages name hosts and paths the caller supplied, not controller internals.
- **Panics on request data.** All non-test `unwrap`/`expect`/indexing sites were checked: the remaining ones operate on constants, database values written by Moss, values validated earlier in the same handler (for example `sched::create` after `validate`), or `Regex::new` on literals.
- **Modpack archives** (`content/modpack/zipsafe.rs`): per-entry and total size caps, entry cap, header size enforced while reading, `safe_join` for every target, symlinks and `.moss` refused.
- **Blueprints**: imported JSON is reduced to known fields; property names and values are restricted, secret-looking and network-binding keys are dropped; a blueprint cannot name a path.
- **Config editors**: the editable files are a fixed catalogue; world folder names cannot start with `.` or contain separators; writes go through `safe_join`, undo snapshots and a 1 MiB limit; secret fields are never returned.
- **Auth throttling**: failed credentials per client address, ticket cap, setup claim limiter; the limiter map is swept.
- **Secrets files**: `backup-secrets.json` and `setup-token` are created 0600 with `create_new`; support-bundle redaction covers webhooks, JWTs, API tokens, bearer headers, key/value secrets, emails and long hex or base64 runs.
- **Webhook and S3 outbound**: HTTPS only, destination resolved once and pinned, no redirects, private ranges refused unless explicitly allowed.

## Accepted risks

- **Same-user game servers** can read everything Moss can (database, other servers, secrets) unless container isolation is used; containers share the kernel and the server directory is writable by the game. See the threat model.
- **Check-then-use races** in `safe_join` against a process that can create symlinks inside the server directory (the game server itself). Closing it needs `openat2`/`O_NOFOLLOW` descriptor-relative handling throughout; not done.
- **CSP `connect-src`** still lists `ws:` and `wss:` for any host because older WebKit versions do not apply `'self'` to WebSocket URLs. A script injection (none known) could exfiltrate over a WebSocket. Tightening it is planned once those browsers are out of support.
- **`style-src 'unsafe-inline'`** remains for the UI framework's inline styles.
- **Webhook resolution oracle.** The error text for a refused target names the non-public address it resolved to; only administrators can configure webhooks.
- **YAML parsing** of an edited config file (1 MiB cap) uses a general parser; pathological alias expansion is bounded by the library's own limits, not by Moss.
- **Folder import** reads small metadata files (`server.properties`, `velocity.toml`) without a size cap; the path is administrator-supplied and symlinks are not copied.
- **Container game-only network** relies on Docker's behaviour with IP masquerading off. It was verified against a command-recording stub only; the host and other containers on the same bridge stay reachable from the server. Documented in the architecture notes.
- **No per-user request rate limit** and no concurrency limit: rely on the reverse proxy.
- **Staging port range, ticket cap and outstanding operations** are bounded counts, not quotas per user.

## Reporting

See [SECURITY.md](../SECURITY.md).
