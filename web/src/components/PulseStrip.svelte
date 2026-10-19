<script lang="ts">
  import { buildPulse } from '../lib/pulse';
  import type { TimelineEvent } from '../lib/api';
  import { relTime } from '../lib/format';
  // Pulse timeline: 24 h uptime bar with event ticks above it.
  let { events, running, compact = false }: { events: TimelineEvent[] | null; running: boolean; compact?: boolean } = $props();
  const p = $derived(events ? buildPulse(events, running) : null);
  const color = { up: 'var(--a2)', down: 'var(--s3)', crash: 'var(--danger)' };
  const tickColor: Record<string, string> = { start: 'var(--a1)', ready: 'var(--a1)', stop: 'var(--muted)', crash: 'var(--danger)', restart: 'var(--warn)', backup: 'var(--info)', install: 'var(--info)', update: 'var(--info)', undo: 'var(--warn)' };
</script>

{#if !p}
  <div class="skel" style="height:{compact ? 22 : 34}px"></div>
{:else}
  <div class="pulse" class:compact role="img" aria-label="Last 24 hours: {p.uptimePct}% uptime, {p.ticks.length} events">
    <div class="ticks">
      {#each p.ticks as t}
        <i style="left:{t.pos * 100}%;background:{tickColor[t.kind]}" class:big={t.kind === 'crash'} title="{t.kind} · {t.detail} · {relTime(t.time)}"></i>
      {/each}
    </div>
    <div class="bar2">
      {#each p.segs as s}<b style="left:{s.from * 100}%;width:{(s.to - s.from) * 100}%;background:{color[s.kind]}"></b>{/each}
    </div>
    {#if !compact}
      <div class="scale xs faint"><span>24 h ago</span><span class="tnum">{p.uptimePct}% up</span><span>now</span></div>
    {/if}
  </div>
{/if}

<style>
  .pulse { width: 100%; }
  .ticks { position: relative; height: 12px; }
  .compact .ticks { height: 8px; }
  .ticks i { position: absolute; bottom: 0; width: 2px; height: 8px; border-radius: 2px; transform: translateX(-1px); }
  .ticks i.big { height: 12px; }
  .compact .ticks i { height: 6px; } .compact .ticks i.big { height: 8px; }
  .bar2 { position: relative; height: 8px; border-radius: 99px; background: var(--s3); overflow: hidden; }
  .compact .bar2 { height: 5px; }
  .bar2 b { position: absolute; top: 0; bottom: 0; }
  .scale { display: flex; justify-content: space-between; margin-top: 6px; }
</style>
