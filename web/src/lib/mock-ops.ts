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
