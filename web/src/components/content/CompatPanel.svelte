<script lang="ts">
  import Icon from '../Icon.svelte';
  import FindingList from './FindingList.svelte';
  import { api, errMsg, type ContentFinding, type ContentItem } from '../../lib/api';
  import { countFindings, groupFindings, sevLabel } from '../../lib/content';
  import { relTime } from '../../lib/format';

  let { serverId, items, reloadKey = 0, busy = null, onaction }: {
    serverId: string; items: ContentItem[]; reloadKey?: number; busy?: string | null; onaction: (f: ContentFinding) => void;
  } = $props();

  const sevs = ['critical', 'warn', 'info'] as const;
  let findings = $state<ContentFinding[] | null>(null);
  let checkedAt = $state<string | null>(null);
  let err = $state('');
  let loading = $state(false);
  let collapsed = $state(false);
  let seq = 0;

  async function run() {
    const my = ++seq;
    loading = true; err = '';
    try {
      const r = await api.contentCheck(serverId);
      if (my !== seq) return;
      findings = r.findings; checkedAt = r.checked_at;
    } catch (e) { if (my === seq) err = errMsg(e); }
    if (my === seq) loading = false;
  }
  $effect(() => { serverId; reloadKey; void run(); });
  // Re-render "checked 2 minutes ago" without refetching.
  let tick = $state(Date.now());
  $effect(() => { const t = setInterval(() => (tick = Date.now()), 30000); return () => clearInterval(t); });

  const counts = $derived(countFindings(findings ?? []));
  const groups = $derived(groupFindings(findings ?? []));
  const names = $derived(new Map(items.map((i) => [i.id, i.title])));
  const clean = $derived(!!findings && !findings.length);
  const tone = $derived(counts.critical ? 'danger' : counts.warn ? 'warn' : counts.info ? 'info' : 'ok');
</script>

<section class="card flat comp" class:bad={counts.critical > 0} aria-labelledby="comp-h">
  <div class="top">
    <span class="ic {tone}"><Icon name={clean ? 'check-circle' : counts.critical ? 'x-circle' : counts.total ? 'alert' : 'shield'} size={18} /></span>
    <div class="grow">
      <h3 id="comp-h">Compatibility</h3>
      <div class="xs muted" role="status" aria-live="polite">
        {#if err}Check failed.
        {:else if !findings}Checking…
        {:else if clean}No problems found · checked {relTime(checkedAt, tick)}
        {:else}{counts.total} finding{counts.total === 1 ? '' : 's'} · checked {relTime(checkedAt, tick)}{/if}
      </div>
    </div>
    <div class="row g6 chips">
      {#each sevs as s}
        {#if counts[s]}<span class="badge {s === 'critical' ? 'danger' : s}" title="{sevLabel[s]}">{counts[s]} {sevLabel[s].toLowerCase()}</span>{/if}
      {/each}
    </div>
    <button class="btn sm" class:spin={loading} disabled={loading} onclick={run}><Icon name="refresh" size={14} />{loading ? 'Checking…' : 'Re-check'}</button>
    {#if counts.total}<button class="btn ghost icon sm" aria-expanded={!collapsed} aria-label={collapsed ? 'Show findings' : 'Hide findings'} onclick={() => (collapsed = !collapsed)}><Icon name={collapsed ? 'chevron-right' : 'chevron-down'} size={15} /></button>{/if}
  </div>
  {#if err}
    <div class="banner danger" role="alert"><Icon name="alert" size={16} /><div class="grow">{err}</div></div>
  {:else if !findings}
    <div class="skel" style="height:44px"></div>
  {:else if counts.total && !collapsed}
    <div class="groups">
      {#each groups as g (g.severity)}
        <div class="grp">
          <div class="gh xs">{sevLabel[g.severity]} · {g.findings.length}</div>
          <FindingList findings={g.findings} {busy} {onaction} itemName={(id) => names.get(id)} />
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .comp { margin-bottom: 16px; display: flex; flex-direction: column; gap: 12px; padding: 12px 14px; }
  .comp.bad { border-color: color-mix(in srgb, var(--danger), transparent 60%); }
  .top { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .ic { width: 34px; height: 34px; border-radius: 10px; display: grid; place-items: center; flex: none; }
  .ic.ok { background: var(--a-soft); color: var(--a-text); } .ic.warn { background: var(--warn-soft); color: var(--warn); }
  .ic.danger { background: var(--danger-soft); color: var(--danger); } .ic.info { background: var(--info-soft); color: var(--info); }
  .groups { display: flex; flex-direction: column; gap: 12px; }
  .gh { text-transform: uppercase; letter-spacing: .06em; color: var(--faint); font-weight: 600; margin-bottom: 6px; }
  @media (max-width: 560px) { .chips { order: 5; width: 100%; } }
</style>
