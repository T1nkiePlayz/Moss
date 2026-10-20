<script lang="ts">
  import States from '../components/States.svelte';
  import OpCard from '../components/OpCard.svelte';
  import { api, errMsg, type Activity } from '../lib/api';
  import { ops, servers, feed, refreshOps } from '../lib/stores.svelte';
  import { relTime, dateTime } from '../lib/format';

  let tab = $state<'ops' | 'log'>('ops');
  let sid = $state('');
  let log = $state<Activity[] | null>(null);
  let err = $state('');
  async function load() { err = ''; try { log = await api.activity(100, sid || undefined); } catch (e) { err = errMsg(e); } }
  $effect(() => { sid; load(); });
  $effect(() => { feed.latest; feed.rev; if (log) load(); });
  const shown = $derived(ops.list.filter((o) => !sid || o.server_id === sid));
</script>

<div class="page-head">
  <div><h1>Activity</h1><p>Operations in progress and the audit trail.</p></div>
  <select class="select" style="width:auto;min-width:180px" aria-label="Filter by server" bind:value={sid}>
    <option value="">All servers</option>{#each servers.list as s}<option value={s.id}>{s.name}</option>{/each}
  </select>
</div>

<div class="tabs" role="tablist">
  <button class="tab" role="tab" aria-selected={tab === 'ops'} onclick={() => (tab = 'ops')} style="background:none;border:0;cursor:pointer">Operations <span class="tnum faint">{shown.length}</span></button>
  <button class="tab" role="tab" aria-selected={tab === 'log'} onclick={() => (tab = 'log')} style="background:none;border:0;cursor:pointer">Audit log</button>
</div>

{#if tab === 'ops'}
  {#if !ops.loaded}<States kind="loading" rows={3} />
  {:else if !shown.length}<States kind="empty" icon="layers" title="No operations" text="Installs, backups, restores and updates are listed here." retry={refreshOps} />
  {:else}<div class="grid cols-auto" style="--gap:12px">{#each shown as o (o.id)}<OpCard op={o} />{/each}</div>{/if}
{:else if err}<States kind="error" text={err} retry={load} />
{:else if !log}<States kind="loading" rows={5} />
{:else if !log.length}<States kind="empty" icon="activity" title="No activity yet" text="Actions taken in Moss are recorded here." />
{:else}
  <div class="table-wrap">
    <table>
      <thead><tr><th>When</th><th>Who</th><th>Action</th><th>Target</th><th>Detail</th></tr></thead>
      <tbody>
        {#each log as a (a.id)}
          <tr><td class="muted" title={dateTime(a.time)}>{relTime(a.time)}</td><td>{a.actor}</td><td><code>{a.action}</code></td><td>{a.target}</td><td class="muted ellipsis" style="max-width:320px">{a.detail}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}
