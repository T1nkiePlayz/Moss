<script lang="ts">
  import OpCard from '../../components/OpCard.svelte';
  import States from '../../components/States.svelte';
  import { api, errMsg, type Operation, type Server } from '../../lib/api';
  import { onOp, ops } from '../../lib/stores.svelte';
  import { setupOps } from '../../lib/setup-ops';

  let { server: s }: { server: Server } = $props();
  let loaded = $state<Operation[] | null>(null);
  let err = $state('');
  $effect(() => {
    const id = s.id;
    loaded = null; err = '';
    api.operations(50, id).then((l) => { if (id === s.id) loaded = l; }, (e) => { err = errMsg(e); });
  });
  // Merge the fetched history with live updates from the shared store (store wins: it is newer).
  let seen = $state<Operation[]>([]);
  $effect(() => onOp((o) => { if (o.server_id === s.id) seen = [o, ...seen.filter((x) => x.id !== o.id)]; }));
  const list = $derived(setupOps(s.id, [...(loaded ?? []), ...seen, ...ops.list]));
</script>

{#if err}
  <States kind="error" title="Could not load the setup log" text={err} retry={() => { err = ''; loaded = null; api.operations(50, s.id).then((l) => (loaded = l), (e) => (err = errMsg(e))); }} />
{:else if loaded === null}
  <States kind="loading" rows={2} />
{:else if !list.length}
  <States kind="empty" icon="download" title="No setup log" text="Moss keeps the log of installing or importing a server here. This server has none recorded." />
{:else}
  <div class="col" style="gap:12px">
    {#each list as o, i (o.id)}
      <OpCard op={o} open={i === 0} />
    {/each}
  </div>
{/if}
