<script lang="ts">
  import Sparkline from './Sparkline.svelte';
  import Icon from './Icon.svelte';
  let { value, label, sub, icon, history = [], warnAt = 75, dangerAt = 90 }: {
    value: number; label: string; sub: string; icon: string; history?: number[]; warnAt?: number; dangerAt?: number;
  } = $props();
  const R = 30, C = 2 * Math.PI * R;
  const v = $derived(Math.max(0, Math.min(100, value)));
  const col = $derived(v >= dangerAt ? 'var(--danger)' : v >= warnAt ? 'var(--warn)' : 'var(--a1)');
</script>

<div class="gauge">
  <div class="ring">
    <svg width="76" height="76" viewBox="0 0 76 76" role="img" aria-label="{label} {Math.round(v)} percent">
      <circle cx="38" cy="38" r={R} fill="none" stroke="var(--s3)" stroke-width="7" />
      <circle cx="38" cy="38" r={R} fill="none" stroke={col} stroke-width="7" stroke-linecap="round" stroke-dasharray="{(v / 100) * C} {C}" transform="rotate(-90 38 38)" style="transition: stroke-dasharray .6s var(--ease), stroke .3s" />
    </svg>
    <div class="num tnum">{Math.round(v)}<small>%</small></div>
  </div>
  <div class="grow">
    <div class="row g6 muted sm"><Icon name={icon} size={14} />{label}</div>
    <div class="sm tnum" style="margin:2px 0 6px">{sub}</div>
    <Sparkline data={history} height={26} min={0} max={100} color={col} label="{label} history" format={(x) => Math.round(x) + '%'} />
  </div>
</div>

<style>
  .gauge { display: flex; gap: 14px; align-items: center; }
  .ring { position: relative; width: 76px; height: 76px; flex: none; }
  .num { position: absolute; inset: 0; display: grid; place-items: center; font-size: 18px; font-weight: 650; }
  .num small { font-size: 10px; color: var(--muted); margin-left: 1px; align-self: center; }
  .num { display: flex; align-items: center; justify-content: center; }
</style>
