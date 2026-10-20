<script lang="ts">
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import FindingList from './content/FindingList.svelte';
  import { bytes } from '../lib/format';
  import { criticalConflicts, versionDiff } from '../lib/content';
  import type { ChangePlan } from '../lib/api';
  // "Review changes" modal shared by install / update / remove flows.
  let { plan, title = 'Review changes', confirmLabel = 'Apply changes', busy = false, alt, onconfirm, oncancel }: {
    plan: ChangePlan; title?: string; confirmLabel?: string; busy?: boolean; alt?: { label: string; run: () => void }; onconfirm: () => void; oncancel: () => void;
  } = $props();
  const tone = { install: 'ok', replace: 'warn', remove: 'danger' } as const;
  const sign = { install: '+', replace: '↻', remove: '−' } as const;
  const conflicts = $derived(plan.conflicts ?? []);
  const critical = $derived(criticalConflicts(plan));
  const minor = $derived(conflicts.filter((c) => c.severity !== 'critical'));
  let ack = $state(false);
  const blocked = $derived(critical.length > 0 && !ack);
</script>

<Modal {title} onclose={oncancel} width={600}>
  <p class="muted sm">Nothing changes until you apply. Touched files are backed up, so you can undo afterwards.</p>
  {#if critical.length}
    <section class="blocker" role="alert" aria-label="Blocking problems">
      <div class="bh"><Icon name="x-circle" size={18} /><b>This change would break things</b></div>
      <p class="sm">{critical.length === 1 ? 'A critical conflict was' : `${critical.length} critical conflicts were`} found. The server may fail to start if you continue.</p>
      <FindingList findings={critical} open />
      <label class="row ack"><input type="checkbox" class="check" bind:checked={ack} /><span class="sm">I understand and want to continue anyway.</span></label>
    </section>
  {/if}
  <ul class="items">
    {#each plan.items as it}
      {@const diff = versionDiff(it)}
      <li>
        <span class="badge {tone[it.action]}">{sign[it.action]} {it.action}</span>
        <div class="grow">
          <div class="ellipsis"><b>{it.title}</b>{#if diff}<span class="tnum diff">{diff}</span>{/if}</div>
          <div class="xs faint mono ellipsis">{it.filename}{#if it.size} · {bytes(it.size)}{/if}</div>
          <div class="xs muted">{it.reason}</div>
        </div>
      </li>
    {/each}
  </ul>
  {#if minor.length}
    <h4 class="sub">Compatibility notes</h4>
    <FindingList findings={minor} />
  {/if}
  {#each plan.warnings as w}<div class="banner warn mt-s"><Icon name="alert" size={16} />{w}</div>{/each}
  {#if plan.requires_restart}<div class="banner info mt-s"><Icon name="refresh" size={16} />A server restart is required for this to take effect.</div>{/if}
  {#snippet footer()}
    <button class="btn" onclick={oncancel}>Cancel</button>
    {#if alt}<button class="btn" disabled={busy} onclick={alt.run}>{alt.label}</button>{/if}
    <button class="btn {critical.length ? 'danger solid' : 'primary'}" disabled={busy || blocked} onclick={onconfirm} data-autofocus={critical.length ? undefined : ''}>{busy ? 'Working…' : critical.length ? `${confirmLabel} anyway` : confirmLabel}</button>
  {/snippet}
</Modal>

<style>
  .items { list-style: none; margin: 12px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  li { display: flex; gap: 12px; align-items: flex-start; padding: 10px 12px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); }
  .diff { margin-left: 8px; font-weight: 500; font-size: var(--fs-xs); color: var(--a-text); }
  .sub { margin: 14px 0 6px; font-size: var(--fs-sm); color: var(--muted); }
  .blocker { margin-top: 12px; padding: 12px; border-radius: var(--r); border: 2px solid var(--danger); background: var(--danger-soft); display: flex; flex-direction: column; gap: 8px; }
  .bh { display: flex; align-items: center; gap: 8px; color: var(--danger); }
  .ack { gap: 10px; cursor: pointer; padding-top: 4px; }
</style>
