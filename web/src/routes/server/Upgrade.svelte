<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import States from '../../components/States.svelte';
  import OpCard from '../../components/OpCard.svelte';
  import StagedTest from '../../components/content/StagedTest.svelte';
  import { can } from '../../lib/auth.svelte';
  import { api, errMsg, type Server, type UpgradeContent, type UpgradePlan, type Versions } from '../../lib/api';
  import { contentCounts, findBackupId, isDowngrade, visibleVersions } from '../../lib/content';
  import { ops, toastError, trackOp } from '../../lib/stores.svelte';
  import { power } from '../../lib/actions';
  import { serverHref } from '../../lib/route';

  let { server: s }: { server: Server } = $props();

  let versions = $state<Versions | null>(null);
  let vErr = $state('');
  let snapshots = $state(false);
  let target = $state('');
  let loaderVer = $state('');
  let plan = $state<UpgradePlan | null>(null);
  let planErr = $state('');
  let planning = $state(false);
  let strategy = $state<UpgradeContent>('update');
  let ack = $state(false);
  let showOk = $state(false);
  let busy = $state(false);
  let startErr = $state('');
  let opId = $state<string | null>(null);
  let stopping = $state(false);
  let seq = 0;

  async function loadVersions() {
    vErr = '';
    try {
      versions = await api.versions(s.runtime);
      if (!target) {
        const next = visibleVersions(versions.versions, false).find((v) => v.id !== s.game_version);
        target = next?.id ?? s.game_version;
      }
    } catch (e) { vErr = errMsg(e); }
  }
  $effect(() => { s.runtime; versions = null; target = ''; void loadVersions(); });

  const opts = $derived(visibleVersions(versions?.versions ?? [], snapshots, target));
  const loaders = $derived(versions?.loader_versions?.[target] ?? []);
  const downgrade = $derived(isDowngrade(s.game_version, target));
  const unchanged = $derived(target === s.game_version && (!loaderVer || loaderVer === s.loader_version));
  // Any change to the inputs invalidates a plan that was computed for different ones.
  $effect(() => { target; loaderVer; seq++; plan = null; planErr = ''; ack = false; });
  $effect(() => { target; loaderVer = ''; });
  // After a finished upgrade the server's own version changes: start from a clean slate.
  $effect(() => { s.game_version; s.loader_version; plan = null; });

  async function runPlan() {
    const my = ++seq;
    planning = true; planErr = ''; plan = null; ack = false; startErr = '';
    try {
      const p = await api.upgradePlan(s.id, target, loaderVer || undefined);
      if (my !== seq) return;
      plan = p; showOk = false;
    } catch (e) { if (my === seq) planErr = errMsg(e); }
    if (my === seq) planning = false;
  }

  const counts = $derived(plan ? contentCounts(plan.content) : null);
  const rows = $derived.by(() => {
    if (!plan) return [];
    const rank = { missing: 0, unknown: 1, update: 2, ok: 3 } as const;
    return plan.content.map((r, i) => [r, i] as const).sort((a, b) => rank[a[0].status] - rank[b[0].status] || a[1] - b[1]).map((x) => x[0]);
  });
  const shownRows = $derived(showOk || rows.filter((r) => r.status !== 'ok').length === 0 ? rows : rows.filter((r) => r.status !== 'ok'));
  const kinds = ['ok', 'update', 'missing', 'unknown'] as const;
  const statusTone = { ok: 'ok', update: 'info', missing: 'danger', unknown: 'warn' } as const;
  const statusText = { ok: 'Compatible', update: 'Update', missing: 'No build', unknown: 'Unknown' } as const;

  const op = $derived(opId ? ops.list.find((o) => o.id === opId) : undefined);
  const live = $derived(!!op && (op.state === 'queued' || op.state === 'running'));
  const active = $derived(['running', 'starting', 'stopping', 'installing'].includes(s.state));
  const backupId = $derived(op ? findBackupId(op.log) : null);
  let testing = $state(false);
  const canTry = $derived(can('content.manage') && can('config.write'));
  const tryable = $derived(!!plan && plan.can_proceed && (!downgrade || ack) && !busy);
  const ready = $derived(!!plan && plan.can_proceed && !active && (!downgrade || ack) && !busy && !live);

  async function stop() {
    stopping = true;
    await power(s, 'stop');
    stopping = false;
  }
  async function installJava() {
    if (!plan) return;
    try { trackOp((await api.installJava(plan.java.to)).operation); plan.java.installed = true; } catch (e) { toastError(e); }
  }
  async function confirm() {
    if (!plan) return;
    busy = true; startErr = '';
    try {
      const r = await api.upgrade(s.id, { game_version: plan.to.game_version, loader_version: loaderVer || plan.to.loader_version || undefined, content: strategy, backup: true, ...(downgrade ? { confirm_downgrade: true } : {}) });
      trackOp(r.operation);
      opId = r.operation.id;
    } catch (e) { startErr = errMsg(e); }
    busy = false;
  }
  const strategies: { id: UpgradeContent; label: string; text: string }[] = [
    { id: 'update', label: 'Update to compatible versions', text: 'Replace each add-on with its newest build for the new version. Pinned items are left alone.' },
    { id: 'disable_incompatible', label: 'Disable incompatible add-ons', text: 'Keep what already works and disable add-ons that have no build for the new version. You can re-enable them later.' },
    { id: 'keep', label: 'Keep everything as it is', text: 'Do not touch add-ons. Some may fail to load or stop the server from starting.' },
  ];
</script>

<div class="page">
  <div class="card form">
    <div class="row between wrap">
      <div>
        <h2>Version upgrade</h2>
        <p class="muted sm">Currently <b>{s.runtime} {s.game_version}</b>{s.loader_version ? ` · loader ${s.loader_version}` : ''} · Java {s.java_major}</p>
      </div>
    </div>

    {#if vErr}<States kind="error" text={vErr} retry={loadVersions} />
    {:else if !versions}<div class="skel" style="height:80px"></div>
    {:else}
      <div class="grid2">
        <div class="field">
          <label for="up-ver">Target version</label>
          <select id="up-ver" class="select" bind:value={target} disabled={live}>
            {#each opts as v (v.id)}<option value={v.id}>{v.id}{v.id === s.game_version ? ' (current)' : ''}{v.id === versions.recommended ? ' (recommended)' : ''}{v.stable ? '' : ' (snapshot)'}</option>{/each}
          </select>
          <label class="row g6 sm muted tog"><input type="checkbox" class="check" bind:checked={snapshots} disabled={live} />Show snapshots and pre-releases</label>
        </div>
        {#if loaders.length}
          <div class="field">
            <label for="up-loader">Loader version <span class="faint">(optional)</span></label>
            <select id="up-loader" class="select" bind:value={loaderVer} disabled={live}>
              <option value="">Automatic{loaders.find((l) => l.recommended) ? ` (${loaders.find((l) => l.recommended)!.id})` : ''}</option>
              {#each loaders as l (l.id)}<option value={l.id}>{l.id}{l.recommended ? ' (recommended)' : ''}{l.id === s.loader_version ? ' (current)' : ''}</option>{/each}
            </select>
            <span class="hint">Leave on automatic to use the recommended build for this game version.</span>
          </div>
        {/if}
      </div>
      <div class="row">
        <button class="btn primary" class:spin={planning} disabled={planning || live || !target || unchanged} onclick={runPlan}><Icon name="search" size={15} />{planning ? 'Checking…' : 'Check upgrade'}</button>
        {#if unchanged}<span class="xs muted">Pick a different version or loader build to upgrade.</span>{/if}
      </div>
    {/if}
  </div>

  <div aria-live="polite" class="results">
    {#if planErr}<div class="banner danger" role="alert"><Icon name="x-circle" size={16} /><div class="grow">{planErr}</div><button class="btn sm" onclick={runPlan}>Retry</button></div>{/if}
    {#if planning}<div class="skel" style="height:120px"></div>{/if}

    {#if plan}
      <div class="card form rise">
        <div class="row wrap between">
          <h2>Upgrade plan</h2>
          <span class="badge {plan.can_proceed ? 'ok' : 'danger'}">{plan.can_proceed ? 'Ready to upgrade' : 'Blocked'}</span>
        </div>

        {#each plan.blockers as b}<div class="banner danger"><Icon name="x-circle" size={16} /><div><b>Blocker</b> · {b}</div></div>{/each}

        <div class="cards">
          <div class="mini"><div class="xs muted">Game version</div><div class="v tnum">{plan.from.game_version} <Icon name="chevron-right" size={13} /> {plan.to.game_version}</div>{#if downgrade}<span class="badge warn">downgrade</span>{/if}</div>
          {#if plan.from.loader_version || plan.to.loader_version}
            <div class="mini"><div class="xs muted">Loader</div><div class="v tnum">{plan.from.loader_version ?? '—'} <Icon name="chevron-right" size={13} /> {plan.to.loader_version ?? '—'}</div></div>
          {/if}
          <div class="mini"><div class="xs muted">Java</div><div class="v tnum">{plan.java.from} <Icon name="chevron-right" size={13} /> {plan.java.to}</div>
            {#if plan.java.from === plan.java.to}<span class="badge ok">no change</span>{:else if plan.java.installed}<span class="badge ok">installed</span>{:else}<span class="badge warn">not installed</span>{/if}</div>
          <div class="mini"><div class="xs muted">Server jar</div><div class="v">{plan.server_jar.available ? 'Available' : 'Not available'}</div>
            <span class="badge {plan.server_jar.available ? 'ok' : 'danger'}">{plan.server_jar.available ? 'ready' : 'missing'}</span></div>
        </div>
        {#if plan.server_jar.note}<p class="sm muted">{plan.server_jar.note}</p>{/if}
        {#if !plan.java.installed && plan.java.from !== plan.java.to}
          <div class="banner warn" style="align-items:center"><Icon name="alert" size={16} /><div class="grow">Java {plan.java.to} is required and is not installed yet.</div><button class="btn sm" onclick={installJava}>Install Java {plan.java.to}</button></div>
        {/if}
        {#each plan.warnings as w}<div class="banner warn"><Icon name="alert" size={16} />{w}</div>{/each}

        {#if plan.content.length}
          <div>
            <div class="row between wrap" style="margin-bottom:8px">
              <h3>Add-ons <span class="muted sm">({plan.content.length})</span></h3>
              <div class="row g6 wrap">
                {#if counts}{#each kinds as k}{#if counts[k]}<span class="badge {statusTone[k]}">{counts[k]} {statusText[k].toLowerCase()}</span>{/if}{/each}{/if}
              </div>
            </div>
            <div class="table-wrap">
              <table>
                <thead><tr><th>Name</th><th>Status</th><th>Version</th><th>Reason</th></tr></thead>
                <tbody>
                  {#each shownRows as r (r.item)}
                    <tr>
                      <td><div class="ellipsis nm" title={r.title}><b>{r.title}</b></div></td>
                      <td><span class="badge {statusTone[r.status]}">{statusText[r.status]}</span></td>
                      <td class="tnum">{r.current ?? '—'}{#if r.target && r.target !== r.current}<span class="faint">&nbsp;→&nbsp;</span>{r.target}{/if}</td>
                      <td class="muted why">{r.reason ?? ''}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
            {#if counts && counts.ok && rows.some((r) => r.status !== 'ok')}
              <button class="btn ghost sm" style="margin-top:6px" aria-expanded={showOk} onclick={() => (showOk = !showOk)}>{showOk ? 'Hide' : 'Show'} {counts.ok} compatible</button>
            {/if}
          </div>

          <fieldset class="strat">
            <legend class="label">What should happen to your add-ons?</legend>
            {#each strategies as st}
              <label class="opt"><input type="radio" name="strategy" value={st.id} bind:group={strategy} disabled={live} /><span><b>{st.label}</b><span class="xs muted">{st.text}</span></span></label>
            {/each}
          </fieldset>
        {/if}

        <div class="banner info"><Icon name="archive" size={16} /><div>A backup is taken automatically before anything changes. If the upgrade goes wrong, restore it from the <a href={serverHref(s.id, 'backups')}>Backups</a> tab.</div></div>

        <div class="confirm" class:ok={ready}>
          <h3>Confirm</h3>
          <ul class="checks">
            <li class:bad={active}><Icon name={active ? 'x-circle' : 'check-circle'} size={16} />
              {#if active}The server must be stopped first (it is {s.state}).{#if s.state === 'running' || s.state === 'starting'}<button class="btn sm danger" disabled={stopping} onclick={stop}><Icon name="stop" size={12} />{stopping ? 'Stopping…' : 'Stop server'}</button>{/if}{:else}The server is stopped.{/if}
            </li>
            <li class:bad={!plan.can_proceed}><Icon name={plan.can_proceed ? 'check-circle' : 'x-circle'} size={16} />{plan.can_proceed ? 'No blockers.' : 'Resolve the blockers above to continue.'}</li>
          </ul>
          {#if downgrade}
            <label class="row ack"><input type="checkbox" class="check" bind:checked={ack} /><span class="sm"><b>This is a downgrade.</b> Worlds saved by a newer version may be corrupted or fail to load. I have a backup and want to continue.</span></label>
          {/if}
          {#if startErr}<div class="banner danger" role="alert"><Icon name="x-circle" size={16} />{startErr}</div>{/if}
          <div class="row end g6">{#if canTry}<button class="btn lg" disabled={!tryable} onclick={() => (testing = true)} title="Starts a temporary copy of this server, upgrades the copy and checks that it starts">Try upgrade on a copy first</button>{/if}<button class="btn primary lg" disabled={!ready} onclick={confirm}>{busy ? 'Starting…' : `Upgrade to ${plan.to.game_version}`}</button></div>
        </div>
      </div>
    {/if}

    {#if op}
      <div class="card form rise">
        <OpCard {op} open />
        {#if op.state === 'completed'}
          <div class="banner"><span style="color:var(--a-text)"><Icon name="check-circle" size={18} /></span><div>Upgrade finished. Start the server and watch the console for the first boot.{#if backupId} A safety backup was taken first: <code>{backupId}</code>.{/if}</div></div>
        {:else if backupId || op.state === 'failed'}
          <div class="banner info"><Icon name="undo" size={16} /><div><b>Roll back</b> · {#if backupId}restore backup <code>{backupId}</code>{:else}restore the pre-upgrade backup{/if} from the <a href={serverHref(s.id, 'backups')}>Backups</a> tab (In place) to return to {s.runtime} {plan?.from.game_version ?? 'the previous version'}.</div></div>
        {/if}
      </div>
    {/if}
  </div>
</div>

{#if testing && plan}
  <StagedTest server={s} request={{ kind: 'upgrade', upgrade: { game_version: plan.to.game_version, loader_version: loaderVer || plan.to.loader_version || undefined, content: strategy, ...(downgrade ? { confirm_downgrade: true } : {}) } }} title="Try the upgrade to {plan.to.game_version} on a copy first" applyLabel="Upgrade the real server" onclose={() => (testing = false)} onpromoted={(o) => (opId = o.id)} />
{/if}

<style>
  .page { display: flex; flex-direction: column; gap: 16px; max-width: 920px; }
  .results { display: flex; flex-direction: column; gap: 16px; }
  .form { display: flex; flex-direction: column; gap: 16px; }
  .grid2 { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 16px; }
  .tog { cursor: pointer; align-items: center; }
  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 10px; }
  .mini { padding: 10px 12px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); display: flex; flex-direction: column; gap: 4px; align-items: flex-start; min-width: 0; }
  .mini .v { font-weight: 650; display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
  .nm { max-width: 240px; }
  .why { white-space: normal; min-width: 200px; font-size: var(--fs-sm); padding-top: 6px; padding-bottom: 6px; }
  td { height: auto; min-height: var(--row); padding-top: 9px; padding-bottom: 9px; }
  .strat { border: 0; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .opt { display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); cursor: pointer; }
  .opt:has(input:checked) { border-color: var(--a2); background: var(--a-soft); }
  .opt span { display: flex; flex-direction: column; }
  .opt input { margin-top: 3px; accent-color: var(--a2); }
  .confirm { border: 1px solid var(--line2); border-radius: var(--r); padding: 14px; display: flex; flex-direction: column; gap: 10px; background: var(--s1); }
  .confirm.ok { border-color: color-mix(in srgb, var(--a2), transparent 40%); }
  .checks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .checks li { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: var(--fs-sm); color: var(--a-text); }
  .checks li.bad { color: var(--danger); }
  .ack { gap: 10px; align-items: flex-start; padding: 10px 12px; background: var(--warn-soft); border: 1px solid color-mix(in srgb, var(--warn), transparent 65%); border-radius: var(--r-sm); cursor: pointer; }
  .ack input { margin-top: 2px; }
</style>
