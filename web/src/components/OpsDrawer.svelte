<script lang="ts">
  import Modal from './Modal.svelte';
  import OpCard from './OpCard.svelte';
  import States from './States.svelte';
  import { ops, ui } from '../lib/stores.svelte';
  const active = $derived(ops.list.filter((o) => o.state === 'queued' || o.state === 'running'));
  const recent = $derived(ops.list.filter((o) => o.state !== 'queued' && o.state !== 'running').slice(0, 12));
</script>

{#if ui.ops}
  <Modal title="Operations" side width={440} onclose={() => (ui.ops = false)}>
    {#if !ops.loaded}<States kind="loading" />
    {:else if !ops.list.length}<States kind="empty" icon="layers" title="No operations yet" text="Installs, backups and restores show up here with live progress." />
    {:else}
      <div class="col" style="gap:14px">
        {#if active.length}<div class="label">Running now</div>{#each active as o (o.id)}<OpCard op={o} open />{/each}{/if}
        {#if recent.length}<div class="label">Recent</div>{#each recent as o (o.id)}<OpCard op={o} />{/each}{/if}
        <a class="btn" href="#/activity" onclick={() => (ui.ops = false)}>Open Activity page</a>
      </div>
    {/if}
  </Modal>
{/if}
