<script lang="ts">
  // Tiny SVG area/line chart with optional hover read-out.
  let { data, height = 36, min = 0, max, color = 'var(--a1)', fill = true, grid = false, format = (v: number) => String(Math.round(v)), times, label = 'Chart' }: {
    data: number[]; height?: number; min?: number; max?: number; color?: string; fill?: boolean; grid?: boolean;
    format?: (v: number) => string; times?: number[]; label?: string;
  } = $props();
  const W = 200;
  const hi = $derived(max ?? Math.max(1, ...data) * 1.1);
  const pts = $derived(data.map((v, i) => [(i / Math.max(1, data.length - 1)) * W, height - 2 - ((Math.min(v, hi) - min) / (hi - min || 1)) * (height - 4)] as const));
  const line = $derived(pts.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(1)} ${y.toFixed(1)}`).join(''));
  const area = $derived(pts.length ? `${line}L${W} ${height}L0 ${height}Z` : '');
  const gid = 'g' + Math.random().toString(36).slice(2, 8);
  let hover = $state<number | null>(null);
  function move(e: PointerEvent) {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    hover = Math.max(0, Math.min(data.length - 1, Math.round(((e.clientX - r.left) / r.width) * (data.length - 1))));
  }
</script>

<div class="spark" style="height:{height}px">
  <svg viewBox="0 0 {W} {height}" preserveAspectRatio="none" role="img" aria-label="{label}{data.length ? ', latest ' + format(data[data.length - 1]) : ''}"
    onpointermove={move} onpointerleave={() => (hover = null)}>
    <defs><linearGradient id={gid} x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color={color} stop-opacity=".32" /><stop offset="1" stop-color={color} stop-opacity="0" /></linearGradient></defs>
    {#if grid}{#each [0.25, 0.5, 0.75] as g}<line x1="0" x2={W} y1={height * g} y2={height * g} stroke="var(--line)" stroke-width="1" vector-effect="non-scaling-stroke" />{/each}{/if}
    {#if fill && area}<path d={area} fill="url(#{gid})" />{/if}
    {#if line}<path d={line} fill="none" stroke={color} stroke-width="1.6" stroke-linejoin="round" vector-effect="non-scaling-stroke" />{/if}
    {#if hover !== null && pts[hover]}<line x1={pts[hover][0]} x2={pts[hover][0]} y1="0" y2={height} stroke="var(--line2)" vector-effect="non-scaling-stroke" />{/if}
  </svg>
  {#if hover !== null && pts[hover]}
    <div class="tip tnum" style="left:{(pts[hover][0] / W) * 100}%" class:r={pts[hover][0] > W * 0.7}>
      {format(data[hover])}{#if times?.[hover]}<span class="muted"> · {new Date(times[hover]).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}</span>{/if}
    </div>
  {/if}
</div>

<style>
  .spark { position: relative; width: 100%; }
  svg { width: 100%; height: 100%; display: block; overflow: visible; touch-action: pan-y; }
  .tip { position: absolute; top: -26px; transform: translateX(-50%); background: var(--s3); border: 1px solid var(--line2); padding: 1px 8px; border-radius: 6px; font-size: 11px; white-space: nowrap; pointer-events: none; z-index: 2; }
  .tip.r { transform: translateX(-100%); }
</style>
