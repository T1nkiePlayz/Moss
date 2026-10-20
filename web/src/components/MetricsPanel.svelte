<script lang="ts">
  import Icon from './Icon.svelte';
  import RangeSelect from './RangeSelect.svelte';
  import RangeChart from './RangeChart.svelte';
  import States from './States.svelte';
  import type { MetricRange, MetricSample, MetricSeries } from '../lib/api';
  import { errMsg } from '../lib/api';
  import { poll } from '../lib/stores.svelte';
  import { refreshMs, stepLabel, type ChartDef } from '../lib/metrics';

  let { title, icon, load, defs, fallback }: {
    title: string; icon: string; load: (r: MetricRange) => Promise<MetricSeries>; defs: ChartDef[]; fallback?: () => Promise<MetricSample[]>;
  } = $props();

  let range = $state<MetricRange>('1h');
  let data = $state<MetricSeries | null>(null);
  let err = $state('');
  let seq = 0;

  async function fetchIt() {
    const my = ++seq, r = range;
    try {
      let s: MetricSeries;
      try { s = await load(r); } catch (e) { if (r === '1h' && fallback) s = { range: r, step_s: 5, samples: await fallback() }; else throw e; }
      if (my === seq) { data = s; err = ''; }
    } catch (e) { if (my === seq) err = errMsg(e); }
  }
  $effect(() => { range; data = null; err = ''; return poll(fetchIt, refreshMs[range]); });
  const samples = $derived(data?.samples ?? []);
  const shown = $derived(defs.filter((d) => !d.show || d.show(samples)));
</script>

<section class="card" aria-label={title}>
  <div class="card-title"><Icon name={icon} size={14} />{title}<span class="grow"></span>{#if data}<span class="xs faint" style="text-transform:none;letter-spacing:0">{stepLabel(data.step_s)} resolution</span>{/if}<RangeSelect bind:value={range} label="{title} time range" /></div>
  {#if err && !data}<States kind="error" text={err} retry={fetchIt} />
  {:else if !data}<div class="grid cols-2"><div class="skel" style="height:172px"></div><div class="skel" style="height:172px"></div></div>
  {:else if !samples.length}<p class="muted sm" style="min-height:48px">No samples for this range yet. Charts fill in while the server runs.</p>
  {:else}
    <div class="grid cols-2" aria-live="off">
      {#each shown as d (d.id)}
        {@const last = samples[samples.length - 1]}
        <div class="ch">
          <div class="row"><Icon name={d.icon} size={13} /><span class="sm muted">{d.label}</span><span class="grow"></span><b class="tnum">{d.format(Number(last[d.avg] ?? 0))}</b></div>
          <RangeChart {samples} avg={d.avg} peak={d.peak} {range} color={d.color} format={d.format} max={d.max} label={d.label} />
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .ch { min-width: 0; }
</style>
