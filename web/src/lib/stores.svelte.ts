import { api, openEvents, errMsg, type Activity, type AppNotification, type Finding, type HostStatus, type Operation, type Server, type Stream } from './api';
import { toast, toastError, ui } from './feedback.svelte';

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

export * from './prefs.svelte';
export * from './feedback.svelte';

// ---------- servers ----------
export const servers = $state<{ list: Server[]; loaded: boolean; error: string; live: boolean }>({ list: [], loaded: false, error: '', live: false });
export const serverById = (id: string) => servers.list.find((s) => s.id === id);

/** Copy changed fields of `src` onto the existing proxy so unchanged fields (notably `id`) don't
 *  re-trigger effects in open tabs. Nested values are compared structurally. */
function patch<T extends object>(dst: T, src: T) {
  const d = dst as Record<string, unknown>, s = src as Record<string, unknown>;
  for (const k of Object.keys(d)) if (!(k in s)) delete d[k];
  for (const [k, v] of Object.entries(s)) {
    const cur = d[k];
    if (cur === v) continue;
    if (v && typeof v === 'object' && cur && typeof cur === 'object' && JSON.stringify(cur) === JSON.stringify(v)) continue;
    d[k] = v;
  }
}

export async function refreshServers() {
  try {
    const next = await api.servers();
    const byId = new Map(servers.list.map((x) => [x.id, x]));
    servers.list = next.map((n) => { const o = byId.get(n.id); if (o) { patch(o, n); return o; } return n; });
    servers.error = '';
  } catch (e) { servers.error = errMsg(e); }
  servers.loaded = true;
}
function upsertServer(s: Server) {
  const i = servers.list.findIndex((x) => x.id === s.id);
  if (i >= 0) patch(servers.list[i], s); else servers.list.push(s);
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
  if (i >= 0) ops.list[i] = o; else { ops.list.unshift(o); if (ops.list.length > 200) ops.list.length = 200; }
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
export const feed = $state<{ latest: Activity | null; rev: number }>({ latest: null, rev: 0 });

// ---------- notification inbox (live via SSE `notification`) ----------
export const inbox = $state<{ items: AppNotification[]; unread: number; loaded: boolean; error: string }>({ items: [], unread: 0, loaded: false, error: '' });
export async function refreshNotifications() {
  try { const r = await api.notifications({ limit: 50 }); inbox.items = r.items; inbox.unread = r.unread; inbox.error = ''; } catch (e) { inbox.error = errMsg(e); }
  inbox.loaded = true;
}
function onNotification(n: AppNotification) {
  const had = inbox.items.some((x) => x.id === n.id);
  inbox.items = [n, ...inbox.items.filter((x) => x.id !== n.id)].slice(0, 100);
  if (had || n.read) return;
  inbox.unread++;
  if (n.severity === 'critical') toast('error', n.title, { body: n.body, timeout: 10000, action: n.server_id ? { label: 'Open', run: () => { location.hash = `#/servers/${encodeURIComponent(n.server_id!)}`; } } : undefined });
}
/** Mark some (or, without ids, all) notifications read; optimistic, reconciled on failure. */
export async function markRead(ids?: string[]) {
  const hit = inbox.items.filter((n) => !n.read && (!ids || ids.includes(n.id)));
  hit.forEach((n) => (n.read = true));
  inbox.unread = ids ? Math.max(0, inbox.unread - hit.length) : 0;
  try { await api.markRead(ids ? { ids } : { all: true }); } catch (e) { toastError(e, 'Could not mark as read'); void refreshNotifications(); }
}
/** SSE `resync`: events were dropped, so refetch every list. */
function resync() { void refreshServers(); void refreshOps(); void refreshNotifications(); feed.rev++; }

// ---------- live connection ----------
let stream: Stream | null = null;
let retry = 0;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let fallback: (() => void) | null = null;

function connect() {
  stream?.close();
  stream = openEvents({
    onOpen: () => { retry = 0; servers.live = true; void refreshServers(); void refreshOps(); void refreshNotifications(); },
    onError: () => {
      servers.live = false;
      stream?.close();
      clearTimeout(retryTimer);
      retryTimer = setTimeout(connect, Math.min(30000, 1000 * 2 ** retry++));
    },
    op: upsertOp,
    server: upsertServer,
    activity: (a) => (feed.latest = a),
    notification: onNotification,
    resync,
  });
}
export function startLive() {
  void refreshServers();
  void refreshOps();
  connect();
  // Fallback poll only while SSE is down.
  fallback = poll(() => { if (!servers.live) { void refreshServers(); void refreshNotifications(); } }, 10000);
}
export function stopLive() {
  stream?.close();
  clearTimeout(retryTimer);
  fallback?.();
  servers.list = [];
  servers.loaded = false;
  ops.list = [];
  inbox.items = [];
  inbox.unread = 0;
  inbox.loaded = false;
}

// ---------- host status & global health ----------
/** Largest memory a server may be given: the host's total memory rounded down to 256 MB (no cap until the host is known). */
export const maxMemoryMb = () => { const t = host.status?.host.mem_total; return t ? Math.max(256, Math.floor(t / 1048576 / 256) * 256) : 262144; };
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
