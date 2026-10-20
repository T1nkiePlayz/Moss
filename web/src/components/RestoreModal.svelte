<script lang="ts">
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import { apiOps, errMsg, type Backup, type RestorePreview } from '../lib/api';
  import { bytes, dateTime, num } from '../lib/format';
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

  const srv = $derived(serverById(backup.server_id));
  const running = $derived(srv?.state === 'running' || srv?.state === 'starting' || srv?.state === 'stopping');
  const only = $derived(mode === 'in_place' ? onlyParam(worlds, selected) : undefined);
  const noneSelected = $derived(mode === 'in_place' && worlds.length > 1 && selected.length === 0);
  const sum = $derived(preview ? summarizePreview(preview) : null);

  async function fetchPreview() {
    const my = ++seq;
    if (noneSelected) { loading = false; preview = null; err = ''; return; }
    loading = true; err = '';
    try {
      const p = await apiOps.previewRestore(backup.id, mode, only);
      if (my === seq) preview = p;
    } catch (e) { if (my === seq) { err = errMsg(e); preview = null; } }
    if (my === seq) loading = false;
  }
  $effect(() => {
    mode; only; // re-run when the choice changes
    const t = setTimeout(() => void fetchPreview(), 200);
    return () => clearTimeout(t);
  });

  function toggleWorld(w: string, on: boolean) { picked = on ? [...selected, w] : selected.filter((x) => x !== w); }

  async function go() {
    if (!sum) return;
    const inPlace = mode === 'in_place';
    const r = await confirmDialog({
      title: inPlace ? 'Replace the current server data?' : 'Restore as a new server?',
      body: sum.headline,
      consequence: inPlace ? `A pre-restore backup is taken first${only ? `. Only ${only.join(', ')} will be restored` : ''}.` : undefined,
      confirmLabel: inPlace ? 'Restore in place' : 'Create new server', danger: inPlace && sum.destructive,
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
  const groupLabel = { added: 'Added', changed: 'Changed', removed: 'Removed' } as const;
</script>

<Modal title="Restore backup" {onclose} width={660}>
  <p class="muted sm">{dateTime(backup.created_at)} · {bytes(backup.size)}{srv ? ` · ${srv.name}` : ''}{backup.kind === 'pre-change' ? ' · automatic pre-change backup' : ''}</p>

  <div class="col" style="margin-top:14px">
    <label class="opt" class:sel={mode === 'new_server'}><input type="radio" class="check" bind:group={mode} value="new_server" /><div><b>Restore as a new server</b><div class="sm muted">Safe. Creates a copy and leaves the current server untouched.</div></div></label>
    <label class="opt" class:sel={mode === 'in_place'} class:off={running}><input type="radio" class="check" bind:group={mode} value="in_place" disabled={running} /><div><b>Replace the current server</b><div class="sm muted">Overwrites files with this backup. A pre-restore backup is taken first.</div></div></label>
  </div>
  {#if running}<div class="banner info mt-s"><Icon name="info" size={16} />In-place restore needs the server stopped. Stop it first to enable that option.</div>{/if}

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
    {#if loading}
      <div class="skel" style="height:96px"></div>
    {:else if err}
      <div class="banner danger" role="alert"><Icon name="alert" size={16} /><div class="grow">Could not preview this restore: {err}</div><button class="btn sm" onclick={fetchPreview}>Retry</button></div>
    {:else if noneSelected}
      <p class="muted sm">Choose at least one world to preview.</p>
    {:else if sum && preview}
      <p class="head" class:warn={sum.destructive}>{sum.headline}</p>
      <dl class="stats">
        {#each sum.rows as r}<div class="st {r.tone}"><dt>{r.label}</dt><dd class="tnum">{num(r.count)}</dd></div>{/each}
        {#if sum.showDelta}<div class="st"><dt>Size change</dt><dd class="tnum {sum.sizeTone}">{sum.sizeDelta}</dd></div>
        {:else}<div class="st"><dt>Backup size</dt><dd class="tnum">{bytes(preview.archive.size)}</dd></div>{/if}
      </dl>
      <p class="xs muted">Backup: {num(preview.archive.files)} files, {bytes(preview.archive.size)}{preview.current ? ` · Current: ${num(preview.current.files)} files, ${bytes(preview.current.size)}` : ''}</p>
      {#each sum.rows.filter((r) => r.samples.length) as r (r.key)}
        <details>
          <summary>{groupLabel[r.key as 'added']} files ({num(r.count)})</summary>
          <ul class="paths mono xs">{#each r.samples as pth}<li>{pth}</li>{/each}</ul>
          {#if r.more}<p class="xs muted">…and {num(r.more)} more not listed.</p>{/if}
        </details>
      {/each}
      {#each preview.warnings as w}<div class="banner warn mt-s"><Icon name="alert" size={16} />{w}</div>{/each}
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn {mode === 'in_place' ? 'danger solid' : 'primary'}" disabled={busy || loading || !sum || noneSelected || (mode === 'in_place' && running)} onclick={go}>Review and restore…</button>
  {/snippet}
</Modal>

<style>
  .opt { display: flex; gap: 12px; align-items: flex-start; padding: 12px; border: 1px solid var(--line2); border-radius: var(--r-sm); cursor: pointer; }
  .opt.sel { border-color: var(--a2); background: var(--a-soft); } .opt.off { opacity: .5; cursor: not-allowed; }
  .opt input { margin-top: 3px; }
  .worlds { border: 1px solid var(--line); border-radius: var(--r-sm); padding: 10px 12px; margin: 12px 0 0; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .worlds legend { padding: 0 6px; }
  .prev-h { margin: 18px 0 8px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); }
  .prev { min-height: 96px; }
  .head { margin: 0 0 10px; font-weight: 550; } .head.warn { color: var(--warn); }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 8px; margin: 0 0 8px; }
  .st { padding: 8px 10px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); }
  .st dt { font-size: var(--fs-xs); color: var(--muted); } .st dd { margin: 2px 0 0; font-size: var(--fs-lg); font-weight: 650; }
  .st.ok dd, dd.ok { color: var(--a-text); } .st.warn dd { color: var(--warn); } .st.danger dd, dd.danger { color: var(--danger); }
  details { margin-top: 8px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); }
  summary { cursor: pointer; padding: 8px 12px; font-size: var(--fs-sm); font-weight: 550; }
  .paths { list-style: none; margin: 0; padding: 4px 12px 10px; max-height: 180px; overflow: auto; } .paths li { padding: 1px 0; overflow-wrap: anywhere; }
  details > p { padding: 0 12px 10px; margin: 0; }
</style>
