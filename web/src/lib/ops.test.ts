import { describe, expect, it } from 'vitest';
import type { AppNotification, MetricSample, RestorePreview } from './api';
import { badgeText, dayLabel, groupNotifications, upsertNotification, worst } from './notify';
import { canDeleteBackup, deleteHint, isPreChange, onlyParam, signedBytes, summarizePreview } from './backupPreview';
import { encodeQr, qrPath } from './qr';
import { groupSecret, isCode, normalizeCode, otpauthUri } from './totp';
import { needsFallback, summarize, tickLabel } from './metrics';
import { expiryStatus } from './tokens';
import { describeLimits, limitWarnings } from './limits';

const NOW = new Date(2026, 9, 20, 15, 0, 0).getTime();
const n = (id: string, minsAgo: number, o: Partial<AppNotification> = {}): AppNotification => ({ id, time: new Date(NOW - minsAgo * 60000).toISOString(), kind: 'crash', severity: 'info', title: id, body: '', server_id: null, read: false, ...o });

describe('notifications', () => {
  it('groups by day, newest first, unread critical first within a day', () => {
    const g = groupNotifications([n('old', 60 * 24 * 3), n('a', 10, { read: true }), n('b', 20, { severity: 'critical' }), n('c', 30, { severity: 'warn' }), n('y', 60 * 20)], NOW);
    expect(g.map((x) => x.label)).toEqual(['Today', 'Yesterday', expect.any(String)]);
    expect(g[0].items.map((x) => x.id)).toEqual(['b', 'c', 'a']);
    expect(g[1].items.map((x) => x.id)).toEqual(['y']);
  });
  it('labels days and badges', () => {
    expect(dayLabel(NOW - 1000, NOW)).toBe('Today');
    expect(dayLabel(NOW - 86400000, NOW)).toBe('Yesterday');
    expect(badgeText(3)).toBe('3');
    expect(badgeText(120)).toBe('99+');
    expect(groupNotifications([], NOW)).toEqual([]);
  });
  it('worst severity and upsert', () => {
    expect(worst([n('a', 1), n('b', 1, { severity: 'warn' })])).toBe('warn');
    expect(worst([])).toBe('info');
    const l = upsertNotification([n('a', 10)], n('b', 5));
    expect(l.map((x) => x.id)).toEqual(['b', 'a']);
    expect(upsertNotification(l, n('a', 1, { read: true })).map((x) => x.id)).toEqual(['a', 'b']);
    expect(upsertNotification(l, n('a', 1, { read: true }))[0].read).toBe(true);
  });
});

const pv = (o: Partial<RestorePreview> = {}): RestorePreview => ({
  mode: 'in_place', archive: { files: 100, size: 1e9, created_at: '' }, current: { files: 110, size: 1.1e9 }, added: 2, removed: 12, changed: 30, unchanged: 68, size_delta: -1e8,
  samples: { added: ['a'], removed: ['b'], changed: Array.from({ length: 60 }, (_, i) => 'f' + i) }, worlds: ['world'], warnings: [], ...o,
});
describe('backup preview', () => {
  it('summarises an in-place restore', () => {
    const s = summarizePreview(pv());
    expect(s.destructive).toBe(true);
    expect(s.headline).toContain('42 existing files');
    expect(s.sizeDelta).toBe('−95.4 MB');
    expect(s.sizeTone).toBe('danger');
    const changed = s.rows.find((r) => r.key === 'changed')!;
    expect(changed.samples).toHaveLength(50);
    expect(changed.more).toBe(0);
  });
  it('reports samples that were truncated', () => {
    const r = summarizePreview(pv({ changed: 1204 })).rows.find((x) => x.key === 'changed')!;
    expect(r.samples).toHaveLength(50);
    expect(r.more).toBe(1154);
  });
  it('nothing to change and new-server modes', () => {
    expect(summarizePreview(pv({ added: 0, removed: 0, changed: 0 })).noChange).toBe(true);
    const ns = summarizePreview(pv({ mode: 'new_server', current: null, added: 100, unchanged: 0, removed: 0, changed: 0, size_delta: 0 }));
    expect(ns.destructive).toBe(false);
    expect(ns.headline).toContain('new server');
    expect(ns.rows.map((r) => r.key)).toEqual(['added']);
    expect(ns.showDelta).toBe(false);
  });
  it('signed bytes and world selection', () => {
    expect(signedBytes(0)).toBe('±0 B');
    expect(signedBytes(2048)).toBe('+2.0 KB');
    expect(onlyParam(['world', 'world_nether'], ['world', 'world_nether'])).toBeUndefined();
    expect(onlyParam(['world', 'world_nether'], ['world_nether'])).toEqual(['world_nether']);
    expect(onlyParam(['world', 'world_nether'], [])).toEqual([]);
  });
  it('pre-change and locked rules', () => {
    expect(isPreChange({ kind: 'pre-change' })).toBe(true);
    expect(isPreChange({ kind: 'manual' })).toBe(false);
    expect(canDeleteBackup({ locked: true })).toBe(false);
    expect(canDeleteBackup({})).toBe(true);
    expect(deleteHint({ locked: true })).toMatch(/Unlock/);
  });
});

describe('QR and TOTP', () => {
  const uri = otpauthUri({ issuer: 'Moss', account: 'dev@moss.local', secret: 'JBSW Y3DP EHPK 3PXP' });
  it('builds the key URI', () => {
    expect(uri).toBe('otpauth://totp/Moss:dev%40moss.local?secret=JBSWY3DPEHPK3PXP&issuer=Moss&algorithm=SHA1&digits=6&period=30');
    expect(groupSecret('JBSWY3DPEHPK3PXP')).toBe('JBSW Y3DP EHPK 3PXP');
  });
  it('normalises codes', () => {
    expect(normalizeCode('123 456 789')).toBe('123456');
    expect(isCode('123456')).toBe(true);
    expect(isCode('12345')).toBe(false);
  });
  it('encodes to a valid-sized, deterministic matrix with finder patterns', () => {
    const m = encodeQr(uri);
    expect(m.length).toBe(m[0].length);
    expect((m.length - 17) % 4).toBe(0);
    for (const [x, y] of [[0, 0], [m.length - 7, 0], [0, m.length - 7]]) {
      expect(m[y][x] && m[y][x + 6] && m[y + 6][x] && m[y + 6][x + 6] && m[y + 3][x + 3]).toBe(true);
      expect(m[y + 1][x + 1]).toBe(false);
    }
    expect(JSON.stringify(encodeQr(uri))).toBe(JSON.stringify(m));
    expect(encodeQr('hi').length).toBe(21);
    expect(() => encodeQr('x'.repeat(300))).toThrow();
  });
  it('emits an SVG path with a quiet zone', () => {
    const { d, size } = qrPath(encodeQr('hi'), 4);
    expect(size).toBe(29);
    expect(d.startsWith('M4 4h7')).toBe(true);
  });
});

describe('metrics, tokens, limits', () => {
  const s = (cpu_avg: number, cpu_max: number): MetricSample => ({ t: 0, cpu_avg, cpu_max, mem_avg: 0, mem_max: 0 });
  it('summarises min/avg/max', () => {
    expect(summarize([s(10, 30), s(20, 90), s(30, 50)], 'cpu_avg', 'cpu_max')).toEqual({ min: 10, avg: 20, max: 90 });
    expect(summarize([], 'cpu_avg')).toBeNull();
    expect(needsFallback([s(1, 1), s(1, 1)])).toBe(true);
    expect(needsFallback([s(1, 1), s(1, 1), s(1, 1)])).toBe(false);
    expect(tickLabel(Date.UTC(2026, 0, 1, 12), '30d')).toMatch(/\d/);
  });
  it('token expiry', () => {
    expect(expiryStatus(null).text).toBe('Never expires');
    expect(expiryStatus(new Date(NOW - 1000).toISOString(), NOW)).toMatchObject({ expired: true, tone: 'danger' });
    expect(expiryStatus(new Date(NOW + 3 * 86400000).toISOString(), NOW)).toMatchObject({ text: 'Expires in 3 days', tone: 'warn' });
    expect(expiryStatus(new Date(NOW + 60 * 86400000).toISOString(), NOW).tone).toBe('');
  });
  it('limit explanations', () => {
    const l = { memory_mb: null, cpu_pct: null, pids: null, enforced: false, backend: 'none' as const, detail: null };
    expect(describeLimits(l, 'macOS').reason).toMatch(/macOS has no cgroups/);
    expect(describeLimits(l, 'Linux').reason).toMatch(/Delegate=yes/);
    expect(describeLimits({ ...l, enforced: true, backend: 'cgroup2' }, 'Linux').badge).toBe('Enforced (cgroup v2)');
    expect(limitWarnings({ memory: '2048', memoryHeap: 4096 })).toHaveLength(1);
    expect(limitWarnings({ memory: '', memoryHeap: 4096 })).toHaveLength(0);
  });
});
