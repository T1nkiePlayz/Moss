// Restore-preview summarising (pure). Shapes follow POST /backups/{id}/preview.
import type { RestorePreview } from './api';
import { bytes, num } from './format';

export type GroupKey = 'added' | 'removed' | 'changed';
export interface PreviewRow { key: GroupKey | 'unchanged'; label: string; count: number; tone: 'ok' | 'danger' | 'warn' | ''; samples: string[]; more: number }
export interface PreviewSummary {
  rows: PreviewRow[]; showDelta: boolean; sizeDelta: string; sizeTone: 'ok' | 'danger' | ''; headline: string; destructive: boolean; noChange: boolean;
}
export const signedBytes = (n: number) => (n === 0 ? '±0 B' : `${n > 0 ? '+' : '−'}${bytes(Math.abs(n))}`);

export function summarizePreview(p: RestorePreview): PreviewSummary {
  const row = (key: GroupKey, label: string, tone: PreviewRow['tone']): PreviewRow => {
    const samples = (p.samples?.[key] ?? []).slice(0, 50);
    return { key, label, count: p[key], tone, samples, more: Math.max(0, p[key] - samples.length) };
  };
  const rows: PreviewRow[] = p.mode === 'in_place'
    ? [row('added', 'Files restored that are missing now', 'ok'), row('changed', 'Files overwritten', 'warn'), row('removed', 'Files deleted (not in the backup)', 'danger'),
       { key: 'unchanged', label: 'Files already identical', count: p.unchanged, tone: '', samples: [], more: 0 }]
    : [row('added', 'Files in the new server', 'ok')];
  const inPlace = p.mode === 'in_place';
  const destructive = inPlace && (p.removed > 0 || p.changed > 0);
  const noChange = inPlace && p.added + p.removed + p.changed === 0;
  const headline = !inPlace ? `Creates a new server with ${num(p.archive.files)} files (${bytes(p.archive.size)}). The current server is untouched.`
    : noChange ? 'The server already matches this backup. Nothing would change.'
    : `${num(p.changed + p.removed)} existing file${p.changed + p.removed === 1 ? '' : 's'} will be replaced or deleted, ${num(p.added)} added. Size ${signedBytes(p.size_delta)}.`;
  return { rows, showDelta: inPlace, sizeDelta: signedBytes(p.size_delta), sizeTone: p.size_delta < 0 ? 'danger' : p.size_delta > 0 ? 'ok' : '', headline, destructive, noChange };
}

/** Top-level worlds to send as `only`; undefined when everything is selected (restore all). */
export function onlyParam(all: string[], selected: string[]): string[] | undefined {
  const sel = all.filter((w) => selected.includes(w));
  return sel.length === all.length ? undefined : sel;
}

/** A pre-change backup is automatic; manual/scheduled ones are not. Locked backups are never deleted. */
export const isPreChange = (b: { kind: string }) => b.kind === 'pre-change';
export const canDeleteBackup = (b: { locked?: boolean }) => !b.locked;
export const deleteHint = (b: { locked?: boolean }) => (b.locked ? 'Locked backups can\'t be deleted. Unlock it first.' : 'Delete backup');
