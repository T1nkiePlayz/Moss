<script lang="ts">
  import Icon from './Icon.svelte';
  import States from './States.svelte';
  import { runFinding } from '../lib/actions';
  import { serverById } from '../lib/stores.svelte';
  import type { Finding } from '../lib/api';
  let { findings, loaded = true, error = '', retry, showServer = true }: {
    findings: Finding[]; loaded?: boolean; error?: string; retry?: () => void; showServer?: boolean;
  } = $props();
  const order = { critical: 0, warn: 1, info: 2 };
  const sorted = $derived([...findings].sort((a, b) => order[a.severity] - order[b.severity]));
  const sevIcon = { critical: 'x-circle', warn: 'alert', info: 'info' } as const;
  const sevLabel = { critical: 'Critical', warn: 'Warning', info: 'Notice' } as const;
  let busy = $state<string | null>(null);
  async function go(f: Finding) { busy = f.id; await runFinding(f); busy = null; }
</script>

{#if error && !findings.length}
  <States kind="error" text={error} {retry} />
{:else if !loaded}
  <States kind="loading" rows={2} />
{:else if !sorted.length}
  <div class="banner" style="align-items:center"><span style="color:var(--a-text)"><Icon name="check-circle" size={20} /></span><div><b>No findings.</b> <span class="muted">Moss checked crash loops, backups, disk, Java, EULA and ports. Everything looks fine.</span></div></div>
{:else}
  <div class="col">
    {#each sorted as f (f.id)}
      <article class="finding {f.severity}">
        <div class="ic"><Icon name={sevIcon[f.severity]} size={18} /></div>
        <div class="grow">
          <div class="row wrap g6">
            <h4>{f.title}</h4>
            <span class="badge {f.severity === 'critical' ? 'danger' : f.severity === 'warn' ? 'warn' : 'info'}">{sevLabel[f.severity]}</span>
            {#if showServer && f.server_id && serverById(f.server_id)}<a class="tag" href="#/servers/{f.server_id}">{serverById(f.server_id)?.name}</a>{/if}
          </div>
          <p class="muted sm">{f.detail}</p>
          {#if f.evidence.length}
            <details>
              <summary class="xs faint">Evidence ({f.evidence.length})</summary>
              <ul class="mono xs">{#each f.evidence as ev}<li>{ev}</li>{/each}</ul>
            </details>
          {/if}
        </div>
        {#if f.action}<button class="btn sm {f.severity === 'critical' ? 'primary' : ''}" disabled={busy === f.id} onclick={() => go(f)}>{f.action.label}</button>{/if}
      </article>
    {/each}
  </div>
{/if}

<style>
  .finding { display: flex; gap: 12px; align-items: flex-start; padding: 12px 14px; border-radius: var(--r); background: var(--s1); border: 1px solid var(--line); border-left: 3px solid var(--info); }
  .finding.warn { border-left-color: var(--warn); } .finding.critical { border-left-color: var(--danger); background: linear-gradient(90deg, var(--danger-soft), var(--s1) 40%); }
  .ic { color: var(--info); margin-top: 1px; } .warn .ic { color: var(--warn); } .critical .ic { color: var(--danger); }
  summary { cursor: pointer; margin-top: 4px; width: fit-content; }
  ul { margin: 6px 0 0; padding: 8px 10px 8px 24px; background: var(--bg); border-radius: 8px; color: var(--muted); }
  @media (max-width: 560px) { .finding { flex-wrap: wrap; } .finding .btn { margin-left: 30px; } }
</style>
