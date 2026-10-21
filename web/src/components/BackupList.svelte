<script lang="ts">
  import Icon from './Icon.svelte';
  import RestoreModal from './RestoreModal.svelte';
  import { api, download, backupDownloadPath, type Backup } from '../lib/api';
import { apiOps } from '../lib/api-ops';
  import { isPreChange, canDeleteBackup, deleteHint } from '../lib/backupPreview';
  import { canCopyToRemote, isRemoteOnly, remoteBadge } from '../lib/remote';
  import { bytes, relTime, dateTime } from '../lib/format';
  import { confirmDialog, serverById, toast, toastError, trackOp } from '../lib/stores.svelte';
  import { can } from '../lib/auth.svelte';

  let { backups, showServer = false, onchange }: { backups: Backup[]; showServer?: boolean; onchange: () => void } = $props();
  const manage = $derived(can('backup.manage'));
  let restoring = $state<Backup | null>(null);

  const badge = (b: Backup) => (b.status === 'ok' ? (b.verified_at ? ['ok', 'Verified'] : ['', 'Unverified']) : b.status === 'failed' ? ['danger', 'Failed'] : b.status === 'verifying' ? ['info', 'Verifying'] : ['warn', 'Running']);

  async function verify(b: Backup) {
    try { trackOp((await api.verifyBackup(b.id)).operation); toast('info', 'Verification started'); } catch (e) { toastError(e, 'Verify failed'); }
  }
  async function del(b: Backup) {
    if (!canDeleteBackup(b)) return;
    const r = await confirmDialog({ title: 'Delete this backup?', body: `${dateTime(b.created_at)} · ${bytes(b.size)}${b.note ? ' · ' + b.note : ''}. This cannot be undone.`, confirmLabel: 'Delete backup', danger: true });
    if (!r.ok) return;
    try { await api.deleteBackup(b.id); toast('info', 'Backup deleted'); onchange(); } catch (e) { toastError(e, 'Delete failed'); onchange(); }
  }
  async function lock(b: Backup) {
    if (b.locked) {
      const r = await confirmDialog({ title: 'Unlock this backup?', body: 'Unlocked backups can be pruned automatically and deleted. Pre-change backups beyond the newest 3 are removed.', confirmLabel: 'Unlock' });
      if (!r.ok) return;
    }
    try { await apiOps.patchBackup(b.id, { locked: !b.locked }); toast('info', b.locked ? 'Backup unlocked' : 'Backup locked', { body: b.locked ? undefined : 'It will never be pruned or deleted.', timeout: 2600 }); onchange(); } catch (e) { toastError(e, 'Could not change the lock'); }
  }
  async function toRemote(b: Backup) {
    try { trackOp((await apiOps.uploadBackup(b.id)).operation); toast('info', 'Copy started'); onchange(); } catch (e) { toastError(e, 'Could not start the copy'); }
  }
  async function fromRemote(b: Backup) {
    try { trackOp((await apiOps.fetchBackup(b.id)).operation); toast('info', 'Download started', { body: `Fetching from ${b.remote_destination ?? 'the destination'} and checking its checksum.` }); } catch (e) { toastError(e, 'Could not start the download'); }
  }
  const dl = (b: Backup) => download(backupDownloadPath(b.id), `backup-${b.id.slice(0, 8)}.zip`).catch((e) => toastError(e, 'Download failed'));
</script>

<div class="table-wrap">
  <table>
    <thead><tr><th>Created</th>{#if showServer}<th>Server</th>{/if}<th>Kind</th><th class="num">Size</th><th class="ck">Checksum</th><th>Status</th><th></th></tr></thead>
    <tbody>
      {#each backups as b (b.id)}
        {@const [tone, lab] = badge(b)}
        <tr class:pre={isPreChange(b)}>
          <td><div title={dateTime(b.created_at)}>{relTime(b.created_at)}</div>{#if b.note}<div class="xs muted ellipsis" style="max-width:220px">{b.note}</div>{/if}
            {#if b.worlds?.length}<div class="chips" aria-label="Worlds in this backup">{#each b.worlds as w}<span class="wchip mono">{w}</span>{/each}</div>{/if}</td>
          {#if showServer}<td><a href="#/servers/{b.server_id}">{serverById(b.server_id)?.name ?? b.server_id.slice(0, 8)}</a></td>{/if}
          <td>{#if isPreChange(b)}<span class="badge info" title="Taken automatically before a change such as a restore, update or import. Only the newest 3 are kept."><Icon name="shield" size={12} />Pre-change</span><div class="xs muted">Safety copy</div>{:else}<span class="badge">{b.kind}</span>{/if}{#if !b.consistent}<span class="badge warn" style="margin-left:4px" title="Taken while the server was running without a save-all pause">Crash-consistent</span>{/if}</td>
          <td class="num">{bytes(b.size)}</td>
          <td class="ck"><code class="xs" title={b.sha256}>{b.sha256.slice(0, 10)}…</code></td>
          <td><div class="stack"><span class="badge {tone}">{#if tone === 'ok'}<Icon name="check" size={12} />{/if}{lab}</span>{#if b.locked}<span class="badge warn"><Icon name="lock" size={12} />Locked</span>{/if}{#if remoteBadge(b)}{@const rb = remoteBadge(b)!}<span class="badge {rb.tone}" title={rb.title}><Icon name="upload" size={12} />{rb.label}</span>{/if}</div>{#if b.remote === 'failed' && b.remote_error}<div class="xs muted ellipsis" style="max-width:200px" title={b.remote_error}>{b.remote_error}</div>{/if}</td>
          <td class="num"><span class="row g4" style="justify-content:flex-end">
            <button class="btn ghost icon sm" aria-label="Verify backup" title="Verify checksum" disabled={!manage || b.status !== 'ok'} onclick={() => verify(b)}><Icon name="shield" size={14} /></button>
            <button class="btn sm" disabled={!manage || b.status !== 'ok'} onclick={() => (restoring = b)}><Icon name="history" size={13} />Restore</button>
            {#if isRemoteOnly(b)}<button class="btn sm" disabled={!manage || b.status !== 'ok'} title="This backup is only at the destination. Download it to this machine." onclick={() => fromRemote(b)}><Icon name="download" size={13} />Download from remote</button>
            {:else}<button class="btn ghost icon sm" aria-label="Download backup" disabled={b.status !== 'ok'} onclick={() => dl(b)}><Icon name="download" size={14} /></button>{/if}
            {#if canCopyToRemote(b)}<button class="btn ghost icon sm" aria-label={b.remote === 'failed' ? 'Retry copy to remote' : 'Copy to remote'} title={b.remote === 'failed' ? 'Retry copying to the destination' : 'Copy to the destination'} disabled={!manage} onclick={() => toRemote(b)}><Icon name="upload" size={14} /></button>{/if}
            <button class="btn ghost icon sm" aria-label={b.locked ? 'Unlock backup' : 'Lock backup'} aria-pressed={!!b.locked} title={b.locked ? 'Locked: never pruned or deleted. Click to unlock.' : 'Lock to protect from pruning and deletion'} disabled={!manage} onclick={() => lock(b)}><Icon name={b.locked ? 'lock' : 'unlock'} size={14} /></button>
            <button class="btn ghost icon sm" class:locked={!canDeleteBackup(b)} aria-label={b.locked ? 'Delete backup (locked, unlock first)' : 'Delete backup'} aria-disabled={!canDeleteBackup(b) || !manage} title={deleteHint(b)} disabled={!manage} onclick={() => del(b)}><Icon name="trash" size={14} /></button>
          </span></td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

{#if restoring}
  <RestoreModal backup={restoring} onclose={() => (restoring = null)} />
{/if}

<style>
  tr.pre td:first-child { box-shadow: inset 3px 0 0 var(--info); }
  tr.pre { background: color-mix(in srgb, var(--info-soft), transparent 40%); }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 3px; max-width: 210px; }
  @media (max-width: 1360px) { .ck { display: none; } }
  .wchip { font-size: calc(11px * var(--ts, 1)); padding: 0 6px; border-radius: 6px; background: var(--s3); color: var(--muted); border: 1px solid var(--line); }
  .stack { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; padding: 6px 0; }
  td, th { padding-inline: 10px; }
  .btn.locked { opacity: .4; cursor: not-allowed; }
</style>
