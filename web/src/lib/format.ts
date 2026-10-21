// Formatting helpers built on Intl; all pure and locale-aware.
const nf = new Intl.NumberFormat();
const cf = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 });
const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto', style: 'short' });
const df = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });

export const num = (n: number) => nf.format(n);
export const compact = (n: number) => cf.format(n);

export function bytes(n: number | null | undefined, digits = 1): string {
  if (n == null || !isFinite(n)) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  let v = Math.max(0, n);
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${i === 0 ? v.toFixed(0) : v.toFixed(v >= 100 ? 0 : digits)} ${u[i]}`;
}

export const pct = (n: number | null | undefined) => (n == null ? '—' : `${Math.round(n)}%`);

/** 93784 -> "1d 2h", 3700 -> "1h 1m", 42 -> "42s" */
export function duration(s: number | null | undefined): string {
  if (s == null) return '—';
  s = Math.floor(s);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

export function relTime(iso: string | number | null | undefined, now = Date.now()): string {
  if (iso == null) return '—';
  const t = typeof iso === 'number' ? iso : Date.parse(iso);
  if (isNaN(t)) return '—';
  const diff = (t - now) / 1000;
  const a = Math.abs(diff);
  if (a < 5) return 'just now';
  if (a < 60) return rtf.format(Math.round(diff), 'second');
  if (a < 3600) return rtf.format(Math.round(diff / 60), 'minute');
  if (a < 86400) return rtf.format(Math.round(diff / 3600), 'hour');
  if (a < 86400 * 30) return rtf.format(Math.round(diff / 86400), 'day');
  return df.format(t);
}

export const dateTime = (iso: string | null | undefined) => (iso ? df.format(Date.parse(iso)) : '—');

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function pluralize(n: number, one: string, many = one + 's') {
  return `${num(n)} ${n === 1 ? one : many}`;
}

export function initials(s: string) {
  return (s.match(/[A-Za-z0-9]/g) || ['?']).slice(0, 2).join('').toUpperCase();
}

const RUNTIME_NAMES: Record<string, string> = {
  vanilla: 'Vanilla', paper: 'Paper', purpur: 'Purpur', folia: 'Folia', fabric: 'Fabric', quilt: 'Quilt', forge: 'Forge', neoforge: 'NeoForge',
  velocity: 'Velocity', custom: 'Custom', sim: 'Simulated', bedrock: 'Bedrock', terraria: 'Terraria', factorio: 'Factorio', valheim: 'Valheim',
  gmod: "Garry's Mod", cs2: 'Counter-Strike 2', tf2: 'Team Fortress 2', l4d2: 'Left 4 Dead 2', rust: 'Rust', palworld: 'Palworld',
  zomboid: 'Project Zomboid', satisfactory: 'Satisfactory', l4d: 'Left 4 Dead', css: 'Counter-Strike: Source',
  dods: 'Day of Defeat: Source', hl2dm: 'Half-Life 2: Deathmatch', nmrih: 'No More Room in Hell', ark: 'ARK: Survival Evolved', sdtd: '7 Days to Die',
};

/** Display name of a runtime id ("cs2" → "Counter-Strike 2"); unknown ids are shown as they are. */
export function runtimeName(id: string): string { return RUNTIME_NAMES[id] ?? id; }

/** "Paper 1.21.4", "Valheim" (games installed at "latest" carry no useful version), "Custom". */
export function runtimeLabel(s: { runtime: string; game_version: string }): string {
  const v = s.game_version && !['latest', 'custom'].includes(s.game_version) ? ` ${s.game_version}` : '';
  return runtimeName(s.runtime) + v;
}
