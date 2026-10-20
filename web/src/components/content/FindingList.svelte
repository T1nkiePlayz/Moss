<script lang="ts">
  import Icon from '../Icon.svelte';
  import type { ContentFinding } from '../../lib/api';
  import { sevLabel, sortFindings } from '../../lib/content';
  // Expandable list of content findings with their evidence and one action button each.
  let { findings, onaction, busy = null, open = false, block = false, itemName }: {
    findings: ContentFinding[]; onaction?: (f: ContentFinding) => void; busy?: string | null; open?: boolean; block?: boolean; itemName?: (id: string) => string | undefined;
  } = $props();
  // svelte-ignore state_referenced_locally
  let expanded = $state<Record<string, boolean>>(open ? Object.fromEntries(findings.map((f) => [f.id, true])) : {});
  const icon = { critical: 'x-circle', warn: 'alert', info: 'info' } as const;
  const sorted = $derived(sortFindings(findings));
</script>

<ul class="fl">
  {#each sorted as f (f.id)}
    {@const on = !!expanded[f.id]}
    {@const label = f.item ? itemName?.(f.item) : undefined}
    <li class="f {f.severity}" class:block={block && f.severity === 'critical'}>
      <div class="head">
        <span class="sev"><Icon name={icon[f.severity]} size={16} label={sevLabel[f.severity]} /></span>
        <button class="t" aria-expanded={on} aria-controls="fd-{f.id}" onclick={() => (expanded[f.id] = !on)}>
          <span class="title">{f.title}</span>
          {#if label}<span class="xs faint ellipsis">{label}</span>{/if}
          <span class="chev"><Icon name={on ? 'chevron-down' : 'chevron-right'} size={14} /></span>
        </button>
        {#if f.action && onaction}
          <button class="btn sm {f.severity === 'critical' ? 'danger' : ''}" disabled={busy === f.id} onclick={() => onaction(f)}>{busy === f.id ? 'Working…' : f.action.label}</button>
        {/if}
      </div>
      {#if on}
        <div class="body" id="fd-{f.id}">
          <p class="sm muted">{f.detail}</p>
          {#if f.evidence.length}
            <ul class="ev mono xs">{#each f.evidence as e}<li>{e}</li>{/each}</ul>
          {/if}
        </div>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .fl { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .f { border: 1px solid var(--line); border-left-width: 3px; border-radius: var(--r-sm); background: var(--s2); }
  .f.critical { border-left-color: var(--danger); } .f.warn { border-left-color: var(--warn); } .f.info { border-left-color: var(--info); }
  .f.block { background: var(--danger-soft); border-color: color-mix(in srgb, var(--danger), transparent 40%); border-left-color: var(--danger); border-left-width: 5px; }
  .head { display: flex; align-items: center; gap: 8px; padding: 6px 8px 6px 10px; min-height: 40px; }
  .sev { display: grid; flex: none; }
  .critical .sev { color: var(--danger); } .warn .sev { color: var(--warn); } .info .sev { color: var(--info); }
  .t { flex: 1 1 150px; min-width: 0; display: flex; align-items: center; gap: 8px; background: none; border: 0; padding: 4px 2px; text-align: left; cursor: pointer; color: inherit; border-radius: 6px; }
  .title { font-weight: 600; font-size: var(--fs-sm); min-width: 0; overflow-wrap: anywhere; }
  .t .ellipsis { max-width: 160px; flex: none; }
  .chev { margin-left: auto; color: var(--faint); display: grid; flex: none; }
  .body { padding: 0 12px 10px 36px; display: flex; flex-direction: column; gap: 6px; }
  .ev { margin: 0; padding: 8px 10px; list-style: none; background: var(--bg); border-radius: 8px; color: var(--muted); display: flex; flex-direction: column; gap: 2px; overflow-wrap: anywhere; }
  @media (max-width: 560px) { .head { flex-wrap: wrap; } .t .ellipsis { display: none; } }
</style>
