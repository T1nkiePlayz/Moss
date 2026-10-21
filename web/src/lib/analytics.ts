// Player analytics helpers (pure): chart bars and the weekday x hour heatmap, both in the viewer's local time.
import type { AnalyticsDays, PlayerAnalytics } from './api';

export interface Bar { t: number; peak: number; avg: number }
type Hourly = PlayerAnalytics['hourly'];

const dayKey = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;

/** Hourly bars for a week; one bar per local day (highest peak, mean of the hourly averages) for longer ranges. */
export function bars(hourly: Hourly, days: AnalyticsDays): Bar[] {
  const pts = hourly.map((h) => ({ t: Date.parse(h.hour), peak: h.peak, avg: h.avg }));
  if (days === 7) return pts;
  const out: { t: number; peak: number; sum: number; n: number; key: string }[] = [];
  for (const p of pts) {
    const key = dayKey(new Date(p.t));
    const last = out[out.length - 1];
    if (last && last.key === key) { last.peak = Math.max(last.peak, p.peak); last.sum += p.avg; last.n++; }
    else out.push({ t: p.t, peak: p.peak, sum: p.avg, n: 1, key });
  }
  return out.map((b) => ({ t: b.t, peak: b.peak, avg: b.sum / b.n }));
}

export const weekdays = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

/** 7 rows (Monday first) x 24 columns of the mean player count; `null` where there is no data. */
export function heatmap(hourly: Hourly): { cells: (number | null)[][]; max: number } {
  const sum = Array.from({ length: 7 }, () => new Array<number>(24).fill(0));
  const n = Array.from({ length: 7 }, () => new Array<number>(24).fill(0));
  for (const h of hourly) {
    const d = new Date(h.hour);
    const r = (d.getDay() + 6) % 7, c = d.getHours();
    sum[r][c] += h.avg; n[r][c]++;
  }
  const cells = sum.map((row, r) => row.map((v, c) => (n[r][c] ? v / n[r][c] : null)));
  return { cells, max: Math.max(0, ...cells.flat().map((v) => v ?? 0)) };
}
