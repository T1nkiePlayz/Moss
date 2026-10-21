<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import States from '../components/States.svelte';
  import WorkflowEditor from '../components/WorkflowEditor.svelte';
  import { errMsg, type Workflow, type WebhookEventType } from '../lib/api';
import { apiOps } from '../lib/api-ops';
  import { confirmDialog, servers, toast, toastError } from '../lib/stores.svelte';
  import { relTime, dateTime } from '../lib/format';
  import { describeAction, scopeText, workflowStatus } from '../lib/workflows';

  let list = $state<Workflow[] | null>(null);
  let types = $state<WebhookEventType[]>([]);
  let err = $state('');
  let adding = $state(false);
  let editing = $state<Workflow | null>(null);

  async function load() { err = ''; try { const r = await apiOps.workflows(); list = r.items; types = r.event_types; } catch (e) { err = errMsg(e); } }
  void load();
  const names = (id: string) => servers.list.find((s) => s.id === id)?.name ?? 'a removed server';
  async function toggle(w: Workflow, on: boolean) {
    try { await apiOps.patchWorkflow(w.id, { enabled: on }); } catch (e) { toastError(e, 'Could not change the workflow'); }
    void load();
  }
  async function remove(w: Workflow) {
    const r = await confirmDialog({ title: `Remove ${w.name}?`, body: 'The workflow stops reacting to events and its run history is deleted.', confirmLabel: 'Remove workflow', danger: true });
    if (!r.ok) return;
    try { await apiOps.removeWorkflow(w.id); toast('info', 'Workflow removed'); void load(); } catch (e) { toastError(e, 'Could not remove'); }
  }
  const done = () => { adding = false; editing = null; void load(); };
</script>

<div class="page-head">
  <div><h1>Workflows</h1><p>When something happens, do something: restart after a crash, greet players, back up after a stop.</p></div>
  <button class="btn primary" onclick={() => (adding = true)} disabled={!types.length}><Icon name="plus" size={15} />Add workflow</button>
</div>

{#if err}<States kind="error" text={err} retry={load} />
{:else if !list}<States kind="loading" rows={3} />
{:else if !list.length}<States kind="empty" icon="zap" title="No workflows yet" text="A workflow reacts to an event, such as a crash or a player joining, and runs actions like restarting the server or sending a command." />
{:else}
  <ul class="wl">
    {#each list as w (w.id)}
      {@const s = workflowStatus(w)}
      <li class="card">
        <div class="row g8 wrap">
          <input type="checkbox" class="switch" role="switch" id={`wf-on-${w.id}`} checked={w.enabled} aria-label={`Run ${w.name}`} onchange={(e) => toggle(w, e.currentTarget.checked)} />
          <b>{w.name}</b><span class="badge {s.tone}">{s.label}</span>
          <span class="grow"></span>
          <button class="btn sm" onclick={() => (editing = w)}><Icon name="edit" size={13} />Edit</button>
          <button class="btn ghost icon sm" aria-label={`Remove ${w.name}`} onclick={() => remove(w)}><Icon name="trash" size={14} /></button>
        </div>
        <div class="xs muted" style="margin-top:6px">When <code>{w.trigger}</code> on {scopeText(w, names)}{#if w.condition} if {w.condition.field} {w.condition.op} {w.condition.value}{/if}</div>
        <ol class="xs steps">{#each w.actions as a}<li>{describeAction(a, names)}</li>{/each}</ol>
        <div class="xs muted">{#if w.last_run_at}Last run <span title={dateTime(w.last_run_at)}>{relTime(w.last_run_at)}</span>{#if w.last_result && w.last_result !== 'ok'}: {w.last_result}{/if}{:else}Has not run yet.{/if}</div>
        {#if w.disabled_reason}<div class="banner danger" role="status" style="margin-top:8px"><Icon name="alert" size={16} />{w.disabled_reason}</div>{/if}
      </li>
    {/each}
  </ul>
{/if}

{#if adding}<WorkflowEditor {types} onclose={() => (adding = false)} ondone={done} />{/if}
{#if editing}<WorkflowEditor wf={editing} {types} onclose={() => (editing = null)} ondone={done} />{/if}

<style>
  .wl { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 12px; }
  .steps { margin: 6px 0; padding-left: 20px; }
</style>
