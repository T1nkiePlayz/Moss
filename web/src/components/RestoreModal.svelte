<script lang="ts">
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import RestoreSummary from './RestoreSummary.svelte';
  import StagedModal from './StagedModal.svelte';
  import { ApiError, errMsg, type Backup, type RestorePreview } from '../lib/api';
import { apiOps } from '../lib/api-ops';
  import { isRemoteOnly } from '../lib/remote';
  import { bytes, dateTime } from '../lib/format';
  import { untrack } from 'svelte';
  import { summarizePreview, onlyParam } from '../lib/backupPreview';
  import { confirmDialog, serverById, toast, toastError, trackOp } from '../lib/stores.svelte';

  let { backup, onclose }: { backup: Backup; onclose: () => void } = $props();
  const worlds = $derived(backup.worlds ?? []);
  let mode = $state<'new_server' | 'in_place'>('new_server');
  let picked = $state<string[] | null>(null); // null = all worlds
  const selected = $derived(picked ?? worlds);
  let preview = $state<RestorePreview | null>(null);
  let loading = $state(true);
  let err = $state('');
  let busy = $state(false);
  let seq = 0;
  let compare = $state(true); // in-place: unpack and compare first, then promote
  let stagedId = $state('');
  let remoteOnly = $state(untrack(() => isRemoteOnly(backup)));

  const srv = $derived(serverById(backup.server_id));
  const running = $derived(srv?.state === 'running' || srv?.state === 'starting' || srv?.state === 'stopping');
  const only = $derived(mode === 'in_place' ? onlyParam(worlds, selected) : undefined);
  const noneSelected = $derived(mode === 'in_place' && worlds.length > 1 && selected.length === 0);
  const sum = $derived(preview ? summarizePreview(preview) : null);

  async function fetchPreview() {
    const my = ++seq;
    if (noneSelected || remoteOnly) { loading = false; preview = null; err = ''; return; }
    loading = true; err = '';
    try {
      const p = await apiOps.previewRestore(backup.id, mode, only);
      if (my === seq) preview = p;
    } catch (e) {
      if (my === seq && e instanceof ApiError && e.code === 'backup.remote_only') remoteOnly = true;
      else if (my === seq) { err = errMsg(e); preview = null; }
    }
    if (my === seq) loading = false;
  }
  $effect(() => {
    mode; only; // re-run when the choice changes
    const t = setTimeout(() => void fetchPreview(), 200);
    return () => clearTimeout(t);
  });

  function toggleWorld(w: string, on: boolean) { picked = on ? [...selected, w] : selected.filter((x) => x !== w); }

  const staged = $derived(mode === 'in_place' && compare);
  const dest = $derived(backup.remote_destination ?? 'the backup destination');
  const canGo = $derived(!busy && !noneSelected && !(mode === 'in_place' && running) && (remoteOnly || (!loading && !!sum)));

  async function go() {
    const inPlace = mode === 'in_place';
    if (staged) {
      busy = true;
      try { const r = await apiOps.prepareRestore(backup.id, only); trackOp(r.operation); stagedId = r.staged_id; } catch (e) { toastError(e, 'Could not prepare the restore'); }
      busy = false;
      return;
    }
    const headline = sum?.headline ?? `Moss downloads this backup from ${dest}, checks its checksum, then restores it.`;
    const r = await confirmDialog({
      title: inPlace ? 'Replace the current server data?' : 'Restore as a new server?',
      body: headline,
      consequence: inPlace ? `A pre-restore backup is taken first${only ? `. Only ${only.join(', ')} will be restored` : ''}.` : undefined,
      confirmLabel: inPlace ? 'Restore in place' : 'Create new server', danger: inPlace && (sum?.destructive ?? true),
    });
    if (!r.ok) return;
    busy = true;
    try {
      trackOp((await apiOps.restore(backup.id, mode, only)).operation);
      toast('info', 'Restore started', { body: inPlace ? 'A pre-restore backup is taken first.' : 'A new server will be created.' });
      onclose();
    } catch (e) { toastError(e, 'Restore failed'); }
    busy = false;
  }
</script>

{#if stagedId}
  <StagedModal id={stagedId} {onclose} />
{:else}
<Modal title="Restore backup" {onclose} width={660}>
  <p class="muted sm">{dateTime(backup.created_at)} · {bytes(backup.size)}{srv ? ` · ${srv.name}` : ''}{backup.kind === 'pre-change' ? ' · automatic pre-change backup' : ''}</p>

  <div class="col" style="margin-top:14px">
    <label class="opt" class:sel={mode === 'new_server'}><input type="radio" class="check" bind:group={mode} value="new_server" /><div><b>Restore as a new server</b><div class="sm muted">Safe. Creates a copy and leaves the current server untouched.</div></div></label>
    <label class="opt" class:sel={mode === 'in_place'} class:off={running}><input type="radio" class="check" bind:group={mode} value="in_place" disabled={running} /><div><b>Replace the current server</b><div class="sm muted">Overwrites files with this backup. A pre-restore backup is taken first.</div></div></label>
  </div>
  {#if running}<div class="banner info mt-s"><Icon name="info" size={16} />In-place restore needs the server stopped. Stop it first to enable that option.</div>{/if}

  {#if mode === 'in_place'}
    <label class="row g8 mt-s"><input type="checkbox" class="check" bind:checked={compare} /><span><b>Compare first, then switch</b> (recommended)<span class="hint" style="display:block">Unpacks the backup next to the server and shows exactly what would change. The server is only touched when you promote it.</span></span></label>
  {/if}

  {#if mode === 'in_place' && worlds.length > 1}
    <fieldset class="worlds">
      <legend class="label">Restore only these worlds</legend>
      <div class="row wrap g16">
        {#each worlds as w}<label class="row g6"><input type="checkbox" class="check" checked={selected.includes(w)} onchange={(e) => toggleWorld(w, e.currentTarget.checked)} /><code>{w}</code></label>{/each}
      </div>
      <span class="hint">Unselected folders and configuration files stay exactly as they are now.</span>
      {#if noneSelected}<span class="err" role="alert">Select at least one world.</span>{/if}
    </fieldset>
  {/if}

  <h3 class="sm prev-h">What will change</h3>
  <div class="prev" aria-live="polite" aria-busy={loading}>
    {#if remoteOnly}
      <div class="banner info"><Icon name="info" size={16} /><div>This backup is only stored on {dest}, so there is nothing to compare yet. Moss downloads it and checks its checksum before touching anything.{#if !staged} Choose Compare first to see the changes before they are applied.{/if}</div></div>
    {:else if loading}
      <div class="skel" style="height:96px"></div>
    {:else if err}
      <div class="banner danger" role="alert"><Icon name="alert" size={16} /><div class="grow">Could not preview this restore: {err}</div><button class="btn sm" onclick={fetchPreview}>Retry</button></div>
    {:else if noneSelected}
      <p class="muted sm">Choose at least one world to preview.</p>
    {:else if sum && preview}
      <RestoreSummary {preview} {sum} />
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn {mode === 'in_place' && !staged ? 'danger solid' : 'primary'}" disabled={!canGo} onclick={go}>{staged ? (busy ? 'Preparing…' : 'Prepare and compare…') : 'Review and restore…'}</button>
  {/snippet}
</Modal>
{/if}

<style>
  .opt { display: flex; gap: 12px; align-items: flex-start; padding: 12px; border: 1px solid var(--line2); border-radius: var(--r-sm); cursor: pointer; }
  .opt.sel { border-color: var(--a2); background: var(--a-soft); } .opt.off { opacity: .5; cursor: not-allowed; }
  .opt input { margin-top: 3px; }
  .worlds { border: 1px solid var(--line); border-radius: var(--r-sm); padding: 10px 12px; margin: 12px 0 0; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .worlds legend { padding: 0 6px; }
  .prev-h { margin: 18px 0 8px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); }
  .prev { min-height: 96px; }
</style>
