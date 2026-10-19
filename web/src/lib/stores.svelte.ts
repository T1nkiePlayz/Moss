import { api, openEvents, errMsg, type Activity, type Finding, type HostStatus, type Operation, type Server, type Stream } from './api';

// ---------- visibility + polling ----------
export const page = $state({ visible: typeof document === 'undefined' || document.visibilityState === 'visible' });
document.addEventListener('visibilitychange', () => (page.visible = document.visibilityState === 'visible'));

/** Run fn now and every `ms` while the tab is visible. Returns a stop function. */
export function poll(fn: () => unknown, ms: number): () => void {
  let t: ReturnType<typeof setInterval> | undefined;
  const start = () => { if (t) return; void fn(); t = setInterval(() => void fn(), ms); };
  const stop = () => { clearInterval(t); t = undefined; };
  const onVis = () => (document.visibilityState === 'visible' ? start() : stop());
  document.addEventListener('visibilitychange', onVis);
  onVis();
  return () => { stop(); document.removeEventListener('visibilitychange', onVis); };
}

// ---------- preferences ----------
export interface Prefs {
  theme: 'system' | 'dark' | 'light' | 'contrast';
  accent: 'moss' | 'teal' | 'amber' | 'violet';
  density: 'comfortable' | 'compact';
  motion: 'system' | 'reduce';
  sidebar: 'full' | 'icons';
  serversView: 'cards' | 'table';
}
const defaults: Prefs = { theme: 'system', accent: 'moss', density: 'comfortable', motion: 'system', sidebar: 'full', serversView: 'cards' };
const loadPrefs = (): Prefs => { try { return { ...defaults, ...JSON.parse(localStorage.getItem('moss.prefs') || '{}') }; } catch { return { ...defaults }; } };
export const prefs = $state<Prefs>(loadPrefs());
const mq = matchMedia('(prefers-color-scheme: light)');
export function applyPrefs() {
  const d = document.documentElement;
  d.dataset.theme = prefs.theme === 'system' ? (mq.matches ? 'light' : 'dark') : prefs.theme;
  d.dataset.accent = prefs.accent;
  d.dataset.density = prefs.density;
  d.dataset.motion = prefs.motion;
  try { localStorage.setItem('moss.prefs', JSON.stringify($state.snapshot(prefs))); } catch { /* storage unavailable */ }
}
mq.addEventListener('change', () => prefs.theme === 'system' && applyPrefs());
export function setPref<K extends keyof Prefs>(k: K, v: Prefs[K]) { prefs[k] = v; applyPrefs(); }
export const cycleTheme = () => {
  const cur = document.documentElement.dataset.theme;
  setPref('theme', cur === 'dark' ? 'light' : 'dark');
};

// ---------- toasts ----------
export interface Toast { id: number; kind: 'info' | 'success' | 'warn' | 'error'; title: string; body?: string; action?: { label: string; run: () => void }; timeout: number }
export const toasts = $state<{ list: Toast[] }>({ list: [] });
let tid = 0;
export function toast(kind: Toast['kind'], title: string, o: { body?: string; action?: Toast['action']; timeout?: number } = {}) {
  const t: Toast = { id: ++tid, kind, title, body: o.body, action: o.action, timeout: o.timeout ?? (o.action ? 9000 : kind === 'error' ? 8000 : 4000) };
  toasts.list.push(t);
  if (toasts.list.length > 5) toasts.list.shift();
  setTimeout(() => dismissToast(t.id), t.timeout);
}
export const dismissToast = (id: number) => { toasts.list = toasts.list.filter((t) => t.id !== id); };
export const toastError = (e: unknown, title = 'Something went wrong') => toast('error', title, { body: errMsg(e) });

/** Toast with an Undo button wired to POST /undo/{id}/revert. */
export function undoToast(title: string, undoId: string, after?: () => void) {
  toast('success', title, {
    action: {
      label: 'Undo',
      run: async () => {
        try { await api.revert(undoId); toast('info', 'Reverted'); after?.(); } catch (e) { toastError(e, 'Undo failed'); }
      },
    },
  });
}

// ---------- confirm dialog ----------
export interface ConfirmReq {
  title: string; body?: string; confirmLabel?: string; danger?: boolean; typed?: string; checkbox?: string; consequence?: string;
}
export const dialog = $state<{ req: (ConfirmReq & { resolve: (r: { ok: boolean; checked: boolean }) => void }) | null }>({ req: null });
export function confirmDialog(req: ConfirmReq) {
  return new Promise<{ ok: boolean; checked: boolean }>((resolve) => { dialog.req = { ...req, resolve }; });
}

// ---------- ui flags ----------
export const ui = $state({ palette: false, ops: false, drawer: false, userMenu: false });

// ---------- servers ----------
export const servers = $state<{ list: Server[]; loaded: boolean; error: string; live: boolean }>({ list: [], loaded: false, error: '', live: false });
export const serverById = (id: string) => servers.list.find((s) => s.id === id);

export async function refreshServers() {
  try {
    servers.list = await api.servers();
    servers.error = '';
  } catch (e) { servers.error = errMsg(e); }
  servers.loaded = true;
}
function upsertServer(s: Server) {
  const i = servers.list.findIndex((x) => x.id === s.id);
  if (i >= 0) servers.list[i] = s; else servers.list.push(s);
}

// ---------- operations ----------
export const ops = $state<{ list: Operation[]; loaded: boolean }>({ list: [], loaded: false });
export const activeOps = () => ops.list.filter((o) => o.state === 'queued' || o.state === 'running');
const opSubs = new Set<(o: Operation) => void>();
/** Subscribe to operation updates (returns unsubscribe). */
export function onOp(fn: (o: Operation) => void) { opSubs.add(fn); return () => opSubs.delete(fn); }
const seenTerminal = new Set<string>();
function upsertOp(o: Operation) {
  const i = ops.list.findIndex((x) => x.id === o.id);
  const prev = i >= 0 ? ops.list[i] : null;
  if (i >= 0) ops.list[i] = o; else ops.list.unshift(o);
  const terminal = o.state !== 'queued' && o.state !== 'running';
  if (terminal && !seenTerminal.has(o.id) && (prev || true)) {
    seenTerminal.add(o.id);
    if (prev && (prev.state === 'queued' || prev.state === 'running' || !ops.loaded)) {
      if (o.state === 'completed') toast('success', o.title, { body: 'Completed' });
      else if (o.state === 'failed') toast('error', `${o.title} failed`, { body: o.error ?? undefined, action: { label: 'Details', run: () => (ui.ops = true) } });
    }
  }
  opSubs.forEach((f) => f(o));
}
/** Track an operation returned by an action endpoint immediately. */
export function trackOp(o: Operation | undefined | null) {
  if (!o) return;
  upsertOp(o);
  ui.ops = false;
}
export async function refreshOps() {
  try {
    const l = await api.operations(50);
    l.forEach((o) => { if (o.state !== 'queued' && o.state !== 'running') seenTerminal.add(o.id); });
    ops.list = l;
  } catch { /* shown elsewhere */ }
  ops.loaded = true;
}

// ---------- activity feed (live) ----------
export const feed = $state<{ latest: Activity | null }>({ latest: null });

// ---------- live connection ----------
let stream: Stream | null = null;
let retry = 0;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let fallback: (() => void) | null = null;

function connect() {
  stream?.close();
  stream = openEvents({
    onOpen: () => { retry = 0; servers.live = true; void refreshServers(); },
    onError: () => {
      servers.live = false;
      stream?.close();
      clearTimeout(retryTimer);
      retryTimer = setTimeout(connect, Math.min(30000, 1000 * 2 ** retry++));
    },
    op: upsertOp,
    server: upsertServer,
    activity: (a) => (feed.latest = a),
  });
}
export function startLive() {
  void refreshServers();
  void refreshOps();
  connect();
  // Fallback poll only while SSE is down.
  fallback = poll(() => { if (!servers.live) void refreshServers(); }, 10000);
}
export function stopLive() {
  stream?.close();
  clearTimeout(retryTimer);
  fallback?.();
  servers.list = [];
  servers.loaded = false;
  ops.list = [];
}

// ---------- host status & global health ----------
export const host = $state<{ status: HostStatus | null; cpu: number[]; mem: number[]; error: string }>({ status: null, cpu: [], mem: [], error: '' });
export async function refreshHost() {
  try {
    const s = await api.status();
    host.status = s;
    host.cpu = [...host.cpu, s.host.cpu_pct].slice(-60);
    host.mem = [...host.mem, (s.host.mem_used / s.host.mem_total) * 100].slice(-60);
    host.error = '';
  } catch (e) { host.error = errMsg(e); }
}
export const health = $state<{ list: Finding[]; loaded: boolean; error: string }>({ list: [], loaded: false, error: '' });
export async function refreshHealth() {
  try { health.list = await api.allHealth(); health.error = ''; } catch (e) { health.error = errMsg(e); }
  health.loaded = true;
}
