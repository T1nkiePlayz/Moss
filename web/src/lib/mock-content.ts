// Dev-only mock of WP-A (content v2, compatibility check, upgrade assistant, modpacks). Imported by mock.ts only.
// Findings are computed from the item list, so toggling/removing/installing changes the check result like the real thing.
import type * as A from './api';
import { ApiError } from './api';
import type { ModpackHit } from './api-extra';

export interface Ctx {
  uid: () => string; now: () => string; ago: (ms: number) => string;
  op: (kind: string, title: string, server_id: string, stages: string[], opts?: { fail?: boolean; done?: () => void; indeterminate?: boolean }) => A.Operation;
  hits: A.Hit[]; cfHits: A.Hit[]; log: (a: string, t: string, d?: string) => void; pushUndo: (id: string, title: string, files: string[]) => string;
  backups: A.Backup[]; content: Record<string, A.ContentItem[]>; emitServer: (s: A.Server) => void;
}
const DAY = 86400000;

/** Mock Hangar search results (slugs are the project ids, like the real API). */
export const hangarHits: A.Hit[] = [['ViaVersion', 'ViaVersion', 'ViaVersion', 'Allow newer clients to join older servers.', 4200000, ['misc']], ['Chunky', 'Chunky', 'pop4959', 'Pre-generates chunks, quickly and efficiently.', 900000, ['world_management']], ['Geyser', 'Geyser', 'GeyserMC', 'Let Bedrock players join Java servers.', 700000, ['misc']], ['ViaBackwards', 'ViaBackwards', 'ViaVersion', 'Allow older clients to join newer servers.', 2100000, ['misc']], ['Floodgate', 'Floodgate', 'GeyserMC', 'Bedrock players can join without a Java account.', 650000, ['misc']], ['CoreProtect', 'CoreProtect', 'Intelli', 'Fast block logging, rollback and restore.', 1500000, ['admin_tools']], ['squaremap', 'squaremap', 'jpenilla', 'Minimalistic and lightweight world map viewer.', 310000, ['world_management']], ['TAB', 'TAB', 'NEZNAMY', 'Tablist, nametag and scoreboard plugin.', 980000, ['chat']], ['Maintenance', 'Maintenance', 'kennytv', 'Enable maintenance mode with a custom MOTD.', 420000, ['admin_tools']]]
  .map(([project_id, title, author, description, downloads, categories]) => ({ project_id: project_id as string, slug: project_id as string, title: title as string, description: description as string, author: author as string, icon_url: null, downloads: downloads as number, follows: 0, categories: categories as string[], updated: new Date(Date.now() - 4 * DAY).toISOString(), client_side: null, server_side: null }));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Dep = { id: string; required: boolean; range: string | null };

// ---- runtimes / versions ----
export const extraRuntimes: A.Runtime[] = [
  { id: 'neoforge', name: 'NeoForge', category: 'modded', tier: 'stable', content_kind: 'mods', description: 'The modern Forge fork. Minecraft 1.20.2 and newer.', installable: true },
  { id: 'forge', name: 'Forge', category: 'modded', tier: 'experimental', content_kind: 'mods', description: 'The classic mod loader. Modern installers only (1.17+).', installable: true },
  { id: 'quilt', name: 'Quilt', category: 'modded', tier: 'stable', content_kind: 'mods', description: 'Fabric-compatible loader with its own mod ecosystem.', installable: true },
];
const MC = ['1.21.5', '1.21.4', '1.21.3', '1.21.1', '1.20.6', '1.20.4', '1.20.1', '1.19.4', '1.18.2', '1.16.5', '25w05a', '24w46a'];
export function versionsFor(id: string): A.Versions {
  const min = id === 'neoforge' ? '1.20.2' : id === 'forge' ? '1.17' : '0';
  const ok = (v: string) => !/^\d+\.\d+/.test(v) ? min === '0' : min === '0' || v.split('.').map(Number).reduce((n, x, i) => n + x * [10000, 100, 1][i], 0) >= min.split('.').map(Number).reduce((n, x, i) => n + x * [10000, 100, 1][i], 0);
  const list = MC.filter(ok);
  const out: A.Versions = { versions: list.map((v) => ({ id: v, stable: /^\d+\.\d+(\.\d+)?$/.test(v) })), recommended: list.includes('1.21.4') ? '1.21.4' : list[0] };
  if (['fabric', 'quilt', 'forge', 'neoforge'].includes(id)) {
    const base = id === 'neoforge' ? '21.4.' : id === 'forge' ? '54.0.' : id === 'quilt' ? '0.27.' : '0.16.';
    out.loader_versions = Object.fromEntries(list.filter((v) => /^\d/.test(v) && !/[a-z]/.test(v)).map((v, i) => [v, [3, 2, 1, 0].map((n) => ({ id: `${base}${n + i}`, recommended: n === 2 }))]));
  }
  return out;
}

// ---- inventory ----
function item(s: A.Server, o: { title: string; filename: string; v: string; source?: A.ContentItem['source']; provider?: A.ContentItem['provider']; pinned?: boolean; enabled?: boolean; meta?: Partial<A.ContentMeta> | null; project?: string; age?: number }): A.ContentItem {
  const source = o.source ?? 'moss';
  const provider = o.provider ?? (source === 'manual' ? 'manual' : 'modrinth');
  const slug = o.project ?? o.title.toLowerCase().replace(/\W+/g, '');
  const loader = s.runtime === 'fabric' ? 'fabric' : s.content_kind === 'datapacks' ? 'datapack' : s.runtime === 'velocity' ? 'velocity' : 'paper';
  return {
    id: source === 'manual' ? `manual:${o.filename}` : crypto.randomUUID(), provider, project_id: provider === 'manual' ? null : slug, version_id: provider === 'manual' ? null : crypto.randomUUID(),
    title: o.title, version_number: o.v, filename: o.filename, kind: s.content_kind, sha512: provider === 'manual' ? null : 'a1b2c3d4'.repeat(16), installed_at: new Date(Date.now() - (o.age ?? 4) * DAY).toISOString(),
    pinned: !!o.pinned, source, enabled: o.enabled ?? true, identified: source !== 'manual', issues: [],
    meta: o.meta === null ? null : { id: slug, name: o.title, version: o.v, authors: ['Moss Mock Authors'], description: `${o.title} for ${s.runtime}.`, loader, environment: 'both', depends: [], breaks: [], ...o.meta },
  };
}
export function seedContent(s: A.Server): A.ContentItem[] {
  const d = (id: string, required = true, range: string | null = null): Dep => ({ id, required, range });
  if (s.name === 'Modded Fabric') return [
    item(s, { title: 'Fabric API', filename: 'fabric-api-0.114.jar', v: '0.114.0', pinned: true, meta: { id: 'fabric-api', environment: 'both' } }),
    item(s, { title: 'Sodium', filename: 'sodium-fabric-0.6.jar', v: '0.6.0', meta: { id: 'sodium', environment: 'client', depends: [d('fabricloader', true, '>=0.15.0'), d('minecraft', true, '~1.21')] } }),
    item(s, { title: 'Lithium', filename: 'lithium-fabric-0.14.jar', v: '0.14.3', meta: { id: 'lithium', environment: 'server' } }),
    item(s, { title: 'Sodium Extra', filename: 'sodium-extra-0.5.4.jar', v: '0.5.4', meta: { id: 'sodium-extra', environment: 'client', depends: [d('sodium', true, '>=0.7.0')] } }),
    item(s, { title: 'Iris Shaders', filename: 'iris-1.8.0.jar', v: '1.8.0', source: 'identified', meta: { id: 'iris', environment: 'client', description: 'A shader pack loader. Rendering only: does nothing on a dedicated server.' } }),
    item(s, { title: 'Create', filename: 'create-1.21.1-6.0.0.jar', v: '6.0.0', source: 'manual', meta: { id: 'create', loader: 'forge', environment: 'both' } }),
    item(s, { title: 'Phosphor', filename: 'phosphor-0.8.1.jar', v: '0.8.1', meta: { id: 'phosphor', breaks: ['starlight'] } }),
    item(s, { title: 'Starlight', filename: 'starlight-1.1.2.jar', v: '1.1.2', meta: { id: 'starlight' } }),
    item(s, { title: 'FerriteCore', filename: 'ferritecore-7.0.0.jar', v: '7.0.0', source: 'modpack', meta: { id: 'ferritecore' } }),
    item(s, { title: 'my-custom-tweaks', filename: 'my-custom-tweaks.jar', v: '', source: 'manual', meta: { id: 'my-custom-tweaks', name: 'my-custom-tweaks', version: '1.0.0' } }),
    item(s, { title: 'flaky-mod', filename: 'flaky-mod.jar', v: '0.3.1', source: 'manual', meta: { id: 'flaky-mod' } }),
    item(s, { title: 'corrupt-lib', filename: 'corrupt-lib.jar', v: '', source: 'manual', meta: null }),
    item(s, { title: 'Old Optimizer', filename: 'old-optimizer-2.1.jar', v: '2.1.0', enabled: false, meta: { id: 'old-optimizer' } }),
  ];
  if (s.name === 'Survival SMP') return [
    item(s, { title: 'LuckPerms', filename: 'LuckPerms-5.4.jar', v: '5.4.145', pinned: true, meta: { id: 'luckperms', environment: 'server' } }),
    item(s, { title: 'Chunky', filename: 'Chunky-1.4.jar', v: '1.4.28', meta: { id: 'chunky', environment: 'server' } }),
    item(s, { title: 'EssentialsX', filename: 'EssentialsX.jar', v: '2.20.1', source: 'manual', meta: { id: 'essentials', depends: [d('vault', false)] } }),
    item(s, { title: 'WorldGuard', filename: 'worldguard-bukkit-7.0.9.jar', v: '7.0.9', source: 'identified', provider: 'curseforge', project: 'cf-worldguard', meta: { id: 'worldguard', depends: [d('worldedit', true, '>=7.2')] } }),
    item(s, { title: 'Dynmap', filename: 'Dynmap-3.7.jar', v: '3.7', meta: { id: 'dynmap', description: 'Declares api-version 1.16; loads through the compatibility layer.' } }),
    item(s, { title: 'LogBlock', filename: 'LogBlock-1.19.jar', v: '1.19.0', source: 'manual', meta: { id: 'logblock' } }),
    item(s, { title: 'old-economy', filename: 'old-economy.jar', v: '0.9', source: 'manual', enabled: false, meta: { id: 'old-economy' } }),
  ];
  if (s.name === 'Creative Plots') return [
    item(s, { title: 'LuckPerms', filename: 'LuckPerms-5.4.jar', v: '5.4.145', meta: { id: 'luckperms', environment: 'server' } }),
    item(s, { title: 'PlotSquared', filename: 'PlotSquared-7.4.jar', v: '7.4.1', meta: { id: 'plotsquared', environment: 'server' } }),
  ];
  if (s.content_kind === 'plugins' || s.content_kind === 'proxy_plugins') return [item(s, { title: 'LuckPerms', filename: 'LuckPerms-5.4.jar', v: '5.4.145', meta: { id: 'luckperms' } })];
  return [];
}

const BUILTIN = new Set(['minecraft', 'java', 'fabricloader', 'quilt_loader', 'neoforge', 'forge', 'paper', 'spigot', 'bukkit', 'velocity']);
const okLoaders = (s: A.Server) => (s.runtime === 'fabric' ? ['fabric', 'quilt', 'unknown'] : s.runtime === 'forge' || s.runtime === 'neoforge' ? ['forge', 'neoforge', 'unknown'] : s.runtime === 'quilt' ? ['quilt', 'fabric', 'unknown'] : ['paper', 'spigot', 'bukkit', 'velocity', 'bungee', 'datapack', 'unknown']);
const cmp = (a: string, b: string) => { const x = a.split('.').map(Number), y = b.split('.').map(Number); for (let i = 0; i < 3; i++) { const d = (x[i] || 0) - (y[i] || 0); if (d) return d; } return 0; };

export function findingsFor(s: A.Server, items: A.ContentItem[]): A.ContentFinding[] {
  const out: A.ContentFinding[] = [];
  const on = items.filter((i) => i.enabled);
  const f = (id: string, severity: A.ContentSeverity, item: A.ContentItem | null, title: string, detail: string, evidence: string[], action: A.ContentFinding['action']) => out.push({ id, severity, item: item?.id ?? null, title, detail, evidence, action });
  for (const it of on) {
    const m = it.meta;
    if (!m) { f(`bad:${it.id}`, 'critical', it, `${it.filename} is unreadable`, 'The file is not a valid jar (the zip directory is damaged), so the server cannot load it.', ['java.util.zip.ZipException: zip END header not found', `size ${it.filename.length * 1371} bytes`], { label: 'Remove file', kind: 'remove', item: it.id }); continue; }
    for (const dep of m.depends) {
      if (!dep.required || BUILTIN.has(dep.id)) continue;
      const have = on.find((o) => o.meta?.id === dep.id);
      const off = items.find((o) => !o.enabled && o.meta?.id === dep.id);
      if (!have && off) f(`dep:${it.id}:${dep.id}`, 'warn', it, `${dep.id} is installed but disabled`, `${it.title} needs ${dep.id}.`, [`${m.id} requires ${dep.id}${dep.range ? ' ' + dep.range : ''}`, `${off.filename}.disabled is present`], { label: `Enable ${off.title}`, kind: 'toggle', item: off.id });
      else if (!have) f(`dep:${it.id}:${dep.id}`, 'warn', it, `Missing required dependency: ${dep.id}`, `${it.title} will not load until ${dep.id} is installed.`, [`${m.id} requires ${dep.id}${dep.range ? ' ' + dep.range : ''}`, `no installed file provides "${dep.id}"`], { label: `Install ${dep.id}`, kind: 'install', project_id: dep.id });
      else if (dep.range?.startsWith('>=') && have.meta && cmp(have.meta.version, dep.range.slice(2)) < 0) f(`range:${it.id}:${dep.id}`, 'warn', it, `${have.title} is too old for ${it.title}`, `${it.title} needs ${dep.id} ${dep.range}, but ${have.meta.version} is installed.`, [`${m.id} requires ${dep.id} ${dep.range}`, `installed ${dep.id} ${have.meta.version}`], { label: `Update ${have.title}`, kind: 'update', item: have.id });
    }
    for (const b of m.breaks) {
      const other = on.find((o) => o.meta?.id === b);
      if (other) f(`breaks:${it.id}:${b}`, 'critical', it, `${it.title} is incompatible with ${other.title}`, `${it.title} declares that it breaks ${b}. Running both usually crashes on startup.`, [`${m.id} breaks ${b}`, `${other.filename} is enabled`], { label: `Disable ${it.title}`, kind: 'toggle', item: it.id });
    }
    if (!okLoaders(s).includes(m.loader)) f(`loader:${it.id}`, 'critical', it, `${it.title} is built for ${m.loader}`, `This server runs ${s.runtime}; a ${m.loader} mod cannot load here.`, [`${it.filename}: loader "${m.loader}"`, `server runtime "${s.runtime}"`], { label: 'Remove', kind: 'remove', item: it.id });
    if (m.environment === 'client') f(`client:${it.id}`, 'warn', it, `${it.title} is client-only`, 'This mod only affects the game client. On a server it is dead weight and can cause crashes on some versions.', [`${m.id}: environment "client"`], { label: 'Disable', kind: 'toggle', item: it.id });
    if (m.id === 'dynmap') f(`api:${it.id}`, 'info', it, `${it.title} targets an older API`, 'Loads fine today, but check for an update when you upgrade the game version.', ['plugin.yml api-version: 1.16', `server game version ${s.game_version}`], { label: 'Open', kind: 'open', item: it.id });
  }
  return out;
}
const withIssues = (s: A.Server, items: A.ContentItem[]) => { const fs = findingsFor(s, items); return items.map((i) => ({ ...i, issues: fs.filter((x) => x.item === i.id) })); };
const bump = (v: string | null) => (v ? v.replace(/(\d+)(?!.*\d)/, (n) => String(+n + 1)) : '1.0.1');

// ---- modpacks ----
const packs = new Map<string, A.ModpackInspect>();
function inspected(name: string, kind: 'mr' | 'cf' | 'warn' | 'forge', size = 48e6): A.ModpackInspect {
  const base: A.ModpackInspect = {
    token: crypto.randomUUID(), format: kind === 'cf' ? 'curseforge' : 'mrpack', name, version: '5.2.0', summary: 'Performance, visuals and quality of life for survival servers.',
    game_version: kind === 'forge' ? '1.20.1' : '1.21.4', loader: { id: kind === 'forge' ? 'forge' : 'fabric', version: kind === 'forge' ? '47.3.0' : '0.16.9' },
    files: { total: 74, server: 41, client_only: 33, unavailable: 0 }, unavailable: [], overrides: { files: 112, size: 3.4e6 }, size, warnings: [],
  };
  if (kind === 'cf') {
    base.files = { total: 120, server: 96, client_only: 22, unavailable: 2 };
    base.unavailable = [
      { name: 'Journeymap 5.10.jar', reason: 'Author disabled third-party downloads', url: 'https://www.curseforge.com/minecraft/mc-mods/journeymap' },
      { name: 'Xaero\'s Minimap 24.4.jar', reason: 'Author disabled third-party downloads', url: 'https://www.curseforge.com/minecraft/mc-mods/xaeros-minimap' },
    ];
  }
  if (kind === 'warn' || kind === 'forge') base.warnings = [kind === 'forge' ? 'Forge support is experimental; test the pack before inviting players.' : 'The pack ships 3 server-overrides that replace config files you may have edited.'];
  packs.set(base.token, base);
  return base;
}
function inspect(body: unknown): A.ModpackInspect {
  if (body instanceof FormData) {
    const f = body.get('file') as File;
    const n = f.name.toLowerCase();
    if (n.includes('bad')) throw new ApiError(422, 'modpack.invalid', `${f.name} is not a valid modpack: modrinth.index.json is missing.`);
    return inspected(f.name.replace(/\.(mrpack|zip)$/i, ''), n.includes('cf') || n.endsWith('.zip') ? 'cf' : n.includes('forge') ? 'forge' : n.includes('warn') ? 'warn' : 'mr', f.size);
  }
  const b = body as { provider: A.ProviderId; project_id: string };
  if (/unknown|missing/.test(b.project_id)) throw new ApiError(404, 'modpack.not_found', `No ${b.provider === 'modrinth' ? 'Modrinth' : 'CurseForge'} modpack matches "${b.project_id}".`);
  return inspected(b.project_id.replace(/[-_]/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()), b.provider === 'curseforge' ? 'cf' : /warn/.test(b.project_id) ? 'warn' : /forge/.test(b.project_id) ? 'forge' : 'mr');
}
const packItems = (s: A.Server, token: string): A.ContentItem[] => {
  const pk = packs.get(token);
  return ['Sodium', 'Lithium', 'FerriteCore', 'Entity Culling', 'ModernFix'].map((t, i) => item(s, { title: t, filename: `${t.toLowerCase().replace(/\W+/g, '-')}-${i + 1}.0.jar`, v: `${i + 1}.0.0`, source: 'modpack', age: 0, meta: { id: t.toLowerCase().replace(/\W+/g, '') } })).slice(0, pk ? 5 : 0);
};
export const itemsFromPack = packItems;

// ---- upgrade ----
function upgradePlan(s: A.Server, c: A.ContentItem[], b: { game_version: string; loader_version?: string }): A.UpgradePlan {
  const to = b.game_version;
  const javaFor = (v: string) => { const [, mi, pa] = v.match(/^1\.(\d+)(?:\.(\d+))?/) ?? []; const minor = +mi, patch = +(pa ?? 0); return !mi ? 21 : minor <= 16 ? 8 : minor < 20 || (minor === 20 && patch < 5) ? 17 : 21; };
  const jt = javaFor(to);
  const snapshot = !/^\d+\.\d+(\.\d+)?$/.test(to);
  const missingJar = to === '1.16.5' && s.runtime !== 'vanilla';
  const lv = s.runtime === 'fabric' || s.runtime === 'quilt' || s.runtime === 'forge' || s.runtime === 'neoforge' ? b.loader_version ?? versionsFor(s.runtime).loader_versions?.[to]?.find((l) => l.recommended)?.id ?? null : null;
  const dn = cmp(to, s.game_version) < 0;
  const rows: A.UpgradePlan['content'] = c.map((it, i) => {
    const base = { item: it.id, title: it.title, current: it.version_number || it.meta?.version || null, target: null as string | null, version_id: null as string | null, reason: null as string | null };
    if (!it.identified || !it.project_id) return { ...base, status: 'unknown' as const, reason: 'Manual file: Moss cannot tell which project this is. Identify it first, or replace it by hand.' };
    if (dn && i % 4 === 3) return { ...base, status: 'missing' as const, reason: `${it.title} has no build for ${to}.` };
    if (snapshot && i % 3 === 0) return { ...base, status: 'missing' as const, reason: `No release of ${it.title} supports the snapshot ${to} yet.` };
    if (i % 2 === 1) return { ...base, status: 'update' as const, target: bump(it.version_number), version_id: crypto.randomUUID(), reason: `Needs ${bump(it.version_number)} for ${to}.` };
    return { ...base, status: 'ok' as const, target: it.version_number, reason: 'Works on the new version as is.' };
  });
  const blockers: string[] = [], warnings: string[] = [];
  if (missingJar) blockers.push(`No ${s.runtime} server build exists for ${to}.`);
  if (to === s.game_version && !b.loader_version) blockers.push('The server is already on that version.');
  if (dn) warnings.push('Worlds that were opened in a newer version cannot always be loaded by an older one.');
  if (snapshot) warnings.push('Snapshots are unstable and may corrupt worlds. Only use them on a copy of your world.');
  if (rows.some((r) => r.status === 'missing') && !blockers.length) warnings.push('Some add-ons have no build for this version; choose how to handle them below.');
  return {
    from: { game_version: s.game_version, loader_version: s.loader_version }, to: { game_version: to, loader_version: lv },
    java: { from: s.java_major, to: jt, installed: jt !== 17 }, server_jar: { available: !missingJar, note: missingJar ? `${s.runtime} has not published builds for ${to}.` : `${s.runtime} build #${100 + to.length * 7} (stable channel).` },
    content: rows, blockers, warnings, can_proceed: !blockers.length,
  };
}

export const NONE = Symbol('no route');
// ---- staged copies and modpack versions ----
const packsBy: Record<string, A.PackInfo[]> = {};
const runs: Record<string, A.StagingRun[]> = {};
function packsFor(s: A.Server, uid: () => string, now: () => string): A.PackInfo[] {
  if (s.content_kind !== 'mods') return [];
  return (packsBy[s.id] ??= [{ id: uid(), name: 'Fabulous Survival', version: '5.1.0', format: 'mrpack', game_version: s.game_version, loader: 'fabric', mode: 'add', installed_at: now(), files: 41, pinned: false, items: 41, override_files: 12,
    source: { provider: 'modrinth', project_id: 'fabulous-survival', version_id: 'v510' }, can_check: true }]);
}
function packDiff(from: string, to: string): A.PackDiff {
  const ch = (title: string, path: string, f: string | null, t: string | null, reason: string | null = null): A.PackChange => ({ title, path, from: f, to: t, reason });
  return {
    versions: { from: { version: from, game_version: '1.21.4', loader: 'fabric', loader_version: '0.16.9' }, to: { version: to, game_version: '1.21.4', loader: 'fabric', loader_version: '0.16.9' } },
    files: {
      added: [ch('Sodium Extra', 'mods/sodium-extra-0.6.jar', null, '0.6.0')], removed: [ch('Old Minimap', 'mods/oldminimap.jar', '1.2.0', null)],
      updated: [ch('Lithium', 'mods/lithium.jar', '0.14.3', '0.14.7'), ch('Ferrite Core', 'mods/ferritecore.jar', '7.0.0', '7.1.1')],
      held: [ch('Iris Shaders', 'mods/iris.jar', '1.8.0', '1.8.4', 'pinned')], unchanged: 36,
    },
    overrides: { added: ['config/sodium-extra.json'], updated: ['config/lithium.properties'], user_modified: [{ path: 'config/ferritecore.mixin.properties', reason: 'You changed this file; the pack has a newer one. It is left as it is.' }], removed_upstream: [], unchanged: 9 },
    blockers: [], warnings: [], can_apply: true, changes: true, signature: `sig-${to}`,
  };
}
/** The staging run advances building -> testing -> healthy (or unhealthy when the test is asked to fail via the server name). */
function advance(run: A.StagingRun, bad: boolean) {
  setTimeout(() => { if (run.state === 'building') run.state = 'testing'; }, 1200);
  setTimeout(() => {
    if (run.state !== 'testing') return;
    run.state = bad ? 'unhealthy' : 'healthy';
    run.expires_at = new Date(Date.now() + DAY).toISOString();
    run.can_promote = !bad && run.kind !== 'none';
    run.report = {
      healthy: !bad, reasons: bad ? ['The log contains a fatal error: java.lang.NoClassDefFoundError: net/example/Missing'] : [], ready_secs: bad ? null : 14.2, ready_timeout_s: 180, watch_s: 30, crashed: false,
      errors: bad ? 3 : 0, warnings: bad ? 2 : 4, samples: bad ? ['[10:00:05 ERROR]: Could not load mod example', 'java.lang.NoClassDefFoundError: net/example/Missing'] : [], fatal: bad ? 'java.lang.NoClassDefFoundError: net/example/Missing' : null,
      changes: run.report.changes ?? [], signature: 'sig', notes: run.include_world ? [] : ['The copy starts with a fresh world; world-specific problems will not show up.'], copy: { files: 214, bytes: 3.1e8, world: run.include_world }, port: run.port ?? 30000, tested_at: new Date().toISOString(),
    };
  }, 3600);
}

/** Returns NONE when the route is not a content/modpack/upgrade route. */
export async function handleContent(c: Ctx, s: A.Server, sub: string, method: string, b: Record<string, any>): Promise<unknown> {
  const list = (c.content[s.id] ??= []);
  const dec = (x: string) => decodeURIComponent(x);
  let m: RegExpMatchArray | null;
  const R = (re: RegExp) => (m = sub.match(re));
  const find = (id: string) => { const it = list.find((x) => x.id === id); if (!it) throw new ApiError(404, 'content.not_found', 'That item is no longer installed.'); return it; };
  const hit = (pid: string) => [...c.hits, ...c.cfHits].find((x) => x.project_id === pid);

  if (sub === '/content' && method === 'GET') return { items: withIssues(s, list) };
  if (sub === '/content/check') {
    await sleep(500);
    if (s.runtime === 'velocity') throw new ApiError(502, 'content.check_failed', 'Could not reach Modrinth to look up dependency data. Try again in a minute.');
    return { findings: findingsFor(s, list), checked_at: c.now() };
  }
  if (sub === '/content/identify') {
    const o = c.op('content.identify', 'Identifying unknown files', s.id, ['Hashing files', 'Querying Modrinth', 'Querying CurseForge', 'Linking matches'], {
      done: () => {
        let n = 0;
        for (const it of list) if (!it.identified && it.meta && !/flaky/.test(it.filename)) { it.identified = true; it.source = 'identified'; it.provider = 'modrinth'; it.project_id = it.meta.id; it.version_id = c.uid(); it.sha512 = 'd4e5f6'.repeat(21).slice(0, 128); n++; }
        o.log.push(`Matched ${n} of ${n + list.filter((i) => !i.identified).length} unknown files`);
      },
    });
    return { operation: o };
  }
  if (sub === '/content/plan') {
    const h = hit(b.project_id);
    const title = h?.title ?? String(b.project_id);
    const plan: A.ChangePlan = {
      items: [{ action: 'install', title, filename: `${b.project_id}-2.4.1.jar`, size: 2.1e6, reason: 'Selected by you', version: '2.4.1', from_version: null }],
      warnings: s.state === 'running' ? ['The server is running; a restart is needed to load it.'] : [], requires_restart: true, conflicts: [],
    };
    if (b.project_id === 'worldedit') plan.items.push({ action: 'install', title: 'FastAsyncWorldEdit support lib', filename: 'lib-1.0.jar', size: 420000, reason: 'Required dependency', version: '1.0.0', from_version: null });
    if (b.project_id === 'coreprotect') plan.conflicts = [
      { id: 'cx1', severity: 'critical', item: null, title: 'CoreProtect conflicts with LogBlock', detail: 'Both plugins log block changes through the same events. Running both is known to double-log and can corrupt the rollback database.', evidence: ['LogBlock-1.19.jar is installed and enabled', 'Both register BlockPlaceEvent and BlockBreakEvent at MONITOR priority'], action: { label: 'Disable LogBlock', kind: 'toggle', item: 'manual:LogBlock-1.19.jar' } },
      { id: 'cx2', severity: 'warn', item: null, title: 'Dynmap hooks are not supported', detail: 'The Dynmap integration was removed in CoreProtect 22.', evidence: ['Dynmap 3.7 is installed'], action: null },
    ];
    if (b.project_id === 'dynmap') plan.conflicts = [{ id: 'cx3', severity: 'info', item: null, title: 'Opens a web port', detail: 'Dynmap serves a map on port 8123. Open it in your firewall if you want players to reach it.', evidence: [], action: null }];
    return plan;
  }
  if (sub === '/content' && method === 'POST') {
    const h = hit(b.project_id);
    return { operation: c.op('content.install', `Installing ${h?.title ?? b.project_id}`, s.id, ['Resolving dependencies', 'Downloading', 'Verifying sha512', 'Placing files'], { done: () => {
      const add = (title: string, pid: string) => list.push(item(s, { title, filename: `${pid}-2.4.1.jar`, v: '2.4.1', provider: b.provider ?? 'modrinth', project: pid, age: 0 }));
      add(h?.title ?? b.project_id, b.project_id);
      if (b.project_id === 'worldedit') list.push(item(s, { title: 'FastAsyncWorldEdit support lib', filename: 'lib-1.0.jar', v: '1.0.0', age: 0 }));
      c.pushUndo(s.id, `Installed ${h?.title ?? b.project_id}`, [`${b.project_id}.jar`]); c.log('content.install', s.name, h?.title ?? '');
    } }) };
  }
  if (sub === '/content/check-updates') return { updates: list.filter((x) => x.provider !== 'manual' && !x.pinned && x.enabled).slice(0, 3).map((x) => ({ item_id: x.id, title: x.title, current: x.version_number ?? '', latest: bump(x.version_number), version_id: c.uid() })) };
  if (sub === '/content/update-all/plan') {
    const ups = list.filter((x) => x.provider !== 'manual' && !x.pinned && x.enabled).slice(0, 3);
    return { items: ups.map((x) => ({ action: 'replace', title: x.title, filename: x.filename, size: 1.4e6, reason: 'Newer compatible version', version: bump(x.version_number), from_version: x.version_number })), warnings: ups.length && s.state === 'running' ? ['The server is running; updated files load after a restart.'] : [], requires_restart: true, conflicts: [] } satisfies A.ChangePlan;
  }
  if (sub === '/content/update-all') {
    return { operation: c.op('content.update', 'Updating all content', s.id, ['Backing up files', 'Downloading 3 files', 'Verifying sha512', 'Replacing files'], { done: () => { for (const x of list.filter((y) => y.provider !== 'manual' && !y.pinned && y.enabled).slice(0, 3)) x.version_number = bump(x.version_number); c.pushUndo(s.id, 'Updated all content', list.map((x) => x.filename)); } }) };
  }
  if (R(/^\/content\/([^/]+)\/toggle$/)) {
    const it = find(dec(m![1]));
    await sleep(350);
    if (/flaky/.test(it.filename)) throw new ApiError(500, 'content.toggle_failed', `Could not rename ${it.filename}: permission denied`);
    it.enabled = !!b.enabled;
    return { item: withIssues(s, [it])[0], requires_restart: true };
  }
  if (R(/^\/content\/([^/]+)\/update$/)) {
    const it = find(dec(m![1]));
    return { operation: c.op('content.update', `Updating ${it.title}`, s.id, ['Downloading', 'Replacing'], { done: () => { it.version_number = bump(it.version_number); } }) };
  }
  if (R(/^\/content\/([^/]+)$/)) {
    const it = find(dec(m![1]));
    if (method === 'PATCH') { it.pinned = !!b.pinned; return; }
    return { operation: c.op('content.remove', `Removing ${it.title}`, s.id, ['Backing up files', 'Removing'], { done: () => { list.splice(list.indexOf(it), 1); c.pushUndo(s.id, `Removed ${it.title}`, [it.filename]); } }) };
  }

  if (sub === '/upgrade/plan') { await sleep(600); return upgradePlan(s, list, b as never); }
  if (sub === '/upgrade') {
    if (s.state === 'running' || s.state === 'starting' || s.state === 'stopping') throw new ApiError(409, 'upgrade.server_running', 'Stop the server before upgrading.');
    const p = upgradePlan(s, list, b as never);
    if (!p.can_proceed) throw new ApiError(422, 'upgrade.blocked', p.blockers[0]);
    if (cmp(b.game_version, s.game_version) < 0 && !b.confirm_downgrade) throw new ApiError(400, 'upgrade.confirm_downgrade', 'Downgrades need confirm_downgrade.', [{ path: 'confirm_downgrade', message: 'Confirm the downgrade' }]);
    const bid = c.uid();
    const o = c.op('upgrade', `Upgrading ${s.name} to ${b.game_version}`, s.id, ['Creating pre-change backup', 'Downloading server jar', 'Replacing loader files', `Applying content: ${b.content}`, 'Recording version'], {
      done: () => {
        s.game_version = b.game_version; s.java_major = p.java.to; if (p.to.loader_version) s.loader_version = p.to.loader_version;
        if (b.content === 'update') for (const r of p.content) { const it = list.find((x) => x.id === r.item); if (it && r.status === 'update') it.version_number = r.target; }
        if (b.content === 'disable_incompatible') for (const r of p.content) { const it = list.find((x) => x.id === r.item); if (it && r.status === 'missing') it.enabled = false; }
        c.emitServer(s); c.log('server.upgrade', s.name, b.game_version);
      },
    });
    c.backups.push({ id: bid, server_id: s.id, created_at: c.now(), size: 1.1e9, sha256: 'cd'.repeat(32), status: 'ok', verified_at: null, note: `Before upgrade to ${b.game_version}`, kind: 'pre-change', consistent: true, files: 1700 });
    o.log.push(`Pre-change backup ${bid} created`);
    return { operation: o };
  }

  if (sub === '/staging/estimate') return { files: 1840, copy_bytes: 3.1e8, world_bytes: 1.2e9, free_bytes: 5e10, port_range: [30000, 30999], defaults: { ready_timeout_s: 180, watch_s: 30 }, kinds: ['none', 'update_all', 'upgrade', 'pack_update'] } satisfies A.StagingEstimate;
  if (sub === '/staging' && method === 'GET') return runs[s.id] ?? [];
  if (sub === '/staging' && method === 'POST') {
    const list0 = runs[s.id] ??= [];
    if (list0.some((r) => r.state === 'building' || r.state === 'testing')) throw new ApiError(409, 'staging.busy', 'A test copy is already being built for this server; wait for it or cancel it.');
    const changes = b.kind === 'update_all' ? list.filter((x) => x.provider !== 'manual' && !x.pinned && x.enabled).slice(0, 3).map((x) => ({ title: x.title, action: 'replace', to: bump(x.version_number) }))
      : b.kind === 'upgrade' ? [{ title: `${s.runtime} ${s.game_version}`, action: 'upgrade', to: b.upgrade?.game_version ?? null }]
      : b.kind === 'pack_update' ? [{ title: 'Lithium', action: 'update', to: '0.14.7' }, { title: 'Old Minimap', action: 'remove', to: null }] : [];
    const run: A.StagingRun = { id: c.uid(), server_id: s.id, kind: b.kind, state: 'building', include_world: !!b.include_world, report: { changes }, port: 30000 + list0.length, operation_id: null, created_at: c.now(), expires_at: new Date(Date.now() + DAY).toISOString(), expired: false, can_promote: false };
    list0.unshift(run);
    const o = c.op('staging.test', `Test a copy of ${s.name}`, s.id, ['Working out what would change', 'Copying the server', 'Starting the copy', 'Watching the copy for problems'], {});
    run.operation_id = o.id;
    advance(run, /broken/i.test(s.name));
    return { run, operation: o };
  }
  if (R(/^\/staging\/([^/]+)\/log$/)) return { lines: ['[10:00:01 INFO]: Starting minecraft server version 1.21.4', '[10:00:04 WARN]: Can\'t keep up! Is the server overloaded?', '[10:00:09 INFO]: Done (8.1s)! For help, type "help"'], truncated: false };
  if (R(/^\/staging\/([^/]+)\/promote$/)) {
    const run = (runs[s.id] ?? []).find((r) => r.id === dec(m![1]));
    if (!run) throw new ApiError(404, 'not_found', 'Test copy not found.');
    if (!run.can_promote) throw new ApiError(409, 'staging.not_healthy', 'Only a copy that passed its checks can be applied to the real server.');
    return { operation: c.op('staging.promote', `Apply the tested change to ${s.name}`, s.id, ['Taking a pre-change backup', 'Applying the change', 'Removing the test copy'], { done: () => { runs[s.id] = (runs[s.id] ?? []).filter((r) => r !== run); } }) };
  }
  if (R(/^\/staging\/([^/]+)$/)) {
    const run = (runs[s.id] ?? []).find((r) => r.id === dec(m![1]));
    if (!run) throw new ApiError(404, 'not_found', 'Test copy not found.');
    if (method === 'DELETE') { runs[s.id] = (runs[s.id] ?? []).filter((r) => r !== run); return { state: 'discarded' }; }
    return run;
  }
  if (sub === '/modpacks' && method === 'GET') return { packs: packsFor(s, c.uid, c.now) };
  if (R(/^\/modpacks\/([^/]+)\/check-update$/)) {
    await sleep(500);
    const pk = packsFor(s, c.uid, c.now).find((x) => x.id === dec(m![1]));
    if (!pk) throw new ApiError(404, 'not_found', 'That modpack is no longer installed.');
    if (pk.pinned) return { pinned: true, up_to_date: null, current: { version: pk.version, version_id: null }, message: 'This pack is pinned to its current version. Unpin it to check for updates.' } satisfies A.PackCheck;
    if (pk.version === '5.2.0') return { pinned: false, up_to_date: true, current: { version: pk.version, version_id: 'v520' }, latest: { version_id: 'v520', version: '5.2.0' } } satisfies A.PackCheck;
    return { pinned: false, up_to_date: false, current: { version: pk.version, version_id: 'v510' }, latest: { version_id: 'v520', version: '5.2.0' }, token: `pack-${pk.id}`, diff: packDiff(pk.version, '5.2.0') } satisfies A.PackCheck;
  }
  if (R(/^\/modpacks\/([^/]+)\/update$/)) {
    const pk = packsFor(s, c.uid, c.now).find((x) => x.id === dec(m![1]));
    if (!pk) throw new ApiError(404, 'not_found', 'That modpack is no longer installed.');
    if (pk.pinned) throw new ApiError(409, 'modpack.pinned', 'This pack is pinned to its current version; unpin it to update.');
    return { operation: c.op('modpack.update', `Update ${pk.name} on ${s.name}`, s.id, ['Comparing the new pack version with what is installed', 'Taking a pre-change backup', 'Downloading files (2/2)', 'Applying the pack update'], { done: () => { pk.version = '5.2.0'; c.pushUndo(s.id, `Update ${pk.name} to 5.2.0`, ['mods/']); } }) };
  }
  if (R(/^\/modpacks\/([^/]+)$/) && method === 'PATCH') {
    const pk = packsFor(s, c.uid, c.now).find((x) => x.id === dec(m![1]));
    if (!pk) throw new ApiError(404, 'not_found', 'That modpack is no longer installed.');
    pk.pinned = !!b.pinned;
    return { id: pk.id, pinned: pk.pinned };
  }
  if (sub === '/modpacks' && method === 'POST') {
    const pk = packs.get(b.token);
    if (!pk) throw new ApiError(410, 'modpack.token_expired', 'That modpack upload expired. Choose it again.');
    return { operation: c.op('modpack.import', `Importing ${pk.name}`, s.id, ['Creating pre-change backup', 'Downloading 41 files', 'Verifying hashes', 'Extracting overrides', 'Recording provenance'], { done: () => {
      if (b.mode === 'replace') list.splice(0, list.length, ...list.filter((x) => x.source === 'manual'));
      list.push(...packItems(s, b.token)); c.pushUndo(s.id, `Imported ${pk.name}`, ['mods/']); c.log('modpack.import', s.name, pk.name);
    } }) };
  }
  return NONE;
}

/** POST /modpacks/inspect (outside any server). */
export async function inspectModpack(body: unknown): Promise<A.ModpackInspect> {
  await sleep(700);
  return inspect(body);
}

const PACKS: Omit<ModpackHit, 'provider'>[] = [
  { project_id: 'fabulously-optimized', slug: 'fabulously-optimized', title: 'Fabulously Optimized', description: 'Improve performance and graphics with a curated set of Fabric mods.', author: 'robotkoer', icon_url: null, downloads: 8_400_000, game_versions: ['1.21.4', '1.21.3'], loaders: ['fabric'] },
  { project_id: 'better-mc-forge', slug: 'better-mc-forge', title: 'Better MC [FORGE]', description: 'A huge adventure pack with new dimensions, bosses and structures.', author: 'LunaPixelStudios', icon_url: null, downloads: 3_100_000, game_versions: ['1.20.1'], loaders: ['forge'] },
  { project_id: 'all-the-mods-10', slug: 'all-the-mods-10', title: 'All the Mods 10', description: 'Kitchen-sink pack with 400+ mods for NeoForge.', author: 'ATMTeam', icon_url: null, downloads: 1_250_000, game_versions: ['1.21.1'], loaders: ['neoforge'] },
  { project_id: 'cobblemon-fabric', slug: 'cobblemon-fabric', title: 'Cobblemon Official Modpack', description: 'Catch and battle Pokemon in Minecraft.', author: 'Cobblemon', icon_url: null, downloads: 920_000, game_versions: ['1.21.1'], loaders: ['fabric'] },
  { project_id: 'rlcraft', slug: 'rlcraft', title: 'RLCraft', description: 'Brutal survival: dragons, thirst and temperature.', author: 'Shivaxi', icon_url: null, downloads: 700_000, game_versions: ['1.12.2'], loaders: ['forge'] },
];
/** GET /modpacks/search */
export async function searchModpacks(qs: URLSearchParams): Promise<{ items: ModpackHit[]; total: number; next_offset: number | null }> {
  await sleep(250);
  const provider = (qs.get('provider') ?? 'modrinth') as A.ProviderId;
  const t = (qs.get('q') ?? '').trim().toLowerCase();
  const all = PACKS.filter((x) => !t || `${x.title} ${x.description} ${x.author}`.toLowerCase().includes(t)).map((x) => ({ ...x, provider, project_id: provider === 'curseforge' ? String(x.title.length * 1000 + x.downloads % 997) : x.project_id }));
  const off = Number(qs.get('offset') ?? 0), lim = Number(qs.get('limit') ?? 20);
  return { items: all.slice(off, off + lim), total: all.length, next_offset: off + lim < all.length ? off + lim : null };
}
