import { describe, expect, it } from 'vitest';
// Hangar provider labelling lives at the bottom of this file.
import type { ContentFinding, PackDiff, StagingRun } from './api';
import {
  compareRelease, contentCounts, countFindings, criticalConflicts, findBackupId, groupFindings, isDowngrade, modpackFileProblem,
  normalizeItem, packChangeText, packDiffCounts, pageUrl, parseModpackRef, sortFindings, sortVersions, sourceBadge, stagingActive, stagingSummary, versionDiff, visibleVersions, worstSeverity, planCounts, removalConflicts,
} from './content';

const f = (id: string, severity: ContentFinding['severity']): ContentFinding => ({ id, severity, item: null, title: id, detail: '', evidence: [], action: null });

describe('findings', () => {
  const list = [f('a', 'info'), f('b', 'critical'), f('c', 'warn'), f('d', 'critical')];
  it('sorts most severe first and keeps order within a severity', () => {
    expect(sortFindings(list).map((x) => x.id)).toEqual(['b', 'd', 'c', 'a']);
  });
  it('groups only non-empty severities', () => {
    expect(groupFindings([f('a', 'info'), f('b', 'critical')]).map((g) => g.severity)).toEqual(['critical', 'info']);
    expect(groupFindings([])).toEqual([]);
  });
  it('counts and finds the worst severity', () => {
    expect(countFindings(list)).toEqual({ critical: 2, warn: 1, info: 1, total: 4 });
    expect(worstSeverity(list)).toBe('critical');
    expect(worstSeverity([f('x', 'info')])).toBe('info');
    expect(worstSeverity([])).toBeNull();
  });
  it('detects critical plan conflicts', () => {
    expect(criticalConflicts({ conflicts: list }).length).toBe(2);
    expect(criticalConflicts({})).toEqual([]);
  });
});

describe('items', () => {
  it('fills v2 defaults', () => {
    const it = normalizeItem({ provider: 'manual' } as never);
    expect(it).toMatchObject({ enabled: true, identified: false, meta: null, issues: [] });
  });
  it('labels sources', () => {
    expect(sourceBadge({ source: 'moss', provider: 'modrinth' }).label).toBe('Modrinth');
    expect(sourceBadge({ source: 'moss', provider: 'curseforge' }).label).toBe('CurseForge');
    expect(sourceBadge({ source: 'identified', provider: 'modrinth' }).label).toBe('Identified');
    expect(sourceBadge({ source: 'modpack', provider: 'modrinth' }).label).toBe('Modpack');
    expect(sourceBadge({ source: 'manual', provider: 'manual' }).label).toBe('Manual');
  });
  it('builds project page links', () => {
    expect(pageUrl({ provider: 'modrinth', project_id: 'sodium' })).toBe('https://modrinth.com/project/sodium');
    expect(pageUrl({ provider: 'curseforge', project_id: '394468' })).toContain('curseforge.com');
    expect(pageUrl({ provider: 'manual', project_id: null })).toBeNull();
  });
});

describe('versions', () => {
  it('compares releases numerically', () => {
    expect(compareRelease('1.21.4', '1.21.10')).toBeLessThan(0);
    expect(compareRelease('1.21', '1.21.0')).toBe(0);
    expect(compareRelease('1.9', '1.10')).toBeLessThan(0);
  });
  it('sorts stable first, newest first, snapshots last', () => {
    const vs = [{ id: '24w46a', stable: false }, { id: '1.20.4', stable: true }, { id: '1.21.10', stable: true }, { id: '25w05a', stable: false }, { id: '1.21.4', stable: true }];
    expect(sortVersions(vs).map((v) => v.id)).toEqual(['1.21.10', '1.21.4', '1.20.4', '25w05a', '24w46a']);
  });
  it('hides snapshots unless asked, but keeps the selected one', () => {
    const vs = [{ id: '25w05a', stable: false }, { id: '1.21.4', stable: true }];
    expect(visibleVersions(vs, false).map((v) => v.id)).toEqual(['1.21.4']);
    expect(visibleVersions(vs, false, '25w05a').map((v) => v.id)).toEqual(['1.21.4', '25w05a']);
    expect(visibleVersions(vs, true).length).toBe(2);
  });
  it('detects downgrades only between releases', () => {
    expect(isDowngrade('1.21.4', '1.20.6')).toBe(true);
    expect(isDowngrade('1.20.6', '1.21.4')).toBe(false);
    expect(isDowngrade('1.21.4', '1.21.4')).toBe(false);
    expect(isDowngrade('1.21.4', '25w05a')).toBe(false);
  });
});

describe('plans', () => {
  it('formats version diffs', () => {
    expect(versionDiff({ version: '2.0', from_version: '1.0' })).toBe('1.0 → 2.0');
    expect(versionDiff({ version: '2.0', from_version: null })).toBe('2.0');
    expect(versionDiff({})).toBeNull();
  });
  it('counts plan actions', () => {
    expect(planCounts({ items: [{ action: 'install' }, { action: 'replace' }, { action: 'replace' }] as never })).toEqual({ install: 1, replace: 2, remove: 0 });
  });
  it('tallies upgrade content statuses', () => {
    const row = (status: 'ok' | 'update' | 'missing' | 'unknown') => ({ item: 'x', title: 'x', status, current: null, target: null, version_id: null, reason: null });
    expect(contentCounts([row('ok'), row('ok'), row('missing'), row('unknown')])).toEqual({ ok: 2, update: 0, missing: 1, unknown: 1 });
  });
  it('finds the rollback backup id in an operation log', () => {
    expect(findBackupId(['[10:00] Stopping', '[10:01] Pre-change backup 0192a3b4-1111-7222-8333-444455556666 created', 'Done'])).toBe('0192a3b4-1111-7222-8333-444455556666');
    expect(findBackupId(['Backup id: abcdef012345 ok'])).toBe('abcdef012345');
    expect(findBackupId(['nothing here'])).toBeNull();
  });
});

describe('modpacks', () => {
  it('parses references', () => {
    expect(parseModpackRef('https://modrinth.com/modpack/fabulously-optimized', 'modrinth')).toEqual({ provider: 'modrinth', project_id: 'fabulously-optimized' });
    expect(parseModpackRef('https://modrinth.com/modpack/fo/version/abc123', 'curseforge')).toEqual({ provider: 'modrinth', project_id: 'fo', version_id: 'abc123' });
    expect(parseModpackRef('https://www.curseforge.com/minecraft/modpacks/all-the-mods-9/files/5555', 'modrinth')).toEqual({ provider: 'curseforge', project_id: 'all-the-mods-9', version_id: '5555' });
    expect(parseModpackRef('cf:123456', 'modrinth')).toEqual({ provider: 'curseforge', project_id: '123456' });
    expect(parseModpackRef('P7dR8mSH', 'modrinth')).toEqual({ provider: 'modrinth', project_id: 'P7dR8mSH' });
    expect(parseModpackRef('https://example.com/x', 'modrinth')).toBeNull();
    expect(parseModpackRef('  ', 'modrinth')).toBeNull();
    expect(parseModpackRef('not a url', 'modrinth')).toBeNull();
  });
  it('pre-checks uploads', () => {
    expect(modpackFileProblem({ name: 'a.mrpack', size: 10 })).toBeNull();
    expect(modpackFileProblem({ name: 'a.zip', size: 10 })).toBeNull();
    expect(modpackFileProblem({ name: 'a.jar', size: 10 })).toMatch(/mrpack/);
    expect(modpackFileProblem({ name: 'a.zip', size: 600 * 1024 * 1024 })).toMatch(/512/);
    expect(modpackFileProblem({ name: 'a.zip', size: 0 })).toMatch(/empty/);
  });
});

describe('removalConflicts', () => {
  const mk = (id: string, title: string, depends: { id: string; required: boolean; range: string | null }[] = [], enabled = true) =>
    ({ id, title, enabled, meta: { id, name: title, version: '1', authors: [], description: '', loader: 'fabric', environment: 'both', depends, breaks: [] } }) as never;
  it('reports enabled dependents of removed items', () => {
    const sodium = mk('sodium', 'Sodium');
    const extra = mk('sodium-extra', 'Sodium Extra', [{ id: 'sodium', required: true, range: '>=0.5' }]);
    const off = mk('x', 'Off', [{ id: 'sodium', required: true, range: null }], false);
    const opt = mk('y', 'Opt', [{ id: 'sodium', required: false, range: null }]);
    const r = removalConflicts([sodium, extra, off, opt], [sodium]);
    expect(r.map((f) => f.item)).toEqual(['sodium-extra']);
    expect(r[0].severity).toBe('critical');
    expect(removalConflicts([sodium, extra], [sodium, extra])).toEqual([]);
  });
});

describe('staged copies and pack diffs', () => {
  const run = (state: StagingRun['state'], report: StagingRun['report'] = {}): StagingRun => ({ id: 'r', server_id: 's', kind: 'update_all', state, include_world: false, report, port: 30000, operation_id: null, created_at: '', expires_at: '', expired: false, can_promote: false });
  it('knows which states are still running', () => {
    expect(['building', 'testing'].every((s) => stagingActive(run(s as StagingRun['state'])))).toBe(true);
    expect(['healthy', 'unhealthy', 'failed'].some((s) => stagingActive(run(s as StagingRun['state'])))).toBe(false);
  });
  it('summarises a healthy copy with its start time and log counts', () => {
    const s = stagingSummary(run('healthy', { ready_secs: 14.2, errors: 0, warnings: 1 }));
    expect(s.tone).toBe('ok');
    expect(s.headline).toContain('14.2 seconds');
    expect(s.lines[0]).toBe('0 errors and 1 warning in the log.');
  });
  it('explains an unhealthy copy and says the real server was left alone', () => {
    const s = stagingSummary(run('unhealthy', { reasons: ['The server crashed.'], errors: 1, warnings: 0 }));
    expect(s.tone).toBe('danger');
    expect(s.headline).toContain('not applied');
    expect(s.lines).toContain('The server crashed.');
  });
  it('shows the error of a failed test and a neutral message while building', () => {
    expect(stagingSummary(run('failed', { error: 'There are no content updates to try.' })).lines).toEqual(['There are no content updates to try.']);
    expect(stagingSummary(run('building')).tone).toBe('info');
  });
  it('counts a pack diff and formats a change', () => {
    const ch = (t: string, f: string | null, to: string | null) => ({ title: t, path: `mods/${t}.jar`, from: f, to, reason: null });
    const d = { files: { added: [ch('a', null, '1')], removed: [], updated: [ch('b', '1', '2'), ch('c', '1', '2')], held: [], unchanged: 3 }, overrides: { added: ['x'], updated: [], user_modified: [{ path: 'y', reason: '' }], removed_upstream: [], unchanged: 0 } } as unknown as PackDiff;
    expect(packDiffCounts(d)).toEqual({ added: 1, removed: 0, updated: 2, held: 0, configNew: 1, configUpdated: 0, kept: 1 });
    expect(packChangeText(ch('b', '1', '2'))).toBe('1 → 2');
    expect(packChangeText(ch('a', null, '1'))).toBe('1');
    expect(packChangeText(ch('r', '9', null))).toBe('9');
  });
});

describe('hangar provider', () => {
  it('is labelled and linked', () => {
    expect(sourceBadge({ source: 'moss', provider: 'hangar' }).label).toBe('Hangar');
    expect(pageUrl({ provider: 'hangar', project_id: 'ViaVersion' })).toContain('hangar.papermc.io');
  });
});
