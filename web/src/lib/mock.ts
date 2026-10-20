// In-memory backend for `VITE_MOCK=1`. Never imported in production builds.
import type * as A from './api';
import { ApiError } from './api';
import { extraRuntimes, handleContent, inspectModpack, itemsFromPack, NONE, seedContent, versionsFor } from './mock-content';

const uid = () => crypto.randomUUID();
const now = () => new Date().toISOString();
const ago = (ms: number) => new Date(Date.now() - ms).toISOString();
const MIN = 60000, HOUR = 3600000, DAY = 86400000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const caps = ['console', 'files', 'content', 'backups', 'players', 'properties', 'schedules'];
function mk(p: Partial<A.Server> & { name: string; runtime: string }): A.Server {
  return {
    id: 'srv-' + p.name.toLowerCase().replace(/\W+/g, '-'), game_version: '1.21.4', loader_version: null, java_major: 21, port: 25565, memory_mb: 4096, flags_profile: 'balanced',
    autostart: true, restart_policy: { enabled: true, max_per_10m: 3 }, eula_accepted: true, desired: 'stopped', state: 'stopped',
    state_detail: null, state_since: ago(HOUR), uptime_s: null, pid: null, players: null, motd: null, cpu_pct: null, mem_bytes: null,
    content_kind: 'plugins', tags: [], notes: '', created_at: ago(30 * DAY), capabilities: caps, ...p,
  };
}
const servers: A.Server[] = [
  mk({ name: 'Survival SMP', runtime: 'paper', desired: 'running', state: 'running', state_since: ago(26 * HOUR), uptime_s: 26 * 3600 + 421, pid: 4211, players: { online: 3, max: 20, sample: ['Alex_Builds', 'creeper_hug', 'Notch2'] }, motd: 'Welcome to the Survival SMP', cpu_pct: 18, mem_bytes: 2.4e9, tags: ['friends', 'main'], notes: 'Weekly restart Sunday 04:00.' }),
  mk({ name: 'Creative Plots', runtime: 'purpur', port: 25566, memory_mb: 2048, tags: ['creative'], created_at: ago(12 * DAY) }),
  mk({ name: 'Modded Fabric', runtime: 'fabric', loader_version: '0.16.9', port: 25567, memory_mb: 6144, content_kind: 'mods', desired: 'running', state: 'crashed', state_detail: 'Exited with code 1 (OutOfMemoryError)', state_since: ago(14 * MIN), tags: ['mods'], created_at: ago(5 * DAY), capabilities: caps.filter((c) => c !== 'players') }),
  mk({ name: 'Velocity Proxy', runtime: 'velocity', game_version: '3.4.0', port: 25577, memory_mb: 512, java_major: 21, content_kind: 'proxy_plugins', desired: 'running', state: 'running', state_since: ago(3 * DAY), uptime_s: 3 * 86400 + 12, pid: 2210, players: { online: 3, max: 100, sample: [] }, cpu_pct: 2, mem_bytes: 3.1e8, tags: ['edge'], capabilities: caps.filter((c) => !['backups', 'properties'].includes(c)) }),
];
const sv = (id: string) => {
  const s = servers.find((x) => x.id === id);
  if (!s) throw new ApiError(404, 'server.not_found', 'Server not found');
  return s;
};

// ---- runtimes ----
const runtimes: A.Runtime[] = [
  { id: 'vanilla', name: 'Vanilla', category: 'official', tier: 'stable', content_kind: 'datapacks', description: 'Mojang\'s official server. No plugins or mods.', installable: true },
  { id: 'paper', name: 'Paper', category: 'plugins', tier: 'stable', content_kind: 'plugins', description: 'High-performance fork with Bukkit/Spigot plugin support. The sensible default.', installable: true },
  { id: 'purpur', name: 'Purpur', category: 'plugins', tier: 'stable', content_kind: 'plugins', description: 'Paper fork with extra gameplay configuration.', installable: true },
  { id: 'folia', name: 'Folia', category: 'plugins', tier: 'experimental', content_kind: 'plugins', description: 'Regionised multithreading for very large servers. Plugin support is limited.', installable: true },
  { id: 'fabric', name: 'Fabric', category: 'modded', tier: 'stable', content_kind: 'mods', description: 'Lightweight, fast-updating mod loader.', installable: true },
  { id: 'velocity', name: 'Velocity', category: 'proxy', tier: 'stable', content_kind: 'proxy_plugins', description: 'Modern proxy to link several servers together.', installable: true },
  { id: 'custom', name: 'Custom command', category: 'custom', tier: 'experimental', content_kind: 'none', description: 'Run any command in a managed directory.', installable: true },
  ...extraRuntimes,
];

// ---- operations & events ----
const ops: A.Operation[] = [];
type Handlers = A.EventHandlers;
const listeners = new Set<Handlers>();
const emitOp = (o: A.Operation) => listeners.forEach((l) => l.op(structuredClone(o)));
const emitServer = (s: A.Server) => listeners.forEach((l) => l.server(structuredClone(s)));
const emitAct = (a: A.Activity) => listeners.forEach((l) => l.activity(a));

const activity: A.Activity[] = [];
function log(action: string, target: string, detail = '') {
  const a = { id: uid(), time: now(), actor: 'dev@moss.local', action, target, detail };
  activity.unshift(a);
  emitAct(a);
}

function startOp(kind: string, title: string, server_id: string | null, stages: string[], opts: { fail?: boolean; done?: () => void; indeterminate?: boolean } = {}): A.Operation {
  const o: A.Operation = { id: uid(), kind, title, server_id, state: 'queued', stage: 'Queued', progress: 0, log: [], error: null, created_at: now(), started_at: null, ended_at: null, cancellable: true };
  ops.unshift(o);
  emitOp(o);
  let i = 0;
  const tick = setInterval(() => {
    if (o.state === 'cancelled') return clearInterval(tick);
    o.state = 'running';
    o.started_at ||= now();
    if (i < stages.length) {
      o.stage = stages[i];
      o.log.push(`[${new Date().toLocaleTimeString()}] ${stages[i]}…`);
      o.progress = opts.indeterminate && i % 2 === 0 ? null : i / stages.length;
      i++;
    } else {
      clearInterval(tick);
      o.ended_at = now();
      o.cancellable = false;
      o.progress = 1;
      if (opts.fail) { o.state = 'failed'; o.error = 'Download failed: connection reset by peer'; }
      else { o.state = 'completed'; o.stage = 'Done'; opts.done?.(); }
    }
    emitOp(o);
  }, 1100);
  return o;
}

// ---- virtual file system ----
const fs: Record<string, Record<string, string>> = {};
const BIN = /\.(jar|dat|gz|png|zip|mca)$/i;
const propsRaw = (s: A.Server) => `#Minecraft server properties\n#Generated by Moss\nmotd=${s.motd ?? 'A Minecraft Server'}\nmax-players=${s.players?.max ?? 20}\ngamemode=survival\ndifficulty=normal\nhardcore=false\npvp=true\nlevel-name=world\nlevel-seed=\nview-distance=10\nsimulation-distance=8\nspawn-protection=16\nallow-nether=true\ngenerate-structures=true\nspawn-monsters=true\nonline-mode=true\nwhite-list=false\nenforce-whitelist=false\nserver-port=${s.port}\nenable-command-block=false\nallow-flight=false\nenable-rcon=false\nsync-chunk-writes=true\n`;
function seedFs(s: A.Server) {
  fs[s.id] = {
    'server.properties': propsRaw(s),
    'eula.txt': '#By changing the setting below to TRUE you are indicating your agreement to the Minecraft EULA.\neula=true\n',
    'ops.json': '[\n  {\n    "uuid": "00000000-0000-0000-0000-000000000000",\n    "name": "Alex_Builds",\n    "level": 4\n  }\n]\n',
    'whitelist.json': '[]\n',
    'bukkit.yml': 'settings:\n  allow-end: true\n  warn-on-overload: true\n  connection-throttle: 4000\n',
    'logs/latest.log': '[10:00:01] [Server thread/INFO]: Starting minecraft server\n[10:00:08] [Server thread/INFO]: Done (6.2s)! For help, type "help"\n',
    'logs/2026-10-18-1.log.gz': '\0\0',
    'world/level.dat': '\0\0',
    'world/region/': '',
    'plugins/EssentialsX.jar': '\0'.repeat(1400),
    'plugins/Essentials/config.yml': '# EssentialsX config\nlocale: en\nteleport-cooldown: 5\n',
    'config/': '',
  };
}
servers.forEach(seedFs);
function listDir(id: string, path: string): A.FileList {
  const f = fs[id] ?? {};
  const prefix = path ? path.replace(/\/$/, '') + '/' : '';
  const dirs = new Set<string>();
  const entries: A.FileEntry[] = [];
  for (const [k, v] of Object.entries(f)) {
    if (!k.startsWith(prefix)) continue;
    const rest = k.slice(prefix.length);
    if (!rest) continue;
    const [first, ...more] = rest.split('/');
    if (more.length) dirs.add(first);
    else entries.push({ name: first, kind: 'file', size: v.length, modified: ago(((first.length * 7919) % 400) * MIN) });
  }
  const d = [...dirs].sort().map((name): A.FileEntry => ({ name, kind: 'dir', size: 0, modified: ago(3 * HOUR) }));
  entries.sort((a, b) => a.name.localeCompare(b.name));
  return { path, entries: [...d, ...entries.filter((e) => !e.name.endsWith('/'))] };
}

// ---- other fixtures ----
const content: Record<string, A.ContentItem[]> = {};
function seedContentFor(s: A.Server) { content[s.id] = seedContent(s); }
servers.forEach(seedContentFor);

const backups: A.Backup[] = [];
function seedBackups(s: A.Server, n: number) {
  for (let i = 0; i < n; i++)
    backups.push({ id: uid(), server_id: s.id, created_at: ago((i * 20 + 3) * HOUR), size: (1.2 + Math.sin(i) * 0.1) * 1e9, sha256: [...Array(64)].map(() => '0123456789abcdef'[(Math.random() * 16) | 0]).join(''), status: 'ok', verified_at: i % 2 ? ago(i * 10 * HOUR) : null, note: i === 0 ? 'Before updating plugins' : '', kind: i % 3 === 0 ? 'manual' : i % 3 === 1 ? 'scheduled' : 'pre-change', consistent: i !== 2, files: 1800 + i * 40 });
}
seedBackups(servers[0], 5);
seedBackups(servers[2], 2);
const schedules: A.Schedule[] = [
  { id: uid(), server_id: servers[0].id, kind: 'backup', command: null, every_minutes: null, daily_at: '04:00', tz_offset_min: 0, enabled: true, keep: 7, last_run: ago(20 * HOUR), next_run: new Date(Date.now() + 4 * HOUR).toISOString(), last_status: 'ok' },
  { id: uid(), server_id: servers[0].id, kind: 'command', command: 'say Saving soon', every_minutes: 180, daily_at: null, tz_offset_min: 0, enabled: false, keep: null, last_run: null, next_run: null, last_status: null },
];
const undo: Record<string, A.UndoEntry[]> = {};
const pushUndo = (sid: string, title: string, files: string[]) => {
  const e = { id: uid(), title, created_at: now(), files };
  (undo[sid] ||= []).unshift(e);
  return e.id;
};
undo[servers[0].id] = [{ id: uid(), title: 'Edited server.properties', created_at: ago(5 * HOUR), files: ['server.properties'] }, { id: uid(), title: 'Installed Chunky', created_at: ago(2 * DAY), files: ['plugins/Chunky-1.4.jar'] }];

const users: A.User[] = [
  { id: uid(), email: 'dev@moss.local', role: 'owner', created_at: ago(40 * DAY) },
  { id: uid(), email: 'sam@example.com', role: 'admin', created_at: ago(20 * DAY) },
  { id: uid(), email: 'jo@example.com', role: 'user', created_at: ago(3 * DAY) },
];
let grants: A.Grant[] = [{ user_id: users[2].id, email: users[2].email, bundle: 'operator' }];
const settings: A.Settings = { default_memory_mb: 4096, backup_keep: 10, backup_before_change: true, modrinth_contact: 'owner@example.com', public_base_url: 'https://moss.example.com', data_dir: '/var/lib/moss', bind: '127.0.0.1:8420', version: '0.1.0-mock' };
const javaInstalls: A.JavaInstall[] = [{ major: 21, path: '/var/lib/moss/java/21', managed: true, version: '21.0.5+11' }, { major: 17, path: '/usr/lib/jvm/java-17', managed: false, version: '17.0.13' }];

function stats(s: A.Server): A.StatSample[] {
  const base = s.cpu_pct ?? 0;
  return [...Array(720)].map((_, i) => ({
    t: Date.now() - (719 - i) * 5000,
    cpu: Math.max(0, base + Math.sin(i / 14) * 8 + Math.sin(i / 3) * 3 + (i > 650 ? 10 : 0)),
    mem: (s.mem_bytes ?? 0) * (0.85 + 0.15 * (i / 720)) + Math.sin(i / 20) * 4e7,
    players: s.players ? Math.max(0, Math.round(s.players.online + Math.sin(i / 40) * 1.4)) : null,
  }));
}
function timeline(s: A.Server): A.TimelineEvent[] {
  const t = (h: number, kind: A.TimelineEvent['kind'], detail: string) => ({ time: ago(h * HOUR), kind, detail });
  const base: A.TimelineEvent[] = [t(23.5, 'start', 'Started'), t(23.4, 'ready', 'Done (6.2s)'), t(12, 'backup', 'Scheduled backup'), t(7, 'update', 'Updated Chunky')];
  if (s.state === 'crashed') return [t(20, 'start', 'Started'), t(19.9, 'ready', 'Ready'), t(1, 'crash', 'OutOfMemoryError'), t(0.9, 'restart', 'Auto restart'), t(0.5, 'crash', 'OutOfMemoryError'), t(0.25, 'crash', 'Exited with code 1')];
  if (s.state === 'stopped') return [t(10, 'stop', 'Stopped by dev'), t(9, 'backup', 'Backup'), t(2, 'undo', 'Reverted server.properties')];
  return [...base, t(3, 'restart', 'Restarted by sam')];
}
function health(): A.Finding[] {
  const f: A.Finding[] = [];
  for (const s of servers) {
    if (s.state === 'crashed') f.push({ id: 'crash-' + s.id, severity: 'critical', server_id: s.id, title: `${s.name} is crash-looping`, detail: 'The server crashed 3 times in the last 30 minutes and automatic restart has been paused.', evidence: ['java.lang.OutOfMemoryError: Java heap space', 'Allocated memory: 6144 MB', 'Last exit code: 1'], action: { label: 'Open console', kind: 'open_tab', tab: 'console' } });
    if (!eulaFixed.has(s.id) && s.runtime === 'purpur') f.push({ id: 'eula-' + s.id, severity: 'warn', server_id: s.id, title: `${s.name}: EULA not accepted`, detail: 'The server cannot start until the Minecraft EULA is accepted.', evidence: ['eula.txt: eula=false'], action: { label: 'Review settings', kind: 'accept_eula', tab: 'settings' } });
    if (['paper', 'purpur', 'fabric'].includes(s.runtime) && !backups.some((b) => b.server_id === s.id && Date.now() - Date.parse(b.created_at) < DAY * 3)) f.push({ id: 'bk-' + s.id, severity: 'warn', server_id: s.id, title: `${s.name} has no recent backup`, detail: 'The newest backup is older than 3 days (or there is none).', evidence: [`Backups: ${backups.filter((b) => b.server_id === s.id).length}`], action: { label: 'Back up now', kind: 'backup' } });
  }
  f.push({ id: 'disk', severity: 'info', server_id: null, title: 'Java 25 is not installed', detail: 'Snapshots newer than 1.21 may need Java 25. Install it to run them.', evidence: ['Installed: 17, 21'], action: { label: 'Install Java 25', kind: 'install_java' } });
  return f;
}
const eulaFixed = new Set<string>();

// ---- server lifecycle ----
function setState(s: A.Server, st: A.ServerState, detail: string | null = null) {
  s.state = st;
  s.state_detail = detail;
  s.state_since = now();
  if (st === 'running') { s.uptime_s = 1; s.pid = 1000 + ((Math.random() * 9000) | 0); s.players = { online: 0, max: s.players?.max ?? 20, sample: [] }; s.cpu_pct = 12; s.mem_bytes = s.memory_mb * 1048576 * 0.5; }
  else if (st !== 'starting' && st !== 'stopping') { s.uptime_s = null; s.pid = null; s.players = null; s.cpu_pct = null; s.mem_bytes = null; }
  emitServer(s);
}
function doAction(s: A.Server, a: string) {
  if (a === 'start') {
    s.desired = 'running'; setState(s, 'starting');
    log('server.start', s.name);
    setTimeout(() => setState(s, 'running'), 2500);
  } else if (a === 'stop' || a === 'kill') {
    s.desired = 'stopped'; setState(s, 'stopping');
    log('server.' + a, s.name);
    setTimeout(() => setState(s, 'stopped'), 1500);
  } else if (a === 'restart') {
    log('server.restart', s.name);
    setState(s, 'stopping');
    setTimeout(() => { setState(s, 'starting'); setTimeout(() => setState(s, 'running'), 2200); }, 1200);
  }
}
setInterval(() => {
  for (const s of servers) if (s.state === 'running') {
    s.uptime_s = (s.uptime_s ?? 0) + 3;
    s.cpu_pct = Math.max(1, Math.min(95, (s.cpu_pct ?? 10) + (Math.random() - 0.5) * 6));
    emitServer(s);
  }
}, 3000);

// ---- console ----
const chatter = [
  '[%T] [Server thread/INFO]: <Alex_Builds> anyone up for the nether run?',
  '[%T] [Server thread/INFO]: creeper_hug joined the game',
  '[%T] [Server thread/INFO]: Alex_Builds has made the advancement [Stone Age]',
  '[%T] [Server thread/WARN]: Can\'t keep up! Is the server overloaded? Running 2312ms or 46 ticks behind',
  '[%T] [Server thread/INFO]: \x1b[32m[Chunky]\x1b[0m Task finished for world. Processed: 4096 chunks (100%)',
  '[%T] [Server thread/INFO]: \x1b[36m[LuckPerms]\x1b[0m Permissions cache refreshed',
  '[%T] [Async Chat Thread - #3/INFO]: <Notch2> brb',
  '[%T] [Server thread/ERROR]: Could not pass event PlayerMoveEvent to Essentials\n',
  '[%T] [Server thread/DEBUG]: Saving chunk 12,-4',
];
const T = () => new Date().toLocaleTimeString('en-GB');
const bl = (_s: A.Server) => [...Array(60)].map((_, i) => chatter[i % chatter.length].replace('%T', T()).trimEnd());
export function openConsole(id: string, h: { onOpen(): void; onClose(): void; onMsg(m: A.ConsoleMsg): void }): A.ConsoleSocket {
  const s = sv(id);
  setTimeout(h.onOpen, 150);
  const iv = setInterval(() => {
    if (s.state === 'running' && Math.random() < 0.7) h.onMsg({ t: 'line', d: chatter[(Math.random() * chatter.length) | 0].replace('%T', T()).trimEnd() });
  }, 1200);
  let last = s.state;
  const off = { op: () => {}, activity: () => {}, onOpen() {}, onError() {}, server: (x: A.Server) => { if (x.id === id && x.state !== last) { last = x.state; h.onMsg({ t: 'state', d: x.state }); } } };
  listeners.add(off);
  return {
    close() { clearInterval(iv); listeners.delete(off); },
    send(cmd) {
      h.onMsg({ t: 'line', d: `[${T()}] [Server thread/INFO]: \x1b[90m> ${cmd}\x1b[0m` });
      setTimeout(() => h.onMsg({ t: 'line', d: `[${T()}] [Server thread/INFO]: Unknown or incomplete command (mock), try "help"` }), 200);
    },
  };
}

export function openEvents(h: Handlers): A.Stream {
  listeners.add(h);
  setTimeout(h.onOpen, 100);
  return { close: () => listeners.delete(h) };
}
export async function blobFor(path: string) { return new Blob([`mock download of ${path}\n`]); }

// ---- router ----
const V = (pid: string, cf = false): A.Project => ({
  project_id: pid, title: [...hits, ...cfHits].find((h) => h.project_id === pid)?.title ?? pid, description: 'A fantastic project.', icon_url: null,
  body: 'This is the long description of the project.\n\nFeatures:\n- Fast\n- Configurable\n- Well maintained', links: cf ? { page: 'https://www.curseforge.com/minecraft/bukkit-plugins/' + pid } : { source: 'https://github.com', issues: 'https://github.com', wiki: 'https://modrinth.com' }, license: 'MIT', gallery: [],
  versions: ['2.4.1', '2.4.0', '2.3.9-beta.1', '2.3.8'].map((v, i) => ({ id: pid + v, name: `v${v}`, version_number: v, channel: v.includes('beta') ? 'beta' : 'release', date: ago((i * 12 + 2) * DAY), downloads: 120000 - i * 20000, files: [{ filename: `${pid}-${v}.jar`, size: 2.1e6, primary: true }], game_versions: ['1.21.4', '1.21.3'], loaders: ['paper', 'fabric'], dependencies: i === 0 ? [{ project_id: 'fabric-api', kind: 'required' as const }] : [], ...(cf && i === 1 ? { downloadable: false } : {}) })),
});
const hits: A.Hit[] = [
  ['luckperms', 'LuckPerms', 'Luck', 'A permissions plugin for Minecraft servers.', 12400000, ['management', 'utility']],
  ['chunky', 'Chunky', 'pop4959', 'Pre-generates chunks, quickly and efficiently.', 9300000, ['worldgen', 'utility']],
  ['worldedit', 'WorldEdit', 'EngineHub', 'In-game voxel editor for builders.', 8100000, ['world-management']],
  ['spark', 'spark', 'lucko', 'A performance profiler for Minecraft.', 7800000, ['optimization']],
  ['essentialsx', 'EssentialsX', 'EssentialsX', 'The essential plugin suite for servers.', 6000000, ['utility', 'economy']],
  ['griefprevention', 'GriefPrevention', 'RoboMWM', 'Claim land and stop griefers.', 2100000, ['protection']],
  ['dynmap', 'Dynmap', 'mikeprimm', 'Live Google-Maps style web map.', 3500000, ['utility']],
  ['coreprotect', 'CoreProtect', 'Intelli', 'Fast, efficient block logging and rollback.', 2800000, ['protection', 'management']],
].map(([project_id, title, author, description, downloads, categories]) => ({ project_id: project_id as string, slug: project_id as string, title: title as string, description: description as string, author: author as string, icon_url: null, downloads: downloads as number, follows: Math.round((downloads as number) / 40), categories: categories as string[], updated: ago(((title as string).length % 9) * DAY), client_side: 'unsupported', server_side: 'required' }));

const cfHits: A.Hit[] = [['cf-worldguard', 'WorldGuard', 'sk89q', 'Protect areas of your world with regions.', 5400000, ['admin-tools']], ['cf-jei', 'Just Enough Items', 'mezz', 'View items and recipes.', 90000000, ['utility']], ['cf-dynmap', 'Dynmap', 'mikeprimm', 'Live web map.', 3000000, ['map-information']]].map(([project_id, title, author, description, downloads, categories]) => ({ project_id: project_id as string, slug: project_id as string, title: title as string, description: description as string, author: author as string, icon_url: null, downloads: downloads as number, follows: 0, categories: categories as string[], updated: ago(3 * DAY), client_side: null, server_side: null }));

export async function handle(method: string, rawPath: string, body: unknown): Promise<unknown> {
  await sleep(50 + Math.random() * 120);
  const u = new URL(rawPath, 'http://x');
  const p = u.pathname;
  const qs = u.searchParams;
  let m: RegExpMatchArray | null;
  const M = (re: RegExp) => (m = p.match(re));
  const b = (body ?? {}) as Record<string, any>;
  const op = (...a: Parameters<typeof startOp>) => ({ operation: startOp(...a) });

  if (p === '/auth/config') return { mode: 'dev', owner_claimed: true };
  if (p === '/me') return { id: users[0].id, email: users[0].email, role: 'owner', permissions: ['server.view', 'server.power', 'server.console.read', 'server.console.write', 'files.read', 'files.write', 'content.manage', 'backup.manage', 'config.write', 'server.delete', 'admin.users', 'admin.settings'] };
  if (p === '/ticket') return { ticket: 'mock' };
  if (p === '/status') return { version: '0.1.0-mock', uptime_s: 9 * 86400 + 4000, auth_mode: 'dev', host: { cpu_pct: 24 + Math.random() * 12, mem_total: 16e9, mem_used: 7.4e9 + Math.random() * 3e8, disk_total: 500e9, disk_free: 312e9, load1: 0.84, hostname: 'moss-host' }, counts: { servers: servers.length, running: servers.filter((s) => s.state === 'running').length, players: servers.reduce((n, s) => n + (s.players?.online ?? 0), 0) } };
  if (p === '/runtimes') return runtimes;
  if (M(/^\/runtimes\/([^/]+)\/versions$/)) return versionsFor(m![1]);
  if (p === '/modpacks/inspect') return inspectModpack(body);
  if (p === '/java' && method === 'GET') return { installs: javaInstalls };
  if (p === '/java/install') return op('java.install', `Installing Java ${b.major}`, null, ['Resolving build', 'Downloading JDK', 'Verifying checksum', 'Extracting'], { done: () => javaInstalls.push({ major: b.major, path: `/var/lib/moss/java/${b.major}`, managed: true, version: `${b.major}.0.1` }) });
  if (p === '/health') return health();
  if (p === '/operations') return ops.slice(0, +(qs.get('limit') ?? 50)).filter((o) => !qs.get('server_id') || o.server_id === qs.get('server_id'));
  if (M(/^\/operations\/([^/]+)\/cancel$/)) { const o = ops.find((x) => x.id === m![1]); if (o) { o.state = 'cancelled'; o.stage = 'Cancelled'; o.ended_at = now(); o.cancellable = false; emitOp(o); } return; }
  if (p === '/activity') return activity.filter((a) => !qs.get('server_id') || a.target.includes(sv(qs.get('server_id')!).name)).slice(0, +(qs.get('limit') ?? 100));
  if (p === '/backups' && method === 'GET') return backups.slice().sort((a, b) => b.created_at.localeCompare(a.created_at));
  if (M(/^\/backups\/([^/]+)\/verify$/)) { const k = backups.find((x) => x.id === m![1])!; k.status = 'verifying'; return op('backup.verify', 'Verifying backup', k.server_id, ['Reading archive', 'Checking hashes'], { done: () => { k.status = 'ok'; k.verified_at = now(); } }); }
  if (M(/^\/backups\/([^/]+)\/restore$/)) { const k = backups.find((x) => x.id === m![1])!; return op('backup.restore', b.mode === 'in_place' ? 'Restoring backup in place' : 'Restoring backup to a new server', k.server_id, ['Pre-restore backup', 'Extracting', 'Finalising'], { done: () => { if (b.mode === 'new_server') { const s = mk({ name: sv(k.server_id).name + ' (restored)', runtime: sv(k.server_id).runtime, port: 25600 }); servers.push(s); seedFs(s); content[s.id] = []; emitServer(s); } } }); }
  if (M(/^\/backups\/([^/]+)$/) && method === 'DELETE') { const i = backups.findIndex((x) => x.id === m![1]); if (i >= 0) backups.splice(i, 1); return; }
  if (M(/^\/schedules\/([^/]+)\/run$/)) { log('schedule.run', 'schedule'); return; }
  if (M(/^\/schedules\/([^/]+)$/)) { const i = schedules.findIndex((x) => x.id === m![1]); if (method === 'DELETE') schedules.splice(i, 1); else Object.assign(schedules[i], b); return schedules[i]; }
  if (M(/^\/undo\/([^/]+)\/revert$/)) { for (const l of Object.values(undo)) { const i = l.findIndex((x) => x.id === m![1]); if (i >= 0) { const [e] = l.splice(i, 1); log('undo.revert', e.title); } } return { ok: true }; }
  if (p === '/users') { if (method === 'POST') { const u = { id: uid(), email: b.email, role: b.role, created_at: now() }; users.push(u); return u; } return users; }
  if (M(/^\/users\/([^/]+)$/)) { const i = users.findIndex((x) => x.id === m![1]); if (method === 'DELETE') users.splice(i, 1); else users[i].role = b.role; return; }
  if (p === '/settings') { if (method === 'PUT') Object.assign(settings, b); return settings; }
  if (p === '/content/search') {
    const qq = (qs.get('q') ?? '').toLowerCase();
    const s = sv(qs.get('server_id')!);
    const r = (qs.get('provider') === 'curseforge' ? cfHits : hits).filter((h) => !qq || (h.title + h.description).toLowerCase().includes(qq));
    return { hits: r.slice(+(qs.get('offset') ?? 0), +(qs.get('offset') ?? 0) + +(qs.get('limit') ?? 20)), total: r.length, compat: { kind: s.content_kind, loader: s.runtime, game_version: s.game_version } };
  }
  if (p === '/content/providers') { const rt = sv(qs.get('server_id')!).runtime; return { providers: [{ id: 'modrinth', name: 'Modrinth', available: true, supported: true, reason: null }, { id: 'curseforge', name: 'CurseForge', available: true, supported: rt !== 'velocity', reason: rt === 'velocity' ? 'CurseForge has no proxy plugins' : null }] }; }
  if (M(/^\/content\/projects\/([^/]+)$/)) return V(decodeURIComponent(m![1]), qs.get('provider') === 'curseforge');

  if (p === '/servers') {
    if (method === 'GET') return servers;
    const s = mk({ name: b.name, runtime: b.runtime, game_version: b.game_version, loader_version: b.loader_version ?? null, memory_mb: b.memory_mb, port: b.port ?? 25570 + servers.length, desired: 'stopped', state: 'installing', content_kind: runtimes.find((r) => r.id === b.runtime)?.content_kind ?? 'none', eula_accepted: !!b.accept_eula });
    servers.push(s); seedFs(s); content[s.id] = b.modpack_token ? itemsFromPack(s, b.modpack_token) : []; emitServer(s);
    log('server.create', s.name);
    const o = startOp('server.install', `Installing ${s.name}`, s.id, ['Resolving build', 'Downloading server jar', 'Verifying checksum', 'Writing config', 'First-run setup'], { done: () => setState(s, 'stopped') });
    return { server: s, operation: o };
  }
  if (M(/^\/servers\/([^/]+)(\/.*)?$/)) {
    const s = sv(m![1]);
    const sub = m![2] ?? '';
    let mm: RegExpMatchArray | null;
    const S = (re: RegExp) => (mm = sub.match(re));
    if (sub === '') {
      if (method === 'GET') return s;
      if (method === 'PATCH') { if (b.accept_eula) { s.eula_accepted = true; eulaFixed.add(s.id); } delete b.accept_eula; Object.assign(s, b); emitServer(s); log('server.update', s.name, Object.keys(b).join(', ')); return s; }
      if (method === 'DELETE') { servers.splice(servers.indexOf(s), 1); log('server.delete', s.name); return; }
    }
    if (S(/^\/actions\/(\w+)$/)) { doAction(s, mm![1]); return { server: s }; }
    if (sub === '/command') { log('server.command', s.name, b.command); return; }
    if (sub === '/stats') return { samples: stats(s) };
    if (sub === '/timeline') return timeline(s);
    if (sub === '/health') return health().filter((f) => f.server_id === s.id);
    if (sub === '/properties') {
      if (method === 'GET') { const raw = fs[s.id]['server.properties']; return { raw, entries: raw.split('\n').filter((l) => l && !l.startsWith('#')).map((l) => ({ key: l.split('=')[0], value: l.slice(l.indexOf('=') + 1) })) }; }
      fs[s.id]['server.properties'] = b.raw;
      log('config.save', s.name, 'server.properties');
      return { undo_id: pushUndo(s.id, 'Edited server.properties', ['server.properties']) };
    }
    if (sub === '/console/backlog') return { lines: bl(s) };
    if (sub === '/files') { if (method === 'DELETE') { const pth = qs.get('path')!; for (const k of Object.keys(fs[s.id])) if (k === pth || k.startsWith(pth + '/')) delete fs[s.id][k]; return; } return listDir(s.id, qs.get('path') ?? ''); }
    if (sub === '/files/content') {
      if (method === 'PUT') { fs[s.id][b.path] = b.content; log('file.save', s.name, b.path); return { undo_id: pushUndo(s.id, `Edited ${b.path}`, [b.path]) }; }
      const pth = qs.get('path')!;
      const c = fs[s.id][pth];
      if (c === undefined) throw new ApiError(404, 'file.not_found', 'File not found');
      return { content: BIN.test(pth) ? '' : c, size: c.length, truncated: false, binary: BIN.test(pth) };
    }
    if (sub === '/files/mkdir') { fs[s.id][b.path + '/'] = ''; return; }
    if (sub === '/files/rename') { for (const k of Object.keys(fs[s.id])) if (k === b.from || k.startsWith(b.from + '/')) { fs[s.id][b.to + k.slice(b.from.length)] = fs[s.id][k]; delete fs[s.id][k]; } return; }
    if (sub === '/files/upload') { const pth = qs.get('path') ?? ''; for (const f of (body as FormData).getAll('file') as File[]) fs[s.id][(pth ? pth + '/' : '') + f.name] = BIN.test(f.name) ? '\0'.repeat(Math.min(f.size, 5000)) : await f.text(); return; }
    if (/^\/(content|upgrade|modpacks)(\/|$)/.test(sub)) {
      const r = await handleContent({ uid, now, ago, op: (...a) => startOp(...a), hits, cfHits, log, pushUndo, backups, content, emitServer }, s, sub, method, b);
      if (r !== NONE) return r;
    }
    if (sub === '/backups') {
      if (method === 'GET') return backups.filter((x) => x.server_id === s.id).sort((a, b) => b.created_at.localeCompare(a.created_at));
      return op('backup.create', `Backing up ${s.name}`, s.id, ['Flushing world', 'Archiving', 'Hashing'], { indeterminate: true, done: () => { backups.push({ id: uid(), server_id: s.id, created_at: now(), size: 1.3e9, sha256: 'ab'.repeat(32), status: 'ok', verified_at: null, note: b.note ?? '', kind: 'manual', consistent: true, files: 1900 }); } });
    }
    if (sub === '/schedules') { if (method === 'GET') return schedules.filter((x) => x.server_id === s.id); const n = { id: uid(), server_id: s.id, command: null, every_minutes: null, daily_at: null, tz_offset_min: 0, enabled: true, keep: null, last_run: null, next_run: ago(-HOUR), last_status: null, ...b } as A.Schedule; schedules.push(n); return n; }
    if (sub === '/undo') return undo[s.id] ?? [];
    if (sub === '/access') { if (method === 'PUT') { grants = (b.grants as any[]).map((g) => ({ ...g, email: users.find((u) => u.id === g.user_id)?.email ?? '' })); return; } return grants; }
  }
  throw new ApiError(404, 'mock.not_found', `Mock has no route for ${method} ${p}`);
}
