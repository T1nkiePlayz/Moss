// In-memory backend for `VITE_MOCK=1`. Never imported in production builds.
import type * as A from './api';
import { ApiError } from './api';
import { handleWebhooks, NONE as NOT_WEBHOOK } from './mock-webhooks';
import { handleWorkflows, NONE as NOT_WORKFLOW } from './mock-workflows';
import { handleRemote, NONE as NOT_REMOTE, seedRemote } from './mock-remote';
import { extraRuntimes, handleContent, hangarHits, inspectModpack, itemsFromPack, NONE, seedContent, versionsFor } from './mock-content';
import { handleOps, handleOpsGlobal, revertConfig } from './mock-ops';
import { nextDailyRun, offsetName } from './tz';

const uid = () => crypto.randomUUID();
const now = () => new Date().toISOString();
const ago = (ms: number) => new Date(Date.now() - ms).toISOString();
const MIN = 60000, HOUR = 3600000, DAY = 86400000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const caps = ['console', 'files', 'content', 'backups', 'players', 'properties', 'schedules', 'java', 'eula'];
/** Capabilities of a non-Minecraft game server: no Java, content or EULA; players when the game can be queried. */
const gameCaps = (...extra: string[]) => ['console', 'files', 'backups', 'schedules', ...extra];
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
  mk({ name: 'Terraria', runtime: 'terraria', game_version: '1.4.4.9', java_major: 0, port: 7777, memory_mb: 2048, content_kind: 'none', desired: 'running', state: 'running', state_since: ago(9 * HOUR), uptime_s: 9 * 3600 + 80, pid: 5120, players: { online: 2, max: 8, sample: ['Guide', 'Dryad_fan'] }, cpu_pct: 4, mem_bytes: 6.2e8, tags: ['game'], created_at: ago(9 * DAY), capabilities: gameCaps('players') }),
  mk({ name: 'Valheim', runtime: 'valheim', game_version: 'latest', java_major: 0, port: 2456, memory_mb: 4096, content_kind: 'none', desired: 'running', state: 'running', state_since: ago(2 * DAY), uptime_s: 2 * 86400 + 900, pid: 5230, players: { online: 4, max: 10, sample: ['Ragnar', 'Freya', 'Bjorn', 'Astrid'] }, cpu_pct: 31, mem_bytes: 3.4e9, tags: ['game', 'friends'], created_at: ago(20 * DAY), capabilities: gameCaps('players') }),
  mk({ name: 'Factorio', runtime: 'factorio', game_version: '2.0.55', java_major: 0, port: 34197, memory_mb: 2048, content_kind: 'none', desired: 'running', state: 'running', state_since: ago(5 * HOUR), uptime_s: 5 * 3600 + 12, pid: 5340, players: { online: 1, max: 0, sample: ['engineer42'] }, cpu_pct: 22, mem_bytes: 1.1e9, tags: ['game'], created_at: ago(14 * DAY), capabilities: gameCaps('players') }),
  mk({ name: 'Bedrock Realm', runtime: 'bedrock', game_version: '1.21.124.2', java_major: 0, port: 19132, memory_mb: 2048, content_kind: 'none', desired: 'stopped', state: 'stopped', tags: ['game', 'bedrock'], created_at: ago(6 * DAY), capabilities: gameCaps('players', 'properties', 'eula') }),
  mk({ name: 'CS2 Wingman', runtime: 'cs2', game_version: 'latest', java_major: 0, port: 27015, memory_mb: 4096, content_kind: 'none', desired: 'running', state: 'running', state_since: ago(3 * HOUR), uptime_s: 3 * 3600 + 300, pid: 5450, players: { online: 6, max: 10, sample: ['s1mple_fan', 'AWPer', 'rush_b', 'eco_king', 'smoke_me', 'flashbang'] }, motd: 'de_dust2', cpu_pct: 46, mem_bytes: 5.8e9, tags: ['game', 'lan'], created_at: ago(3 * DAY), capabilities: gameCaps('players') }),
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
  ...gameRuntimes(),
];
function gameRuntimes(): A.Runtime[] {
  const g = (id: string, name: string, description: string, port: number, protocol: 'udp' | 'tcp', platforms: string[], tracking: 'none' | 'log' | 'a2s', max: number, extra: Partial<A.Runtime> = {}): A.Runtime =>
    ({ id, name, category: 'game', tier: 'experimental', content_kind: 'none', description, installable: true, default_port: port, protocol, platforms, player_tracking: tracking, max_players: max, ...extra });
  const steam = 'via SteamCMD (anonymous). Linux x86_64 only.';
  return [
    g('bedrock', 'Minecraft Bedrock', 'Official Bedrock Dedicated Server (Windows 10/11, consoles, phones). Linux only. Needs the Minecraft EULA.', 19132, 'udp', ['linux'], 'log', 10, { tier: 'stable', needs_eula: true }),
    g('terraria', 'Terraria', 'Official Terraria dedicated server. Creates a medium world on first start. Linux and macOS.', 7777, 'tcp', ['linux', 'macos'], 'log', 8, { tier: 'stable' }),
    g('factorio', 'Factorio', 'Official headless Factorio server (stable or experimental). Creates a save on install. Linux only.', 34197, 'udp', ['linux'], 'log', 0, { tier: 'stable' }),
    g('valheim', 'Valheim', `Valheim dedicated server ${steam} A password is generated into moss-game.cfg.`, 2456, 'udp', ['linux'], 'a2s', 10),
    g('gmod', "Garry's Mod", `Garry's Mod dedicated server ${steam} Starts on gm_construct.`, 27015, 'udp', ['linux'], 'a2s', 16),
    g('cs2', 'Counter-Strike 2', `Counter-Strike 2 dedicated server ${steam} Large download (30 GB or more).`, 27015, 'udp', ['linux'], 'a2s', 10),
    g('tf2', 'Team Fortress 2', `Team Fortress 2 dedicated server ${steam} Starts on ctf_2fort.`, 27015, 'udp', ['linux'], 'a2s', 24),
    g('l4d2', 'Left 4 Dead 2', `Left 4 Dead 2 dedicated server ${steam}`, 27015, 'udp', ['linux'], 'a2s', 8),
    g('rust', 'Rust', `Rust dedicated server ${steam} Needs 8 GB or more RAM.`, 28015, 'udp', ['linux'], 'a2s', 50),
    g('palworld', 'Palworld', `Palworld dedicated server ${steam} 16 GB RAM recommended. Player counts are not available.`, 8211, 'udp', ['linux'], 'none', 32),
    g('zomboid', 'Project Zomboid', `Project Zomboid dedicated server ${steam} Admin password is generated into moss-game.cfg.`, 16261, 'udp', ['linux'], 'none', 16),
    g('satisfactory', 'Satisfactory', `Satisfactory dedicated server ${steam} 12 GB RAM recommended.`, 7777, 'udp', ['linux'], 'none', 4),
  ];
}
const gameVersions: Record<string, A.Versions> = {
  bedrock: { versions: [{ id: '1.21.124.2', stable: true }], recommended: '1.21.124.2' },
  terraria: { versions: [{ id: '1.4.5.0', stable: true }, { id: '1.4.4.9', stable: true }], recommended: '1.4.5.0' },
  factorio: { versions: [{ id: '2.0.55', stable: true }, { id: '2.1.1', stable: false }], recommended: '2.0.55' },
};
for (const id of ['valheim', 'gmod', 'cs2', 'tf2', 'l4d2', 'rust', 'palworld', 'zomboid', 'satisfactory']) gameVersions[id] = { versions: [{ id: 'latest', stable: true }], recommended: 'latest' };

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
const gameWorlds: Record<string, string[]> = { valheim: ['Dedicated'], terraria: ['moss'], factorio: ['moss'], cs2: [], bedrock: ['Bedrock level'] };
const srvNamed = (n: string) => servers.find((x) => x.name === n)!;
function seedBackups(s: A.Server, n: number, o: { note?: string; size?: number; offset?: number } = {}) {
  const size = o.size ?? 1.2e9, off = o.offset ?? 3;
  for (let i = 0; i < n; i++)
    backups.push({ id: uid(), server_id: s.id, created_at: ago((i * 20 + off) * HOUR), size: size * (1 + Math.sin(i) * 0.08), sha256: [...Array(64)].map(() => '0123456789abcdef'[(Math.random() * 16) | 0]).join(''), status: 'ok', verified_at: i % 2 ? ago(i * 10 * HOUR) : null, note: i === 0 ? (o.note ?? 'Before updating plugins') : '', kind: i % 3 === 0 ? 'manual' : i % 3 === 1 ? 'scheduled' : 'pre-change', consistent: i !== 2, files: Math.round(size / 6e5) + i * 40 });
}
seedBackups(servers[0], 5);
seedBackups(servers[2], 2);
seedBackups(srvNamed('Valheim'), 3, { note: 'Before the Ashlands trip', size: 2.1e8, offset: 1 });
seedBackups(srvNamed('Terraria'), 2, { note: 'Pre-hardmode', size: 4.4e7, offset: 2 });
seedBackups(srvNamed('Factorio'), 2, { note: 'Before rocket launch', size: 9.6e7, offset: 4 });
seedBackups(srvNamed('CS2 Wingman'), 1, { note: 'Server configs', size: 3.1e6, offset: 6 });
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
    if (s.container) s.container.running_in_container = s.isolation === 'container';
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
const gameChatter: Record<string, string[]> = {
  cs2: [
    'L %D - %T: "s1mple_fan<3><[U:1:1001]><CT>" killed "rush_b<5><[U:1:1003]><TERRORIST>" with "awp" (headshot)',
    'L %D - %T: World triggered "Round_Start"',
    'L %D - %T: "AWPer<4><[U:1:1002]><CT>" say "nice shot"',
    'L %D - %T: Team "CT" scored "7" with "3" players',
    'L %D - %T: "eco_king<6><[U:1:1004]><TERRORIST>" purchased "deagle"',
    'L %D - %T: World triggered "Round_End"',
    'L %D - %T: "smoke_me<7><[U:1:1005]><TERRORIST>" threw smokegrenade [-512 1120 64]',
    'L %D - %T: Started map "de_dust2" (CRC "-1846721")',
  ],
  terraria: ['Guide has joined.', '<Dryad_fan> anyone seen the Eye of Cthulhu?', 'Saving world data: 100%', 'Validating world save: 100%', 'Backing up world file', 'Eye of Cthulhu has awoken!', 'Eye of Cthulhu has been defeated!'],
  valheim: [
    '%D %T: Got connection SteamID 76561198000000001',
    '%D %T: Got character ZDOID from Ragnar : -2147384112:1',
    '%D %T: World saved ( 412.218ms )',
    '%D %T: Connections 4 ZDOS:184211  sent:42 recv:388',
    '%D %T: Random event set:army_eikthyr',
    '%D %T: Player connection lost server "Bjorn" that has join code 0',
  ],
  factorio: ['%T [JOIN] engineer42 joined the game', '%T [CHAT] engineer42: the iron belt is starved again', '%T Saving to _autosave3 (non-blocking).', '%T Saving finished', '%T [CHAT] engineer42: launching the first rocket'],
  bedrock: ['[%D %T:000 INFO] Player connected: Steve_BE, xuid: 2535400000000001', '[%D %T:000 INFO] Running AutoCompaction...', '[%D %T:000 INFO] Player disconnected: Steve_BE, xuid: 2535400000000001'],
};
const T = () => new Date().toLocaleTimeString('en-GB');
const D = () => new Date().toISOString().slice(0, 10);
const linesFor = (s: A.Server) => gameChatter[s.runtime] ?? chatter;
const fill = (l: string) => l.replace('%D', D()).replace('%T', T()).trimEnd();
const bl = (s: A.Server) => { const c = linesFor(s); return [...Array(60)].map((_, i) => fill(c[i % c.length])); };
export function openConsole(id: string, h: { onOpen(): void; onClose(): void; onMsg(m: A.ConsoleMsg): void }): A.ConsoleSocket {
  const s = sv(id);
  setTimeout(h.onOpen, 150);
  const iv = setInterval(() => {
    if (s.state === 'running' && Math.random() < 0.7) { const c = linesFor(s); h.onMsg({ t: 'line', d: fill(c[(Math.random() * c.length) | 0]) }); }
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
const V = (pid: string, cf = false, hg = false): A.Project => ({
  project_id: pid, title: [...hits, ...cfHits, ...hangarHits].find((h) => h.project_id === pid)?.title ?? pid, description: 'A fantastic project.', icon_url: null,
  body: 'This is the long description of the project.\n\nFeatures:\n- Fast\n- Configurable\n- Well maintained', links: hg ? { page: 'https://hangar.papermc.io/Owner/' + pid, source: 'https://github.com' } : cf ? { page: 'https://www.curseforge.com/minecraft/bukkit-plugins/' + pid } : { source: 'https://github.com', issues: 'https://github.com', wiki: 'https://modrinth.com' }, license: 'MIT', gallery: [],
  versions: ['2.4.1', '2.4.0', '2.3.9-beta.1', '2.3.8'].map((v, i) => ({ id: pid + v, name: `v${v}`, version_number: v, channel: v.includes('beta') ? 'beta' : 'release', date: ago((i * 12 + 2) * DAY), downloads: 120000 - i * 20000, files: [{ filename: `${pid}-${v}.jar`, size: 2.1e6, primary: true }], game_versions: ['1.21.4', '1.21.3'], loaders: ['paper', 'fabric'], dependencies: i === 0 ? [{ project_id: 'fabric-api', kind: 'required' as const }] : [], ...(cf && i === 1 ? { downloadable: false } : {}), ...(hg && i === 1 ? { downloadable: false, files: [], external_url: 'https://example.com/download', warning: 'Not hosted on Hangar; download it from https://example.com/download and upload it in Files.' } : {}) })),
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

async function handleBase(method: string, rawPath: string, body: unknown): Promise<unknown> {
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
  if (M(/^\/runtimes\/([^/]+)\/versions$/)) return gameVersions[m![1]] ?? versionsFor(m![1]);
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
  if (M(/^\/undo\/([^/]+)\/revert$/)) { revertConfig({ uid, now, fs, pushUndo, log }, m![1]); for (const l of Object.values(undo)) { const i = l.findIndex((x) => x.id === m![1]); if (i >= 0) { const [e] = l.splice(i, 1); log('undo.revert', e.title); } } return { ok: true }; }
  if (p === '/users') { if (method === 'POST') { const u = { id: uid(), email: b.email, role: b.role, created_at: now() }; users.push(u); return u; } return users; }
  if (M(/^\/users\/([^/]+)$/)) { const i = users.findIndex((x) => x.id === m![1]); if (method === 'DELETE') users.splice(i, 1); else users[i].role = b.role; return; }
  if (p === '/settings') { if (method === 'PUT') Object.assign(settings, b); return settings; }
  if (p === '/content/search') {
    const qq = (qs.get('q') ?? '').toLowerCase();
    const s = sv(qs.get('server_id')!);
    const r = (qs.get('provider') === 'curseforge' ? cfHits : qs.get('provider') === 'hangar' ? hangarHits : hits).filter((h) => !qq || (h.title + h.description).toLowerCase().includes(qq));
    return { hits: r.slice(+(qs.get('offset') ?? 0), +(qs.get('offset') ?? 0) + +(qs.get('limit') ?? 20)), total: r.length, compat: { kind: s.content_kind, loader: s.runtime, game_version: s.game_version } };
  }
  if (p === '/content/providers') { const rt = sv(qs.get('server_id')!).runtime; return { providers: [{ id: 'modrinth', name: 'Modrinth', available: true, supported: true, reason: null }, { id: 'hangar', name: 'Hangar', available: true, supported: ['paper', 'purpur', 'folia', 'velocity'].includes(rt), reason: ['paper', 'purpur', 'folia', 'velocity'].includes(rt) ? null : 'Hangar only has plugins for Paper, Purpur, Folia and Velocity.' }, { id: 'curseforge', name: 'CurseForge', available: true, supported: rt !== 'velocity', reason: rt === 'velocity' ? 'CurseForge has no proxy plugins' : null }] }; }
  if (M(/^\/content\/projects\/([^/]+)$/)) return V(decodeURIComponent(m![1]), qs.get('provider') === 'curseforge', qs.get('provider') === 'hangar');

  if (p === '/servers') {
    if (method === 'GET') return servers;
    const rt = runtimes.find((r) => r.id === b.runtime);
    const game = rt?.category === 'game' ? rt : null;
    const s = mk({ name: b.name, runtime: b.runtime, game_version: b.game_version, loader_version: b.loader_version ?? null, memory_mb: b.memory_mb, port: b.port ?? (game?.default_port ?? 25565) + (game ? 0 : 5 + servers.length), desired: 'stopped', state: 'installing', content_kind: rt?.content_kind ?? 'none', eula_accepted: !!b.accept_eula,
      ...(game ? { java_major: 0, capabilities: gameCaps(...(game.player_tracking !== 'none' ? ['players'] : []), ...(game.id === 'bedrock' ? ['properties', 'eula'] : [])) } : {}) });
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
    if (/^\/config(\/|$)/.test(sub)) {
      const r = await handleOps({ uid, now, fs, pushUndo, log }, s, sub, method, b, qs);
      if (r !== NONE) return r;
    }
    if (/^\/(content|upgrade|modpacks|staging)(\/|$)/.test(sub)) {
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

// ===== WP-F2 mock additions: sign-in gates, notifications, tokens, limits, time zones, metrics, support bundle =====
// Demo scenarios via ?scenario=: setup | denied | mfa | mfa-enrol | throttled | empty | errors | macos
const scenario = new URLSearchParams(location.search).get('scenario') ?? '';
let gate = ['setup', 'denied', 'mfa', 'mfa-enrol', 'throttled'].includes(scenario) ? scenario : '';
const macos = scenario === 'macos';
const failing = scenario === 'errors';
const noEngine = scenario === 'noengine';
const NOPE = Symbol('unhandled');
const SETUP_TOKEN = 'moss-setup-demo-token';
let claimTries = 0;
const MFA_CODE = '123456';

servers.push(mk({ name: 'Broken Install', runtime: 'paper', port: 25580, state: 'failed', state_detail: 'Download failed: HTTP 503 from api.papermc.io (3 attempts)', state_since: ago(9 * MIN), created_at: ago(2 * HOUR) }));
seedFs(servers[servers.length - 1]);
seedContent(servers[servers.length - 1]);
const ensureLimits = (s: A.Server) => {
  s.limits ??= {
    memory_mb: s.name === 'Survival SMP' ? 6144 : null, cpu_pct: s.name === 'Survival SMP' ? 200 : null, pids: null,
    enforced: !macos && s.name !== 'Creative Plots', backend: macos || s.name === 'Creative Plots' ? 'none' : 'cgroup2',
    detail: macos ? 'cgroups are not available on macOS' : s.name === 'Creative Plots' ? 'cgroup v2 is mounted read-only for this process (no delegated subtree)' : null,
  };
};
const imageOf = (s: A.Server) => s.container?.image_override ?? `docker.io/library/eclipse-temurin:${s.java_major}-jre`;
const ensureIsolation = (s: A.Server) => {
  s.isolation ??= 'process';
  s.container = { image: imageOf(s), image_override: s.container?.image_override ?? null, running_in_container: s.container?.running_in_container ?? false, restart_needed: false, network: s.container?.network ?? 'open', extra_ports: s.container?.extra_ports ?? [] };
};
const worldSets: Record<string, string[]> = {};
const normalize = () => {
  servers.forEach(ensureLimits);
  servers.forEach(ensureIsolation);
  seedRemote(backups);
  backups.forEach((b, i) => {
    b.worlds ??= worldSets[b.server_id] ??= gameWorlds[sv(b.server_id).runtime] ?? ['world', 'world_nether', 'world_the_end'];
    b.locked ??= i === 1;
  });
  schedules.forEach((x) => { x.tz ??= offsetName(x.tz_offset_min); });
};
schedules[0].tz = 'America/New_York';
schedules[0].next_run = new Date(nextDailyRun('04:00', 'America/New_York') ?? Date.now() + HOUR).toISOString();
schedules.push({ id: uid(), server_id: servers[0].id, kind: 'restart', command: null, every_minutes: null, daily_at: '06:00', tz_offset_min: 330, tz: 'UTC+05:30', enabled: true, keep: null, last_run: null, next_run: new Date(nextDailyRun('06:00', 'UTC+05:30') ?? Date.now()).toISOString(), last_status: null });
Object.assign(settings, { notify_webhook_url: 'https://discord.com/api/webhooks/123456/abcdef', notify_events: ['crash', 'crash_loop', 'backup_failed', 'disk_low', 'restart_failed'], audit_console_commands: 'redacted', access_message: 'Ask Sam in #minecraft on Discord to invite you.' });
javaInstalls.push({ major: 8, path: '/var/lib/moss/java/8', managed: true, version: '8.0.432+6' });

const notifs: A.AppNotification[] = scenario === 'empty' ? [] : [
  { id: uid(), time: ago(4 * MIN), kind: 'crash_loop', severity: 'critical', title: 'Modded Fabric is crash-looping', body: 'Crashed 3 times in 30 minutes. Automatic restarts are paused.', server_id: servers[2].id, read: false },
  { id: uid(), time: ago(50 * MIN), kind: 'backup_failed', severity: 'warn', title: 'Backup failed on Survival SMP', body: 'No space left on device while writing the archive.', server_id: servers[0].id, read: false },
  { id: uid(), time: ago(3 * HOUR), kind: 'disk_low', severity: 'warn', title: 'Disk space is low', body: '8% of the data volume is free (39 GB).', server_id: null, read: false },
  { id: uid(), time: ago(5 * HOUR), kind: 'update_available', severity: 'info', title: 'Paper 1.21.4 build 232 is available', body: 'Survival SMP runs build 227.', server_id: servers[0].id, read: true },
  { id: uid(), time: ago(27 * HOUR), kind: 'restart_failed', severity: 'warn', title: 'Scheduled restart failed', body: 'Velocity Proxy did not stop within 60 seconds and was killed.', server_id: servers[3].id, read: true },
  { id: uid(), time: ago(30 * HOUR), kind: 'security', severity: 'info', title: 'New admin added', body: 'sam@example.com was made an admin by dev@moss.local.', server_id: null, read: true },
  { id: uid(), time: ago(4 * DAY), kind: 'operation_failed', severity: 'critical', title: 'Install failed: Broken Install', body: 'Download failed: HTTP 503 from api.papermc.io', server_id: servers[servers.length - 1].id, read: true },
];
const emitNotif = (n: A.AppNotification) => listeners.forEach((l) => l.notification?.(structuredClone(n)));
const pushNotif = (severity: A.Severity) => {
  const n: A.AppNotification = { id: uid(), time: now(), kind: severity === 'critical' ? 'crash' : 'disk_low', severity, title: severity === 'critical' ? 'Survival SMP crashed' : 'Disk space is getting low', body: severity === 'critical' ? 'Exited with code 137 (killed by the memory limit).' : '12% free.', server_id: servers[0].id, read: false };
  notifs.unshift(n); emitNotif(n);
};
if (!gate && scenario !== 'empty') setTimeout(() => pushNotif('critical'), 9000);
(window as any).__moss = { notify: pushNotif, resync: () => listeners.forEach((l) => l.resync?.()) };

const tokens: (A.ApiToken & { secret?: string })[] = scenario === 'empty' ? [] : [
  { id: uid(), name: 'Grafana', scope: 'read', prefix: 'moss_pat_7Hk2', created_at: ago(12 * DAY), last_used: ago(3 * MIN), expires_at: new Date(Date.now() + 48 * DAY).toISOString() },
  { id: uid(), name: 'Deploy script', scope: 'full', prefix: 'moss_pat_Qm9x', created_at: ago(40 * DAY), last_used: null, expires_at: null },
  { id: uid(), name: 'Old CI runner', scope: 'full', prefix: 'moss_pat_a1B3', created_at: ago(120 * DAY), last_used: ago(70 * DAY), expires_at: ago(DAY) },
];
const rand = (n: number) => [...Array(n)].map(() => 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'[(Math.random() * 57) | 0]).join('');

const pathsFor = (kind: string, n: number) => [...Array(Math.min(50, n))].map((_, i) => ({ added: `world/region/r.${i}.${-i}.mca`, removed: `plugins/OldPlugin/data/cache-${i}.bin`, changed: i % 3 ? `world/region/r.${i}.0.mca` : `world/playerdata/${uid().slice(0, 8)}.dat` })[kind as 'added']);
function preview(k: A.Backup, mode: string, only?: string[]): A.RestorePreview {
  const inPlace = mode === 'in_place';
  const share = only?.length ? only.length / (k.worlds?.length ?? 1) : 1;
  const added = inPlace ? Math.round(12 * share) : k.files, removed = inPlace ? Math.round(340 * share) : 0, changed = inPlace ? Math.round(1204 * share) : 0;
  const unchanged = inPlace ? Math.max(0, k.files - added - changed) : 0;
  return {
    mode: mode as 'in_place', archive: { files: k.files, size: k.size, created_at: k.created_at }, current: inPlace ? { files: k.files - added + removed, size: k.size * 1.07 } : null,
    added, removed, changed, unchanged, size_delta: inPlace ? -Math.round(k.size * 0.07 * share) : 0,
    samples: { added: pathsFor('added', added), removed: pathsFor('removed', removed), changed: pathsFor('changed', changed) }, worlds: k.worlds ?? [],
    warnings: [...(!k.consistent ? ['This backup was taken while the server was running without a save pause, so recent chunks may be inconsistent.'] : []), ...(k.kind === 'pre-change' ? ['This is an automatic pre-change backup; restoring it undoes everything since.'] : [])],
  };
}

// ---- player analytics: a week-plus of plausible data (evening peaks, busier weekends, a Sunday-night restart gap) ----
const playerNames = ['Alex_Builds', 'creeper_hug', 'Notch2', 'PixelPenny', 'RedstoneRay', 'luna_mc', 'Gh0stMiner', 'BlockBard', 'TNT_Tina', 'ender_eli', 'Mossy', 'farmer_fi', 'SkyDiver88', 'quartz_quinn', 'Zombie_Zed'];
function analyticsFor(s: A.Server, days: A.AnalyticsDays): A.PlayerAnalytics {
  const seed = [...s.id].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);
  const rnd = (i: number) => { const x = Math.sin(seed + i * 12.9898) * 43758.5453; return x - Math.floor(x); };
  const cap = s.players?.max ?? 20, iso = (t: number) => new Date(t).toISOString().replace(/\.\d+Z$/, 'Z');
  const top = Math.floor(Date.now() / HOUR) * HOUR;
  const hourly: A.PlayerAnalytics['hourly'] = [];
  for (let i = days * 24 - 1; i >= 0; i--) {
    const d = new Date(top - i * HOUR), h = d.getHours(), dow = d.getDay();
    if (dow === 0 && h === 4) continue; // weekly restart
    const avg = Math.max(0, (0.4 + Math.exp(-((h - 21) ** 2) / 12) * 5.5) * (dow === 0 || dow === 6 ? 1.5 : 1) * (0.7 + rnd(i) * 0.6));
    hourly.push({ hour: iso(d.getTime()), peak: Math.min(cap, Math.max(Math.ceil(avg), Math.round(avg + 1 + rnd(i + 999) * 2))), avg: Math.round(avg * 100) / 100 });
  }
  const unique = days === 7 ? 12 : days === 30 ? 14 : 15;
  const online = new Set(s.players?.sample ?? []);
  const now = Date.now();
  const topPlayers = playerNames.slice(0, unique).map((name, k) => {
    const sessions = Math.max(1, Math.round(days * 1.6 / (k + 1)));
    const isOn = online.has(name);
    return { name, play_seconds: Math.round(sessions * (1500 + rnd(k + 50) * 5400)), sessions, last_seen: iso(isOn ? now : now - (k * 5 + 1) * HOUR), online: isOn };
  }).sort((a, b) => b.play_seconds - a.play_seconds);
  const recent: A.PlayerAnalytics['recent_sessions'] = [];
  [...online].forEach((name, i) => recent.push({ name, joined_at: iso(now - (25 + i * 40) * MIN), left_at: null, seconds: (25 + i * 40) * 60 }));
  for (let i = 0; recent.length < 20; i++) {
    const len = Math.round((20 + rnd(i + 200) * 100) * 60), left = now - (i * 95 + 70) * MIN;
    recent.push({ name: playerNames[(i * 7 + 3) % unique], joined_at: iso(left - len * 1000), left_at: iso(left), seconds: len });
  }
  const sessions = topPlayers.reduce((n, p) => n + p.sessions, 0), total = topPlayers.reduce((n, p) => n + p.play_seconds, 0);
  const pk = hourly.reduce((b, h) => (h.peak >= b.peak ? h : b), hourly[0]);
  return { days, hourly, unique_players: unique, sessions, total_play_seconds: total, avg_session_seconds: Math.round(total / sessions), peak: { count: pk.peak, at: pk.hour }, top_players: topPlayers, recent_sessions: recent };
}

function metricsFor(range: A.MetricRange, base: { cpu: number; mem: number; players?: boolean; disk?: boolean }, sparse = false): A.MetricSeries {
  const step = { '1h': 5, '24h': 3600, '7d': 3600, '30d': 3600 }[range];
  const n = sparse ? 2 : { '1h': 720, '24h': 24, '7d': 168, '30d': 720 }[range];
  return {
    range, step_s: step,
    samples: [...Array(n)].map((_, i) => {
      const t = Date.now() - (n - 1 - i) * step * 1000, w = Math.sin(i / (range === '1h' ? 14 : 5)) * 0.5 + 0.5, spike = Math.max(0, Math.sin(i / 3.3));
      return {
        t, cpu_avg: base.cpu * (0.6 + w * 0.7), cpu_max: Math.min(100, base.cpu * (0.9 + w * 0.8) + spike * 12), mem_avg: base.mem * (0.8 + 0.15 * (i / n)), mem_max: base.mem * (0.88 + 0.15 * (i / n)) + spike * 5e7,
        ...(base.players ? { players_max: Math.round(2 + w * 4 + spike * 2) } : {}), ...(base.disk ? { disk_free: 330e9 - i * (range === '30d' ? 4e7 : 2e8) } : {}),
      };
    }),
  };
}
const err500 = () => new ApiError(500, 'internal', 'The backend returned an unexpected error (mock).', [], 'req_mock123');

const statusPages: A.StatusPage[] = [{ id: 'sp-main', slug: 'k3x9f2m7qa4tz8', path: '/status/k3x9f2m7qa4tz8', title: 'Our servers', description: 'Live status of the community servers.', enabled: true, show_players: true, show_address: 'play.example.com', server_ids: servers.slice(0, 2).map((s) => s.id), created_at: '2026-01-10T12:00:00Z' }];
function handleStatusPages(method: string, p: string, b: Record<string, any>): unknown {
  const bad = (field: string, message: string) => new ApiError(422, 'validation', message, [{ path: field, message }]);
  const check = (x: Record<string, any>, partial: boolean) => {
    if ((!partial || x.title !== undefined) && !String(x.title ?? '').trim()) throw bad('title', 'Give the page a title of up to 80 characters.');
    if (x.slug !== undefined && x.slug !== '' && !/^(?!-)[a-z0-9-]{3,40}(?<!-)$/.test(x.slug)) throw bad('slug', 'Use 3 to 40 lowercase letters, digits or dashes, not starting or ending with a dash.');
    if (x.slug && statusPages.some((q) => q.slug === x.slug && q.id !== x.id)) throw new ApiError(409, 'status_page.slug_taken', 'That address is already used by another status page.', [{ path: 'slug', message: 'Already in use.' }]);
    for (const s of x.server_ids ?? []) if (!servers.some((q) => q.id === s)) throw bad('server_ids', 'One of the chosen servers no longer exists.');
  };
  let m: RegExpMatchArray | null;
  if (p === '/status-pages' && method === 'GET') return { items: statusPages };
  if (p === '/status-pages' && method === 'POST') {
    check(b, false);
    const slug = b.slug || Array.from({ length: 14 }, () => 'abcdefghijklmnopqrstuvwxyz0123456789'[Math.floor(Math.random() * 36)]).join('');
    const page: A.StatusPage = { id: uid(), slug, path: `/status/${slug}`, title: String(b.title).trim(), description: String(b.description ?? '').trim(), enabled: b.enabled ?? true, show_players: !!b.show_players, show_address: String(b.show_address ?? '').trim() || null, server_ids: b.server_ids ?? [], created_at: now() };
    statusPages.push(page);
    return page;
  }
  if ((m = p.match(/^\/status-pages\/([^/]+)$/))) {
    const page = statusPages.find((q) => q.id === m![1]);
    if (!page) throw new ApiError(404, 'not_found', 'status page not found');
    if (method === 'DELETE') { statusPages.splice(statusPages.indexOf(page), 1); return {}; }
    check({ ...b, id: page.id }, true);
    Object.assign(page, Object.fromEntries(Object.entries(b).filter(([k]) => ['title', 'slug', 'description', 'enabled', 'show_players', 'show_address', 'server_ids'].includes(k))));
    page.path = `/status/${page.slug}`;
    return page;
  }
  throw new ApiError(404, 'mock.not_found', `Mock has no route for ${method} ${p}`);
}

async function handleX(method: string, p: string, qs: URLSearchParams, b: Record<string, any>): Promise<unknown> {
  if (p.startsWith('/status-pages')) return handleStatusPages(method, p, b);
  const wh = await handleWebhooks({ uid, now, servers }, method, p, b);
  if (wh !== NOT_WEBHOOK) return wh;
  const wf = await handleWorkflows({ uid, now, servers }, method, p, b);
  if (wf !== NOT_WORKFLOW) return wf;
  const rem = await handleRemote({ uid, now, op: (...a) => startOp(...a), backups, servers }, method, p, method === 'DELETE' ? { ...b, force: qs.get('force') === '1' } : b);
  if (rem !== NOT_REMOTE) return rem;
  let m: RegExpMatchArray | null;
  const M = (re: RegExp) => (m = p.match(re));
  if (p === '/auth/config') return { mode: 'dev', owner_claimed: !(gate === 'setup'), setup_required: gate === 'setup', access_message: 'Ask Sam in #minecraft on Discord to invite you.' };
  if (p === '/me') {
    if (gate === 'setup') throw new ApiError(403, 'auth.setup_required', 'This instance has not been claimed yet.');
    if (gate === 'denied') throw new ApiError(403, 'auth.not_provisioned', 'Your account is not provisioned on this instance.');
    if (gate === 'throttled') throw new ApiError(429, 'auth.throttled', 'Too many failed sign-in attempts from this address.');
    const mfa = gate === 'mfa' || gate === 'mfa-enrol';
    return { id: users[0].id, email: users[0].email, role: 'owner', mfa: { required: mfa, satisfied: !mfa }, permissions: ['server.view', 'server.power', 'server.console.read', 'server.console.write', 'files.read', 'files.write', 'content.manage', 'backup.manage', 'config.write', 'server.delete', 'admin.users', 'admin.settings'] };
  }
  if (p === '/setup/claim') {
    if (++claimTries > 5) throw new ApiError(429, 'auth.throttled', 'Too many attempts. Try again in a minute.');
    if (b.token !== SETUP_TOKEN) throw new ApiError(403, 'setup.invalid_token', `That setup token is not correct (try "${SETUP_TOKEN}" in this demo).`);
    gate = ''; claimTries = 0;
    return { role: 'owner' };
  }
  if (p === '/timezones') return { zones: ['UTC', ...Intl.supportedValuesOf('timeZone')] };
  if (p === '/notifications' && method === 'GET') {
    if (failing) throw err500();
    const items = notifs.filter((n) => !qs.get('unread') || !n.read).slice(0, +(qs.get('limit') ?? 50));
    return { items, unread: notifs.filter((n) => !n.read).length };
  }
  if (p === '/notifications/read') { const ids = b.ids as string[] | undefined; notifs.forEach((n) => { if (b.all || ids?.includes(n.id)) n.read = true; }); return; }
  if (p === '/notifications/test') {
    await sleep(600);
    return /fail/i.test(String(settings.notify_webhook_url)) ? { delivered: false, detail: 'Gave up after 3 attempts: the server answered HTTP 502.' } : { delivered: true, detail: 'HTTP 204 from discord.com in 212 ms.' };
  }
  if (p === '/tokens') {
    if (failing) throw err500();
    if (method === 'GET') return tokens.map(({ secret: _s, ...t }) => t);
    if (!String(b.name ?? '').trim()) throw new ApiError(422, 'validation', 'Name is required', [{ path: 'name', message: 'Required' }]);
    const secret = 'moss_pat_' + rand(40);
    const t = { id: uid(), name: b.name as string, scope: b.scope as A.TokenScope, prefix: secret.slice(0, 13), created_at: now(), last_used: null, expires_at: b.expires_in_days ? new Date(Date.now() + b.expires_in_days * DAY).toISOString() : null };
    tokens.unshift(t);
    return { id: t.id, token: secret, name: t.name, scope: t.scope, expires_at: t.expires_at };
  }
  if (M(/^\/tokens\/([^/]+)$/)) { const i = tokens.findIndex((x) => x.id === m![1]); if (i >= 0) tokens.splice(i, 1); return; }
  if (p === '/support-bundle') {
    if (failing) throw err500();
    return { files: [{ name: 'moss.txt', size: 2150 }, { name: 'config.json', size: 4410 }, { name: 'servers.json', size: 12980 }, { name: 'health.json', size: 3320 }, { name: 'activity.json', size: 188400 }, { name: 'operations.json', size: 40210 }, { name: 'console/survival-smp.log', size: 61230 }, { name: 'console/modded-fabric.log', size: 58044 }], redactions: ['Tokens, JWTs and API keys are replaced by [redacted]', 'Webhook URLs are replaced by [redacted]', 'E-mail addresses are hashed (sha256, first 8 characters)', 'Console output keeps only the last 300 lines per server'] };
  }
  if (M(/^\/java\/(\d+)$/) && method === 'DELETE') {
    const major = +m![1], j = javaInstalls.find((x) => x.major === major);
    if (!j?.managed) throw new ApiError(400, 'java.not_managed', 'Only Java installs managed by Moss can be removed.');
    if (major === 21) throw new ApiError(409, 'java.in_use', 'Java 21 is needed by Survival SMP and Velocity Proxy.');
    javaInstalls.splice(javaInstalls.indexOf(j), 1);
    return;
  }
  if (p === '/metrics/host') { if (failing) throw err500(); return metricsFor(qs.get('range') as A.MetricRange, { cpu: 28, mem: 7.5e9, disk: true }); }
  if (M(/^\/backups\/([^/]+)$/) && (method === 'PATCH' || method === 'DELETE')) {
    const k = backups.find((x) => x.id === m![1]);
    if (!k) throw new ApiError(404, 'backup.not_found', 'Backup not found');
    if (method === 'DELETE') {
      if (k.locked) throw new ApiError(409, 'backup.locked', 'This backup is locked. Unlock it before deleting.');
      backups.splice(backups.indexOf(k), 1); return;
    }
    if (b.locked !== undefined) k.locked = b.locked;
    if (b.note !== undefined) k.note = b.note;
    return k;
  }
  if (M(/^\/backups\/([^/]+)\/preview$/)) {
    const k = backups.find((x) => x.id === m![1]);
    if (!k) throw new ApiError(404, 'backup.not_found', 'Backup not found');
    await sleep(450);
    if (failing) throw new ApiError(500, 'backup.preview_failed', 'The archive could not be read (mock).');
    return preview(k, b.mode, b.only);
  }
  if (M(/^\/servers\/([^/]+)\/analytics$/)) {
    if (failing) throw err500();
    const s = sv(m![1]);
    if (!s.capabilities.includes('players')) throw new ApiError(404, 'not_found', 'No player analytics for this server');
    const d = Number(qs.get('days') ?? 7);
    return analyticsFor(s, (d === 30 || d === 90 ? d : 7) as A.AnalyticsDays);
  }
  if (M(/^\/servers\/([^/]+)\/metrics$/)) {
    if (failing) throw err500();
    const s = sv(m![1]);
    if (s.state === 'stopped' || s.state === 'failed') return { range: qs.get('range'), step_s: 3600, samples: [] };
    return metricsFor(qs.get('range') as A.MetricRange, { cpu: s.cpu_pct ?? 6, mem: s.mem_bytes ?? 1e9, players: !!s.players }, s.name === 'Velocity Proxy' && qs.get('range') === '30d');
  }
  if (M(/^\/servers\/([^/]+)$/) && method === 'PATCH' && ('memory_limit_mb' in b || 'cpu_limit_pct' in b || 'pids_limit' in b)) {
    const s = sv(m![1]);
    const fields: { path: string; message: string }[] = [];
    if (b.cpu_limit_pct != null && (b.cpu_limit_pct < 10 || b.cpu_limit_pct > 800)) fields.push({ path: 'cpu_limit_pct', message: 'Must be between 10 and 800' });
    if (b.pids_limit != null && (b.pids_limit < 64 || b.pids_limit > 32768)) fields.push({ path: 'pids_limit', message: 'Must be between 64 and 32768' });
    if (fields.length) throw new ApiError(422, 'validation', 'Some values are out of range', fields);
    ensureLimits(s);
    Object.assign(s.limits!, { memory_mb: b.memory_limit_mb ?? null, cpu_pct: b.cpu_limit_pct ?? null, pids: b.pids_limit ?? null });
    log('server.update', s.name, 'resource limits');
    emitServer(s);
    return s;
  }
  if (p === '/container/status') {
    return noEngine
      ? { available: false, engine: null, version: null, rootless: null, message: 'Neither Docker nor Podman was found. Install one of them (rootless Podman is the safest choice), make sure the user running Moss can use it, then check again.', outbound_restricted: false, game_only_supported: false, default_image: 'docker.io/library/eclipse-temurin:<java version>-jre' }
      : { available: true, engine: 'podman', version: '5.2.2', rootless: true, message: null, outbound_restricted: false, game_only_supported: false, default_image: 'docker.io/library/eclipse-temurin:<java version>-jre' };
  }
  if (M(/^\/servers\/([^/]+)\/container\/pull$/)) {
    const s = sv(m![1]);
    return op2('container.pull', `Download ${imageOf(s)}`, s.id, ['Pulling fs layer', 'Downloading', 'Extracting', 'Verifying'], () => {});
  }
  if (M(/^\/servers\/([^/]+)$/) && method === 'PATCH' && ('isolation' in b || 'container_image' in b || 'network' in b || 'extra_ports' in b)) {
    const s = sv(m![1]);
    const fields: { path: string; message: string }[] = [];
    if (b.isolation && !['process', 'container'].includes(b.isolation)) fields.push({ path: 'isolation', message: 'process | container' });
    if (b.container_image && !/^(ghcr\.io\/|(docker\.io\/library\/)?eclipse-temurin:|azul\/zulu-openjdk:|amazoncorretto:)/.test(b.container_image)) fields.push({ path: 'container_image', message: 'Only eclipse-temurin, azul/zulu-openjdk, amazoncorretto and ghcr.io images are allowed.' });
    if (b.network && !['open', 'game-only'].includes(b.network)) fields.push({ path: 'network', message: 'open | game-only' });
    if (b.extra_ports && (b.extra_ports.length > 8 || b.extra_ports.some((x: { port: number }) => x.port < 1024 || x.port > 65535))) fields.push({ path: 'extra_ports', message: 'At most 8 ports, each 1024-65535' });
    if (fields.length) throw new ApiError(400, 'validation', 'Some values are not allowed', fields);
    if (b.network === 'game-only') throw new ApiError(409, 'container.network_failed', 'Game-only networking is not available with Podman: Moss only knows how to restrict outbound traffic safely with Docker.', [{ path: 'network', message: 'game-only is not available' }]);
    if (b.isolation === 'container' && noEngine) throw new ApiError(409, 'container.engine_missing', 'Neither Docker nor Podman was found. Install one of them, then try again.');
    if (b.isolation) s.isolation = b.isolation;
    if ('container_image' in b) s.container = { ...s.container!, image_override: b.container_image ?? null };
    if (b.network) s.container = { ...s.container!, network: b.network };
    if (b.extra_ports) s.container = { ...s.container!, extra_ports: b.extra_ports };
    ensureIsolation(s);
    s.container!.restart_needed = (s.state === 'running' || s.state === 'starting') && s.container!.running_in_container !== (s.isolation === 'container');
    log('server.isolation', s.name, s.isolation);
    if (s.isolation === 'container') startOp('container.pull', `Download ${imageOf(s)}`, s.id, ['Pulling fs layer', 'Downloading', 'Extracting'], {});
    emitServer(s);
    return s;
  }
  if (M(/^\/servers\/([^/]+)\/actions\/reinstall$/)) {
    const s = sv(m![1]);
    if (s.state !== 'failed' && s.state !== 'stopped') throw new ApiError(409, 'server.bad_state', 'Reinstall needs the server stopped or the install failed.');
    setState(s, 'installing');
    return op2('server.reinstall', b.keep_data ? `Repairing ${s.name}` : `Reinstalling ${s.name}`, s.id, ['Resolving build', 'Downloading server jar', 'Verifying checksum', b.keep_data ? 'Keeping world and config' : 'Writing config'], () => setState(s, 'stopped'));
  }
  if (M(/^\/servers\/([^/]+)\/schedules$/) && method === 'POST') {
    const s = sv(m![1]);
    const tz = (b.tz as string | undefined) ?? offsetName(b.tz_offset_min ?? 0);
    const next = b.daily_at ? nextDailyRun(b.daily_at, tz) : Date.now() + (b.every_minutes ?? 60) * MIN;
    const n = { id: uid(), server_id: s.id, command: null, every_minutes: null, daily_at: null, tz_offset_min: 0, enabled: true, keep: null, last_run: null, next_run: new Date(next ?? Date.now()).toISOString(), last_status: null, ...b, tz } as A.Schedule;
    schedules.push(n);
    return n;
  }
  return NOPE;
}
const op2 = (kind: string, title: string, sid: string, stages: string[], done: () => void) => ({ operation: startOp(kind, title, sid, stages, { done }) });

/** The mock router: WP-F2 routes first, then the base routes; fixtures are normalised to the contract on every call. */
export async function handle(method: string, rawPath: string, body: unknown): Promise<unknown> {
  normalize();
  const u = new URL(rawPath, 'http://x');
  const x = await handleX(method, u.pathname, u.searchParams, (body ?? {}) as Record<string, any>);
  if (x !== NOPE) { await sleep(60 + Math.random() * 100); return x; }
  const og = await handleOpsGlobal({ uid, now, servers, backups, content, fs, call: (m, pp, bb) => handleBase(m, pp, bb), log }, method, u.pathname, u.searchParams, (body instanceof FormData ? {} : (body ?? {})) as Record<string, any>, body);
  if (og !== NONE) { await sleep(60 + Math.random() * 100); return og; }
  const r = await handleBase(method, rawPath, body);
  if (u.pathname === '/status') (r as A.HostStatus).host.os = macos ? 'macOS' : 'Linux';
  return r;
}

/** Supabase auth REST calls for the MFA gate (mock mode answers instead of GoTrue). */
export async function handleAuth(method: string, path: string, body: unknown): Promise<unknown> {
  await sleep(250);
  const b = (body ?? {}) as Record<string, any>;
  if (path === 'user') return { id: users[0].id, email: users[0].email, factors: gate === 'mfa' ? [{ id: 'f-existing', factor_type: 'totp', status: 'verified' }] : [] };
  if (path === 'factors' && method === 'POST') {
    const secret = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP';
    return { id: 'f-new', type: 'totp', totp: { qr_code: '', secret, uri: `otpauth://totp/Moss:${encodeURIComponent(users[0].email)}?secret=${secret}&issuer=Moss` } };
  }
  if (/^factors\/[^/]+$/.test(path) && method === 'DELETE') return {};
  if (/^factors\/[^/]+\/challenge$/.test(path)) return { id: 'ch-1', expires_at: Math.floor(Date.now() / 1000) + 300 };
  if (/^factors\/[^/]+\/verify$/.test(path)) {
    if (b.code !== MFA_CODE) throw Object.assign(new Error('Invalid TOTP code entered'), { code: 'mfa_verification_failed' });
    gate = '';
    return { access_token: 'mock', refresh_token: 'mock', expires_in: 3600, user: { email: users[0].email } };
  }
  throw new ApiError(404, 'mock.not_found', `Mock auth has no route for ${method} ${path}`);
}
