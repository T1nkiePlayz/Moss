// Time-zone helpers for schedules (pure; Intl based). A zone is an IANA name or a legacy fixed offset "UTC+HH:MM".
export const browserZone = (): string => {
  try { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; } catch { return 'UTC'; }
};

/** "UTC+02:00", "UTC-0530", "UTC+2" -> minutes; anything else -> null. */
export function parseFixedOffset(tz: string): number | null {
  const m = /^UTC([+-])(\d{1,2})(?::?(\d{2}))?$/.exec(tz);
  if (!m) return null;
  const v = parseInt(m[2], 10) * 60 + parseInt(m[3] ?? '0', 10);
  return m[1] === '-' ? -v : v;
}
export function offsetName(min: number): string {
  if (!min) return 'UTC';
  const a = Math.abs(min);
  return `UTC${min < 0 ? '-' : '+'}${String(Math.floor(a / 60)).padStart(2, '0')}:${String(a % 60).padStart(2, '0')}`;
}

const cache = new Map<string, Intl.DateTimeFormat | null>();
function dtf(zone: string): Intl.DateTimeFormat | null {
  if (!cache.has(zone)) {
    try { cache.set(zone, new Intl.DateTimeFormat('en-US', { timeZone: zone, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric' })); } catch { cache.set(zone, null); }
  }
  return cache.get(zone)!;
}
export const isValidZone = (zone: string) => parseFixedOffset(zone) !== null || dtf(zone) !== null;

export interface Wall { y: number; mo: number; d: number; h: number; mi: number }
/** Wall-clock fields of instant `t` in `zone`. */
export function wallClock(zone: string, t: number): Wall & { s: number } {
  const fx = parseFixedOffset(zone);
  if (fx !== null) {
    const x = new Date(t + fx * 60000);
    return { y: x.getUTCFullYear(), mo: x.getUTCMonth() + 1, d: x.getUTCDate(), h: x.getUTCHours(), mi: x.getUTCMinutes(), s: x.getUTCSeconds() };
  }
  const f = dtf(zone) ?? dtf('UTC')!;
  const o: Record<string, number> = {};
  for (const p of f.formatToParts(t)) if (p.type !== 'literal') o[p.type] = parseInt(p.value, 10);
  return { y: o.year, mo: o.month, d: o.day, h: o.hour % 24, mi: o.minute, s: o.second };
}
/** Offset from UTC in minutes at instant `t`. */
export function offsetMinutes(zone: string, t = Date.now()): number {
  const fx = parseFixedOffset(zone);
  if (fx !== null) return fx;
  const w = wallClock(zone, t);
  return Math.round((Date.UTC(w.y, w.mo - 1, w.d, w.h, w.mi, w.s) - Math.floor(t / 1000) * 1000) / 60000);
}

/** The instant at which `zone`'s wall clock reads w. A skipped hour resolves to the first valid instant after the gap. */
export function zonedToInstant(w: Wall, zone: string): number {
  const guess = Date.UTC(w.y, w.mo - 1, w.d, w.h, w.mi);
  const t1 = guess - offsetMinutes(zone, guess) * 60000;
  return guess - offsetMinutes(zone, t1) * 60000;
}

/** Next occurrence (ms) of "HH:MM" wall-clock in `zone` strictly after `now`. */
export function nextDailyRun(hhmm: string, zone: string, now = Date.now()): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm);
  if (!m) return null;
  const today = wallClock(zone, now);
  const at = (add: number) => {
    const d = new Date(Date.UTC(today.y, today.mo - 1, today.d + add));
    return zonedToInstant({ y: d.getUTCFullYear(), mo: d.getUTCMonth() + 1, d: d.getUTCDate(), h: +m[1], mi: +m[2] }, zone);
  };
  let t = at(0);
  if (t <= now) t = at(1);
  return t;
}

export function formatInZone(t: number | string, zone: string): string {
  const ms = typeof t === 'string' ? Date.parse(t) : t;
  if (isNaN(ms)) return '—';
  const fx = parseFixedOffset(zone);
  const o: Intl.DateTimeFormatOptions = { weekday: 'short', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' };
  try {
    if (fx !== null) return new Intl.DateTimeFormat('en-US', { ...o, timeZone: 'UTC' }).format(ms + fx * 60000);
    return new Intl.DateTimeFormat('en-US', { ...o, timeZone: zone, timeZoneName: 'short' }).format(ms);
  } catch { return new Date(ms).toISOString(); }
}

export interface ZoneSched { tz?: string; tz_offset_min?: number; daily_at?: string | null; next_run?: string | null; enabled?: boolean }
/** Schedules written by old clients only have tz_offset_min. */
export const scheduleZone = (s: ZoneSched): string => s.tz || offsetName(s.tz_offset_min ?? 0);
export const zoneLabel = (zone: string) => (parseFixedOffset(zone) !== null && zone !== 'UTC' ? `${zone} (fixed offset)` : zone.replace(/_/g, ' '));

/** "Daily at 04:00 Europe/Berlin" */
export const dailyWording = (s: ZoneSched): string => `Daily at ${s.daily_at} ${zoneLabel(scheduleZone(s))}`;

export interface NextRun { zone: string; inZone: string; local: string; same: boolean }
/** Next run shown in the schedule's zone and in the viewer's zone (collapsed when both show the same clock). */
export function nextRunDisplay(s: ZoneSched, viewer = browserZone()): NextRun | null {
  if (!s.next_run) return null;
  const t = Date.parse(s.next_run);
  if (isNaN(t)) return null;
  const zone = scheduleZone(s);
  const inZone = formatInZone(t, zone);
  const local = formatInZone(t, viewer);
  return { zone, inZone, local, same: offsetMinutes(zone, t) === offsetMinutes(viewer, t) };
}

export function observesDst(zone: string, year = new Date().getUTCFullYear()): boolean {
  if (parseFixedOffset(zone) !== null) return false;
  return offsetMinutes(zone, Date.UTC(year, 0, 1)) !== offsetMinutes(zone, Date.UTC(year, 6, 1));
}
export function dstNote(zone: string): string | null {
  if (parseFixedOffset(zone) !== null && zone !== 'UTC') return 'Fixed offset: this schedule does not follow daylight-saving changes.';
  if (!observesDst(zone)) return null;
  return 'Follows the zone\'s wall clock across daylight-saving changes. A skipped hour runs just after the gap, a repeated hour runs once.';
}

/** Case/underscore-insensitive search ("new york" finds America/New_York); prefix matches rank first. */
export function filterZones(zones: string[], query: string, limit = 60): string[] {
  const n = (x: string) => x.toLowerCase().replace(/[_/]+/g, ' ');
  const q = n(query).trim();
  if (!q) return zones.slice(0, limit);
  const starts: string[] = [], has: string[] = [];
  for (const z of zones) {
    const s = n(z);
    if (s.startsWith(q) || s.split(' ').some((w) => w.startsWith(q))) starts.push(z);
    else if (s.includes(q)) has.push(z);
  }
  return [...starts, ...has].slice(0, limit);
}
