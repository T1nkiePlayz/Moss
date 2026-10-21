<script lang="ts">
  // Avg line over a max band, with a min/avg/max read-out. Falls back to the compact Sparkline when there is too little data.
  import Sparkline from './Sparkline.svelte';
  import type { MetricRange, MetricSample } from '../lib/api';
  import { needsFallback, series, summarize, tickLabel } from '../lib/metrics';

  let { samples, avg, peak, range, height = 96, min = 0, max, color = 'var(--a1)', format = (v: number) => String(Math.round(v)), label }: {
    samples: MetricSample[]; avg: keyof MetricSample; peak: keyof MetricSample; range: MetricRange; height?: number; min?: number; max?: number;
    color?: string; format?: (v: number) => string; label: string;
  } = $props();
  const W = 200;
  const a = $derived(series(samples, avg));
  const p = $derived(series(samples, peak));
  const hi = $derived(Math.max(max ?? 0, ...p, 1) * 1.05);
  const y = (v: number) => height - 2 - ((Math.min(v, hi) - min) / (hi - min || 1)) * (height - 4);
  const x = (i: number) => (i / Math.max(1, a.length - 1)) * W;
  const path = (d: number[]) => d.map((v, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)} ${y(v).toFixed(1)}`).join('');
  const band = $derived(`${path(p)}L${W} ${y(a[a.length - 1] ?? 0)}${[...a].reverse().map((v, i) => `L${x(a.length - 1 - i).toFixed(1)} ${y(v).toFixed(1)}`).join('')}Z`);
  const st = $derived(summarize(samples, avg, peak));
  let hover = $state<number | null>(null);
  const gid = 'r' + Math.random().toString(36).slice(2, 8);
  function move(e: PointerEvent) {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    hover = Math.max(0, Math.min(a.length - 1, Math.round(((e.clientX - r.left) / r.width) * (a.length - 1))));
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); hover = Math.max(0, Math.min(a.length - 1, (hover ?? a.length - 1) + (e.key === 'ArrowLeft' ? -1 : 1))); }
    else if (e.key === 'Escape') hover = null;
  }
  const summary = $derived(st ? `${label}: min ${format(st.min)}, average ${format(st.avg)}, max ${format(st.max)}` : label);
</script>

{#if needsFallback(samples)}
  <div class="fb"><Sparkline data={a} {height} {min} {max} {color} {format} {label} /><p class="xs muted">Not enough history for this range yet.</p></div>
{:else}
  <div class="rc" style="height:{height}px">
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <svg viewBox="0 0 {W} {height}" preserveAspectRatio="none" role="img" tabindex="0" aria-label={summary} onpointermove={move} onpointerleave={() => (hover = null)} onkeydown={key} onblur={() => (hover = null)}>
      <defs><linearGradient id={gid} x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color={color} stop-opacity=".28" /><stop offset="1" stop-color={color} stop-opacity=".04" /></linearGradient></defs>
      {#each [0.25, 0.5, 0.75] as g}<line x1="0" x2={W} y1={height * g} y2={height * g} stroke="var(--line)" vector-effect="non-scaling-stroke" />{/each}
      <path d={band} fill="url(#{gid})" />
      <path d={path(a)} fill="none" stroke={color} stroke-width="1.6" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
      {#if hover !== null}<line x1={x(hover)} x2={x(hover)} y1="0" y2={height} stroke="var(--line2)" vector-effect="non-scaling-stroke" />{/if}
    </svg>
    {#if hover !== null && samples[hover]}
      <div class="tip tnum" class:r={x(hover) > W * 0.6} style="left:{(x(hover) / W) * 100}%" role="status">
        <b>avg {format(a[hover])}</b> · max {format(p[hover])}<span class="muted"> · {tickLabel(samples[hover].t, range)}</span>
      </div>
    {/if}
  </div>
  {#if st}<dl class="sum xs tnum"><div><dt>Min</dt><dd>{format(st.min)}</dd></div><div><dt>Avg</dt><dd>{format(st.avg)}</dd></div><div><dt>Max</dt><dd>{format(st.max)}</dd></div></dl>{/if}
{/if}

<style>
  .rc { position: relative; width: 100%; margin-top: 18px; }
  svg { width: 100%; height: 100%; display: block; overflow: visible; touch-action: pan-y; border-radius: 4px; }
  .tip { position: absolute; top: -28px; transform: translateX(-50%); background: var(--s3); border: 1px solid var(--line2); padding: 1px 8px; border-radius: 6px; font-size: calc(11px * var(--ts, 1)); white-space: nowrap; pointer-events: none; z-index: 2; }
  .tip.r { transform: translateX(-100%); }
  .sum { display: flex; gap: 18px; margin: 8px 0 0; color: var(--muted); } .sum div { display: flex; gap: 5px; } .sum dt { color: var(--faint); } .sum dd { margin: 0; color: var(--text); font-weight: 550; }
  .fb { margin-top: 18px; } .fb p { margin: 6px 0 0; }
</style>
