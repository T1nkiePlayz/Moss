// Long-term metrics helpers (pure).
import type { MetricRange, MetricSample } from './api';

export interface ChartDef { id: string; label: string; icon: string; avg: keyof MetricSample; peak: keyof MetricSample; color?: string; format: (v: number) => string; max?: number; show?: (s: MetricSample[]) => boolean }

export const ranges: { id: MetricRange; label: string }[] = [{ id: '1h', label: '1 h' }, { id: '24h', label: '24 h' }, { id: '7d', label: '7 d' }, { id: '30d', label: '30 d' }];
export const rangeSeconds: Record<MetricRange, number> = { '1h': 3600, '24h': 86400, '7d': 7 * 86400, '30d': 30 * 86400 };
/** How often a view of this range is worth refetching (ms). */
export const refreshMs: Record<MetricRange, number> = { '1h': 5000, '24h': 60000, '7d': 300000, '30d': 300000 };

export interface Stat { min: number; avg: number; max: number }
/** min of the averages, mean of the averages, and the highest peak. */
export function summarize(samples: MetricSample[], avg: keyof MetricSample, peak: keyof MetricSample = avg): Stat | null {
  const a = samples.map((s) => s[avg]).filter((v): v is number => typeof v === 'number' && isFinite(v));
  if (!a.length) return null;
  const p = samples.map((s) => s[peak]).filter((v): v is number => typeof v === 'number' && isFinite(v));
  return { min: Math.min(...a), avg: a.reduce((x, y) => x + y, 0) / a.length, max: Math.max(...(p.length ? p : a)) };
}
export const series = (samples: MetricSample[], k: keyof MetricSample) => samples.map((s) => (typeof s[k] === 'number' ? (s[k] as number) : 0));

/** Time label that fits the range: clock for short ranges, day + clock for long ones. */
export function tickLabel(t: number, range: MetricRange): string {
  const d = new Date(t);
  const clock = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return range === '1h' ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : range === '24h' ? clock : `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${clock}`;
}
/** Too few points to draw a band: use the compact sparkline instead. */
export const needsFallback = (samples: MetricSample[]) => samples.length < 3;
export const stepLabel = (s: number) => (s >= 3600 ? `${Math.round(s / 3600)} h` : s >= 60 ? `${Math.round(s / 60)} min` : `${s} s`);
