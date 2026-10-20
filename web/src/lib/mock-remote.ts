// Dev-only mock of remote backup destinations and staged (compare-then-promote) restores. Imported by mock.ts only.
import type * as A from './api';
import { ApiError } from './api';

export interface RCtx {
  uid: () => string; now: () => string;
  op: (kind: string, title: string, server_id: string, stages: string[], opts?: { fail?: boolean; done?: () => void; indeterminate?: boolean }) => A.Operation;
  backups: A.Backup[]; servers: A.Server[];
}
export const NONE = Symbol('not remote');
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const bad = (field: string, message: string) => new ApiError(422, 'validation', message, [{ path: field, message }]);

const dests: A.BackupDestination[] = [
  { id: 'dest-nas', name: 'Home NAS', kind: 'folder', path: '/mnt/nas/moss', is_default: true, keep_local: 2, keep_remote: 14, created_at: '2026-01-02T10:00:00Z' },
];
const overrides = new Map<string, string>();
const staged = new Map<string, A.StagedRestore & { live: boolean }>();
type Status = A.StagedRestore['status'];

/** Give the seeded backups a realistic remote state (once). */
export function seedRemote(backups: A.Backup[]) {
  backups.forEach((b, i) => {
    if (b.remote) return;
    b.remote = i < 3 ? 'stored' : i === 3 ? 'failed' : 'none';
    b.local_present = i !== 2;
    if (b.remote === 'stored') { b.remote_destination = 'Home NAS'; b.remote_at = b.created_at; }
    if (b.remote === 'failed') b.remote_error = 'The folder /mnt/nas/moss is not available; is the disk mounted?';
  });
}
const view = (d: A.BackupDestination, backups: A.Backup[]) => ({ ...d, stored_backups: backups.filter((b) => b.remote === 'stored' && b.remote_destination === d.name).length });
const find = (id: string) => dests.find((d) => d.id === id) ?? (() => { throw new ApiError(404, 'not_found', 'destination not found'); })();

function check(b: A.DestinationInput, editing = false) {
  const name = (b.name ?? '').trim();
  if (!name) throw bad('name', 'Give the destination a name.');
  if (b.kind === 'folder') {
    if (!(b.path ?? '').startsWith('/')) throw bad('path', 'Use the full path of an existing folder.');
  } else {
    if (!/^https:\/\//i.test(b.endpoint ?? '')) throw bad('endpoint', 'The address must start with https://.');
    if (!(b.bucket ?? '').trim()) throw bad('bucket', 'Enter the bucket name.');
    if (!editing && (!b.access_key || !b.secret_key)) throw bad('access_key', 'Enter both the access key and the secret key.');
  }
}
const probe = async (b: A.DestinationInput): Promise<A.DestinationTest> => {
  await sleep(700);
  const down = /fail|down/i.test(`${b.endpoint ?? ''}${b.path ?? ''}`);
  const steps = [{ step: 'Write a small file', ok: !down, detail: down ? 'could not reach the destination (mock)' : '12 bytes' }];
  if (!down) steps.push({ step: 'Read it back', ok: true, detail: 'checksum matches' }, { step: 'List the folder', ok: true, detail: 'found it' }, { step: 'Remove the test file', ok: true, detail: 'done' });
  return { ok: !down, steps };
};
const effective = (sid: string) => { const o = overrides.get(sid); return o === '' ? null : (o ? dests.find((d) => d.id === o) : dests.find((d) => d.is_default)) ?? null; };
const setting = (sid: string): A.ServerDestination => {
  const o = overrides.get(sid), e = effective(sid);
  return { destination_id: o === undefined ? 'default' : o === '' ? 'none' : o, effective: e && { id: e.id, name: e.name, kind: e.kind } };
};

export async function handleRemote(c: RCtx, method: string, p: string, b: Record<string, any>): Promise<unknown> {
  let m: RegExpMatchArray | null;
  const M = (re: RegExp) => (m = p.match(re));
  const bk = (id: string) => c.backups.find((x) => x.id === id) ?? (() => { throw new ApiError(404, 'backup.not_found', 'Backup not found'); })();
  if (p === '/backup-destinations' && method === 'GET') return dests.map((d) => view(d, c.backups));
  if (p === '/backup-destinations' && method === 'POST') {
    check(b);
    const d: A.BackupDestination = { id: c.uid(), name: String(b.name).trim(), kind: b.kind, is_default: b.is_default ?? dests.length === 0, keep_local: b.keep_local ?? 0, keep_remote: b.keep_remote ?? 14, created_at: c.now(),
      ...(b.kind === 'folder' ? { path: b.path } : { endpoint: b.endpoint, region: b.region ?? '', bucket: b.bucket, prefix: b.prefix ?? '', path_style: !!b.path_style, access_key: 'set', secret_key: 'set' }) };
    if (d.is_default) dests.forEach((x) => (x.is_default = false));
    dests.push(d);
    return view(d, c.backups);
  }
  if (p === '/backup-destinations/test') { check(b); return probe(b); }
  if (M(/^\/backup-destinations\/([^/]+)\/test$/)) { const d = find(m![1]); return probe(d); }
  if (M(/^\/backup-destinations\/([^/]+)$/)) {
    const d = find(m![1]);
    if (method === 'PATCH') {
      const next = { ...d, ...Object.fromEntries(Object.entries(b).filter(([k, v]) => v !== undefined && k !== 'access_key' && k !== 'secret_key')) } as A.BackupDestination;
      check(next, true);
      if (b.access_key || b.secret_key) { next.access_key = 'set'; next.secret_key = 'set'; }
      if (next.is_default) dests.forEach((x) => (x.is_default = false));
      Object.assign(d, next);
      return view(d, c.backups);
    }
    const stored = c.backups.filter((x) => x.remote === 'stored' && x.remote_destination === d.name);
    if (stored.length && b.force !== true) {
      throw new ApiError(409, 'destination.in_use', `${stored.length} backup(s) are stored on ${d.name}. Removing it keeps the files there but Moss forgets them (backups that exist only there disappear from the list). Confirm to continue.`);
    }
    for (const x of stored) { if (x.local_present === false) c.backups.splice(c.backups.indexOf(x), 1); else { x.remote = 'none'; x.remote_destination = null; } }
    dests.splice(dests.indexOf(d), 1);
    return;
  }
  if (M(/^\/servers\/([^/]+)\/backup-destination$/)) {
    if (method === 'GET') return setting(m![1]);
    const v = String(b.destination_id);
    if (v === 'default') overrides.delete(m![1]); else if (v === 'none') overrides.set(m![1], ''); else { find(v); overrides.set(m![1], v); }
    return setting(m![1]);
  }
  if (M(/^\/backups\/([^/]+)\/upload$/)) {
    const k = bk(m![1]);
    const d = effective(k.server_id);
    if (!d) throw new ApiError(409, 'backup.no_destination', 'This server has no backup destination; add one in Settings, Backups.');
    k.remote = 'uploading'; k.remote_error = null; k.remote_destination = d.name;
    return { operation: c.op('backup.upload', `Copy backup to ${d.name}`, k.server_id, ['Reading archive', 'Uploading', 'Verifying the copy'], { done: () => { k.remote = 'stored'; k.remote_at = c.now(); } }) };
  }
  if (M(/^\/backups\/([^/]+)\/fetch$/)) {
    const k = bk(m![1]);
    return { operation: c.op('backup.fetch', 'Download backup from remote', k.server_id, ['Downloading', 'Checking the SHA-256'], { done: () => { k.local_present = true; } }) };
  }
  if (M(/^\/backups\/([^/]+)\/preview$/)) {
    if (bk(m![1]).local_present === false) throw new ApiError(409, 'backup.remote_only', 'This backup is only on the remote destination. Download it first to preview it.');
    return NONE;
  }
  if (M(/^\/backups\/([^/]+)\/restore$/) && b.staged === true) {
    const k = bk(m![1]);
    if (b.mode !== 'in_place') throw bad('staged', 'Staged restores are for in-place restores.');
    const s = { id: c.uid(), backup_id: k.id, server_id: k.server_id, only: b.only ?? null, status: 'preparing' as Status, error: null, created_at: c.now(), live: false };
    staged.set(s.id, s);
    const operation = c.op('backup.stage', 'Preparing restore', k.server_id, [...(k.local_present === false ? ['Downloading from remote'] : []), 'Extracting to a staging folder', 'Checking the staged files'], { done: () => { s.status = 'ready'; } });
    return { operation, staged_id: s.id };
  }
  if (M(/^\/servers\/([^/]+)\/staged-restores$/)) return [...staged.values()].filter((s) => s.server_id === m![1]).map(({ live: _l, ...s }) => s);
  if (M(/^\/staged-restores\/([^/]+)(\/promote)?$/)) {
    const s = staged.get(m![1]);
    if (!s) throw new ApiError(404, 'not_found', 'staged restore not found');
    if (m![2]) {
      const srv = c.servers.find((x) => x.id === s.server_id);
      if (srv && ['running', 'starting', 'stopping'].includes(srv.state)) throw new ApiError(409, 'server.running', 'Stop the server before promoting.');
      s.status = 'promoting';
      return { operation: c.op('backup.promote', 'Promoting staged restore', s.server_id, ['Safety backup', 'Checking the staged files', 'Swapping folders'], { done: () => { s.status = 'promoted'; } }) };
    }
    if (method === 'DELETE') { staged.delete(s.id); return; }
    const { live: _l, ...v } = s;
    if (s.status !== 'ready') return v;
    const k = bk(s.backup_id), srv = c.servers.find((x) => x.id === s.server_id);
    const running = !!srv && ['running', 'starting', 'stopping'].includes(srv.state);
    const checks = [{ id: 'launch_files', ok: true, blocking: true, message: 'The server jar and launch files are present.' },
      ...(k.worlds ?? ['world']).filter((w) => !s.only || s.only.includes(w)).map((w) => ({ id: `world:${w}`, ok: true, blocking: true, message: `${w} has a level.dat.` }))];
    const share = s.only?.length ? s.only.length / (k.worlds?.length ?? 1) : 1;
    const added = Math.round(12 * share), removed = Math.round(340 * share), changed = Math.round(1204 * share);
    return { ...v, can_promote: !running, checks,
      diff: { archive: { files: k.files, size: k.size }, current: { files: k.files - added + removed, size: k.size * 1.07 }, added, removed, changed, unchanged: Math.max(0, k.files - added - changed), size_delta: -Math.round(k.size * 0.07 * share),
        samples: { added: ['world/region/r.0.0.mca'], removed: ['world/region/r.9.9.mca'], changed: ['world/level.dat', 'plugins/Essentials/config.yml'] } },
      warnings: ['A safety backup of the current state is taken first, and the replaced files are kept under data/removed.', ...(running ? ['The server is running; stop it before promoting.'] : [])] };
  }
  return NONE;
}
