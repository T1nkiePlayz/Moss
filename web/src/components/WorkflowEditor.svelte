<script lang="ts">
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import States from './States.svelte';
  import { apiOps, ApiError, errMsg, type Workflow, type WebhookEventType, type WorkflowAction, type WorkflowPlan, type WorkflowRun } from '../lib/api';
  import { ACTION_TYPES, CONDITION_FIELDS, CONDITION_OPS, MAX_ACTIONS, MAX_WAIT, PLACEHOLDERS, blankAction, buildCondition, hasServer, move, workflowProblems } from '../lib/workflows';
  import { servers, toast } from '../lib/stores.svelte';
  import { dateTime } from '../lib/format';

  let { wf = null, types, onclose, ondone }: { wf?: Workflow | null; types: WebhookEventType[]; onclose: () => void; ondone: () => void } = $props();
  const w0 = untrack(() => wf);
  const editing = !!w0;
  let name = $state(w0?.name ?? '');
  let trigger = $state(w0?.trigger ?? 'server.crashed');
  let scope = $state<'all' | 'some'>(w0?.server_ids.length ? 'some' : 'all');
  let serverIds = $state<string[]>([...(w0?.server_ids ?? [])]);
  let cond = $state({ on: !!w0?.condition, field: w0?.condition?.field ?? 'players.online', op: w0?.condition?.op ?? '>=', value: String(w0?.condition?.value ?? '') });
  let actions = $state<WorkflowAction[]>(w0 ? structuredClone($state.snapshot(w0.actions)) : [blankAction('restart')]);
  let newType = $state<WorkflowAction['type']>('command');
  let busy = $state(false);
  let shown = $state(false);
  let server = $state<Record<string, string>>({});
  let top = $state('');
  let plan = $state<WorkflowPlan | null>(null);
  let planning = $state(false);
  let runs = $state<WorkflowRun[] | null>(null);

  const body = $derived({ name: name.trim(), trigger, server_ids: scope === 'all' ? [] : serverIds, condition: buildCondition(cond), actions: $state.snapshot(actions) as WorkflowAction[] });
  const local = $derived(shown ? workflowProblems(body) : {});
  const msg = (k: string) => local[k] || server[k] || '';
  const toggle = (list: string[], v: string, on: boolean) => (on ? [...list.filter((x) => x !== v), v] : list.filter((x) => x !== v));
  const describe = $derived(types.find((t) => t.type === trigger)?.description ?? '');
  const aboutServer = $derived(trigger !== 'update.available');

  function retype(i: number, t: string) {
    const old = actions[i], nu = blankAction(t as WorkflowAction['type']);
    if (hasServer(old) && hasServer(nu)) nu.server = old.server;
    actions[i] = nu;
  }
  async function loadRuns() { if (w0) { try { runs = (await apiOps.workflowRuns(w0.id)).items; } catch { runs = []; } } }
  void loadRuns();

  async function save(e: Event) {
    e.preventDefault(); shown = true; server = {}; top = '';
    if (Object.keys(workflowProblems(body)).length) return;
    if (scope === 'some' && !serverIds.length) { server.server_ids = 'Choose at least one server, or pick any server.'; return; }
    busy = true;
    try {
      if (w0) { await apiOps.patchWorkflow(w0.id, body); toast('success', 'Workflow saved'); }
      else { await apiOps.addWorkflow(body); toast('success', 'Workflow added'); }
      ondone();
    } catch (er) {
      if (er instanceof ApiError && er.fields.length) { for (const x of er.fields) server[x.path] = x.message; } else top = errMsg(er);
    }
    busy = false;
  }
  async function dryRun() {
    if (!w0) return;
    planning = true; plan = null; top = '';
    try { plan = await apiOps.testWorkflow(w0.id, serverIds.length ? { server_id: serverIds[0] } : {}); } catch (er) { top = errMsg(er); }
    planning = false;
  }
</script>

<Modal title={editing ? 'Edit workflow' : 'Add a workflow'} {onclose} width={760}>
  <form id="wff" class="col" onsubmit={save} novalidate>
    <div class="field"><label for="wf-name">Name</label><input id="wf-name" class="input" bind:value={name} maxlength="60" placeholder="Welcome new players, restart after a crash…" aria-invalid={!!msg('name')} data-autofocus />{#if msg('name')}<span class="err" role="alert">{msg('name')}</span>{/if}</div>

    <div class="field"><label for="wf-trig">When this happens</label>
      <select id="wf-trig" class="select" bind:value={trigger} aria-describedby="wf-trig-h" aria-invalid={!!msg('trigger')}>{#each types as t}<option value={t.type}>{t.type}</option>{/each}</select>
      <span class="hint" id="wf-trig-h">{describe}</span>{#if msg('trigger')}<span class="err" role="alert">{msg('trigger')}</span>{/if}</div>

    <fieldset class="grp">
      <legend class="label">On which servers</legend>
      <div class="row g6" role="radiogroup" aria-label="Which servers">
        <button type="button" class="chip" role="radio" aria-checked={scope === 'all'} onclick={() => (scope = 'all')}>Any server I can manage</button>
        <button type="button" class="chip" role="radio" aria-checked={scope === 'some'} onclick={() => (scope = 'some')}>Only some</button>
      </div>
      {#if scope === 'some'}
        <div class="col" style="margin-top:8px">{#each servers.list as s (s.id)}<label class="row g8"><input type="checkbox" class="check" checked={serverIds.includes(s.id)} onchange={(e) => (serverIds = toggle(serverIds, s.id, e.currentTarget.checked))} /><span>{s.name}</span></label>{/each}</div>
      {/if}
      {#if !aboutServer}<span class="hint">This event is not about one server, so it only triggers workflows that cover any server.</span>{/if}
      {#if msg('server_ids')}<span class="err" role="alert">{msg('server_ids')}</span>{/if}
    </fieldset>

    <fieldset class="grp">
      <legend class="label">Only if (optional)</legend>
      <label class="row g8"><input type="checkbox" class="check" bind:checked={cond.on} /><span>Check a detail of the event first</span></label>
      {#if cond.on}
        <div class="row g8 wrap" style="margin-top:8px">
          <label class="sr" for="wf-cf">Detail to check</label>
          <select id="wf-cf" class="select" style="width:auto" bind:value={cond.field}>{#each CONDITION_FIELDS as f}<option value={f.value}>{f.label}</option>{/each}</select>
          <label class="sr" for="wf-co">Comparison</label>
          <select id="wf-co" class="select" style="width:auto" bind:value={cond.op}>{#each CONDITION_OPS as o}<option value={o.value}>{o.label}</option>{/each}</select>
          <label class="sr" for="wf-cv">Value</label>
          <input id="wf-cv" class="input" style="width:160px" bind:value={cond.value} aria-invalid={!!msg('condition')} />
        </div>
        {#if msg('condition')}<span class="err" role="alert">{msg('condition')}</span>{/if}
      {/if}
    </fieldset>

    <fieldset class="grp">
      <legend class="label">Then do, in order</legend>
      <ol class="acts">
        {#each actions as a, i (i)}
          <li>
            <div class="row g8 wrap">
              <span class="tnum faint" aria-hidden="true">{i + 1}.</span>
              <label class="sr" for="wf-at{i}">Action {i + 1}</label>
              <select id="wf-at{i}" class="select" style="width:auto" value={a.type} onchange={(e) => retype(i, e.currentTarget.value)}>{#each ACTION_TYPES as t}<option value={t.type}>{t.label}</option>{/each}</select>
              {#if hasServer(a)}
                <label class="sr" for="wf-as{i}">Server for action {i + 1}</label>
                <select id="wf-as{i}" class="select" style="width:auto" value={a.server} onchange={(e) => ((actions[i] as { server: string }).server = e.currentTarget.value)}>
                  {#if aboutServer}<option value="trigger">The server in the event</option>{/if}
                  {#each servers.list as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
                </select>
              {/if}
              <span class="grow"></span>
              <button type="button" class="btn ghost icon sm" aria-label={`Move action ${i + 1} up`} disabled={i === 0} onclick={() => (actions = move(actions, i, -1))}><Icon name="arrow-up" size={14} /></button>
              <button type="button" class="btn ghost icon sm" aria-label={`Move action ${i + 1} down`} disabled={i === actions.length - 1} onclick={() => (actions = move(actions, i, 1))}><Icon name="chevron-down" size={14} /></button>
              <button type="button" class="btn ghost icon sm" aria-label={`Remove action ${i + 1}`} onclick={() => (actions = actions.filter((_, j) => j !== i))}><Icon name="trash" size={14} /></button>
            </div>
            {#if a.type === 'command'}
              <label class="sr" for="wf-ac{i}">Console command for action {i + 1}</label>
              <input id="wf-ac{i}" class="input mono" bind:value={a.command} maxlength="200" placeholder="say Welcome {'{player}'}!" autocomplete="off" spellcheck="false" aria-invalid={!!msg(`actions[${i}]`)} />
              <span class="hint">You can use {PLACEHOLDERS.join(' ')}. Commands are recorded in the activity log like any other console command.</span>
            {:else if a.type === 'wait'}
              <div class="row g8"><label for="wf-aw{i}">Seconds (1 to {MAX_WAIT})</label><input id="wf-aw{i}" class="input" type="number" min="1" max={MAX_WAIT} style="width:100px" bind:value={a.seconds} aria-invalid={!!msg(`actions[${i}]`)} /></div>
            {:else if a.type === 'notify'}
              <label class="sr" for="wf-an{i}">Notification title for action {i + 1}</label>
              <input id="wf-an{i}" class="input" bind:value={a.title} maxlength="100" placeholder="Title, e.g. {'{server}'} crashed" aria-invalid={!!msg(`actions[${i}]`)} />
              <label class="sr" for="wf-ab{i}">Notification text for action {i + 1}</label>
              <input id="wf-ab{i}" class="input" bind:value={a.body} maxlength="500" placeholder="Text (optional)" />
            {:else if a.type === 'webhook'}
              <label class="sr" for="wf-au{i}">Web address for action {i + 1}</label>
              <input id="wf-au{i}" class="input mono" type="url" bind:value={a.url} placeholder="https://example.com/hook" autocomplete="off" spellcheck="false" aria-invalid={!!msg(`actions[${i}]`)} />
              <span class="hint">Moss sends the event as JSON to this HTTPS address.</span>
            {:else if a.type === 'backup'}
              <span class="hint">The next action starts when the backup has finished.</span>
            {/if}
            {#if msg(`actions[${i}]`)}<span class="err" role="alert">{msg(`actions[${i}]`)}</span>{/if}
          </li>
        {/each}
      </ol>
      {#if msg('actions')}<span class="err" role="alert">{msg('actions')}</span>{/if}
      {#if actions.length < MAX_ACTIONS}
        <div class="row g8" style="margin-top:8px">
          <label class="sr" for="wf-new">Kind of action to add</label>
          <select id="wf-new" class="select" style="width:auto" bind:value={newType}>{#each ACTION_TYPES as t}<option value={t.type}>{t.label}</option>{/each}</select>
          <button type="button" class="btn" onclick={() => (actions = [...actions, blankAction(newType)])}><Icon name="plus" size={14} />Add action</button>
        </div>
      {/if}
      <span class="hint">If an action fails, the ones after it do not run. A workflow runs with your permissions and is switched off if you lose access to a server it uses. At most 10 runs per 10 minutes.</span>
    </fieldset>

    {#if editing}
      <fieldset class="grp">
        <legend class="label">Dry run</legend>
        <div class="row g8 wrap">
          <button type="button" class="btn" onclick={dryRun} disabled={planning}>{planning ? 'Checking…' : 'Dry run with a sample event'}</button>
          <span class="hint">Shows what the saved workflow would do. Nothing is executed. Save first to include your edits.</span>
        </div>
        {#if plan}
          <div class="plan" role="status">
            <p class="sm"><b>{plan.would_run ? 'Would run.' : 'Would not run.'}</b> {plan.trigger_matches ? '' : 'The sample event is not covered by this workflow. '}{plan.condition_holds ? '' : 'The condition does not hold for the sample event. '}</p>
            <ol class="steps">{#each plan.steps as s}<li>{s.description}{#if !s.allowed} <span class="badge danger">{s.problem}</span>{/if}</li>{/each}</ol>
          </div>
        {/if}
      </fieldset>
      <fieldset class="grp">
        <legend class="label">Recent runs</legend>
        {#if !runs}<States kind="loading" rows={2} />
        {:else if !runs.length}<p class="muted sm">It has not run yet.</p>
        {:else}
          <ul class="runs" aria-label={`Recent runs of ${w0?.name}`}>{#each runs as r (r.id)}
            <li><div class="row g8 wrap"><span class="badge {r.ok ? 'ok' : 'danger'}">{r.ok ? 'Done' : 'Stopped'}</span><span class="tnum xs">{dateTime(r.at)}</span><code>{r.event.type}</code>{#if r.event.server}<span class="xs muted">{r.event.server.name}</span>{/if}</div>
              <div class="xs muted">{r.log.join(' · ')}</div></li>
          {/each}</ul>
        {/if}
      </fieldset>
    {/if}
    {#if top}<div class="banner danger" role="alert"><Icon name="alert" size={16} />{top}</div>{/if}
  </form>
  {#snippet footer()}
    <button class="btn" type="button" onclick={onclose}>Cancel</button>
    <button class="btn primary" type="submit" form="wff" disabled={busy}>{busy ? 'Saving…' : editing ? 'Save changes' : 'Add workflow'}</button>
  {/snippet}
</Modal>

<style>
  .grp { border: 0; padding: 0; margin: 0; min-width: 0; }
  .acts { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
  .acts li { display: flex; flex-direction: column; gap: 6px; padding: 10px; border: 1px solid var(--line); border-radius: 10px; }
  .plan { margin-top: 10px; padding: 10px; border: 1px solid var(--line); border-radius: 10px; }
  .steps { margin: 6px 0 0; padding-left: 20px; }
  .runs { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .runs li { overflow-wrap: anywhere; }
</style>
