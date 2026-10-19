# CurseForge

Moss can browse and install plugins, mods and datapacks from CurseForge next to Modrinth. A provider switch appears in the content catalogue once CurseForge is available. Installed items remember where they came from, and updates are checked against the same provider.

CurseForge API access needs a key, and the key must stay secret. Moss supports two ways to supply it.

## Option A: the Supabase edge function (default for shared deployments)

The repository ships an Edge Function, [`supabase/functions/curseforge`](../supabase/functions/curseforge/index.ts), that holds the key so it never reaches any Moss install. Moss forwards the signed-in user's access token and the function confirms it with Supabase Auth before doing anything. The publishable key alone is not enough.

1. Get a CurseForge API key (see *Terms* below).
2. In the Supabase dashboard open **Edge Functions → Secrets** and add `CURSEFORGE_API_KEY` with your key. Via the CLI: `supabase secrets set CURSEFORGE_API_KEY=...`.
3. Deploy the function if it is not already: `supabase functions deploy curseforge` (it ships with `verify_jwt = true` in `supabase/config.toml`).
4. Restart Moss. In supabase auth mode it finds the function at `<SUPABASE_URL>/functions/v1/curseforge` on its own. `moss doctor` prints which route is active.

Sanity check: `GET <SUPABASE_URL>/functions/v1/curseforge/health` with your publishable key in the `apikey` header returns `{"ok":true,"configured":true}` once the secret is set.

## Option B: your own key, directly

Set `CURSEFORGE_API_KEY` in `.env.local` (never in `.env`, which is committed). Moss then calls `api.curseforge.com` itself. This also works in dev mode.

`MOSS_CURSEFORGE_PROXY=off` turns CurseForge off entirely; `MOSS_CURSEFORGE_PROXY=<url>` points at a different edge function URL.

## How it behaves

- **Compatibility.** Plugins for Paper, Purpur and Folia (Bukkit plugins), mods for Fabric, datapacks for Vanilla. Velocity has no CurseForge content. Searches and file lists are filtered by the server's game version and loader. CurseForge does not say whether a mod runs on servers, so Moss skips files tagged client-only and warns about the rest.
- **Author opt-outs are respected.** Projects whose authors disabled third-party distribution are hidden from search. If a specific file has no download URL, Moss refuses to install it, links to the project page, and never guesses CDN URLs. Download it from CurseForge and upload it in **Files**.
- **Dependencies.** Required dependencies are resolved into the change plan, like on Modrinth. Everything installs through the same plan, undo snapshot and checksum verification (SHA-1 from CurseForge).
- **Downloads.** The CurseForge CDN requires the API key on file downloads. With the edge function, files stream through it by id (the function reads the real URL from CurseForge, never from the caller, and only talks to `forgecdn.net`). With your own key, Moss sends the key in the `x-api-key` header and never follows redirects with it.

## Rate limiting and caching

CurseForge publishes no request figures, so everything is conservative:

- **Moss:** one limiter for the whole instance, at most about 2.8 calls a second and 4 in flight. A `429` from CurseForge pauses every CurseForge call for its `Retry-After` and the UI shows the message. Update checks use one batch request for all installed CurseForge items.
- **Edge function:** 40 calls a minute per user, 6 downloads a minute and 2 at once per user, at most 8 upstream calls a second per worker, and an upstream `429` is passed on with its `Retry-After`.
- **No caching.** The 3rd Party API terms forbid saving or caching API data, so neither Moss nor the function caches CurseForge responses (all responses are `Cache-Control: no-store`). Moss keeps only what an installed item needs: project and file ids, title, version label and file name.

## Terms

Third-party keys come from an application to CurseForge for Studios (Overwolf) and are covered by their *3rd Party API Terms*. Read them before you ship a key to other people. In particular: the key is not transferable and may not be shared with third parties, the terms forbid concealing identity or location behind a proxy, and usage is subject to a quota Overwolf can change. Moss does not decide whether running a shared edge function fits your key's terms; that is your call as the key holder. A personal key falls under the CurseForge Core terms instead, and Option B (everyone uses their own key) avoids sharing a key at all.
