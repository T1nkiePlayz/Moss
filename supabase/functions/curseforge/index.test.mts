// Exercises the edge function's handler under Node (>= 22.18, which strips types) with a fake Deno and fetch.
// Run: node supabase/functions/curseforge/index.test.mts
import assert from 'node:assert/strict';

let handler: (r: Request) => Promise<Response>;
const env: Record<string, string> = { CURSEFORGE_API_KEY: 'testkey', SUPABASE_URL: 'https://p.supabase.co', SUPABASE_ANON_KEY: 'anon' };
(globalThis as any).Deno = { env: { get: (k: string) => env[k] }, serve: (h: typeof handler) => { handler = h; } };
const upstreamKeys: (string | undefined)[] = [];
(globalThis as any).fetch = async (u: unknown, init: any) => {
  const url = String(u);
  if (url.endsWith('/auth/v1/user')) {
    const sub = JSON.parse(Buffer.from(init.headers.authorization.slice(7).split('.')[1], 'base64url').toString()).sub;
    return sub === 'forged' ? new Response('{}', { status: 401 }) : new Response(JSON.stringify({ id: sub }), { status: 200 });
  }
  upstreamKeys.push(init?.headers?.['x-api-key']);
  if (url.includes('/files/7')) return new Response(JSON.stringify({ data: { downloadUrl: null, fileLength: 10 } }));
  if (url.includes('/files/8')) return new Response(JSON.stringify({ data: { downloadUrl: 'https://evil.example/x.jar', fileLength: 10 } }));
  if (url.includes('/files/9')) return new Response(JSON.stringify({ data: { downloadUrl: 'https://edge.forgecdn.net/files/1/2/a.jar', fileLength: 3 } }));
  if (url.startsWith('https://edge.forgecdn.net')) return new Response('abc', { headers: { 'content-length': '3' } });
  return new Response(JSON.stringify({ data: [], pagination: {} }));
};
await import('./index.ts');

const jwt = (sub: string) => 'h.' + Buffer.from(JSON.stringify({ sub })).toString('base64url') + '.s';
const call = async (path: string, init: RequestInit = {}, sub = 'u1', auth = 'Bearer ' + jwt(sub)) => {
  const r = await handler(new Request('https://x.supabase.co/functions/v1/curseforge' + path, { ...init, headers: auth ? { authorization: auth } : {} }));
  return { status: r.status, text: await r.text(), cache: r.headers.get('cache-control') };
};
const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

// authentication: the public publishable key and forged tokens never reach CurseForge
assert.equal((await call('/v1/mods/5', {}, 'u', '')).status, 401);
assert.equal((await call('/v1/mods/5', {}, 'u', 'Bearer sb_publishable_abc')).status, 401);
assert.equal((await call('/v1/mods/5', {}, 'forged')).status, 401);
assert.equal(upstreamKeys.length, 0);
assert.equal(JSON.parse((await call('/health')).text).configured, true);
await pause(1100);

// validation: fixed game, allowed classes, bounded paging, no path tricks
assert.equal((await call('/v1/mods/search?classId=5&searchFilter=worldedit&gameVersion=1.21.1&pageSize=99')).status, 200);
assert.equal((await call('/v1/mods/search?classId=999')).status, 400);
assert.equal((await call('/v1/mods/search?classId=6&modLoaderType=4')).status, 400);
assert.equal((await call('/v1/mods/12/../../secret')).status, 404);
assert.equal((await call('/v1/mods', { method: 'POST', body: JSON.stringify({ modIds: ['x'] }) })).status, 400);
assert.equal((await call('/v1/mods', { method: 'POST', body: JSON.stringify({ modIds: [1, 2] }) })).status, 200);
assert.equal((await call('/v1/mods/5')).cache, 'no-store');
assert.ok(upstreamKeys.length > 0 && upstreamKeys.every((k) => k === 'testkey'));
await pause(1100);

// batch lookups used by modpack import and identify
const post = (path: string, body: unknown) => call(path, { method: 'POST', body: JSON.stringify(body) });
assert.equal((await post('/v1/mods/files', { fileIds: [1, 2] })).status, 200);
assert.equal((await post('/v1/mods/files', { fileIds: Array.from({ length: 101 }, (_, i) => i + 1) })).status, 400);
assert.equal((await post('/v1/fingerprints/432', { fingerprints: [0, 4294967295] })).status, 200);
assert.equal((await post('/v1/fingerprints/432', { fingerprints: [-1] })).status, 400);
assert.equal((await post('/v1/fingerprints/999', { fingerprints: [1] })).status, 404);
await pause(1100);

// downloads: distribution-disabled files are refused, only the CDN host is contacted, bytes are streamed
assert.equal((await call('/download/5/7')).status, 403);
assert.equal((await call('/download/5/8')).status, 502);
const d = await call('/download/5/9');
assert.deepEqual([d.status, d.text], [200, 'abc']);
await pause(1100);

// per-user rate limit: 40 calls a minute, then 429
let limited = 0, ok = 0;
for (let i = 0; i < 45; i++) {
  if (i % 6 === 0) await pause(1100); // stay under the global per-second cap
  (await call('/v1/mods/5', {}, 'u2')).status === 429 ? limited++ : ok++;
}
assert.equal(ok, 40);
assert.equal(limited, 5);
console.log('curseforge edge function: all checks passed');
