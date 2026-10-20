// Pure helpers for remote backup destinations (badges, form checks, staged-restore summary).
import type { Backup, DestinationInput, RestorePreview, StagedRestore } from './api';

export type Tone = 'ok' | 'warn' | 'danger' | 'info' | '';
export interface RemoteBadge { tone: Tone; label: string; title: string }

/** What the Backups table shows for a backup's remote copy; null when it was never meant to be copied. */
export function remoteBadge(b: Backup): RemoteBadge | null {
  const where = b.remote_destination ? ` on ${b.remote_destination}` : '';
  switch (b.remote) {
    case 'stored': return { tone: 'ok', label: b.local_present === false ? 'Remote only' : 'Copied', title: `A verified copy is stored${where}.` };
    case 'uploading': return { tone: 'info', label: 'Copying…', title: `Being copied${where}.` };
    case 'failed': return { tone: 'danger', label: 'Copy failed', title: b.remote_error || 'The copy to the destination failed.' };
    default: return null;
  }
}
export const isRemoteOnly = (b: Backup) => b.local_present === false;
export const canCopyToRemote = (b: Backup) => b.status === 'ok' && b.kind !== 'pre-change' && b.local_present !== false && (b.remote === 'none' || b.remote === 'failed' || b.remote === undefined);

export type Check = { ok: boolean; blocking: boolean };
export const blockingFailures = (cs: Check[] = []) => cs.filter((c) => !c.ok && c.blocking).length;

/** The staged diff in the shape the restore preview summary understands. */
export function stagedAsPreview(s: StagedRestore): RestorePreview | null {
  const d = s.diff;
  if (!d) return null;
  return { mode: 'in_place', archive: { ...d.archive, created_at: s.created_at }, current: d.current, added: d.added, removed: d.removed, changed: d.changed, unchanged: d.unchanged, size_delta: d.size_delta, samples: d.samples, worlds: s.only ?? [], warnings: s.warnings ?? [] };
}

/** Quick client-side hints; the server validates everything again. Returns field -> message. */
export function destinationProblems(f: DestinationInput, editing = false): Record<string, string> {
  const p: Record<string, string> = {};
  if (!f.name?.trim()) p.name = 'Give it a name.';
  if (f.kind === 'folder') {
    if (!f.path?.startsWith('/') && !/^[A-Za-z]:[\\/]/.test(f.path ?? '')) p.path = 'Use the full path, for example /mnt/nas/moss.';
  } else {
    const ep = (f.endpoint ?? '').trim();
    if (!/^https?:\/\/[^/\s]+/i.test(ep)) p.endpoint = 'Enter the service address, for example https://s3.us-east-1.amazonaws.com.';
    if (!f.bucket?.trim()) p.bucket = 'Enter the bucket name.';
    if (!f.region?.trim()) p.region = 'Enter the region, for example us-east-1.';
    if (!editing && !f.access_key?.trim()) p.access_key = 'Required.';
    if (!editing && !f.secret_key?.trim()) p.secret_key = 'Required.';
  }
  return p;
}

export const keepText = (n: number, what: string) => (n <= 0 ? `Keep every ${what}` : `Keep the newest ${n}`);
