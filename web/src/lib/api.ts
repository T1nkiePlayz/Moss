// Typed client for docs/api.md. In mock mode (dev only) requests are answered in-memory.
export type ServerState = 'installing' | 'stopped' | 'starting' | 'running' | 'stopping' | 'crashed' | 'failed' | 'needs_attention';
export type ContentKind = 'plugins' | 'mods' | 'proxy_plugins' | 'datapacks' | 'none';
export type Role = 'owner' | 'admin' | 'user';

export interface AuthConfig { mode: 'supabase' | 'dev'; supabase_url?: string; supabase_anon_key?: string; owner_claimed?: boolean }
export interface Me { id: string; email: string; role: Role; permissions: string[] }
export interface HostStatus {
  version: string; uptime_s: number; auth_mode: string;
  host: { cpu_pct: number; mem_total: number; mem_used: number; disk_total: number; disk_free: number; load1: number; hostname: string; os?: string };
  counts: { servers: number; running: number; players: number };
}
export interface Runtime {
  id: string; name: string; category: 'official' | 'plugins' | 'modded' | 'proxy' | 'custom' | 'game';
  tier: 'stable' | 'experimental' | 'planned'; content_kind: ContentKind; description: string; installable: boolean;
  /** Games only (category `game`). */
  default_port?: number; protocol?: 'udp' | 'tcp'; platforms?: string[]; player_tracking?: 'none' | 'log' | 'a2s'; needs_eula?: boolean; max_players?: number;
}
export interface Versions { versions: { id: string; stable: boolean }[]; recommended: string; loader_versions?: Record<string, { id: string; recommended: boolean }[]> }
export interface JavaInstall { major: number; path: string; managed: boolean; version: string }
export interface Server {
  id: string; name: string; runtime: string; game_version: string; loader_version: string | null; java_major: number;
  port: number; memory_mb: number; flags_profile: 'balanced' | 'minimal'; autostart: boolean;
  restart_policy: { enabled: boolean; max_per_10m: number }; eula_accepted: boolean; desired: 'running' | 'stopped';
  state: ServerState; state_detail: string | null; state_since: string; uptime_s: number | null; pid: number | null;
  players: { online: number; max: number; sample: string[] } | null; motd: string | null;
  cpu_pct: number | null; mem_bytes: number | null; content_kind: ContentKind; tags: string[]; notes: string;
  created_at: string; capabilities: string[];
}
export interface CreateServer {
  name: string; runtime: string; game_version: string; loader_version?: string; memory_mb: number; port?: number;
  accept_eula: boolean; flags_profile?: string; autostart?: boolean; custom?: { command: string[] }; modpack_token?: string;
}
export interface StatSample { t: number; cpu: number; mem: number; players: number | null }
export interface TimelineEvent { time: string; kind: 'start' | 'ready' | 'stop' | 'crash' | 'restart' | 'backup' | 'install' | 'update' | 'undo'; detail: string }
export interface FileEntry { name: string; kind: 'dir' | 'file' | 'symlink'; size: number; modified: string }
export interface FileList { path: string; entries: FileEntry[] }
export interface FileContent { content: string; size: number; truncated: boolean; binary: boolean }
export type ProviderId = 'modrinth' | 'curseforge' | 'hangar';
export interface ProviderInfo { id: ProviderId; name: string; available: boolean; supported: boolean; reason: string | null }
export interface Hit {
  project_id: string; slug: string; title: string; description: string; author: string; icon_url: string | null;
  downloads: number; follows: number; categories: string[]; updated: string; client_side: string | null; server_side: string | null;
}
export interface SearchResult { hits: Hit[]; total: number; compat: { kind: ContentKind; loader: string; game_version: string } }
export interface ProjectVersion {
  id: string; name: string; version_number: string; channel: 'release' | 'beta' | 'alpha'; date: string; downloads: number;
  files: { filename: string; size: number; primary: boolean }[]; game_versions: string[]; loaders: string[];
  dependencies: { project_id: string; kind: 'required' | 'optional' | 'incompatible' }[]; downloadable?: boolean; warning?: string | null; external_url?: string | null;
}
export interface Project {
  project_id: string; title: string; description: string; body: string; icon_url: string | null;
  links: { source?: string; issues?: string; wiki?: string; page?: string }; license: string; gallery: string[]; versions: ProjectVersion[];
}
export type ContentSeverity = 'info' | 'warn' | 'critical';
/** Content finding (GET content/check, plan conflicts, item issues); distinct from the host-level health Finding. */
export interface ContentFinding {
  id: string; severity: ContentSeverity; item: string | null; title: string; detail: string; evidence: string[];
  action: { label: string; kind: 'toggle' | 'install' | 'remove' | 'update' | 'open'; item?: string; project_id?: string } | null;
}
export type ContentLoader = 'fabric' | 'quilt' | 'forge' | 'neoforge' | 'paper' | 'spigot' | 'bukkit' | 'velocity' | 'bungee' | 'datapack' | 'unknown';
export interface ContentMeta {
  id: string; name: string; version: string; authors: string[]; description: string; loader: ContentLoader;
  environment: 'server' | 'client' | 'both' | 'unknown'; depends: { id: string; required: boolean; range: string | null }[]; breaks: string[];
}
export interface ContentItem {
  id: string; provider: ProviderId | 'manual'; project_id: string | null; version_id: string | null; title: string;
  version_number: string | null; filename: string; kind: string; sha512: string | null; installed_at: string; pinned: boolean;
  source: 'moss' | 'manual' | 'identified' | 'modpack'; enabled: boolean; identified: boolean; meta: ContentMeta | null; issues: ContentFinding[];
}
export interface ChangePlan {
  items: { action: 'install' | 'replace' | 'remove'; title: string; filename: string; size: number; reason: string; version?: string; from_version?: string | null }[];
  warnings: string[]; requires_restart: boolean; conflicts?: ContentFinding[];
}
export interface UpgradePlan {
  from: { game_version: string; loader_version: string | null }; to: { game_version: string; loader_version: string | null };
  java: { from: number; to: number; installed: boolean }; server_jar: { available: boolean; note: string | null };
  content: { item: string; title: string; status: 'ok' | 'update' | 'missing' | 'unknown'; current: string | null; target: string | null; version_id: string | null; reason: string | null }[];
  blockers: string[]; warnings: string[]; can_proceed: boolean;
}
export type UpgradeContent = 'update' | 'disable_incompatible' | 'keep';
// ---- staged updates ("try on a copy first") and modpack versions ----
export type StagingKind = 'none' | 'update_all' | 'upgrade' | 'pack_update';
export type StagingState = 'building' | 'testing' | 'healthy' | 'unhealthy' | 'failed';
export interface StagingReport {
  healthy: boolean; reasons: string[]; error?: string; ready_secs: number | null; ready_timeout_s: number; watch_s: number; crashed: boolean;
  errors: number; warnings: number; samples: string[]; fatal: string | null; changes: { title: string; action: string; to: string | null }[];
  signature: string | null; notes: string[]; copy: { files: number; bytes: number; world: boolean }; port: number; tested_at: string;
}
export interface StagingRun {
  id: string; server_id: string; kind: StagingKind; state: StagingState; include_world: boolean; report: Partial<StagingReport>; port: number | null;
  operation_id: string | null; created_at: string; expires_at: string; expired: boolean; can_promote: boolean;
}
export interface StagingEstimate { files: number; copy_bytes: number; world_bytes: number; free_bytes: number; port_range: [number, number]; defaults: { ready_timeout_s: number; watch_s: number }; kinds: StagingKind[] }
export interface StagingRequest {
  kind: StagingKind; include_world?: boolean; upgrade?: { game_version: string; loader_version?: string; content: UpgradeContent; confirm_downgrade?: boolean }; pack?: { install_id: string; token: string };
}
export interface PackInfo {
  id: string; name: string; version: string; format: string; game_version: string; loader: string; mode: string; installed_at: string; files: number;
  pinned: boolean; items: number; override_files: number; source: { provider: string; project_id: string; version_id: string | null } | null; can_check: boolean;
}
export interface PackChange { title: string; path: string; from: string | null; to: string | null; reason: string | null }
export interface PackDiff {
  versions: { from: { version: string; game_version: string; loader: string; loader_version: string | null }; to: { version: string; game_version: string; loader: string; loader_version: string | null } };
  files: { added: PackChange[]; removed: PackChange[]; updated: PackChange[]; held: PackChange[]; unchanged: number };
  overrides: { added: string[]; updated: string[]; user_modified: { path: string; reason: string }[]; removed_upstream: string[]; unchanged: number };
  blockers: string[]; warnings: string[]; can_apply: boolean; changes: boolean; signature: string;
}
export interface PackCheck { pinned: boolean; up_to_date: boolean | null; message?: string; current: { version: string; version_id: string | null }; latest?: { version_id: string | null; version: string }; token?: string; diff?: PackDiff }

export interface ModpackInspect {
  token: string; format: 'mrpack' | 'curseforge'; name: string; version: string; summary: string | null; game_version: string;
  loader: { id: string; version: string | null }; files: { total: number; server: number; client_only: number; unavailable: number };
  unavailable: { name: string; reason: string; url?: string | null }[]; overrides: { files: number; size: number }; size: number; warnings: string[];
}
export interface Backup {
  id: string; server_id: string; created_at: string; size: number; sha256: string; status: 'running' | 'ok' | 'failed' | 'verifying';
  verified_at: string | null; note: string; kind: 'manual' | 'scheduled' | 'pre-change'; consistent: boolean; files: number;
}
export interface Schedule {
  id: string; server_id: string; kind: 'backup' | 'restart' | 'start' | 'stop' | 'command'; command: string | null;
  every_minutes: number | null; daily_at: string | null; tz_offset_min: number; enabled: boolean; keep: number | null;
  last_run: string | null; next_run: string | null; last_status: string | null;
}
export interface Operation {
  id: string; kind: string; title: string; server_id: string | null;
  state: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled' | 'interrupted'; stage: string; progress: number | null;
  log: string[]; error: string | null; created_at: string; started_at: string | null; ended_at: string | null; cancellable: boolean;
}
export interface ConfigFileInfo { id: string; title: string; path: string; format: 'yaml' | 'toml'; exists: boolean }
export interface ConfigField {
  key: string; type: 'bool' | 'int' | 'float' | 'string' | 'enum' | string; default: unknown; group: string; description: string; min: number | null; max: number | null; options: string[];
  restart: boolean; secret: boolean; danger: boolean; mixed: boolean; present: boolean; value: unknown;
}
export interface ConfigSnapshot { id: string; title: string; created_at: string }
export interface ConfigFile { id: string; title: string; path: string; format: 'yaml' | 'toml'; exists: boolean; raw: string; parse_error: string | null; fields: ConfigField[]; history: ConfigSnapshot[] }
export interface ConfigDiff { current: string; previous: string | null; snapshot: ConfigSnapshot | null }
export interface SavedView { id: string; scope: 'servers' | 'content'; name: string; filters: Record<string, unknown>; sort: string; created_at: string }
export interface Blueprint { id: string; name: string; runtime: string; game_version: string; loader_version: string | null; memory_mb: number; properties: number; content: number; schedules: number; created_by: string; created_at: string }
export interface ImportDetection {
  runtime: string | null; game_version: string | null; loader_version: string | null; port: number | null; jar: string | null; has_world: boolean;
  java_major: number | null; java_installed: boolean; name: string; size: number; notes: string[]; token?: string;
}
export interface ImportRequest { name: string; path?: string; token?: string; runtime?: string; game_version?: string; loader_version?: string; memory_mb?: number; port?: number; accept_eula: boolean }
export interface SearchResults {
  q: string; servers: { id: string; name: string; runtime: string; game_version: string }[];
  backups: { id: string; server_id: string; server_name: string; created_at: string; note: string; kind: string }[];
  content: { id: string; server_id: string; server_name: string | null; title: string; filename: string; kind: string }[];
  files: { server_id: string; server_name: string; path: string }[];
}
export interface UndoEntry { id: string; title: string; created_at: string; files: string[] }
export interface Activity { id: string; time: string; actor: string; action: string; target: string; detail: string }
export interface Finding {
  id: string; severity: 'info' | 'warn' | 'critical'; server_id: string | null; title: string; detail: string; evidence: string[];
  action: { label: string; kind: 'start' | 'backup' | 'open_tab' | 'accept_eula' | 'install_java'; tab?: string; major?: number } | null;
}
export interface User { id: string; email: string; role: Role; created_at?: string }
export interface Grant { user_id: string; email: string; bundle: 'manager' | 'operator' | 'viewer' }
export interface Settings {
  default_memory_mb: number; backup_keep: number; backup_before_change: boolean; modrinth_contact: string;
  public_base_url: string; data_dir: string; bind: string; version: string;
}

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string, public fields: { path: string; message: string }[] = [], public requestId?: string) {
    super(message);
  }
}

// ---- transport ----
let getToken: () => Promise<string | null> = async () => null;
let onUnauthorized: (e: ApiError) => void = () => {};
export function configureAuth(t: () => Promise<string | null>, u: (e: ApiError) => void) {
  getToken = t;
  onUnauthorized = u;
}

type MockMod = typeof import('./mock');
let mock: MockMod | null = null;
export async function initApi() {
  if (import.meta.env.VITE_MOCK) mock = await import('./mock');
}
export const isMock = () => mock !== null;

const BASE = '/api/v1';
const AUTH_GATES = new Set(['auth.not_provisioned', 'auth.setup_required', 'auth.mfa_required', 'auth.throttled']);
const inflight = new Map<string, Promise<unknown>>();
const cache = new Map<string, { at: number; v: unknown }>();

interface Opts { ttl?: number; signal?: AbortSignal; auth?: boolean }

async function raw(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<Response> {
  const headers: Record<string, string> = {};
  const tok = await getToken();
  if (tok) headers.Authorization = `Bearer ${tok}`;
  let payload: BodyInit | undefined;
  if (body instanceof FormData) payload = body;
  else if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    payload = JSON.stringify(body);
  }
  return fetch(BASE + path, { method, headers, body: payload, signal });
}

async function parseError(res: Response): Promise<ApiError> {
  let code = `http.${res.status}`, message = res.statusText || 'Request failed', fields: { path: string; message: string }[] = [], rid: string | undefined;
  try {
    const j = await res.json();
    if (j?.error) ({ code = code, message = message, fields = [], request_id: rid } = j.error);
  } catch { /* non-JSON error body */ }
  return new ApiError(res.status, code, message, fields, rid);
}

async function exec<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  if (mock) return (await mock.handle(method, path, body)) as T;
  const res = await raw(method, path, body, signal);
  if (!res.ok) {
    const e = await parseError(res);
    if (res.status === 401 || AUTH_GATES.has(e.code)) onUnauthorized(e);
    throw e;
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export function get<T>(path: string, o: Opts = {}): Promise<T> {
  const c = cache.get(path);
  if (o.ttl && c && Date.now() - c.at < o.ttl) return Promise.resolve(c.v as T);
  const f = inflight.get(path);
  if (f) return f as Promise<T>;
  const p = exec<T>('GET', path, undefined, o.signal)
    .then((v) => {
      if (o.ttl) cache.set(path, { at: Date.now(), v });
      return v;
    })
    .finally(() => inflight.delete(path));
  inflight.set(path, p);
  return p;
}
export function send<T = void>(method: 'POST' | 'PUT' | 'PATCH' | 'DELETE', path: string, body?: unknown): Promise<T> {
  cache.clear(); // any mutation invalidates the tiny TTL cache
  return exec<T>(method, path, body);
}
export const clearCache = () => cache.clear();

const q = (o: Record<string, string | number | undefined | null>) => {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(o)) if (v !== undefined && v !== null && v !== '') p.set(k, String(v));
  const s = p.toString();
  return s ? '?' + s : '';
};
const e = encodeURIComponent;

export const api = {
  authConfig: () => get<AuthConfig>('/auth/config'),
  me: () => get<Me>('/me'),
  status: () => get<HostStatus>('/status'),
  runtimes: () => get<Runtime[]>('/runtimes', { ttl: 60000 }),
  versions: (id: string) => get<Versions>(`/runtimes/${e(id)}/versions`, { ttl: 60000 }),
  java: () => get<{ installs: JavaInstall[] }>('/java'),
  installJava: (major: number) => send<{ operation: Operation }>('POST', '/java/install', { major }),

  servers: () => get<Server[]>('/servers'),
  server: (id: string) => get<Server>(`/servers/${e(id)}`),
  createServer: (b: CreateServer) => send<{ server: Server; operation: Operation }>('POST', '/servers', b),
  patchServer: (id: string, b: Partial<Pick<Server, 'name' | 'memory_mb' | 'port' | 'flags_profile' | 'autostart' | 'restart_policy' | 'tags' | 'notes' | 'game_version'> & { accept_eula: boolean }>) =>
    send<Server>('PATCH', `/servers/${e(id)}`, b),
  deleteServer: (id: string, data: boolean) => send('DELETE', `/servers/${e(id)}?delete_data=${data}`),
  action: (id: string, a: 'start' | 'stop' | 'restart' | 'kill') => send<{ server: Server }>('POST', `/servers/${e(id)}/actions/${a}`),
  command: (id: string, command: string) => send('POST', `/servers/${e(id)}/command`, { command }),
  stats: (id: string) => get<{ samples: StatSample[] }>(`/servers/${e(id)}/stats`),
  timeline: (id: string, limit = 200) => get<TimelineEvent[]>(`/servers/${e(id)}/timeline${q({ limit })}`),
  health: (id: string) => get<Finding[]>(`/servers/${e(id)}/health`),
  allHealth: () => get<Finding[]>('/health'),
  properties: (id: string) => get<{ raw: string; entries: { key: string; value: string }[] }>(`/servers/${e(id)}/properties`),
  saveProperties: (id: string, raw: string) => send<{ undo_id: string }>('PUT', `/servers/${e(id)}/properties`, { raw }),
  backlog: (id: string, lines = 500) => get<{ lines: string[] }>(`/servers/${e(id)}/console/backlog${q({ lines })}`),

  files: (id: string, path: string) => get<FileList>(`/servers/${e(id)}/files${q({ path })}`),
  fileContent: (id: string, path: string) => get<FileContent>(`/servers/${e(id)}/files/content${q({ path })}`),
  saveFile: (id: string, path: string, content: string) => send<{ undo_id: string }>('PUT', `/servers/${e(id)}/files/content`, { path, content }),
  mkdir: (id: string, path: string) => send('POST', `/servers/${e(id)}/files/mkdir`, { path }),
  rename: (id: string, from: string, to: string) => send('POST', `/servers/${e(id)}/files/rename`, { from, to }),
  deleteFile: (id: string, path: string) => send('DELETE', `/servers/${e(id)}/files${q({ path })}`),
  upload: (id: string, path: string, files: File[]) => {
    const fd = new FormData();
    for (const f of files) fd.append('file', f, f.name);
    return send('POST', `/servers/${e(id)}/files/upload${q({ path })}`, fd);
  },

  search: (p: { server_id: string; q: string; offset?: number; limit?: number; sort?: string; category?: string; provider?: ProviderId }) =>
    get<SearchResult>(`/content/search${q(p)}`, { ttl: 30000 }),
  providers: (server_id: string) => get<{ providers: ProviderInfo[] }>(`/content/providers${q({ server_id })}`, { ttl: 60000 }),
  project: (pid: string, server_id: string, provider?: ProviderId) => get<Project>(`/content/projects/${e(pid)}${q({ server_id, provider })}`, { ttl: 60000 }),
  content: (id: string) => get<{ items: ContentItem[] }>(`/servers/${e(id)}/content`),
  plan: (id: string, project_id: string, version_id?: string, provider?: ProviderId) => send<ChangePlan>('POST', `/servers/${e(id)}/content/plan`, { project_id, version_id, provider }),
  install: (id: string, project_id: string, version_id?: string, provider?: ProviderId) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/content`, { project_id, version_id, provider }),
  removeContent: (id: string, item: string) => send<{ operation: Operation }>('DELETE', `/servers/${e(id)}/content/${e(item)}`),
  checkUpdates: (id: string) => send<{ updates: { item_id: string; title: string; current: string; latest: string; version_id: string }[]; warnings?: string[] }>('POST', `/servers/${e(id)}/content/check-updates`),
  updateContent: (id: string, item: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/content/${e(item)}/update`),
  pin: (id: string, item: string, pinned: boolean) => send('PATCH', `/servers/${e(id)}/content/${e(item)}`, { pinned }),
  // WP-A v2: toggle, identify, compatibility check, update-all, upgrade assistant, modpacks
  toggleContent: (id: string, item: string, enabled: boolean) => send<{ item: ContentItem; requires_restart?: boolean }>('POST', `/servers/${e(id)}/content/${e(item)}/toggle`, { enabled }),
  identify: (id: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/content/identify`),
  contentCheck: (id: string) => get<{ findings: ContentFinding[]; checked_at: string }>(`/servers/${e(id)}/content/check`),
  updateAllPlan: (id: string) => send<ChangePlan>('POST', `/servers/${e(id)}/content/update-all/plan`),
  updateAll: (id: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/content/update-all`),
  upgradePlan: (id: string, game_version: string, loader_version?: string) => send<UpgradePlan>('POST', `/servers/${e(id)}/upgrade/plan`, { game_version, loader_version }),
  upgrade: (id: string, b: { game_version: string; loader_version?: string; content: UpgradeContent; backup: true; confirm_downgrade?: boolean }) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/upgrade`, b),
  inspectModpack: (b: { provider: ProviderId; project_id: string; version_id?: string }) => send<ModpackInspect>('POST', '/modpacks/inspect', b),
  importModpack: (id: string, token: string, mode: 'add' | 'replace') => send<{ operation: Operation }>('POST', `/servers/${e(id)}/modpacks`, { token, mode }),
  modpacks: (id: string) => get<{ packs: PackInfo[] }>(`/servers/${e(id)}/modpacks`),
  pinPack: (id: string, install: string, pinned: boolean) => send<{ id: string; pinned: boolean }>('PATCH', `/servers/${e(id)}/modpacks/${e(install)}`, { pinned }),
  checkPack: (id: string, install: string) => send<PackCheck>('POST', `/servers/${e(id)}/modpacks/${e(install)}/check-update`, {}),
  updatePack: (id: string, install: string, token: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/modpacks/${e(install)}/update`, { token }),
  stagingEstimate: (id: string) => get<StagingEstimate>(`/servers/${e(id)}/staging/estimate`),
  stagingRuns: (id: string) => get<StagingRun[]>(`/servers/${e(id)}/staging`),
  stagingRun: (id: string, run: string) => get<StagingRun>(`/servers/${e(id)}/staging/${e(run)}`),
  stagingLog: (id: string, run: string, lines = 200) => get<{ lines: string[]; truncated: boolean }>(`/servers/${e(id)}/staging/${e(run)}/log${q({ lines })}`),
  stagingStart: (id: string, b: StagingRequest) => send<{ run: StagingRun; operation: Operation }>('POST', `/servers/${e(id)}/staging`, b),
  stagingPromote: (id: string, run: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/staging/${e(run)}/promote`),
  stagingDiscard: (id: string, run: string) => send<{ state: string }>('DELETE', `/servers/${e(id)}/staging/${e(run)}`),

  backups: (id: string) => get<Backup[]>(`/servers/${e(id)}/backups`),
  allBackups: () => get<Backup[]>('/backups'),
  createBackup: (id: string, note?: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/backups`, { note }),
  verifyBackup: (id: string) => send<{ operation?: Operation }>('POST', `/backups/${e(id)}/verify`),
  restoreBackup: (id: string, mode: 'new_server' | 'in_place') => send<{ operation?: Operation }>('POST', `/backups/${e(id)}/restore`, { mode }),
  deleteBackup: (id: string) => send('DELETE', `/backups/${e(id)}`),

  schedules: (id: string) => get<Schedule[]>(`/servers/${e(id)}/schedules`),
  createSchedule: (id: string, s: Partial<Schedule>) => send<Schedule>('POST', `/servers/${e(id)}/schedules`, s),
  patchSchedule: (id: string, s: Partial<Schedule>) => send<Schedule>('PATCH', `/schedules/${e(id)}`, s),
  deleteSchedule: (id: string) => send('DELETE', `/schedules/${e(id)}`),
  runSchedule: (id: string) => send('POST', `/schedules/${e(id)}/run`),

  operations: (limit = 50, server_id?: string) => get<Operation[]>(`/operations${q({ limit, server_id })}`),
  cancelOp: (id: string) => send('POST', `/operations/${e(id)}/cancel`),
  prefsGet: () => get<{ data: Record<string, unknown>; updated_at: string | null }>('/me/preferences'),
  prefsSave: (data: Record<string, unknown>) => send<{ data: Record<string, unknown>; updated_at: string }>('PUT', '/me/preferences', { data }),
  views: (scope: SavedView['scope']) => get<SavedView[]>(`/views${q({ scope })}`),
  saveView: (v: { scope: SavedView['scope']; name: string; filters: Record<string, unknown>; sort?: string }) => send<SavedView>('POST', '/views', v),
  deleteView: (id: string) => send('DELETE', `/views/${e(id)}`),
  blueprints: () => get<Blueprint[]>('/blueprints'),
  saveBlueprint: (server_id: string, name: string) => send<Blueprint>('POST', '/blueprints', { server_id, name }),
  exportBlueprint: (id: string) => get<Record<string, unknown>>(`/blueprints/${e(id)}/export`),
  importBlueprint: (doc: Record<string, unknown>) => send<Blueprint>('POST', '/blueprints/import', doc),
  shareBlueprint: (id: string) => get<{ code: string; length: number }>(`/blueprints/${e(id)}/share`),
  importBlueprintCode: (code: string) => send<Blueprint>('POST', '/blueprints/import-code', { code }),
  deleteBlueprint: (id: string) => send('DELETE', `/blueprints/${e(id)}`),
  createFromBlueprint: (id: string, b: { name: string; port?: number; accept_eula: boolean }) => send<{ server: Server; operation: Operation }>('POST', `/blueprints/${e(id)}/create`, b),
  importInspectPath: (path: string) => send<ImportDetection>('POST', '/servers/import/inspect', { path }),
  importInspectFile: (file: File) => { const fd = new FormData(); fd.append('file', file, file.name); return send<ImportDetection>('POST', '/servers/import/inspect', fd); },
  importServer: (b: ImportRequest) => send<{ server: Server; operation: Operation }>('POST', '/servers/import', b),
  searchAll: (text: string, limit = 6) => get<SearchResults>(`/search${q({ q: text, limit })}`),
  configFiles: (id: string) => get<{ files: ConfigFileInfo[] }>(`/servers/${e(id)}/config`),
  configFile: (id: string, file: string) => get<ConfigFile>(`/servers/${e(id)}/config/${e(file)}`),
  saveConfig: (id: string, file: string, body: { raw: string } | { set?: Record<string, unknown>; unset?: string[] }) => send<{ undo_id: string | null; changed: boolean }>('PUT', `/servers/${e(id)}/config/${e(file)}`, body),
  configDiff: (id: string, file: string, snapshot?: string) => get<ConfigDiff>(`/servers/${e(id)}/config/${e(file)}/diff${q({ snapshot })}`),
  undoList: (id: string) => get<UndoEntry[]>(`/servers/${e(id)}/undo`),
  revert: (id: string) => send<{ ok: boolean }>('POST', `/undo/${e(id)}/revert`),
  activity: (limit = 100, server_id?: string) => get<Activity[]>(`/activity${q({ limit, server_id })}`),

  users: () => get<User[]>('/users'),
  addUser: (email: string, role: Role) => send<User>('POST', '/users', { email, role }),
  setRole: (id: string, role: Role) => send('PATCH', `/users/${e(id)}`, { role }),
  deleteUser: (id: string) => send('DELETE', `/users/${e(id)}`),
  access: (id: string) => get<Grant[]>(`/servers/${e(id)}/access`),
  saveAccess: (id: string, grants: { user_id: string; bundle: string }[]) => send('PUT', `/servers/${e(id)}/access`, { grants }),
  settings: () => get<Settings>('/settings'),
  saveSettings: (s: Partial<Settings>) => send<Settings>('PUT', '/settings', s),
};

// ---- downloads / streams ----
export async function download(path: string, filename: string) {
  if (mock) return saveBlob(await mock.blobFor(path), filename);
  const res = await raw('GET', path);
  if (!res.ok) throw await parseError(res);
  saveBlob(await res.blob(), filename);
}
export function saveBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 5000);
}
export const fileDownloadPath = (id: string, path: string) => `/servers/${e(id)}/files/download${q({ path })}`;
export const backupDownloadPath = (id: string) => `/backups/${e(id)}/download`;
export const modpackExportPath = (id: string) => `/servers/${e(id)}/modpacks/export`;
/** Small helpers for lib/upload.ts (XHR upload with progress, lazy-loaded). */
export const apiUrl = (p: string) => BASE + p;
export const authToken = () => getToken();

async function ticket(): Promise<string> {
  const r = await exec<{ ticket: string }>('POST', '/ticket');
  return r.ticket;
}

export interface Stream { close(): void }

export interface EventHandlers { onOpen(): void; onError(): void; op(o: Operation): void; server(s: Server): void; activity(a: Activity): void; notification?(n: AppNotification): void; resync?(): void }
export function openEvents(h: EventHandlers): Stream {
  if (mock) return mock.openEvents(h);
  let es: EventSource | null = null;
  let closed = false;
  ticket()
    .then((t) => {
      if (closed) return;
      es = new EventSource(`${BASE}/events?ticket=${e(t)}`);
      es.onopen = h.onOpen;
      es.onerror = () => {
        es?.close(); // ticket is single use: caller reconnects with a fresh one
        h.onError();
      };
      const on = <T>(n: string, f: (v: T) => void) => es!.addEventListener(n, (ev) => { try { f(JSON.parse((ev as MessageEvent).data)); } catch { /* ignore */ } });
      on('op', h.op);
      on('server', h.server);
      on('activity', h.activity);
      on('notification', (n: AppNotification) => h.notification?.(n));
      es.addEventListener('resync', () => h.resync?.());
    })
    .catch(() => !closed && h.onError());
  return { close: () => { closed = true; es?.close(); } };
}

export interface ConsoleMsg { t: 'line' | 'state'; d: string }
export interface ConsoleSocket extends Stream { send(cmd: string): void }

export function openConsole(id: string, h: { onOpen(): void; onClose(): void; onMsg(m: ConsoleMsg): void }): ConsoleSocket {
  if (mock) return mock.openConsole(id, h);
  let ws: WebSocket | null = null;
  let closed = false;
  ticket()
    .then((t) => {
      if (closed) return;
      const proto = location.protocol === 'https:' ? 'wss' : 'ws';
      ws = new WebSocket(`${proto}://${location.host}${BASE}/servers/${e(id)}/console?ticket=${e(t)}`);
      ws.onopen = h.onOpen;
      ws.onclose = h.onClose;
      ws.onmessage = (ev) => { try { h.onMsg(JSON.parse(ev.data)); } catch { /* ignore */ } };
    })
    .catch(() => !closed && h.onClose());
  return {
    close: () => { closed = true; ws?.close(); },
    send: (d) => ws?.readyState === 1 && ws.send(JSON.stringify({ t: 'cmd', d })),
  };
}

// ===== Container isolation (optional Docker/Podman mode) =====
export type Isolation = 'process' | 'container';
export type ContainerNetwork = 'open' | 'game-only';
export interface ExtraPort { port: number; proto: 'tcp' | 'udp' }
export interface ServerContainer { image: string; image_override: string | null; running_in_container: boolean; restart_needed: boolean; network?: ContainerNetwork; extra_ports?: ExtraPort[] }
export interface Server { isolation?: Isolation; container?: ServerContainer }
export interface ContainerStatus { available: boolean; engine: 'docker' | 'podman' | null; version: string | null; rootless: boolean | null; message: string | null; outbound_restricted: boolean; game_only_supported?: boolean; default_image: string }

export const errMsg = (x: unknown) => (x instanceof ApiError ? x.message : x instanceof Error ? x.message : String(x));

// ===== WP-F2 additions (docs/next-api.md WP-B / WP-C / WP-D); declaration merging keeps the originals untouched =====
export interface AuthConfig { setup_required?: boolean; access_message?: string }
export interface Me { mfa?: { required: boolean; satisfied: boolean } }
export interface Limits { memory_mb: number | null; cpu_pct: number | null; pids: number | null; enforced: boolean; backend: 'cgroup2' | 'none'; detail: string | null }
export interface Server { limits?: Limits }
export interface Backup { worlds?: string[]; locked?: boolean; remote?: RemoteState; remote_error?: string | null; remote_at?: string | null; remote_destination?: string | null; local_present?: boolean }
export type RemoteState = 'none' | 'uploading' | 'stored' | 'failed';
export interface BackupDestination {
  id: string; name: string; kind: 'folder' | 's3'; is_default: boolean; keep_local: number; keep_remote: number; created_at: string; stored_backups?: number;
  path?: string; endpoint?: string; region?: string; bucket?: string; prefix?: string; path_style?: boolean; access_key?: 'set' | 'not set'; secret_key?: 'set' | 'not set';
}
export interface DestinationInput {
  name?: string; kind?: 'folder' | 's3'; path?: string; endpoint?: string; region?: string; bucket?: string; prefix?: string; path_style?: boolean;
  access_key?: string; secret_key?: string; is_default?: boolean; keep_local?: number; keep_remote?: number;
}
export interface DestinationTest { ok: boolean; steps: { step: string; ok: boolean; detail: string }[] }
export interface ServerDestination { destination_id: string; effective: { id: string; name: string; kind: string } | null }
export interface StagedRestore {
  id: string; backup_id: string; server_id: string; only: string[] | null; status: 'preparing' | 'ready' | 'failed' | 'promoting' | 'promoted'; error: string | null; created_at: string;
  can_promote?: boolean; checks?: { id: string; ok: boolean; blocking: boolean; message: string }[]; warnings?: string[];
  diff?: { archive: { files: number; size: number }; current: { files: number; size: number }; added: number; removed: number; changed: number; unchanged: number; size_delta: number; samples: { added: string[]; removed: string[]; changed: string[] } };
}
export interface Schedule { tz?: string }
export type AuditMode = 'redacted' | 'all' | 'off';
export interface Settings { notify_webhook_url?: string; notify_events?: string[]; audit_console_commands?: AuditMode; access_message?: string }

export type NotificationKind = 'crash' | 'crash_loop' | 'backup_failed' | 'disk_low' | 'operation_failed' | 'restart_failed' | 'update_available' | 'security' | 'webhook_disabled';
export type Severity = 'info' | 'warn' | 'critical';
export interface AppNotification { id: string; time: string; kind: NotificationKind; severity: Severity; title: string; body: string; server_id: string | null; read: boolean }
export interface NotificationList { items: AppNotification[]; unread: number }

export type TokenScope = 'read' | 'full';
export interface ApiToken { id: string; name: string; scope: TokenScope; prefix: string; created_at: string; last_used: string | null; expires_at: string | null }
export interface CreatedToken { id: string; token: string; name: string; scope: TokenScope; expires_at: string | null }

export interface RestorePreview {
  mode: 'new_server' | 'in_place'; archive: { files: number; size: number; created_at: string }; current: { files: number; size: number } | null;
  added: number; removed: number; changed: number; unchanged: number; size_delta: number;
  samples: { added: string[]; removed: string[]; changed: string[] }; worlds: string[]; warnings: string[];
}
export type MetricRange = '1h' | '24h' | '7d' | '30d';
export interface MetricSample { t: number; cpu_avg: number; cpu_max: number; mem_avg: number; mem_max: number; players_max?: number; disk_free?: number }
export interface MetricSeries { range: MetricRange; step_s: number; samples: MetricSample[] }
export type AnalyticsDays = 7 | 30 | 90;
export interface PlayerAnalytics {
  days: AnalyticsDays; hourly: { hour: string; peak: number; avg: number }[]; unique_players: number; sessions: number; total_play_seconds: number; avg_session_seconds: number;
  peak: { count: number; at: string | null };
  top_players: { name: string; play_seconds: number; sessions: number; last_seen: string; online: boolean }[];
  recent_sessions: { name: string; joined_at: string; left_at: string | null; seconds: number }[];
}
export interface SupportPreview { files: { name: string; size: number }[]; redactions: string[] }

export interface Webhook { id: string; name: string; url: string; events: string[]; server_ids: string[]; enabled: boolean; disabled_reason: string | null; consecutive_failures: number; secret: 'set' | 'not set' | string; pending: number; last_delivery: { state: string; at: string; status_code: number | null } | null; created_at: string }
export interface WebhookEventType { type: string; description: string }
export interface WebhookList { items: Webhook[]; event_types: WebhookEventType[] }
export interface WebhookDelivery { id: string; event_id: string; type: string; test: boolean; state: 'pending' | 'delivered' | 'dead'; attempts: number; max_attempts: number; created_at: string; finished_at: string | null; next_attempt_at: string | null; status_code: number | null; latency_ms: number | null; response: string | null; error: string | null }
export interface WebhookInput { name?: string; url?: string; events?: string[]; server_ids?: string[]; enabled?: boolean }

/** `url` is a signed, expiring path to the proxied map (`/map/{id}/{token}/`); `expires_at` is unix seconds. */
export interface ServerMap { available: boolean; kind?: 'bluemap' | 'squaremap' | 'pl3xmap' | 'dynmap'; running?: boolean; reachable?: boolean; url?: string; expires_at?: number }
export interface StatusPage { id: string; slug: string; path: string; title: string; description: string; enabled: boolean; show_players: boolean; show_map: boolean; show_address: string | null; server_ids: string[]; created_at: string }
export interface StatusPageInput { title?: string; slug?: string; description?: string; enabled?: boolean; show_players?: boolean; show_map?: boolean; show_address?: string | null; server_ids?: string[] }
export type WorkflowAction =
  | { type: 'start' | 'stop' | 'restart' | 'backup'; server: string }
  | { type: 'command'; server: string; command: string }
  | { type: 'wait'; seconds: number }
  | { type: 'notify'; title: string; body: string }
  | { type: 'webhook'; url?: string; endpoint_id?: string };
export interface WorkflowCondition { field: string; op: string; value: string | number }
export interface Workflow { id: string; name: string; enabled: boolean; trigger: string; server_ids: string[]; condition: WorkflowCondition | null; actions: WorkflowAction[]; created_by: string; created_at: string; last_run_at: string | null; last_result: string | null; disabled_reason: string | null }
export interface WorkflowList { items: Workflow[]; event_types: WebhookEventType[] }
export interface WorkflowInput { name?: string; enabled?: boolean; trigger?: string; server_ids?: string[]; condition?: WorkflowCondition | null; actions?: WorkflowAction[] }
export interface WorkflowRun { id: string; at: string; event: { type: string; server: { id: string; name: string } | null; data: Record<string, unknown> }; ok: boolean; log: string[] }
export interface WorkflowStep { index: number; action: string; description: string; server: string | null; allowed: boolean; problem: string | null }
export interface WorkflowPlan { event: WorkflowRun['event']; trigger_matches: boolean; condition_holds: boolean; would_run: boolean; executed: false; steps: WorkflowStep[] }

export const apiOps = {
  statusPages: () => get<{ items: StatusPage[] }>('/status-pages', { ttl: 0 }),
  addStatusPage: (b: StatusPageInput) => send<StatusPage>('POST', '/status-pages', b),
  patchStatusPage: (id: string, b: StatusPageInput) => send<StatusPage>('PATCH', `/status-pages/${e(id)}`, b),
  removeStatusPage: (id: string) => send('DELETE', `/status-pages/${e(id)}`),
  webhooks: () => get<WebhookList>('/webhooks', { ttl: 0 }),
  addWebhook: (b: WebhookInput) => send<Webhook>('POST', '/webhooks', b),
  patchWebhook: (id: string, b: WebhookInput) => send<Webhook>('PATCH', `/webhooks/${e(id)}`, b),
  removeWebhook: (id: string) => send('DELETE', `/webhooks/${e(id)}`),
  rotateWebhookSecret: (id: string) => send<{ secret: string }>('POST', `/webhooks/${e(id)}/rotate-secret`),
  testWebhookEndpoint: (id: string) => send<{ delivery: WebhookDelivery }>('POST', `/webhooks/${e(id)}/test`),
  webhookDeliveries: (id: string) => get<{ items: WebhookDelivery[] }>(`/webhooks/${e(id)}/deliveries`, { ttl: 0 }),
  retryWebhookDelivery: (id: string, did: string) => send<{ delivery: WebhookDelivery }>('POST', `/webhooks/${e(id)}/deliveries/${e(did)}/retry`),
  workflows: () => get<WorkflowList>('/workflows', { ttl: 0 }),
  addWorkflow: (b: WorkflowInput) => send<Workflow>('POST', '/workflows', b),
  patchWorkflow: (id: string, b: WorkflowInput) => send<Workflow>('PATCH', `/workflows/${e(id)}`, b),
  removeWorkflow: (id: string) => send('DELETE', `/workflows/${e(id)}`),
  testWorkflow: (id: string, b: { server_id?: string } = {}) => send<WorkflowPlan>('POST', `/workflows/${e(id)}/test`, b),
  workflowRuns: (id: string) => get<{ items: WorkflowRun[] }>(`/workflows/${e(id)}/runs`, { ttl: 0 }),
  claimSetup: (token: string) => send<{ role: Role }>('POST', '/setup/claim', { token }),
  notifications: (o: { unread?: boolean; limit?: number } = {}) => get<NotificationList>(`/notifications${q({ unread: o.unread ? 1 : undefined, limit: o.limit ?? 50 })}`),
  markRead: (b: { ids?: string[]; all?: boolean }) => send('POST', '/notifications/read', b),
  testWebhook: () => send<{ delivered: boolean; detail: string }>('POST', '/notifications/test'),
  tokens: (all = false) => get<ApiToken[]>(`/tokens${q({ all: all ? 1 : undefined })}`),
  createToken: (b: { name: string; scope: TokenScope; expires_in_days?: number }) => send<CreatedToken>('POST', '/tokens', b),
  revokeToken: (id: string) => send('DELETE', `/tokens/${e(id)}`),
  timezones: () => get<{ zones: string[] }>('/timezones', { ttl: 600000 }),
  patchBackup: (id: string, b: { locked?: boolean; note?: string }) => send<Backup>('PATCH', `/backups/${e(id)}`, b),
  previewRestore: (id: string, mode: 'new_server' | 'in_place', only?: string[]) => send<RestorePreview>('POST', `/backups/${e(id)}/preview`, { mode, only }),
  restore: (id: string, mode: 'new_server' | 'in_place', only?: string[]) => send<{ operation?: Operation }>('POST', `/backups/${e(id)}/restore`, { mode, only }),
  prepareRestore: (id: string, only?: string[]) => send<{ operation?: Operation; staged_id: string }>('POST', `/backups/${e(id)}/restore`, { mode: 'in_place', only, staged: true }),
  stagedRestores: (sid: string) => get<StagedRestore[]>(`/servers/${e(sid)}/staged-restores`, { ttl: 0 }),
  stagedRestore: (id: string) => get<StagedRestore>(`/staged-restores/${e(id)}`, { ttl: 0 }),
  promoteStaged: (id: string) => send<{ operation?: Operation }>('POST', `/staged-restores/${e(id)}/promote`),
  discardStaged: (id: string) => send('DELETE', `/staged-restores/${e(id)}`),
  uploadBackup: (id: string) => send<{ operation?: Operation }>('POST', `/backups/${e(id)}/upload`),
  fetchBackup: (id: string) => send<{ operation?: Operation }>('POST', `/backups/${e(id)}/fetch`),
  destinations: () => get<BackupDestination[]>('/backup-destinations', { ttl: 0 }),
  addDestination: (b: DestinationInput) => send<BackupDestination>('POST', '/backup-destinations', b),
  patchDestination: (id: string, b: DestinationInput) => send<BackupDestination>('PATCH', `/backup-destinations/${e(id)}`, b),
  removeDestination: (id: string, force = false) => send('DELETE', `/backup-destinations/${e(id)}${q({ force: force ? 1 : undefined })}`),
  testDestination: (id: string) => send<DestinationTest>('POST', `/backup-destinations/${e(id)}/test`),
  testNewDestination: (b: DestinationInput) => send<DestinationTest>('POST', '/backup-destinations/test', b),
  serverDestination: (sid: string) => get<ServerDestination>(`/servers/${e(sid)}/backup-destination`, { ttl: 0 }),
  setServerDestination: (sid: string, destination_id: string) => send<ServerDestination>('PUT', `/servers/${e(sid)}/backup-destination`, { destination_id }),
  metrics: (id: string, range: MetricRange) => get<MetricSeries>(`/servers/${e(id)}/metrics${q({ range })}`),
  map: (id: string) => get<ServerMap>(`/servers/${e(id)}/map`, { ttl: 0 }),
  analytics: (id: string, days: AnalyticsDays) => get<PlayerAnalytics>(`/servers/${e(id)}/analytics${q({ days })}`),
  hostMetrics: (range: MetricRange) => get<MetricSeries>(`/metrics/host${q({ range })}`),
  supportPreview: () => get<SupportPreview>('/support-bundle?preview=1'),
  downloadSupportBundle: () => download('/support-bundle', 'moss-support-bundle.zip'),
  removeJava: (major: number) => send('DELETE', `/java/${major}`),
  reinstall: (id: string, keep_data: boolean) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/actions/reinstall`, { keep_data }),
  containerStatus: () => get<ContainerStatus>('/container/status'),
  patchIsolation: (id: string, b: { isolation?: Isolation; container_image?: string | null; network?: ContainerNetwork; extra_ports?: ExtraPort[] }) => send<Server>('PATCH', `/servers/${e(id)}`, b),
  pullImage: (id: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/container/pull`),
  patchLimits: (id: string, b: { memory_limit_mb: number | null; cpu_limit_pct: number | null; pids_limit: number | null }) => send<Server>('PATCH', `/servers/${e(id)}`, b),
};
/** Supabase auth REST calls in mock mode are answered by the mock backend (dev only). */
export const mockAuthCall = (method: string, path: string, body?: unknown): Promise<any> | null => (mock ? mock.handleAuth(method, path, body) : null);
