<script lang="ts">
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import RestoreSummary from './RestoreSummary.svelte';
  import { errMsg, type StagedRestore } from '../lib/api';
import { apiOps } from '../lib/api-ops';
  import { summarizePreview } from '../lib/backupPreview';
  import { blockingFailures, stagedAsPreview } from '../lib/remote';
  import { confirmDialog, serverById, toast, toastError, trackOp } from '../lib/stores.svelte';

  let { id, onclose }: { id: string; onclose: () => void } = $props();
  let s = $state<StagedRestore | null>(null);
  let err = $state('');
  let busy = $state(false);

  async function load() {
    try { s = await apiOps.stagedRestore(id); err = ''; } catch (e) { err = errMsg(e); }
  }
  $effect(() => {
    void load();
    const t = setInterval(() => { if (!s || s.status === 'preparing' || s.status === 'promoting') void load(); }, 1500);
    return () => clearInterval(t);
  });
  const preview = $derived(s ? stagedAsPreview(s) : null);
  const sum = $derived(preview ? summarizePreview(preview) : null);
  const srv = $derived(s ? serverById(s.server_id) : undefined);
  const blocked = $derived(blockingFailures(s?.checks));

  async function promote() {
    if (!s || !sum) return;
    const r = await confirmDialog({ title: 'Replace the current server data?', body: sum.headline, consequence: 'A safety backup is taken first, and the current files are kept in the removed folder so you can get them back.', confirmLabel: 'Promote the restore', danger: sum.destructive });
    if (!r.ok) return;
    busy = true;
    try { trackOp((await apiOps.promoteStaged(id)).operation); toast('info', 'Restore started', { body: 'Switching to the restored files.' }); onclose(); } catch (e) { toastError(e, 'Could not promote'); }
    busy = false;
  }
  async function discard() {
    busy = true;
    try { await apiOps.discardStaged(id); toast('info', 'Staged restore discarded', { body: 'The server was not changed.' }); onclose(); } catch (e) { toastError(e, 'Could not discard'); }
    busy = false;
  }
</script>

<Modal title="Review restore" {onclose} width={660}>
  <p class="muted sm">{srv ? `${srv.name}. ` : ''}{s?.status === 'ready' ? 'The backup was unpacked next to the live server. ' : ''}Nothing on the server has been replaced yet.</p>
  <div class="mt" aria-live="polite">
    {#if err && !s}<div class="banner danger" role="alert"><Icon name="alert" size={16} /><div class="grow">{err}</div><button class="btn sm" onclick={load}>Retry</button></div>
    {:else if !s || s.status === 'preparing'}<div class="skel" style="height:96px"></div><p class="muted sm">Preparing the restore. Large worlds take a while.</p>
    {:else if s.status === 'failed'}<div class="banner danger" role="alert"><Icon name="alert" size={16} /><div><b>The restore could not be prepared.</b><div>{s.error ?? 'Unknown error.'}</div><div class="muted">The server was not changed. Discard this and try again.</div></div></div>
    {:else if s.status === 'promoting' || s.status === 'promoted'}<div class="banner info"><Icon name="info" size={16} />{s.status === 'promoted' ? 'This restore has been applied.' : 'Switching to the restored files…'}</div>
    {:else if sum && preview}
      <h3 class="sm hd">Checks before switching</h3>
      <ul class="checks">{#each s.checks ?? [] as c}<li class:bad={!c.ok}><Icon name={c.ok ? 'check' : 'alert'} size={14} />{c.message}{#if !c.ok && !c.blocking} <span class="muted">(warning only)</span>{/if}</li>{/each}</ul>
      {#if blocked}<div class="banner danger mt-s" role="alert"><Icon name="alert" size={16} />The restored files are missing something the server needs, so Moss will not switch to them.</div>{/if}
      <h3 class="sm hd">What will change</h3>
      <RestoreSummary {preview} {sum} />
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Decide later</button>
    {#if s && s.status !== 'promoting' && s.status !== 'promoted'}<button class="btn" onclick={discard} disabled={busy}>Discard</button>{/if}
    {#if s?.status === 'ready'}<button class="btn danger solid" onclick={promote} disabled={busy || !s.can_promote || blocked > 0} title={s.can_promote ? '' : 'Stop the server first'}>Promote and replace…</button>{/if}
  {/snippet}
</Modal>

<style>
  .hd { margin: 16px 0 8px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); }
  .checks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-sm); }
  .checks li { display: flex; gap: 8px; align-items: center; } .checks li.bad { color: var(--danger); }
</style>
