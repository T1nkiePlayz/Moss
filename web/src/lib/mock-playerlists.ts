// Dev-only mock of shared player lists. Imported by mock.ts only.
import type * as A from './api-extra';
import { ApiError } from './api';

export const NONE = Symbol('not a player list route');
const bad = (field: string, message: string) => new ApiError(422, 'validation', message, [{ path: field, message }]);
const lists: A.PlayerList[] = [{ id: 'pl-friends', name: 'Friends', kind: 'allow', entries: 2, servers: 0 }, { id: 'pl-griefers', name: 'Griefers', kind: 'ban', entries: 1, servers: 0 }];
const entries = new Map<string, A.PlayerListEntry[]>([
  ['pl-friends', ['Alice', 'Bob'].map((name) => ({ name, uuid: null, reason: '', added_by: 'admin@example.com', added_at: '2026-01-05T10:00:00Z' }))],
  ['pl-griefers', [{ name: 'Grief', uuid: null, reason: 'griefing', added_by: 'admin@example.com', added_at: '2026-01-05T10:00:00Z' }]],
]);
const subs = new Map<string, string[]>();

export async function handlePlayerLists(c: { uid: () => string; now: () => string }, method: string, p: string, b: Record<string, any>): Promise<unknown> {
  let m: RegExpMatchArray | null = null;
  const M = (re: RegExp) => (m = p.match(re));
  const find = (id: string) => { const l = lists.find((x) => x.id === id); if (!l) throw new ApiError(404, 'not_found', 'player list not found'); return l; };
  const sync = () => { for (const l of lists) { l.entries = entries.get(l.id)?.length ?? 0; l.servers = [...subs.values()].filter((s) => s.includes(l.id)).length; } };
  if (M(/^\/servers\/([^/]+)\/player-lists$/)) {
    const id = m![1];
    if (method === 'PUT') { subs.set(id, b.list_ids ?? []); return { list_ids: b.list_ids ?? [] }; }
    sync();
    const mine = subs.get(id) ?? [];
    const r: A.ServerPlayerLists = { lists: lists.map((l) => ({ id: l.id, name: l.name, kind: l.kind, entries: l.entries, subscribed: mine.includes(l.id) })), support: { allow: true, ban: true, note: null }, pending: { allow: 0, ban: 0 }, allowlist_enabled: false };
    return r;
  }
  if (!p.startsWith('/player-lists')) return NONE;
  if (p === '/player-lists') {
    if (method === 'GET') { sync(); return { items: lists }; }
    if (!(b.name ?? '').trim()) throw bad('name', 'Give the list a name of up to 60 characters.');
    if (b.kind !== 'allow' && b.kind !== 'ban') throw bad('kind', 'Choose allow (a whitelist) or ban.');
    const l: A.PlayerList = { id: c.uid(), name: b.name.trim(), kind: b.kind, entries: 0, servers: 0 };
    lists.push(l); entries.set(l.id, []);
    return l;
  }
  if (M(/^\/player-lists\/([^/]+)$/)) {
    const l = find(m![1]);
    if (method === 'PATCH') { l.name = (b.name ?? '').trim() || l.name; return l; }
    lists.splice(lists.indexOf(l), 1); entries.delete(l.id); for (const [k, v] of subs) subs.set(k, v.filter((x) => x !== l.id));
    return;
  }
  if (M(/^\/player-lists\/([^/]+)\/entries$/)) {
    const l = find(m![1]), list = entries.get(l.id)!;
    if (method === 'GET') return { items: list };
    const name = (b.name ?? '').trim();
    if (!name) throw bad('name', 'Enter a player name of up to 64 characters without quotes or control characters.');
    if (list.some((x) => x.name.toLowerCase() === name.toLowerCase())) throw new ApiError(409, 'player.exists', 'That player is already on the list.');
    const e: A.PlayerListEntry = { name, uuid: null, reason: l.kind === 'ban' ? (b.reason ?? '') : '', added_by: 'admin@example.com', added_at: c.now() };
    list.push(e);
    return e;
  }
  if (M(/^\/player-lists\/([^/]+)\/entries\/([^/]+)$/)) {
    const list = entries.get(find(m![1]).id)!, i = list.findIndex((x) => x.name.toLowerCase() === decodeURIComponent(m![2]).toLowerCase());
    if (i < 0) throw new ApiError(404, 'not_found', 'player not found');
    list.splice(i, 1);
    return;
  }
  return NONE;
}
