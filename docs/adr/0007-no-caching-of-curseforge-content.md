# 0007. No caching of CurseForge content, and the edge-function proxy

Status: Accepted

## Context

CurseForge's third-party API terms forbid storing or caching API data and require a secret key that must not be shared. Modrinth, by contrast, publishes a plain rate limit and Moss caches its read responses briefly. A desktop-style tool that is installed by many people cannot ship a CurseForge key inside the binary.

## Decision

- Moss never caches CurseForge responses (`src/providers/curseforge.rs`). It keeps only what an installed item needs: ids, title, version label and file name. Update checks use one batch request. All CurseForge calls pass through one limiter (about 2.8 calls a second, 4 in flight); a 429 pauses every call for its `Retry-After`.
- By default the key lives in a Supabase Edge Function (`supabase/functions/curseforge`). Moss forwards the signed-in user's token and the function verifies it with Supabase Auth. The function allows only fixed routes, game id and class ids, sets `Cache-Control: no-store`, rate limits per user and globally, and streams downloads itself because the CDN needs the key. It reads the download URL from CurseForge's own file record and permits only `forgecdn.net` hosts.
- Alternatively an operator sets `CURSEFORGE_API_KEY` locally and Moss calls the API directly, sending the key only in `x-api-key` and never following redirects with it.
- Files whose authors disabled third-party distribution are never worked around; Moss links to the project page instead.

## Consequences

- Every CurseForge screen costs live requests; the UI is slower and uses quota that a cache would save.
- The key stays out of every install in the proxy setup, at the price of a Supabase dependency and an extra hop.
- Whether a shared function fits a key's terms is the key holder's decision (see `docs/curseforge.md`).
- The edge function's rate limits are in memory per worker; they can only loosen when a worker recycles.
- Modrinth reads are cached for 90 seconds (`src/content/modrinth.rs`), a deliberate asymmetry.
