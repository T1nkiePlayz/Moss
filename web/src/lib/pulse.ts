import type { TimelineEvent } from './api';

export interface PulseSeg { from: number; to: number; kind: 'up' | 'down' | 'crash' }
export interface PulseTick { pos: number; kind: TimelineEvent['kind']; detail: string; time: string }
export interface Pulse { segs: PulseSeg[]; ticks: PulseTick[]; uptimePct: number }

/** Turn timeline events into a 24h (default) strip: uptime segments + event ticks, positions 0..1. */
export function buildPulse(events: TimelineEvent[], running: boolean, hours = 24, now = Date.now()): Pulse {
  const span = hours * 3600_000;
  const t0 = now - span;
  const ev = events.map((e) => ({ ...e, t: Date.parse(e.time) })).filter((e) => e.t >= t0 && e.t <= now).sort((a, b) => a.t - b.t);
  const pos = (t: number) => (t - t0) / span;
  // Infer state at window start from the first event.
  let up = ev.length ? ev[0].kind === 'stop' || ev[0].kind === 'crash' : running;
  let crashed = false;
  let cursor = t0;
  const segs: PulseSeg[] = [];
  const push = (to: number) => {
    if (to > cursor) segs.push({ from: pos(cursor), to: pos(to), kind: up ? 'up' : crashed ? 'crash' : 'down' });
    cursor = to;
  };
  for (const e of ev) {
    if (e.kind === 'start' || e.kind === 'ready' || e.kind === 'restart') { if (!up) { push(e.t); up = true; crashed = false; } }
    else if (e.kind === 'stop' || e.kind === 'crash') { if (up) { push(e.t); up = false; crashed = e.kind === 'crash'; } else if (e.kind === 'crash') crashed = true; }
  }
  if (!ev.length) up = running;
  push(now);
  const upT = segs.filter((s) => s.kind === 'up').reduce((n, s) => n + (s.to - s.from), 0);
  return {
    segs,
    ticks: ev.filter((e) => e.kind !== 'ready').map((e) => ({ pos: pos(e.t), kind: e.kind, detail: e.detail, time: e.time })),
    uptimePct: Math.round(upT * 1000) / 10,
  };
}
