// Pure helpers for the content, upgrade and modpack UIs (unit-tested in content.test.ts).
import type { ChangePlan, ContentFinding, ContentItem, ContentSeverity, ModpackInspect, ProviderId, UpgradePlan } from './api';

const RANK: Record<ContentSeverity, number> = { critical: 0, warn: 1, info: 2 };
export const SEVERITIES: ContentSeverity[] = ['critical', 'warn', 'info'];
export const sevLabel: Record<ContentSeverity, string> = { critical: 'Critical', warn: 'Warning', info: 'Info' };

/** Most severe first; stable for equal severities. */
export function sortFindings<T extends { severity: ContentSeverity }>(fs: T[]): T[] {
  return fs.map((f, i) => [f, i] as const).sort((a, b) => RANK[a[0].severity] - RANK[b[0].severity] || a[1] - b[1]).map((x) => x[0]);
}

export function groupFindings(fs: ContentFinding[]): { severity: ContentSeverity; findings: ContentFinding[] }[] {
  return SEVERITIES.map((severity) => ({ severity, findings: fs.filter((f) => f.severity === severity) })).filter((g) => g.findings.length);
}

export function countFindings(fs: { severity: ContentSeverity }[]): Record<ContentSeverity, number> & { total: number } {
  const c = { critical: 0, warn: 0, info: 0, total: fs.length };
  for (const f of fs) c[f.severity]++;
  return c;
}

export function worstSeverity(fs: { severity: ContentSeverity }[]): ContentSeverity | null {
  return SEVERITIES.find((s) => fs.some((f) => f.severity === s)) ?? null;
}

/** Fill the v2 fields so older or partial payloads cannot break the list. */
export function normalizeItem(it: ContentItem): ContentItem {
  return { ...it, enabled: it.enabled ?? true, identified: it.identified ?? it.provider !== 'manual', meta: it.meta ?? null, issues: it.issues ?? [] };
}

const providerName: Record<string, string> = { modrinth: 'Modrinth', curseforge: 'CurseForge', manual: 'Manual' };
export const providerLabel = (p: string) => providerName[p] ?? p;

/** Badge shown in the Source column. */
export function sourceBadge(it: Pick<ContentItem, 'source' | 'provider'>): { label: string; tone: 'ok' | 'info' | 'warn' | ''; hint: string } {
  if (it.source === 'modpack') return { label: 'Modpack', tone: 'info', hint: 'Installed as part of a modpack' };
  if (it.source === 'identified') return { label: 'Identified', tone: 'info', hint: `Dropped in manually, then matched on ${providerLabel(it.provider)} by file hash` };
  if (it.source === 'manual' || it.provider === 'manual') return { label: 'Manual', tone: '', hint: 'Dropped in manually; Moss cannot update it until it is identified' };
  return { label: providerLabel(it.provider), tone: 'ok', hint: `Installed by Moss from ${providerLabel(it.provider)}` };
}

export function pageUrl(it: Pick<ContentItem, 'provider' | 'project_id'>): string | null {
  if (!it.project_id) return null;
  if (it.provider === 'modrinth') return `https://modrinth.com/project/${encodeURIComponent(it.project_id)}`;
  if (it.provider === 'curseforge') return `https://www.curseforge.com/projects/${encodeURIComponent(it.project_id)}`;
  return null;
}

/** "1.21.4" -> [1,21,4]; anything with a suffix (snapshot, pre-release) is not a plain release. */
export function parseRelease(v: string): number[] | null {
  return /^\d+(\.\d+){1,3}$/.test(v) ? v.split('.').map(Number) : null;
}
export function compareRelease(a: string, b: string): number {
  const x = parseRelease(a), y = parseRelease(b);
  if (!x || !y) return 0;
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d) return d;
  }
  return 0;
}

/** Stable releases newest first, then snapshots (newest id first). */
export function sortVersions<T extends { id: string; stable: boolean }>(vs: T[]): T[] {
  const rel = vs.filter((v) => v.stable).sort((a, b) => compareRelease(b.id, a.id) || b.id.localeCompare(a.id));
  const snap = vs.filter((v) => !v.stable).sort((a, b) => b.id.localeCompare(a.id, undefined, { numeric: true }));
  return [...rel, ...snap];
}
export const visibleVersions = <T extends { id: string; stable: boolean }>(vs: T[], snapshots: boolean, keep?: string) =>
  sortVersions(vs).filter((v) => snapshots || v.stable || v.id === keep);

/** True when both are plain releases and `to` is older. Snapshots are never reported as downgrades. */
export function isDowngrade(from: string, to: string): boolean {
  return !!parseRelease(from) && !!parseRelease(to) && compareRelease(to, from) < 0;
}

export function contentCounts(rows: UpgradePlan['content']): Record<'ok' | 'update' | 'missing' | 'unknown', number> {
  const c = { ok: 0, update: 0, missing: 0, unknown: 0 };
  for (const r of rows) c[r.status]++;
  return c;
}

/** "1.0 -> 2.0" for plan lines, or null when the plan does not say. */
export function versionDiff(it: { version?: string; from_version?: string | null }): string | null {
  if (it.version && it.from_version) return `${it.from_version} → ${it.version}`;
  if (it.version) return it.version;
  return null;
}
export function planCounts(p: Pick<ChangePlan, 'items'>) {
  const c = { install: 0, replace: 0, remove: 0 };
  for (const i of p.items) c[i.action]++;
  return c;
}
export const criticalConflicts = (p: Pick<ChangePlan, 'conflicts'>) => (p.conflicts ?? []).filter((c) => c.severity === 'critical');

/** The upgrade operation log names the pre-change backup; pull the id out for the rollback hint. */
export function findBackupId(log: string[]): string | null {
  for (const l of [...log].reverse()) {
    const m = l.match(/backup(?:\s+id)?\s*[:=#]?\s*"?([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f-]{4,}|[0-9a-f]{12,})/i);
    if (m) return m[1];
  }
  return null;
}

/** Accepts a Modrinth/CurseForge URL, "modrinth:slug", "curseforge:123", or a bare id (provider decides). */
export function parseModpackRef(input: string, fallback: ProviderId): { provider: ProviderId; project_id: string; version_id?: string } | null {
  const t = input.trim();
  if (!t) return null;
  let m = t.match(/^(modrinth|curseforge|cf)\s*:\s*([^\s/]+)(?:\/([^\s/]+))?$/i);
  if (m) return { provider: m[1].toLowerCase() === 'modrinth' ? 'modrinth' : 'curseforge', project_id: m[2], ...(m[3] ? { version_id: m[3] } : {}) };
  if (/^https?:\/\//i.test(t)) {
    let u: URL;
    try { u = new URL(t); } catch { return null; }
    const parts = u.pathname.split('/').filter(Boolean).map(decodeURIComponent);
    if (/(^|\.)modrinth\.com$/.test(u.hostname)) {
      const i = parts.findIndex((p) => p === 'modpack' || p === 'project');
      if (i < 0 || !parts[i + 1]) return null;
      const v = parts[i + 2] === 'version' ? parts[i + 3] : undefined;
      return { provider: 'modrinth', project_id: parts[i + 1], ...(v ? { version_id: v } : {}) };
    }
    if (/(^|\.)curseforge\.com$/.test(u.hostname)) {
      const i = parts.findIndex((p) => p === 'modpacks' || p === 'projects');
      if (i < 0 || !parts[i + 1]) return null;
      const f = parts[i + 2] === 'files' ? parts[i + 3] : undefined;
      return { provider: 'curseforge', project_id: parts[i + 1], ...(f ? { version_id: f } : {}) };
    }
    return null;
  }
  m = t.match(/^[A-Za-z0-9_.-]+$/);
  return m ? { provider: fallback, project_id: t } : null;
}

export const MODPACK_MAX = 512 * 1024 * 1024;
/** Client-side pre-check before a large upload; the server validates again. */
export function modpackFileProblem(f: { name: string; size: number }): string | null {
  if (!/\.(mrpack|zip)$/i.test(f.name)) return 'Choose a .mrpack (Modrinth) or .zip (CurseForge) file.';
  if (f.size > MODPACK_MAX) return 'Modpacks can be at most 512 MiB.';
  if (f.size === 0) return 'That file is empty.';
  return null;
}

export function modpackSummary(m: ModpackInspect): string {
  const f = m.files;
  return `${f.server} server file${f.server === 1 ? '' : 's'} of ${f.total}` + (f.client_only ? `, ${f.client_only} client-only skipped` : '') + (f.unavailable ? `, ${f.unavailable} unavailable` : '');
}

/** Items that would stop working if `removing` went away (required dependencies declared in their metadata). */
export function removalConflicts(all: ContentItem[], removing: ContentItem[]): ContentFinding[] {
  const gone = new Set(removing.map((r) => r.id));
  const byMeta = new Map(removing.filter((r) => r.meta).map((r) => [r.meta!.id, r]));
  const out: ContentFinding[] = [];
  for (const it of all) {
    if (gone.has(it.id) || !it.enabled || !it.meta) continue;
    for (const d of it.meta.depends) {
      const dep = d.required ? byMeta.get(d.id) : undefined;
      if (!dep) continue;
      out.push({
        id: `rm:${it.id}:${d.id}`, severity: 'critical', item: it.id, title: `${it.title} needs ${dep.title}`,
        detail: `${it.title} lists ${d.id} as a required dependency, so it will not load without it.`,
        evidence: [`${it.meta.id} depends on ${d.id}${d.range ? ' ' + d.range : ''}`], action: null,
      });
    }
  }
  return out;
}
