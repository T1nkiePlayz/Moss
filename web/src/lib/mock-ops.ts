// Mock mode for the configuration editors (WP ops-misc). Returns NONE for routes it does not own.
import type * as A from './api';
import { ApiError } from './api';
import { NONE } from './mock-content';

export interface OpsCtx { uid: () => string; now: () => string; fs: Record<string, Record<string, string>>; pushUndo: (sid: string, title: string, files: string[]) => string; log: (a: string, t: string, d?: string) => void }

type F = { id: string; title: string; path: string };
const PAPER_GLOBAL = `# Paper global configuration
_version: 29

chunk-loading-basic:
  # Maximum chunks sent to a player per tick.
  player-max-chunk-send-rate: 100.0
  player-max-chunk-load-rate: 100.0 # -1 means no limit
messages:
  no-permission: <red>I'm sorry, but you do not have permission to perform this command.
misc:
  max-joins-per-tick: 3
  fix-entity-position-desync: true
console:
  enable-brigadier-highlighting: true
`;
const SPIGOT = `# Spigot configuration
settings:
  bungeecord: false
  moved-wrongly-threshold: 0.0625
commands:
  log: true
  tab-complete: 0
`;
const FIELDS: Record<string, Omit<A.ConfigField, 'present' | 'value'>[]> = {
  'paper-global': [
    { key: 'chunk-loading-basic.player-max-chunk-send-rate', type: 'float', default: 100, group: 'Chunk loading', description: 'Chunks sent to each player per tick. Lower it if players on slow connections time out.', min: 1, max: 1000, options: [], restart: false, secret: false, danger: false, mixed: false },
    { key: 'chunk-loading-basic.player-max-chunk-load-rate', type: 'float', default: 100, group: 'Chunk loading', description: 'Chunks loaded for each player per tick. -1 removes the limit.', min: -1, max: 1000, options: [], restart: false, secret: false, danger: false, mixed: false },
    { key: 'misc.max-joins-per-tick', type: 'int', default: 3, group: 'Misc', description: 'How many players may finish joining in one tick. Raise it if reconnect storms are slow.', min: 1, max: 100, options: [], restart: false, secret: false, danger: false, mixed: false },
    { key: 'misc.fix-entity-position-desync', type: 'bool', default: true, group: 'Misc', description: 'Corrects entities that appear in the wrong place for players.', min: null, max: null, options: [], restart: false, secret: false, danger: false, mixed: false },
    { key: 'console.enable-brigadier-highlighting', type: 'bool', default: true, group: 'Console', description: 'Colours commands typed in the server console.', min: null, max: null, options: [], restart: true, secret: false, danger: false, mixed: false },
  ],
  spigot: [
    { key: 'settings.bungeecord', type: 'bool', default: false, group: 'Network', description: 'Turn on only when this server sits behind a proxy that forwards player identity. Leaving it on without a proxy lets anyone impersonate players.', min: null, max: null, options: [], restart: true, secret: false, danger: true, mixed: false },
    { key: 'commands.tab-complete', type: 'int', default: 0, group: 'Commands', description: 'Characters a player must type before tab completion suggests commands.', min: 0, max: 64, options: [], restart: false, secret: false, danger: false, mixed: false },
  ],
  bukkit: [
    { key: 'settings.allow-end', type: 'bool', default: true, group: 'Worlds', description: 'Lets players travel to the End.', min: null, max: null, options: [], restart: true, secret: false, danger: false, mixed: false },
    { key: 'settings.connection-throttle', type: 'int', default: 4000, group: 'Network', description: 'Milliseconds a player must wait before reconnecting. -1 turns it off.', min: -1, max: 600000, options: [], restart: false, secret: false, danger: false, mixed: false },
  ],
};
const DEFS: F[] = [
  { id: 'paper-global', title: 'Paper global', path: 'config/paper-global.yml' },
  { id: 'spigot', title: 'Spigot', path: 'spigot.yml' },
  { id: 'bukkit', title: 'Bukkit', path: 'bukkit.yml' },
];
const defsFor = (s: A.Server): F[] => (s.runtime === 'velocity' ? [{ id: 'velocity', title: 'Velocity', path: 'velocity.toml' }] : s.runtime === 'paper' || s.runtime === 'purpur' ? DEFS : s.runtime === 'fabric' ? [] : DEFS.slice(1));

/** Minimal indentation-based lookup for the mock's seeded YAML (the real backend parses it properly). */
function scan(raw: string) {
  const out = new Map<string, { line: number; value: string }>();
  const stack: { indent: number; key: string }[] = [];
  raw.split('\n').forEach((ln, i) => {
    const m = ln.match(/^(\s*)([A-Za-z0-9_.-]+):(.*)$/);
    if (!m || ln.trim().startsWith('#')) return;
    const indent = m[1].length;
    while (stack.length && stack[stack.length - 1].indent >= indent) stack.pop();
    const rest = m[3].replace(/\s+#.*$/, '').trim();
    stack.push({ indent, key: m[2] });
    if (rest) out.set(stack.map((x) => x.key).join('.'), { line: i, value: rest });
  });
  return out;
}
const parseVal = (v: string): unknown => (v === 'true' ? true : v === 'false' ? false : v !== '' && !isNaN(+v) ? +v : v);

const hist: Record<string, { id: string; title: string; created_at: string; text: string; path: string; sid: string }[]> = {};
export function revertConfig(c: OpsCtx, undoId: string) {
  for (const list of Object.values(hist)) {
    const h = list.find((x) => x.id === undoId);
    if (h) { c.fs[h.sid][h.path] = h.text; return; }
  }
}

export async function handleOps(c: OpsCtx, s: A.Server, sub: string, method: string, b: Record<string, any>, qs: URLSearchParams): Promise<unknown> {
  let m: RegExpMatchArray | null;
  if (sub === '/config' && method === 'GET') {
    const files = c.fs[s.id];
    return { files: defsFor(s).map((d) => ({ id: d.id, title: d.title, path: d.path, format: d.id === 'velocity' ? 'toml' : 'yaml', exists: d.path in files })) };
  }
  if (!(m = sub.match(/^\/config\/([^/]+)(\/diff)?$/))) return NONE;
  const d = defsFor(s).find((x) => x.id === decodeURIComponent(m![1]));
  if (!d) throw new ApiError(404, 'config.unknown_file', 'That file is not available for this server.');
  const files = c.fs[s.id];
  if (!(d.path in files)) { if (d.id === 'paper-global') files[d.path] = PAPER_GLOBAL; else if (d.id === 'spigot') files[d.path] = SPIGOT; }
  const key = s.id + ':' + d.id, list = (hist[key] ||= []);
  const snap = (h: { id: string; title: string; created_at: string }) => ({ id: h.id, title: h.title, created_at: h.created_at });
  if (m[2]) {
    const pick = qs.get('snapshot') ? list.find((x) => x.id === qs.get('snapshot')) : list[0];
    if (qs.get('snapshot') && !pick) throw new ApiError(404, 'not_found', 'That snapshot is not part of this file’s history.');
    return { current: files[d.path] ?? '', previous: pick?.text ?? null, snapshot: pick ? snap(pick) : null };
  }
  if (method === 'GET') {
    const raw = files[d.path];
    if (raw === undefined) return { id: d.id, title: d.title, path: d.path, format: 'yaml', exists: false, raw: '', parse_error: null, fields: [], history: [] };
    const vals = scan(raw);
    return {
      id: d.id, title: d.title, path: d.path, format: 'yaml', exists: true, raw, parse_error: null,
      fields: (FIELDS[d.id] ?? []).map((f) => ({ ...f, present: vals.has(f.key), value: vals.has(f.key) ? parseVal(vals.get(f.key)!.value) : null })),
      history: list.map(snap),
    };
  }
  if (method === 'PUT') {
    if (!(d.path in files)) throw new ApiError(409, 'config.missing', 'This file does not exist yet. Start the server once so it is created.');
    const old = files[d.path];
    let next = old;
    if (typeof b.raw === 'string') {
      if (b.set || b.unset) throw new ApiError(400, 'validation', 'Send either the whole file or a list of changes, not both.');
      if (/\[unclosed/.test(b.raw)) throw new ApiError(400, 'config.invalid', 'The file is not valid YAML: unclosed bracket on line 1.');
      next = b.raw;
    } else {
      const defs = FIELDS[d.id] ?? [], lines = next.split('\n');
      for (const [k, v] of Object.entries(b.set ?? {}) as [string, unknown][]) {
        const f = defs.find((x) => x.key === k);
        if (!f) throw new ApiError(400, 'config.unknown_key', `${k} is not a setting this editor knows. Use the raw editor for it.`);
        if (f.type === 'bool' ? typeof v !== 'boolean' : typeof v !== 'number') throw new ApiError(400, 'config.invalid', `${k} must be ${f.type === 'bool' ? 'true or false' : 'a number'}.`);
        if (typeof v === 'number' && ((f.min != null && v < f.min) || (f.max != null && v > f.max))) throw new ApiError(400, 'config.invalid', `${k} must be between ${f.min} and ${f.max}.`);
        const hit = scan(lines.join('\n')).get(k);
        if (hit) lines[hit.line] = lines[hit.line].replace(/:\s*[^#\s][^#]*?(\s+#.*)?$/, (_, c) => `: ${v}${c ?? ''}`);
        else lines.splice(lines.length - (lines[lines.length - 1] === '' ? 1 : 0), 0, ...k.split('.').map((p, i) => '  '.repeat(i) + p + ':' + (i === k.split('.').length - 1 ? ' ' + v : '')));
      }
      next = lines.join('\n');
    }
    if (next === old) return { undo_id: null, changed: false };
    const title = `Edited ${d.path}`, id = c.pushUndo(s.id, title, [d.path]);
    list.unshift({ id, title, created_at: c.now(), text: old, path: d.path, sid: s.id });
    files[d.path] = next;
    c.log('file.save', s.name, d.path);
    return { undo_id: id, changed: true };
  }
  return NONE;
}

// ---- routes that are not scoped to one server ----
export interface GlobalCtx {
  uid: () => string; now: () => string; servers: A.Server[]; backups: A.Backup[]; content: Record<string, A.ContentItem[]>; fs: Record<string, Record<string, string>>;
  call: (method: string, path: string, body: unknown) => Promise<unknown>; log: (a: string, t: string, d?: string) => void;
}
let prefs: { data: Record<string, unknown>; updated_at: string | null } = { data: {}, updated_at: null };
const views: A.SavedView[] = [
  { id: 'view-running', scope: 'servers', name: 'Running now', filters: { status: 'running' }, sort: '', created_at: new Date(Date.now() - 86400000).toISOString() },
];
const blueprints: (A.Blueprint & { doc: Record<string, any> })[] = [];
const staged = new Map<string, A.ImportDetection>();
const bad = (code: string, msg: string, field?: string) => new ApiError(400, code, msg, field ? [{ path: field, message: msg }] : []);

function detectMock(name: string, hint: string): A.ImportDetection {
  const unknown = /unknown|mystery/i.test(hint);
  const rt = unknown ? null : /fabric/i.test(hint) ? 'fabric' : /vanilla/i.test(hint) ? 'vanilla' : /velocity/i.test(hint) ? 'velocity' : 'paper';
  return {
    runtime: rt, game_version: unknown ? null : rt === 'velocity' ? '3.4.0' : '1.21.4', loader_version: rt === 'fabric' ? '0.16.9' : null, port: unknown ? null : 25812, jar: unknown ? null : 'server.jar', has_world: !unknown,
    java_major: unknown ? null : 21, java_installed: true, name, size: 412_000_000,
    notes: unknown ? ['Moss could not tell what kind of server this is. Choose the runtime below.'] : [],
  };
}

export async function handleOpsGlobal(c: GlobalCtx, method: string, p: string, qs: URLSearchParams, b: Record<string, any>, raw: unknown): Promise<unknown> {
  let m: RegExpMatchArray | null;
  const M = (re: RegExp) => (m = p.match(re));
  if (p === '/me/preferences') {
    if (method === 'GET') return prefs;
    if (!b.data || typeof b.data !== 'object' || Array.isArray(b.data)) throw bad('validation', 'Preferences must be an object.', 'data');
    prefs = { data: b.data, updated_at: c.now() };
    return prefs;
  }
  if (p === '/views') {
    if (method === 'GET') return views.filter((v) => !qs.get('scope') || v.scope === qs.get('scope'));
    const name = String(b.name ?? '').trim();
    if (!['servers', 'content'].includes(b.scope)) throw bad('validation', 'Unknown list.', 'scope');
    if (!name || name.length > 48) throw bad('validation', 'Name must be 1 to 48 characters.', 'name');
    const i = views.findIndex((v) => v.scope === b.scope && v.name.toLowerCase() === name.toLowerCase());
    if (i >= 0) views.splice(i, 1);
    const v: A.SavedView = { id: c.uid(), scope: b.scope, name, filters: b.filters ?? {}, sort: b.sort ?? '', created_at: c.now() };
    views.push(v);
    return v;
  }
  if (M(/^\/views\/([^/]+)$/)) {
    const i = views.findIndex((v) => v.id === m![1]);
    if (i < 0) throw new ApiError(404, 'not_found', 'view not found');
    views.splice(i, 1);
    return;
  }
  if (p === '/blueprints' && method === 'GET') return blueprints.map(({ doc: _d, ...s }) => s);
  if (p === '/blueprints' && method === 'POST') {
    const s = c.servers.find((x) => x.id === b.server_id);
    if (!s) throw new ApiError(404, 'not_found', 'server not found');
    const name = String(b.name ?? '').trim();
    if (!name) throw bad('validation', 'Give the blueprint a name.', 'name');
    const props = Object.fromEntries((c.fs[s.id]?.['server.properties'] ?? '').split('\n').filter((l) => l.includes('=') && !l.startsWith('#')).map((l) => l.split('=') as [string, string]).filter(([k]) => !/port|password|secret|seed|level-name|server-ip/i.test(k)));
    const items = (c.content[s.id] ?? []).filter((i) => i.project_id).map((i) => ({ provider: i.provider, project_id: i.project_id, title: i.title, kind: i.kind }));
    const doc = { format: 'moss-blueprint', version: 1, name, runtime: s.runtime, game_version: s.game_version, loader_version: s.loader_version, memory_mb: s.memory_mb, flags_profile: s.flags_profile, properties: props, content: items, schedules: [] };
    const bp = { id: c.uid(), name, runtime: s.runtime, game_version: s.game_version, loader_version: s.loader_version, memory_mb: s.memory_mb, properties: Object.keys(props).length, content: items.length, schedules: 0, created_by: 'dev@example.com', created_at: c.now(), doc };
    blueprints.unshift(bp);
    c.log('blueprint.save', name);
    const { doc: _d, ...sum } = bp;
    return sum;
  }
  if (p === '/blueprints/import') {
    const d = b as Record<string, any>;
    if (d.format !== 'moss-blueprint' || d.version !== 1) throw bad('blueprint.invalid', 'This is not a valid Moss blueprint: missing the moss-blueprint marker.');
    if (!d.name || !d.runtime || !d.game_version) throw bad('blueprint.invalid', 'This is not a valid Moss blueprint: name, runtime and game version are required.');
    const props = d.properties ?? {}, items = d.content ?? [];
    const bp = { id: c.uid(), name: String(d.name), runtime: d.runtime, game_version: d.game_version, loader_version: d.loader_version ?? null, memory_mb: d.memory_mb ?? 2048, properties: Object.keys(props).length, content: items.length, schedules: (d.schedules ?? []).length, created_by: 'dev@example.com', created_at: c.now(), doc: d };
    blueprints.unshift(bp);
    const { doc: _d, ...sum } = bp;
    return sum;
  }
  if (M(/^\/blueprints\/([^/]+)(\/export|\/create)?$/)) {
    const bp = blueprints.find((x) => x.id === m![1]);
    if (!bp) throw new ApiError(404, 'not_found', 'blueprint not found');
    if (m![2] === '/export') return bp.doc;
    if (m![2] === '/create') {
      if (!b.accept_eula) throw new ApiError(400, 'eula.required', 'You must accept the Minecraft EULA to create a Minecraft server.', [{ path: 'accept_eula', message: 'required' }]);
      if (!String(b.name ?? '').trim()) throw bad('validation', 'Give the new server a name.', 'name');
      const r = (await c.call('POST', '/servers', { name: b.name, runtime: bp.runtime, game_version: bp.game_version, loader_version: bp.loader_version, memory_mb: bp.memory_mb, port: b.port, accept_eula: true })) as { server: A.Server };
      c.content[r.server.id] = (bp.doc.content ?? []).map((i: any) => ({ id: c.uid(), provider: i.provider, project_id: i.project_id, version_id: null, title: i.title, version_number: '1.0', filename: `${i.title}.jar`, kind: i.kind, sha512: null, installed_at: c.now(), pinned: false, source: 'moss' }) as unknown as A.ContentItem);
      return { ...r, blueprint: { id: bp.id, skipped_schedules: [] } };
    }
    blueprints.splice(blueprints.indexOf(bp), 1);
    return;
  }
  if (p === '/servers/import/inspect') {
    if (raw instanceof FormData) {
      const f = raw.get('file') as File | null;
      if (!f || !/\.(zip|tar\.zst)$/i.test(f.name)) throw bad('import.archive', 'Only .zip and .tar.zst files can be imported.');
      const d = detectMock(f.name.replace(/\.(zip|tar\.zst)$/i, ''), f.name);
      const token = c.uid();
      staged.set(token, d);
      return { ...d, token };
    }
    const path = String(b.path ?? '').trim();
    if (!path.startsWith('/')) throw bad('import.path', 'The path must be absolute, for example /srv/minecraft/survival.', 'path');
    if (/^\/(etc|proc|sys|usr|dev)(\/|$)/.test(path) || path === '/') throw bad('import.path', 'That is a system folder, not a Minecraft server.', 'path');
    if (/missing|nonexistent/i.test(path)) throw bad('import.path', 'That folder does not exist or Moss cannot read it.', 'path');
    return detectMock(path.split('/').filter(Boolean).pop() ?? 'server', path);
  }
  if (p === '/servers/import') {
    if (!b.accept_eula) throw new ApiError(400, 'eula.required', 'You must accept the Minecraft EULA to run a Minecraft server.', [{ path: 'accept_eula', message: 'required' }]);
    if (!String(b.name ?? '').trim()) throw bad('validation', 'Give the server a name.', 'name');
    const d = b.token ? staged.get(b.token) : b.path ? detectMock('x', b.path) : undefined;
    if (!d) throw bad('import.token', 'That upload has expired. Upload the file again.');
    const runtime = b.runtime ?? d.runtime, gv = b.game_version ?? d.game_version, port = b.port ?? d.port ?? undefined;
    if (!runtime) throw new ApiError(400, 'runtime.unsupported', 'Choose the server type.', [{ path: 'runtime', message: 'required' }]);
    if (!gv) throw new ApiError(400, 'validation', 'Enter the Minecraft version.', [{ path: 'game_version', message: 'required' }]);
    if (port && c.servers.some((s) => s.port === port)) throw new ApiError(409, 'port.taken', `Port ${port} is already used by another Moss server. Pick a different port.`, [{ path: 'port', message: 'already assigned' }]);
    if (b.token) staged.delete(b.token);
    c.log('server.import', b.name, `${runtime} ${gv}`);
    return c.call('POST', '/servers', { name: b.name, runtime, game_version: gv, loader_version: b.loader_version ?? d.loader_version, memory_mb: b.memory_mb ?? 2048, port, accept_eula: true });
  }
  if (p === '/search') {
    const t = (qs.get('q') ?? '').trim().toLowerCase(), n = +(qs.get('limit') ?? 6);
    const out: A.SearchResults = { q: t, servers: [], backups: [], content: [], files: [] };
    if (t.length < 2) return out;
    const name = (id: string) => c.servers.find((s) => s.id === id)?.name ?? '';
    out.servers = c.servers.filter((s) => [s.name, s.runtime, s.game_version, ...s.tags].some((x) => x.toLowerCase().includes(t))).slice(0, n).map((s) => ({ id: s.id, name: s.name, runtime: s.runtime, game_version: s.game_version }));
    out.backups = c.backups.filter((k) => `${k.note} ${k.kind} ${name(k.server_id)}`.toLowerCase().includes(t)).slice(0, n).map((k) => ({ id: k.id, server_id: k.server_id, server_name: name(k.server_id), created_at: k.created_at, note: k.note, kind: k.kind }));
    for (const [sid, items] of Object.entries(c.content)) for (const i of items) if (out.content.length < n && `${i.title} ${i.filename}`.toLowerCase().includes(t)) out.content.push({ id: i.id, server_id: sid, server_name: name(sid), title: i.title, filename: i.filename, kind: i.kind });
    for (const [sid, files] of Object.entries(c.fs)) for (const path of Object.keys(files)) if (out.files.length < n && !path.endsWith('/') && !/^(world|logs)\//.test(path) && (path.split('/').pop() ?? '').toLowerCase().includes(t)) out.files.push({ server_id: sid, server_name: name(sid), path });
    return out;
  }
  return NONE;
}
