<script lang="ts">
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import { bytes } from '../lib/format';
  import type { ChangePlan } from '../lib/api';
  // "Review changes" modal shared by install / update / remove flows.
  let { plan, title = 'Review changes', confirmLabel = 'Apply changes', busy = false, onconfirm, oncancel }: {
    plan: ChangePlan; title?: string; confirmLabel?: string; busy?: boolean; onconfirm: () => void; oncancel: () => void;
  } = $props();
  const tone = { install: 'ok', replace: 'warn', remove: 'danger' } as const;
  const sign = { install: '+', replace: '↻', remove: '−' } as const;
</script>

<Modal {title} onclose={oncancel} width={560}>
  <p class="muted sm">Nothing changes until you apply. Touched files are backed up, so you can undo afterwards.</p>
  <ul class="items">
    {#each plan.items as it}
      <li>
        <span class="badge {tone[it.action]}">{sign[it.action]} {it.action}</span>
        <div class="grow">
          <div class="ellipsis"><b>{it.title}</b></div>
          <div class="xs faint mono ellipsis">{it.filename}{#if it.size} · {bytes(it.size)}{/if}</div>
          <div class="xs muted">{it.reason}</div>
        </div>
      </li>
    {/each}
  </ul>
  {#each plan.warnings as w}<div class="banner warn mt-s"><Icon name="alert" size={16} />{w}</div>{/each}
  {#if plan.requires_restart}<div class="banner info mt-s"><Icon name="refresh" size={16} />A server restart is required for this to take effect.</div>{/if}
  {#snippet footer()}
    <button class="btn" onclick={oncancel}>Cancel</button>
    <button class="btn primary" disabled={busy} onclick={onconfirm} data-autofocus>{confirmLabel}</button>
  {/snippet}
</Modal>

<style>
  .items { list-style: none; margin: 12px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  li { display: flex; gap: 12px; align-items: flex-start; padding: 10px 12px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); }
</style>
