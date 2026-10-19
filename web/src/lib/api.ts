// Typed client for docs/api.md. In mock mode (dev only) requests are answered in-memory.
export type ServerState = 'installing' | 'stopped' | 'starting' | 'running' | 'stopping' | 'crashed' | 'failed' | 'needs_attention';
export type ContentKind = 'plugins' | 'mods' | 'proxy_plugins' | 'datapacks' | 'none';
export type Role = 'owner' | 'admin' | 'user';

export interface AuthConfig { mode: 'supabase' | 'dev'; supabase_url?: string; supabase_anon_key?: string; owner_claimed?: boolean }
export interface Me { id: string; email: string; role: Role; permissions: string[] }
export interface HostStatus {
  version: string; uptime_s: number; auth_mode: string;
  host: { cpu_pct: number; mem_total: number; mem_used: number; disk_total: number; disk_free: number; load1: number; hostname: string };
  counts: { servers: number; running: number; players: number };
}
export interface Runtime {
  id: string; name: string; category: 'official' | 'plugins' | 'modded' | 'proxy' | 'custom';
  tier: 'stable' | 'experimental' | 'planned'; content_kind: ContentKind; description: string; installable: boolean;
}
export interface Versions { versions: { id: string; stable: boolean }[]; recommended: string }
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
  accept_eula: boolean; flags_profile?: string; autostart?: boolean; custom?: { command: string[] };
}
export interface StatSample { t: number; cpu: number; mem: number; players: number | null }
export interface TimelineEvent { time: string; kind: 'start' | 'ready' | 'stop' | 'crash' | 'restart' | 'backup' | 'install' | 'update' | 'undo'; detail: string }
export interface FileEntry { name: string; kind: 'dir' | 'file' | 'symlink'; size: number; modified: string }
export interface FileList { path: string; entries: FileEntry[] }
export interface FileContent { content: string; size: number; truncated: boolean; binary: boolean }
export interface Hit {
  project_id: string; slug: string; title: string; description: string; author: string; icon_url: string | null;
  downloads: number; follows: number; categories: string[]; updated: string; client_side: string; server_side: string;
}
export interface SearchResult { hits: Hit[]; total: number; compat: { kind: ContentKind; loader: string; game_version: string } }
export interface ProjectVersion {
  id: string; name: string; version_number: string; channel: 'release' | 'beta' | 'alpha'; date: string; downloads: number;
  files: { filename: string; size: number; primary: boolean }[]; game_versions: string[]; loaders: string[];
  dependencies: { project_id: string; kind: 'required' | 'optional' | 'incompatible' }[];
}
export interface Project {
  project_id: string; title: string; description: string; body: string; icon_url: string | null;
  links: { source?: string; issues?: string; wiki?: string; page?: string }; license: string; gallery: string[]; versions: ProjectVersion[];
}
export interface ContentItem {
  id: string; provider: 'modrinth' | 'manual'; project_id: string | null; version_id: string | null; title: string;
  version_number: string | null; filename: string; kind: string; sha512: string | null; installed_at: string; pinned: boolean; source: 'moss' | 'manual';
}
export interface ChangePlan { items: { action: 'install' | 'replace' | 'remove'; title: string; filename: string; size: number; reason: string }[]; warnings: string[]; requires_restart: boolean }
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
    if (res.status === 401 || e.code === 'auth.not_provisioned') onUnauthorized(e);
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

  search: (p: { server_id: string; q: string; offset?: number; limit?: number; sort?: string; category?: string }) =>
    get<SearchResult>(`/content/search${q(p)}`, { ttl: 30000 }),
  project: (pid: string, server_id: string) => get<Project>(`/content/projects/${e(pid)}${q({ server_id })}`, { ttl: 60000 }),
  content: (id: string) => get<{ items: ContentItem[] }>(`/servers/${e(id)}/content`),
  plan: (id: string, project_id: string, version_id?: string) => send<ChangePlan>('POST', `/servers/${e(id)}/content/plan`, { project_id, version_id }),
  install: (id: string, project_id: string, version_id?: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/content`, { project_id, version_id }),
  removeContent: (id: string, item: string) => send<{ operation: Operation }>('DELETE', `/servers/${e(id)}/content/${e(item)}`),
  checkUpdates: (id: string) => send<{ updates: { item_id: string; title: string; current: string; latest: string; version_id: string }[] }>('POST', `/servers/${e(id)}/content/check-updates`),
  updateContent: (id: string, item: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/content/${e(item)}/update`),
  pin: (id: string, item: string, pinned: boolean) => send('PATCH', `/servers/${e(id)}/content/${e(item)}`, { pinned }),

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

async function ticket(): Promise<string> {
  const r = await exec<{ ticket: string }>('POST', '/ticket');
  return r.ticket;
}

export interface Stream { close(): void }

export interface EventHandlers { onOpen(): void; onError(): void; op(o: Operation): void; server(s: Server): void; activity(a: Activity): void }
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

export const errMsg = (x: unknown) => (x instanceof ApiError ? x.message : x instanceof Error ? x.message : String(x));
