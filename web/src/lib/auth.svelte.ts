// Supabase GoTrue via plain fetch; the session lives in localStorage (best effort).
import { api, ApiError, configureAuth, type AuthConfig, type Me } from './api';

interface Session { access_token: string; refresh_token: string; expires_at: number; email?: string }
const KEY = 'moss.session';

export const auth = $state<{
  phase: 'loading' | 'login' | 'ready' | 'denied' | 'error';
  config: AuthConfig | null;
  me: Me | null;
  email: string;
  error: string;
  notice: string;
  busy: boolean;
}>({ phase: 'loading', config: null, me: null, email: '', error: '', notice: '', busy: false });

let session: Session | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let refreshing: Promise<boolean> | null = null;

const load = (): Session | null => { try { return JSON.parse(localStorage.getItem(KEY) || 'null'); } catch { return null; } };
function save(s: Session | null) {
  session = s;
  try { s ? localStorage.setItem(KEY, JSON.stringify(s)) : localStorage.removeItem(KEY); } catch { /* storage unavailable */ }
  clearTimeout(timer);
  if (s) timer = setTimeout(() => void refresh(), Math.max(5000, s.expires_at * 1000 - Date.now() - 60000));
}

const cfg = () => auth.config!;
async function gotrue(path: string, body: unknown, query = '') {
  const res = await fetch(`${cfg().supabase_url}/auth/v1/${path}${query}`, {
    method: 'POST',
    headers: { apikey: cfg().supabase_anon_key ?? '', 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const j = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(j.error_description || j.msg || j.message || `Sign-in failed (${res.status})`);
  return j;
}
function toSession(j: any): Session {
  return {
    access_token: j.access_token,
    refresh_token: j.refresh_token,
    expires_at: j.expires_at ?? Math.floor(Date.now() / 1000) + (j.expires_in ?? 3600),
    email: j.user?.email,
  };
}

async function refresh(): Promise<boolean> {
  if (!session) return false;
  refreshing ??= gotrue('token', { refresh_token: session.refresh_token }, '?grant_type=refresh_token')
    .then((j) => { save(toSession(j)); return true; })
    .catch(() => { save(null); return false; })
    .finally(() => { refreshing = null; });
  return refreshing;
}

async function token(): Promise<string | null> {
  if (auth.config?.mode !== 'supabase' || !session) return null;
  if (session.expires_at * 1000 - Date.now() < 30000) await refresh();
  return session?.access_token ?? null;
}

function consumeHash(): Session | null {
  const h = location.hash;
  if (!h.includes('access_token=')) return null;
  const p = new URLSearchParams(h.replace(/^#\/?/, ''));
  const at = p.get('access_token'), rt = p.get('refresh_token');
  history.replaceState(null, '', location.pathname + location.search + '#/');
  if (!at || !rt) return null;
  const exp = parseInt(p.get('expires_at') || '0') || Math.floor(Date.now() / 1000) + (parseInt(p.get('expires_in') || '3600') || 3600);
  let email: string | undefined;
  try { email = JSON.parse(atob(at.split('.')[1].replace(/-/g, '+').replace(/_/g, '/'))).email; } catch { /* opaque token */ }
  return { access_token: at, refresh_token: rt, expires_at: exp, email };
}

function onUnauthorized(e: ApiError) {
  if (e.code === 'auth.not_provisioned') { auth.phase = 'denied'; return; }
  if (auth.config?.mode === 'supabase' && auth.phase !== 'login') {
    void refresh().then((ok) => { if (!ok) { auth.phase = 'login'; auth.me = null; } });
  }
}

async function loadMe() {
  try {
    auth.me = await api.me();
    auth.email = auth.me.email;
    auth.phase = 'ready';
  } catch (e) {
    if (e instanceof ApiError && e.code === 'auth.not_provisioned') { auth.email = session?.email ?? ''; auth.phase = 'denied'; }
    else if (e instanceof ApiError && e.status === 401) { auth.phase = auth.config?.mode === 'supabase' ? 'login' : 'error'; auth.error = e.message; }
    else { auth.phase = 'error'; auth.error = e instanceof Error ? e.message : String(e); }
  }
}

export async function boot() {
  configureAuth(token, onUnauthorized);
  auth.phase = 'loading';
  try {
    auth.config = await api.authConfig();
  } catch (e) {
    auth.phase = 'error';
    auth.error = e instanceof Error ? e.message : 'Cannot reach the Moss backend';
    return;
  }
  if (auth.config.mode === 'dev') return loadMe();
  const fromHash = consumeHash();
  save(fromHash ?? load());
  if (!session) { auth.phase = 'login'; return; }
  if (session.expires_at * 1000 - Date.now() < 30000 && !(await refresh())) { auth.phase = 'login'; return; }
  await loadMe();
}

async function run(f: () => Promise<void>) {
  auth.busy = true; auth.error = ''; auth.notice = '';
  try { await f(); } catch (e) { auth.error = e instanceof Error ? e.message : String(e); }
  auth.busy = false;
}
export const signIn = (email: string, password: string) =>
  run(async () => {
    save(toSession(await gotrue('token', { email, password }, '?grant_type=password')));
    await loadMe();
  });
export const signUp = (email: string, password: string) =>
  run(async () => {
    const j = await gotrue('signup', { email, password });
    if (j.access_token) { save(toSession(j)); await loadMe(); }
    else auth.notice = 'Check your inbox to confirm your email, then sign in.';
  });
export function oauth(provider: 'github' | 'google') {
  location.href = `${cfg().supabase_url}/auth/v1/authorize?provider=${provider}&redirect_to=${encodeURIComponent(location.origin)}`;
}
export function signOut() {
  const at = session?.access_token;
  if (at && auth.config?.supabase_url) {
    fetch(`${auth.config.supabase_url}/auth/v1/logout`, { method: 'POST', headers: { apikey: auth.config.supabase_anon_key ?? '', Authorization: `Bearer ${at}` } }).catch(() => {});
  }
  save(null);
  auth.me = null;
  auth.phase = auth.config?.mode === 'supabase' ? 'login' : 'loading';
  auth.error = '';
}

export const can = (perm: string) => auth.me?.role === 'owner' || auth.me?.permissions.includes(perm) || false;
export const isAdmin = () => auth.me?.role === 'owner' || auth.me?.role === 'admin';
