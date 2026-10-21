<script lang="ts">
  import Icon from './Icon.svelte';
  import { ApiError, type Server } from '../lib/api';
import { apiOps } from '../lib/api-ops';
  import { host, toast, toastError } from '../lib/stores.svelte';
  import { can } from '../lib/auth.svelte';
  import { describeLimits, limitWarnings } from '../lib/limits';

  let { server: s }: { server: Server } = $props();
  const canEdit = $derived(can('config.write'));
  const L = $derived(s.limits);
  const seedOf = () => ({ mem: s.limits?.memory_mb != null ? String(s.limits.memory_mb) : '', cpu: s.limits?.cpu_pct != null ? String(s.limits.cpu_pct) : '', pids: s.limits?.pids != null ? String(s.limits.pids) : '' });
  let f = $state(seedOf());
  let seeded = '';
  $effect(() => { const k = s.id + JSON.stringify(s.limits ?? null); if (k !== seeded) { seeded = k; f = seedOf(); } });
  let saving = $state(false);
  let errs = $state<Record<string, string>>({});
  const dirty = $derived(JSON.stringify(f) !== JSON.stringify(seedOf()));
  const info = $derived(describeLimits(L, host.status?.host.os));
  const warns = $derived(limitWarnings({ memory: f.mem, memoryHeap: s.memory_mb }));
  const num = (v: string) => (v.trim() === '' ? null : Number(v));

  async function save(e: Event) {
    e.preventDefault(); saving = true; errs = {};
    try {
      await apiOps.patchLimits(s.id, { memory_limit_mb: num(f.mem), cpu_limit_pct: num(f.cpu), pids_limit: num(f.pids) });
      toast('success', 'Resource limits saved', { body: s.state === 'running' || s.state === 'starting' ? 'They apply the next time the server starts.' : undefined });
    } catch (er) {
      if (er instanceof ApiError) errs = Object.fromEntries(er.fields.map((x) => [x.path, x.message]));
      toastError(er, 'Could not save limits');
    }
    saving = false;
  }
</script>

<section class="card" aria-labelledby="lim-t">
  <div class="card-title" id="lim-t"><Icon name="cpu" size={14} />Resource limits</div>
  <div class="status" aria-live="polite">
    {#if !L}
      <span class="badge">Unknown</span><span class="muted sm">This Moss version does not report limit enforcement.</span>
    {:else}
      <span class="badge {L.enforced ? 'ok' : 'warn'}">{#if L.enforced}<Icon name="check" size={12} />{/if}{info.badge}</span>
      <span class="muted sm">Backend: <code>{L.backend}</code></span>
    {/if}
  </div>
  {#if L && !L.enforced}
    <div class="banner warn" style="margin-top:12px"><Icon name="info" size={16} />
      <div><b>Limits are saved but not applied on this host.</b>
        <div class="muted">{info.reason}</div>
        {#if L.detail}<div class="muted xs" style="margin-top:4px">Moss reports: {L.detail}</div>{/if}</div>
    </div>
  {:else if L?.detail}<p class="xs muted" style="margin-top:8px">{L.detail}</p>{/if}

  <form class="lf" onsubmit={save}>
    <div class="field">
      <label for="lm">Memory hard limit (MB)</label>
      <input id="lm" class="input" type="number" min="256" step="128" placeholder="Default ({Math.round(s.memory_mb * 1.25 + 512)})" bind:value={f.mem} readonly={!canEdit} aria-describedby="lm-h" />
      <span class="hint" id="lm-h">Above this the process is killed. Leave empty for the default: heap + 25% + 512 MiB.</span>
      {#if errs.memory_limit_mb}<span class="err">{errs.memory_limit_mb}</span>{/if}
    </div>
    <div class="field">
      <label for="lc">CPU limit (%)</label>
      <input id="lc" class="input" type="number" min="10" step="10" placeholder="No limit" bind:value={f.cpu} readonly={!canEdit} aria-describedby="lc-h" />
      <span class="hint" id="lc-h">100 is one core, 200 two cores. From 10 up to 100 × the number of cores.</span>
      {#if errs.cpu_limit_pct}<span class="err">{errs.cpu_limit_pct}</span>{/if}
    </div>
    <div class="field">
      <label for="lp">Process limit</label>
      <input id="lp" class="input" type="number" min="64" max="32768" step="64" placeholder="No limit" bind:value={f.pids} readonly={!canEdit} aria-describedby="lp-h" />
      <span class="hint" id="lp-h">Maximum threads and processes, 64 to 32768. Guards against fork bombs.</span>
      {#if errs.pids_limit}<span class="err">{errs.pids_limit}</span>{/if}
    </div>
    {#each warns as w}<div class="banner warn full"><Icon name="alert" size={16} />{w}</div>{/each}
    {#if canEdit}<div class="row end full"><button class="btn" type="button" disabled={!dirty} onclick={() => (f = seedOf())}>Reset</button><button class="btn primary" disabled={!dirty || saving}>{saving ? 'Saving…' : 'Save limits'}</button></div>{/if}
  </form>
</section>

<style>
  .status { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .lf { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px 16px; margin-top: 16px; align-items: start; }
  .full { grid-column: 1 / -1; }
  @media (max-width: 860px) { .lf { grid-template-columns: minmax(0, 1fr); } }
</style>
