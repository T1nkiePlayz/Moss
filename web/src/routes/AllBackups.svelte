<script lang="ts">
  import States from '../components/States.svelte';
  import BackupList from '../components/BackupList.svelte';
  import { api, errMsg, type Backup } from '../lib/api';
  import { servers, onOp } from '../lib/stores.svelte';
  import { bytes } from '../lib/format';

  let list = $state<Backup[] | null>(null);
  let err = $state('');
  let sid = $state('');
  async function load() { err = ''; try { list = await api.allBackups(); } catch (e) { err = errMsg(e); } }
  void load();
  $effect(() => onOp((o) => { if (o.kind.startsWith('backup') && o.state !== 'running' && o.state !== 'queued') load(); }));
  const shown = $derived((list ?? []).filter((b) => !sid || b.server_id === sid));
  const total = $derived(shown.reduce((n, b) => n + b.size, 0));
</script>

<div class="page-head">
  <div><h1>Backups</h1><p>{list ? `${shown.length} backups · ${bytes(total)}` : 'Across all servers'}</p></div>
  <select class="select" style="width:auto;min-width:180px" aria-label="Filter by server" bind:value={sid}>
    <option value="">All servers</option>{#each servers.list as s}<option value={s.id}>{s.name}</option>{/each}
  </select>
</div>

{#if err}<States kind="error" text={err} retry={load} />
{:else if !list}<States kind="loading" rows={4} />
{:else if !shown.length}<States kind="empty" icon="archive" title="No backups" text="Open a server's Backups tab to create one or set a schedule." />
{:else}<BackupList backups={shown} showServer onchange={load} />{/if}
