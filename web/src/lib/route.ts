// Pure hash-route parsing/matching (reactive wrapper lives in router.svelte.ts).
export type RouteName =
  | 'overview' | 'servers' | 'new' | 'server' | 'backups' | 'activity' | 'users' | 'settings' | 'notfound';

export interface Route {
  name: RouteName;
  params: Record<string, string>;
  path: string;
  query: URLSearchParams;
}

export function parseHash(hash: string): Route {
  let h = hash.replace(/^#/, '');
  if (!h.startsWith('/')) h = '/' + h;
  const [pathRaw, q = ''] = h.split('?');
  const path = pathRaw.replace(/\/+$/, '') || '/';
  const query = new URLSearchParams(q);
  const seg = path.split('/').filter(Boolean).map(decodeURIComponent);
  const r = (name: RouteName, params: Record<string, string> = {}): Route => ({ name, params, path, query });
  if (seg.length === 0) return r('overview');
  const [a, b, c] = seg;
  if (a === 'servers') {
    if (seg.length === 1) return r('servers');
    if (b === 'new' && seg.length === 2) return r('new');
    if (seg.length <= 3) return r('server', { id: b, tab: c ?? 'overview' });
  }
  if (seg.length === 1 && ['backups', 'activity', 'users', 'settings'].includes(a)) return r(a as RouteName);
  return r('notfound');
}

export const serverHref = (id: string, tab = 'overview') =>
  `#/servers/${encodeURIComponent(id)}${tab === 'overview' ? '' : '/' + tab}`;
