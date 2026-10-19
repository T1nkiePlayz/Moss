<script lang="ts">
  import Icon from './Icon.svelte';
  import { relTime } from '../lib/format';
  import { api } from '../lib/api';
  import { serverById, toastError } from '../lib/stores.svelte';
  import type { Operation } from '../lib/api';
  let { op, open = false }: { op: Operation; open?: boolean } = $props();
  const live = $derived(op.state === 'queued' || op.state === 'running');
  const tone = $derived(op.state === 'completed' ? 'ok' : op.state === 'failed' ? 'danger' : op.state === 'running' ? 'info' : op.state === 'queued' ? '' : 'warn');
  const srv = $derived(op.server_id ? serverById(op.server_id) : null);
  // svelte-ignore state_referenced_locally
  let showLog = $state(open);
  async function cancel() { try { await api.cancelOp(op.id); } catch (e) { toastError(e, 'Cancel failed'); } }
</script>

<article class="op" class:failed={op.state === 'failed'}>
  <div class="row between" style="align-items:flex-start">
    <div class="grow">
      <div class="ellipsis"><b>{op.title}</b></div>
      <div class="xs muted">{#if srv}<a href="#/servers/{srv.id}">{srv.name}</a> · {/if}{relTime(op.started_at ?? op.created_at)}</div>
    </div>
    <span class="badge {tone}">{op.state}</span>
  </div>
  {#if live}
    <div class="row sm" style="margin-top:10px"><span class="grow ellipsis muted">{op.stage}</span>{#if op.progress != null}<span class="tnum">{Math.round(op.progress * 100)}%</span>{/if}</div>
    <div class="bar" class:indet={op.progress == null} role="progressbar" aria-label={op.title} aria-valuemin="0" aria-valuemax="100" aria-valuenow={op.progress == null ? undefined : Math.round(op.progress * 100)}>
      <i style={op.progress == null ? '' : `width:${Math.max(3, op.progress * 100)}%`}></i>
    </div>
  {:else if op.error}
    <div class="banner danger" style="margin-top:10px"><Icon name="x-circle" size={16} /><div>{op.error}<div class="xs muted">Retry by repeating the action{#if srv} from <a href="#/servers/{srv.id}">{srv.name}</a>{/if}; nothing was left half-applied.</div></div></div>
  {:else if op.state === 'interrupted'}
    <div class="banner warn" style="margin-top:10px"><Icon name="alert" size={16} />Interrupted by a restart of Moss. Repeat the action to continue.</div>
  {/if}
  <div class="row xs" style="margin-top:8px">
    {#if op.log.length}<button class="btn ghost sm" aria-expanded={showLog} onclick={() => (showLog = !showLog)}><Icon name={showLog ? 'chevron-down' : 'chevron-right'} size={13} />Log</button>{/if}
    <span class="grow"></span>
    {#if live && op.cancellable}<button class="btn sm danger" onclick={cancel}>Cancel</button>{/if}
  </div>
  {#if showLog && op.log.length}<pre class="log mono xs">{op.log.slice(-8).join('\n')}</pre>{/if}
</article>

<style>
  .op { padding: 12px 14px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r); }
  .op.failed { border-color: color-mix(in srgb, var(--danger), transparent 65%); }
  .log { margin: 6px 0 0; padding: 8px 10px; background: var(--bg); border-radius: 8px; color: var(--muted); max-height: 140px; overflow: auto; white-space: pre-wrap; word-break: break-word; }
</style>
