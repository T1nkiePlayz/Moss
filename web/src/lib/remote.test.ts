import { describe, expect, it } from 'vitest';
import type { Backup, StagedRestore } from './api';
import { blockingFailures, canCopyToRemote, destinationProblems, remoteBadge, stagedAsPreview } from './remote';
import { summarizePreview } from './backupPreview';

const b = (o: Partial<Backup> = {}): Backup => ({ id: 'b', server_id: 's', created_at: '', size: 1, sha256: 'x', status: 'ok', verified_at: null, note: '', kind: 'manual', consistent: true, files: 1, ...o });

describe('remote badge', () => {
  it('is hidden for local-only backups', () => { expect(remoteBadge(b())).toBeNull(); expect(remoteBadge(b({ remote: 'none' }))).toBeNull(); });
  it('names the state', () => {
    expect(remoteBadge(b({ remote: 'stored', remote_destination: 'NAS' }))).toMatchObject({ tone: 'ok', label: 'Copied' });
    expect(remoteBadge(b({ remote: 'stored', local_present: false }))?.label).toBe('Remote only');
    expect(remoteBadge(b({ remote: 'uploading' }))?.tone).toBe('info');
    expect(remoteBadge(b({ remote: 'failed', remote_error: 'no space' }))).toMatchObject({ tone: 'danger', title: 'no space' });
  });
  it('offers a copy only when it makes sense', () => {
    expect(canCopyToRemote(b({ remote: 'failed' }))).toBe(true);
    expect(canCopyToRemote(b({ remote: 'stored' }))).toBe(false);
    expect(canCopyToRemote(b({ kind: 'pre-change' }))).toBe(false);
    expect(canCopyToRemote(b({ local_present: false, remote: 'none' }))).toBe(false);
  });
});

describe('destination form', () => {
  it('requires a name and an absolute folder path', () => {
    expect(Object.keys(destinationProblems({ kind: 'folder', name: '', path: 'relative' }))).toEqual(['name', 'path']);
    expect(destinationProblems({ kind: 'folder', name: 'NAS', path: '/mnt/nas' })).toEqual({});
  });
  it('requires S3 fields, and keys only when adding', () => {
    expect(Object.keys(destinationProblems({ kind: 's3', name: 'B2' }))).toEqual(['endpoint', 'bucket', 'region', 'access_key', 'secret_key']);
    expect(destinationProblems({ kind: 's3', name: 'B2', endpoint: 'https://s3.example.com', bucket: 'x', region: 'r' }, true)).toEqual({});
  });
});

describe('staged restore', () => {
  const s: StagedRestore = { id: 's1', backup_id: 'b', server_id: 'x', only: null, status: 'ready', error: null, created_at: '2026-01-01T00:00:00Z',
    diff: { archive: { files: 10, size: 100 }, current: { files: 9, size: 90 }, added: 2, removed: 1, changed: 3, unchanged: 4, size_delta: 10, samples: { added: ['a'], removed: ['r'], changed: ['c'] } } };
  it('reuses the preview summary', () => {
    const p = stagedAsPreview(s)!;
    expect(summarizePreview(p).destructive).toBe(true);
    expect(stagedAsPreview({ ...s, diff: undefined })).toBeNull();
  });
  it('counts blocking failures only', () => {
    expect(blockingFailures([{ ok: true, blocking: true }, { ok: false, blocking: false }, { ok: false, blocking: true }])).toBe(1);
  });
});
