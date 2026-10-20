// Moss <-> CurseForge proxy (Supabase Edge Function).
//
// Keeps the CurseForge API key (secret CURSEFORGE_API_KEY) out of every Moss install. Moss forwards the signed-in
// user's Supabase access token, which this function confirms with Supabase Auth before doing anything (the public
// publishable key alone is not enough).
//
// Rules this function follows, from the CurseForge API docs and 3rd Party API terms:
//  * Allow-list only: fixed game (Minecraft, 432), fixed endpoints, validated parameters. It is not an open proxy.
//  * No caching or storing of API data (terms section 3.1): every response is `Cache-Control: no-store`.
//  * Rate limiting protects the shared quota: per-user and global limits, Retry-After on 429s, upstream 429 passed through.
//  * Downloads: the CDN requires the key in `x-api-key`, so files are streamed through here. The target URL is read from
//    CurseForge's own file record (never from the caller), must be https on forgecdn.net, and files whose authors
//    disabled third-party distribution (downloadUrl null) are refused rather than worked around.
//
// Routes (relative to /functions/v1/curseforge), mirroring the CurseForge API:
//   GET  /v1/mods/search            GET /v1/mods/{id}            GET /v1/mods/{id}/files
//   GET  /v1/mods/{id}/files/{fid}  POST /v1/mods {modIds}       GET /download/{id}/{fid}
//   POST /v1/mods/files {fileIds}   POST /v1/fingerprints/432 {fingerprints}
//   GET  /health                    (no key use; reports whether the secret is configured)

const API = 'https://api.curseforge.com';
const GAME_ID = '432';
const CLASS_IDS = new Set(['5', '6', '6945']); // Bukkit plugins, mods, data packs
const MAX_DOWNLOAD = 512 * 1024 * 1024;
const CDN_HOSTS = ['edge.forgecdn.net', 'mediafilez.forgecdn.net', 'media.forgecdn.net'];

const USER_PER_MIN = 40; // API calls per user per minute
const DL_PER_MIN = 6; // downloads per user per minute
const DL_CONCURRENT = 2; // concurrent downloads per user
const GLOBAL_PER_SEC = 8; // upstream calls per second per worker (CurseForge publishes no figure; stay modest)

const json = (body: unknown, status = 200, extra: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store', ...extra } });
const err = (status: number, code: string, message: string, extra: Record<string, string> = {}) => json({ error: { code, message } }, status, extra);

// ---- rate limiting (in memory; a worker may be recycled, which only makes the limits looser, never stricter) ----
const hits = new Map<string, number[]>();
// download slots: start times, expiring after the 150 s idle limit so an abandoned stream can never leak a slot
const dlSlots = new Map<string, number[]>();
const slotsOf = (u: string) => { const a = (dlSlots.get(u) ?? []).filter((t) => Date.now() - t < 150_000); dlSlots.set(u, a); return a; };
let gWindow = Math.floor(Date.now() / 1000);
let gCount = 0;
let blockedUntil = 0; // set when CurseForge itself answers 429

function allow(key: string, limit: number): number {
  const now = Date.now();
  const arr = (hits.get(key) ?? []).filter((t) => now - t < 60_000);
  if (arr.length >= limit) { hits.set(key, arr); return Math.ceil((60_000 - (now - arr[0])) / 1000); }
  arr.push(now);
  hits.set(key, arr);
  if (hits.size > 5000) for (const [k, v] of hits) if (!v.some((t) => now - t < 60_000)) hits.delete(k);
  return 0;
}
function globalWait(): number {
  const now = Date.now();
  if (now < blockedUntil) return Math.ceil((blockedUntil - now) / 1000);
  const s = Math.floor(now / 1000);
  if (s !== gWindow) { gWindow = s; gCount = 0; }
  return ++gCount > GLOBAL_PER_SEC ? 1 : 0;
}

// ---- authentication ----
// The platform check (verify_jwt) also admits the public publishable key, which ships in the Moss repository, so it is not
// enough on its own: the caller must hold a real signed-in user's access token. Supabase Auth is the authority; positive
// answers are remembered for 60 s so a burst of calls costs one lookup.
const authCache = new Map<string, { sub: string; until: number }>();

async function userOf(req: Request): Promise<string | null> {
  const token = (req.headers.get('authorization') ?? '').replace(/^Bearer\s+/i, '').trim();
  if (token.split('.').length !== 3 || token.length > 8192) return null; // publishable/secret keys are not JWTs
  const now = Date.now();
  const hit = authCache.get(token);
  if (hit && hit.until > now) return hit.sub;
  const base = Deno.env.get('SUPABASE_URL') ?? '';
  let anon = Deno.env.get('SUPABASE_ANON_KEY') ?? '';
  if (!anon) { try { anon = JSON.parse(Deno.env.get('SUPABASE_PUBLISHABLE_KEYS') ?? '{}').default ?? ''; } catch { /* none */ } }
  if (!base || !anon) return null;
  try {
    const r = await fetch(`${base}/auth/v1/user`, { headers: { apikey: anon, authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(8_000) });
    if (!r.ok) { await r.body?.cancel(); return null; }
    const u = await r.json();
    if (typeof u?.id !== 'string' || !u.id || u.is_anonymous === true) return null;
    authCache.set(token, { sub: u.id, until: now + 60_000 });
    if (authCache.size > 2000) for (const [k, v] of authCache) if (v.until <= now) authCache.delete(k);
    return u.id;
  } catch { return null; }
}

// ---- parameter validation ----
const int = (v: string | null, min: number, max: number): string | null => {
  if (v === null || !/^\d{1,9}$/.test(v)) return null;
  const n = Number(v);
  return n >= min && n <= max ? String(n) : null;
};
const VERSION = /^[0-9]{1,2}\.[0-9]{1,2}(\.[0-9]{1,2})?(-[A-Za-z0-9]{1,12})?$/;

function searchParams(q: URLSearchParams): URLSearchParams | string {
  const out = new URLSearchParams({ gameId: GAME_ID });
  const cls = q.get('classId');
  if (!cls || !CLASS_IDS.has(cls)) return 'classId must be one of ' + [...CLASS_IDS].join(', ');
  out.set('classId', cls);
  const f = q.get('searchFilter');
  if (f) out.set('searchFilter', f.slice(0, 100));
  const gv = q.get('gameVersion');
  if (gv) { if (!VERSION.test(gv)) return 'bad gameVersion'; out.set('gameVersion', gv); }
  const ml = int(q.get('modLoaderType'), 0, 6);
  if (ml && ml !== '0') { if (!gv) return 'modLoaderType needs gameVersion'; out.set('modLoaderType', ml); }
  out.set('sortField', int(q.get('sortField'), 1, 12) ?? '2');
  out.set('sortOrder', q.get('sortOrder') === 'asc' ? 'asc' : 'desc');
  const size = Number(int(q.get('pageSize'), 1, 50) ?? 20);
  const index = Number(int(q.get('index'), 0, 10000) ?? 0);
  if (index + size > 10000) return 'index + pageSize must be at most 10000';
  out.set('pageSize', String(size));
  out.set('index', String(index));
  return out;
}

function filesParams(q: URLSearchParams): URLSearchParams | string {
  const out = new URLSearchParams();
  const gv = q.get('gameVersion');
  if (gv) { if (!VERSION.test(gv)) return 'bad gameVersion'; out.set('gameVersion', gv); }
  const ml = int(q.get('modLoaderType'), 0, 6);
  if (ml && ml !== '0') out.set('modLoaderType', ml);
  const size = Number(int(q.get('pageSize'), 1, 50) ?? 50);
  const index = Number(int(q.get('index'), 0, 10000) ?? 0);
  if (index + size > 10000) return 'index + pageSize must be at most 10000';
  out.set('pageSize', String(size));
  out.set('index', String(index));
  return out;
}

async function upstream(key: string, path: string, init: RequestInit = {}): Promise<Response> {
  return await fetch(API + path, {
    ...init,
    headers: { accept: 'application/json', 'x-api-key': key, ...(init.headers ?? {}) },
    signal: AbortSignal.timeout(20_000),
  });
}

/** Turn an upstream answer into ours; never forward upstream headers (no caching, no key echo). */
async function relay(r: Response): Promise<Response> {
  if (r.status === 429) {
    const wait = Math.min(120, Math.max(1, Number(r.headers.get('retry-after')) || 30));
    blockedUntil = Date.now() + wait * 1000;
    await r.body?.cancel();
    return err(429, 'upstream_rate_limited', 'CurseForge is rate limiting this integration; try again shortly.', { 'retry-after': String(wait) });
  }
  if (r.status === 401 || r.status === 403) {
    await r.body?.cancel();
    return err(502, 'upstream_rejected', 'CurseForge rejected the request. The API key may be invalid, or the quota was exceeded.');
  }
  if (r.status === 404) { await r.body?.cancel(); return err(404, 'not_found', 'Not found on CurseForge.'); }
  if (!r.ok) { await r.body?.cancel(); return err(502, 'upstream_error', `CurseForge answered ${r.status}.`); }
  return json(await r.json());
}

async function download(key: string, modId: string, fileId: string, release: () => void): Promise<Response> {
  const meta = await upstream(key, `/v1/mods/${modId}/files/${fileId}`);
  if (!meta.ok) return await relay(meta);
  const file = (await meta.json()).data;
  const url: string | null = file?.downloadUrl ?? null;
  if (!url) return err(403, 'distribution_disabled', 'The author disabled third-party downloads for this file. Download it from CurseForge and upload it manually.');
  let u: URL;
  try { u = new URL(url); } catch { return err(502, 'bad_file_record', 'CurseForge returned an unusable download URL.'); }
  if (u.protocol !== 'https:' || !CDN_HOSTS.includes(u.hostname)) return err(502, 'bad_file_host', 'Refusing to download from an unexpected host.');
  if (Number(file.fileLength) > MAX_DOWNLOAD) return err(413, 'too_large', 'File is larger than the 512 MiB limit.');
  const r = await fetch(u, { headers: { 'x-api-key': key }, redirect: 'manual', signal: AbortSignal.timeout(140_000) });
  if (r.status === 429) return await relay(r);
  if (!r.ok || !r.body) { await r.body?.cancel(); return err(502, 'cdn_error', `CurseForge CDN answered ${r.status}.`); }
  const len = Number(r.headers.get('content-length'));
  if (len > MAX_DOWNLOAD) { await r.body.cancel(); return err(413, 'too_large', 'File is larger than the 512 MiB limit.'); }
  const headers: Record<string, string> = { 'content-type': 'application/octet-stream', 'cache-control': 'no-store' };
  if (len > 0) headers['content-length'] = String(len);
  return new Response(r.body.pipeThrough(new TransformStream({ flush: release })), { headers });
}

Deno.serve(async (req: Request): Promise<Response> => {
  try {
    const url = new URL(req.url);
    const path = url.pathname.replace(/^\/(functions\/v1\/)?curseforge/, '') || '/';
    const key = Deno.env.get('CURSEFORGE_API_KEY') ?? '';

    if (path === '/health') return json({ ok: true, configured: key.length > 0 });
    if (!key) return err(503, 'not_configured', 'The CURSEFORGE_API_KEY secret has not been set on this Supabase project.');

    const user = await userOf(req);
    if (!user) return err(401, 'unauthorized', 'Sign in to Moss with your account to use CurseForge.');
    const isDownload = path.startsWith('/download/');
    const wait = allow(user + (isDownload ? ':dl' : ''), isDownload ? DL_PER_MIN : USER_PER_MIN);
    if (wait) return err(429, 'rate_limited', 'Too many CurseForge requests; slow down.', { 'retry-after': String(wait) });
    const gw = globalWait();
    if (gw) return err(429, 'busy', 'CurseForge access is busy; try again shortly.', { 'retry-after': String(gw) });

    let m: RegExpMatchArray | null;
    if (req.method === 'GET' && path === '/v1/mods/search') {
      const p = searchParams(url.searchParams);
      if (typeof p === 'string') return err(400, 'bad_request', p);
      return await relay(await upstream(key, `/v1/mods/search?${p}`));
    }
    if (req.method === 'POST' && path === '/v1/mods') {
      const body = await req.json().catch(() => null);
      const ids = Array.isArray(body?.modIds) ? body.modIds : null;
      if (!ids || ids.length === 0 || ids.length > 50 || !ids.every((n: unknown) => Number.isInteger(n) && (n as number) > 0 && (n as number) < 2 ** 31)) return err(400, 'bad_request', 'modIds must be 1 to 50 positive integers.');
      return await relay(await upstream(key, '/v1/mods', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ modIds: ids, filterPcOnly: true }) }));
    }
    if (req.method === 'POST' && path === '/v1/mods/files') {
      const body = await req.json().catch(() => null);
      const ids = Array.isArray(body?.fileIds) ? body.fileIds : null;
      if (!ids || ids.length === 0 || ids.length > 100 || !ids.every((n: unknown) => Number.isInteger(n) && (n as number) > 0 && (n as number) < 2 ** 31)) return err(400, 'bad_request', 'fileIds must be 1 to 100 positive integers.');
      return await relay(await upstream(key, '/v1/mods/files', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ fileIds: ids }) }));
    }
    if (req.method === 'POST' && path === `/v1/fingerprints/${GAME_ID}`) {
      const body = await req.json().catch(() => null);
      const fps = Array.isArray(body?.fingerprints) ? body.fingerprints : null;
      if (!fps || fps.length === 0 || fps.length > 200 || !fps.every((n: unknown) => Number.isInteger(n) && (n as number) >= 0 && (n as number) < 2 ** 32)) return err(400, 'bad_request', 'fingerprints must be 1 to 200 unsigned 32-bit integers.');
      return await relay(await upstream(key, `/v1/fingerprints/${GAME_ID}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ fingerprints: fps }) }));
    }
    if (req.method === 'GET' && (m = path.match(/^\/v1\/mods\/(\d{1,10})$/))) return await relay(await upstream(key, `/v1/mods/${m[1]}`));
    if (req.method === 'GET' && (m = path.match(/^\/v1\/mods\/(\d{1,10})\/files$/))) {
      const p = filesParams(url.searchParams);
      if (typeof p === 'string') return err(400, 'bad_request', p);
      return await relay(await upstream(key, `/v1/mods/${m[1]}/files?${p}`));
    }
    if (req.method === 'GET' && (m = path.match(/^\/v1\/mods\/(\d{1,10})\/files\/(\d{1,10})$/))) return await relay(await upstream(key, `/v1/mods/${m[1]}/files/${m[2]}`));
    if (req.method === 'GET' && (m = path.match(/^\/download\/(\d{1,10})\/(\d{1,10})$/))) {
      if (slotsOf(user).length >= DL_CONCURRENT) return err(429, 'too_many_downloads', 'Too many downloads in progress.', { 'retry-after': '10' });
      const t = Date.now();
      dlSlots.get(user)!.push(t);
      const release = () => { const a = dlSlots.get(user); const i = a ? a.indexOf(t) : -1; if (a && i >= 0) a.splice(i, 1); };
      let res: Response;
      try { res = await download(key, m[1], m[2], release); } catch (e) { release(); throw e; }
      if (!res.ok) release(); // only a successful stream keeps its slot until the body ends
      return res;
    }
    return err(404, 'unknown_route', 'No such route.');
  } catch (e) {
    console.error('curseforge proxy failure', e instanceof Error ? e.message : String(e));
    return err(502, 'proxy_error', 'The CurseForge proxy failed.');
  }
});
