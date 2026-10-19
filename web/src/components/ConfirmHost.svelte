<script lang="ts">
  import Modal from './Modal.svelte';
  import { dialog } from '../lib/stores.svelte';
  let typed = $state('');
  let checked = $state(false);
  const req = $derived(dialog.req);
  const ok = $derived(!req?.typed || typed === req.typed);
  function done(v: boolean) {
    const r = dialog.req;
    dialog.req = null;
    typed = '';
    r?.resolve({ ok: v, checked: v && checked });
    checked = false;
  }
</script>

{#if req}
  <Modal title={req.title} onclose={() => done(false)} width={460}>
    {#if req.body}<p class="muted">{req.body}</p>{/if}
    {#if req.consequence}<div class="banner warn mt-s">{req.consequence}</div>{/if}
    {#if req.checkbox}
      <label class="row mt" style="gap:10px;cursor:pointer"><input type="checkbox" class="check" bind:checked /><span class="sm">{req.checkbox}</span></label>
    {/if}
    {#if req.typed}
      <div class="field mt">
        <label for="cf-typed">Type <code>{req.typed}</code> to confirm</label>
        <input id="cf-typed" class="input" autocomplete="off" bind:value={typed} data-autofocus onkeydown={(e) => e.key === 'Enter' && ok && done(true)} />
      </div>
    {/if}
    {#snippet footer()}
      <button class="btn" onclick={() => done(false)}>Cancel</button>
      <button class="btn {req.danger ? 'danger solid' : 'primary'}" disabled={!ok} onclick={() => done(true)}>{req.confirmLabel ?? 'Confirm'}</button>
    {/snippet}
  </Modal>
{/if}
