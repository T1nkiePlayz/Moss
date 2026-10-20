<script lang="ts">
  import Modal from '../Modal.svelte';
  import Icon from '../Icon.svelte';
  import { api, errMsg, type Operation, type Server, type StagingEstimate, type StagingRequest, type StagingRun } from '../../lib/api';
  import { stagingActive, stagingSummary } from '../../lib/content';
  import { bytes, relTime } from '../../lib/format';
  import { ops, poll, toastError, trackOp } from '../../lib/stores.svelte';

  // "Try this on a copy first": copies the server, applies the change to the copy, starts it and reports.
  let { server: s, request, title, applyLabel = 'Apply to the real server', onclose, onpromoted }: {
    server: Server; request: StagingRequest; title: string; applyLabel?: string; onclose: () => void; onpromoted?: (o: Operation) => void;
  } = $props();

  let est = $state<StagingEstimate | null>(null);
  let run = $state<StagingRun | null>(null);
  let err = $state('');
  let loading = $state(true);
  let world = $state(false);
  let busy = $state(false);
  let logLines = $state<string[] | null>(null);
  let logBusy = $state(false);

  // A test for this change that is still around (running, or finished less than a day ago) is shown again instead of starting a new one.
  $effect(() => {
    let live = true;
    (async () => {
      try {
        const [e, runs] = await Promise.all([api.stagingEstimate(s.id), api.stagingRuns(s.id)]);
        if (!live) return;
        est = e;
        const mine = runs.find((r) => r.kind === request.kind && !r.expired);
        if (mine) run = mine;
      } catch (x) { if (live) err = errMsg(x); }
      if (live) loading = false;
    })();
    return () => { live = false; };
  });
  $effect(() => {
    if (!run || !stagingActive(run)) return;
    const id = run.id;
    return poll(async () => { try { const r = await api.stagingRun(s.id, id); if (run?.id === id) run = r; } catch { if (run?.id === id) run = null; } }, 1500);
  });

  const op = $derived(run?.operation_id ? ops.list.find((o) => o.id === run!.operation_id) : undefined);
  const sum = $derived(run ? stagingSummary(run) : null);
  const need = $derived((est?.copy_bytes ?? 0) + (world ? est?.world_bytes ?? 0 : 0));
  const lowDisk = $derived(!!est && est.free_bytes > 0 && est.free_bytes < need + 64 * 1048576);

  async function start() {
    busy = true; err = ''; logLines = null;
    try {
      const r = await api.stagingStart(s.id, { ...request, include_world: world });
      run = r.run; trackOp(r.operation);
    } catch (x) { err = errMsg(x); }
    busy = false;
  }
  async function promote() {
    if (!run) return;
    busy = true;
    try { const r = await api.stagingPromote(s.id, run.id); trackOp(r.operation); onpromoted?.(r.operation); onclose(); } catch (x) { toastError(x, 'Could not apply the change'); }
    busy = false;
  }
  async function discard() {
    if (!run) return;
    busy = true;
    try { await api.stagingDiscard(s.id, run.id); if (!stagingActive(run)) { run = null; logLines = null; } } catch (x) { toastError(x, 'Could not discard the copy'); }
    busy = false;
  }
  async function showLog() {
    if (!run) return;
    logBusy = true;
    try { logLines = (await api.stagingLog(s.id, run.id, 300)).lines; } catch (x) { toastError(x, 'Could not read the log'); }
    logBusy = false;
  }
  const actionWord: Record<string, string> = { replace: 'update', update: 'update', add: 'add', remove: 'remove', config: 'config', upgrade: 'upgrade' };
</script>

<Modal {title} onclose={onclose} width={620}>
  {#if loading}<p class="muted sm" role="status">Checking free space…</p>
  {:else if !run}
    <p class="sm">Moss makes a temporary copy of <b>{s.name}</b> on a spare port, applies the change to the copy, starts it and watches the log. Your real server is not touched and keeps running.</p>
    {#if est}
      <p class="sm muted">The copy needs about <b class="tnum">{bytes(est.copy_bytes)}</b> of disk space ({bytes(est.free_bytes)} free). It is removed automatically after 24 hours.</p>
      <label class="row g6 opt"><input type="checkbox" class="check" bind:checked={world} /><span class="sm">Include the world <span class="muted tnum">(adds {bytes(est.world_bytes)})</span><br /><span class="xs muted">Takes longer and uses more space, but finds problems that only show up with your world. Without it the copy starts with a fresh world.</span></span></label>
      {#if lowDisk}<div class="banner warn" role="alert"><Icon name="alert" size={16} />There is not enough free disk space for this copy.</div>{/if}
    {/if}
    {#if err}<div class="banner danger" role="alert"><Icon name="x-circle" size={16} />{err}</div>{/if}
  {:else if sum}
    <div class="banner {sum.tone}" role={sum.tone === 'danger' ? 'alert' : 'status'}>
      <Icon name={sum.tone === 'ok' ? 'check-circle' : sum.tone === 'danger' ? 'x-circle' : 'refresh'} size={16} />
      <div class="grow"><b>{sum.headline}</b>{#each sum.lines as l}<div class="sm">{l}</div>{/each}</div>
    </div>
    {#if stagingActive(run) && op}<p class="sm muted" role="status">{op.stage}{op.progress != null ? ` · ${Math.round(op.progress * 100)}%` : ''}</p>{/if}
    {#if run.report.changes?.length}
      <h4 class="sub">What was tried</h4>
      <ul class="chg">{#each run.report.changes as c}<li><span class="badge">{actionWord[c.action] ?? c.action}</span><span class="grow ellipsis">{c.title}</span>{#if c.to}<span class="xs muted tnum">{c.to}</span>{/if}</li>{/each}</ul>
    {/if}
    {#if run.report.samples?.length}
      <h4 class="sub">Problems found in the log</h4>
      <ul class="samples">{#each run.report.samples as l}<li class="mono xs">{l}</li>{/each}</ul>
    {/if}
    {#each run.report.notes ?? [] as n}<p class="xs muted">{n}</p>{/each}
    {#if !stagingActive(run)}
      <p class="xs muted">Tested {relTime(run.report.tested_at ?? run.created_at)}. This copy is kept for 24 hours so you can read its log, then removed.</p>
      <div class="row g6">
        <button class="btn sm" disabled={logBusy} onclick={showLog}>{logBusy ? 'Loading…' : logLines ? 'Reload log' : 'Show log'}</button>
      </div>
      {#if logLines}<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <pre class="log mono xs" tabindex="0" aria-label="Console log of the test copy">{logLines.join('\n') || 'The log is empty.'}</pre>{/if}
    {/if}
    {#if err}<div class="banner danger" role="alert"><Icon name="x-circle" size={16} />{err}</div>{/if}
  {/if}
  {#snippet footer()}
    {#if !run}
      <button class="btn" onclick={onclose}>Cancel</button>
      <button class="btn primary" disabled={busy || loading || lowDisk} onclick={start} data-autofocus>{busy ? 'Starting…' : 'Start test'}</button>
    {:else if stagingActive(run)}
      <button class="btn" onclick={onclose}>Hide</button>
      <button class="btn danger" disabled={busy} onclick={discard}>Cancel test</button>
    {:else}
      <button class="btn" disabled={busy} onclick={discard}>Discard copy</button>
      {#if run.can_promote}<button class="btn primary" disabled={busy} onclick={promote}>{busy ? 'Working…' : applyLabel}</button>
      {:else}<button class="btn primary" disabled={busy} onclick={() => { discard().then(start); }}>Test again</button>{/if}
    {/if}
  {/snippet}
</Modal>

<style>
  .opt { align-items: flex-start; gap: 10px; margin: 12px 0; cursor: pointer; }
  .sub { margin: 14px 0 6px; font-size: var(--fs-sm); color: var(--muted); }
  .chg, .samples { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .chg li { display: flex; gap: 10px; align-items: center; padding: 6px 10px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); }
  .samples li { padding: 4px 8px; background: var(--bg); border-radius: var(--r-sm); overflow-wrap: anywhere; }
  .log { max-height: 240px; overflow: auto; padding: 10px; background: var(--bg); border: 1px solid var(--line); border-radius: var(--r-sm); white-space: pre-wrap; overflow-wrap: anywhere; margin: 8px 0 0; }
</style>
