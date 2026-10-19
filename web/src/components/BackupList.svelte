<script lang="ts">
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';
  import { api, download, backupDownloadPath, type Backup } from '../lib/api';
  import { bytes, relTime, dateTime } from '../lib/format';
  import { confirmDialog, serverById, toast, toastError, trackOp } from '../lib/stores.svelte';
  import { can } from '../lib/auth.svelte';

  let { backups, showServer = false, onchange }: { backups: Backup[]; showServer?: boolean; onchange: () => void } = $props();
  const manage = $derived(can('backup.manage'));
  let restoring = $state<Backup | null>(null);
  let mode = $state<'new_server' | 'in_place'>('new_server');
  let busy = $state(false);

  const badge = (b: Backup) => (b.status === 'ok' ? (b.verified_at ? ['ok', 'Verified'] : ['', 'Unverified']) : b.status === 'failed' ? ['danger', 'Failed'] : b.status === 'verifying' ? ['info', 'Verifying'] : ['warn', 'Running']);

  async function verify(b: Backup) {
    try { trackOp((await api.verifyBackup(b.id)).operation); toast('info', 'Verification started'); } catch (e) { toastError(e, 'Verify failed'); }
  }
  async function del(b: Backup) {
    const r = await confirmDialog({ title: 'Delete this backup?', body: `${dateTime(b.created_at)} · ${bytes(b.size)}${b.note ? ' · ' + b.note : ''}. This cannot be undone.`, confirmLabel: 'Delete backup', danger: true });
    if (!r.ok) return;
    try { await api.deleteBackup(b.id); toast('info', 'Backup deleted'); onchange(); } catch (e) { toastError(e, 'Delete failed'); }
  }
  const srv = $derived(restoring ? serverById(restoring.server_id) : undefined);
  const running = $derived(srv?.state === 'running' || srv?.state === 'starting' || srv?.state === 'stopping');
  async function restore() {
    if (!restoring) return;
    busy = true;
    try { trackOp((await api.restoreBackup(restoring.id, mode)).operation); toast('info', 'Restore started', { body: mode === 'in_place' ? 'A pre-restore backup is taken first.' : 'A new server will be created.' }); restoring = null; } catch (e) { toastError(e, 'Restore failed'); }
    busy = false;
  }
  const dl = (b: Backup) => download(backupDownloadPath(b.id), `backup-${b.id.slice(0, 8)}.zip`).catch((e) => toastError(e, 'Download failed'));
</script>

<div class="table-wrap">
  <table>
    <thead><tr><th>Created</th>{#if showServer}<th>Server</th>{/if}<th>Kind</th><th class="num">Size</th><th>Checksum</th><th>Status</th><th></th></tr></thead>
    <tbody>
      {#each backups as b (b.id)}
        {@const [tone, lab] = badge(b)}
        <tr>
          <td><div title={dateTime(b.created_at)}>{relTime(b.created_at)}</div>{#if b.note}<div class="xs muted ellipsis" style="max-width:220px">{b.note}</div>{/if}</td>
          {#if showServer}<td><a href="#/servers/{b.server_id}">{serverById(b.server_id)?.name ?? b.server_id.slice(0, 8)}</a></td>{/if}
          <td><span class="badge">{b.kind}</span>{#if !b.consistent}<span class="badge warn" style="margin-left:4px" title="Taken while the server was running without a save-all pause">Crash-consistent</span>{/if}</td>
          <td class="num">{bytes(b.size)}</td>
          <td><code class="xs" title={b.sha256}>{b.sha256.slice(0, 10)}…</code></td>
          <td><span class="badge {tone}">{#if tone === 'ok'}<Icon name="check" size={12} />{/if}{lab}</span></td>
          <td class="num"><span class="row g4" style="justify-content:flex-end">
            <button class="btn ghost icon sm" aria-label="Verify backup" title="Verify checksum" disabled={!manage || b.status !== 'ok'} onclick={() => verify(b)}><Icon name="shield" size={14} /></button>
            <button class="btn sm" disabled={!manage || b.status !== 'ok'} onclick={() => { restoring = b; mode = 'new_server'; }}><Icon name="history" size={13} />Restore</button>
            <button class="btn ghost icon sm" aria-label="Download backup" disabled={b.status !== 'ok'} onclick={() => dl(b)}><Icon name="download" size={14} /></button>
            <button class="btn ghost icon sm" aria-label="Delete backup" disabled={!manage} onclick={() => del(b)}><Icon name="trash" size={14} /></button>
          </span></td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

{#if restoring}
  <Modal title="Restore backup" onclose={() => (restoring = null)} width={520}>
    <p class="muted sm">{dateTime(restoring.created_at)} · {bytes(restoring.size)}{srv ? ` · ${srv.name}` : ''}</p>
    <div class="col" style="margin-top:14px">
      <label class="opt" class:sel={mode === 'new_server'}><input type="radio" class="check" bind:group={mode} value="new_server" /><div><b>Restore as a new server</b><div class="sm muted">Safe. Creates a copy and leaves the current server untouched.</div></div></label>
      <label class="opt" class:sel={mode === 'in_place'} class:off={running}><input type="radio" class="check" bind:group={mode} value="in_place" disabled={running} /><div><b>Replace the current server</b><div class="sm muted">Overwrites world and config with this backup. A pre-restore backup is taken first.</div></div></label>
    </div>
    {#if mode === 'in_place'}<div class="banner warn mt-s"><Icon name="alert" size={16} />Everything changed since this backup will be lost (unless you restore the pre-restore backup).</div>{/if}
    {#if running}<div class="banner info mt-s"><Icon name="info" size={16} />In-place restore needs the server stopped. Stop it first to enable that option.</div>{/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (restoring = null)}>Cancel</button>
      <button class="btn {mode === 'in_place' ? 'danger solid' : 'primary'}" disabled={busy || (mode === 'in_place' && running)} onclick={restore}>{mode === 'in_place' ? 'Replace server' : 'Restore as new'}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .opt { display: flex; gap: 12px; align-items: flex-start; padding: 12px; border: 1px solid var(--line2); border-radius: var(--r-sm); cursor: pointer; }
  .opt.sel { border-color: var(--a2); background: var(--a-soft); } .opt.off { opacity: .5; cursor: not-allowed; }
  .opt input { margin-top: 3px; }
</style>
