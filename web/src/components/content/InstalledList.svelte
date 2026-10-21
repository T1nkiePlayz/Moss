<script lang="ts">
  import Icon from '../Icon.svelte';
  import { pageUrl, sourceBadge, sevLabel, worstSeverity } from '../../lib/content';
  import { relTime } from '../../lib/format';
  import type { ContentItem } from '../../lib/api';

  let { items, updates, canManage, toggling, identifying, onopen, ontoggle, onbulktoggle, onbulkremove, onupdate, onpin, onremove, onidentify }: {
    items: ContentItem[]; updates: Record<string, { latest: string; version_id: string }>; canManage: boolean; toggling: Record<string, boolean>; identifying: boolean;
    onopen: (it: ContentItem) => void; ontoggle: (it: ContentItem, enabled: boolean) => void; onbulktoggle: (its: ContentItem[], enabled: boolean) => void;
    onbulkremove: (its: ContentItem[]) => void; onupdate: (it: ContentItem) => void; onpin: (it: ContentItem) => void; onremove: (it: ContentItem) => void; onidentify: () => void;
  } = $props();

  let sel = $state<Record<string, boolean>>({});
  const chosen = $derived(items.filter((i) => sel[i.id]));
  const unknown = $derived(items.filter((i) => !i.identified).length);
  const allOn = $derived(items.length > 0 && chosen.length === items.length);
  let master = $state<HTMLInputElement>();
  $effect(() => { if (master) master.indeterminate = chosen.length > 0 && !allOn; });
  // Drop selections for items that disappeared (removed, reloaded).
  $effect(() => { const ids = new Set(items.map((i) => i.id)); for (const k of Object.keys(sel)) if (!ids.has(k)) delete sel[k]; });
  const all = (on: boolean) => { sel = on ? Object.fromEntries(items.map((i) => [i.id, true])) : {}; };
  const done = (fn: () => void) => { fn(); sel = {}; };
</script>

<div class="bulk" role="toolbar" aria-label="Bulk actions">
  {#if chosen.length}
    <span class="sm" aria-live="polite"><b class="tnum">{chosen.length}</b> selected</span>
    {#if canManage}
      <button class="btn sm" disabled={!chosen.some((i) => i.enabled)} onclick={() => done(() => onbulktoggle(chosen.filter((i) => i.enabled), false))}>Disable</button>
      <button class="btn sm" disabled={!chosen.some((i) => !i.enabled)} onclick={() => done(() => onbulktoggle(chosen.filter((i) => !i.enabled), true))}>Enable</button>
      <button class="btn sm danger" onclick={() => done(() => onbulkremove(chosen))}><Icon name="trash" size={13} />Remove</button>
    {/if}
    <button class="btn sm ghost" onclick={() => all(false)}>Clear</button>
  {:else}
    <span class="sm muted">{items.length} installed{unknown ? ` · ${unknown} not identified` : ''}</span>
  {/if}
  <span class="grow"></span>
  {#if canManage}
    <button class="btn sm" class:spin={identifying} disabled={identifying || !unknown} title={unknown ? 'Match unknown files against Modrinth and CurseForge by hash (Hangar has no hash lookup) so they can be updated' : 'Every file is already identified'} onclick={onidentify}><Icon name="search" size={13} />{identifying ? 'Identifying…' : `Identify unknown files${unknown ? ` (${unknown})` : ''}`}</button>
  {/if}
</div>

<div class="table-wrap">
  <table>
    <thead>
      <tr>
        <th class="ck"><input class="check" type="checkbox" bind:this={master} checked={allOn} aria-label="Select all" onchange={(e) => all(e.currentTarget.checked)} /></th>
        {#if canManage}<th class="swc">On</th>{/if}
        <th>Name</th><th>Source</th><th>Version</th><th class="hide-m">Loader</th><th class="hide-l">Installed</th><th></th>
      </tr>
    </thead>
    <tbody>
      {#each items as it (it.id)}
        {@const sb = sourceBadge(it)}
        {@const sev = worstSeverity(it.issues)}
        {@const url = pageUrl(it)}
        <tr class:off={!it.enabled} class:picked={sel[it.id]}>
          <td class="ck"><input class="check" type="checkbox" bind:checked={sel[it.id]} aria-label="Select {it.title}" /></td>
          {#if canManage}
            <td class="swc"><input class="switch" type="checkbox" role="switch" checked={it.enabled} disabled={toggling[it.id]} aria-label="{it.enabled ? 'Disable' : 'Enable'} {it.title}" onchange={(e) => ontoggle(it, e.currentTarget.checked)} /></td>
          {/if}
          <td>
            <div class="nm">
              <b class="ellipsis" title={it.title}>{it.title}</b>
              {#if !it.enabled}<span class="badge">Disabled</span>{/if}
              {#if sev}
                <button class="iss {sev}" aria-label="{it.issues.length} issue{it.issues.length === 1 ? '' : 's'}, {sevLabel[sev].toLowerCase()}: {it.issues.map((i) => i.title).join('; ')}" title={it.issues.map((i) => i.title).join('\n')} onclick={() => onopen(it)}><Icon name={sev === 'critical' ? 'x-circle' : sev === 'warn' ? 'alert' : 'info'} size={15} /></button>
              {/if}
            </div>
            <div class="xs faint mono ellipsis fn">{it.filename}{it.enabled ? '' : '.disabled'}</div>
          </td>
          <td><span class="badge {sb.tone}" title={sb.hint}>{sb.label}</span></td>
          <td class="tnum">{it.version_number ?? it.meta?.version ?? '—'}{#if updates[it.id]}<span class="badge info" style="margin-left:6px">→ {updates[it.id].latest}</span>{/if}</td>
          <td class="hide-m">
            {#if it.meta}<span class="row g4"><span class="tag">{it.meta.loader}</span>{#if it.meta.environment !== 'unknown'}<span class="tag" title={it.meta.environment === 'client' ? 'Client-only: does nothing on a server' : ''}>{it.meta.environment === 'both' ? 'client+server' : it.meta.environment}</span>{/if}</span>
            {:else}<span class="faint">—</span>{/if}
          </td>
          <td class="muted hide-l">{relTime(it.installed_at)}</td>
          <td class="num">
            <span class="row g4 acts">
              {#if canManage && updates[it.id] && !it.pinned}<button class="btn sm primary" onclick={() => onupdate(it)}>Update</button>{/if}
              {#if canManage && it.provider === 'modrinth'}
                <button class="btn ghost icon sm" aria-pressed={it.pinned} aria-label="{it.pinned ? 'Unpin' : 'Pin'} {it.title}" title={it.pinned ? 'Pinned: updates skip this' : 'Pin version'} onclick={() => onpin(it)} style={it.pinned ? 'color:var(--a-text)' : ''}><Icon name="pin" size={14} /></button>
              {/if}
              <button class="btn ghost icon sm" aria-label="View metadata for {it.title}" title="View metadata" onclick={() => onopen(it)}><Icon name="info" size={14} /></button>
              {#if url}<a class="btn ghost icon sm" href={url} target="_blank" rel="noreferrer noopener" aria-label="Open {it.title} page (new tab)" title="Open project page"><Icon name="external" size={14} /></a>{/if}
              {#if canManage}<button class="btn ghost icon sm" aria-label="Remove {it.title}" title="Remove" onclick={() => onremove(it)}><Icon name="trash" size={14} /></button>{/if}
            </span>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .bulk { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-height: 40px; padding: 0 2px 8px; }
  .bulk .btn { flex: none; }
  .ck, .swc { width: 1%; padding-right: 0; }
  td.ck, td.swc { padding-left: 14px; }
  th.ck { padding-left: 14px; padding-right: 0; } th.swc { padding-left: 14px; padding-right: 0; }
  .nm { display: flex; align-items: center; gap: 6px; max-width: 280px; }
  .nm b { min-width: 0; }
  .fn { max-width: 280px; }
  tr.off .nm b, tr.off td:nth-child(n+4) { opacity: .9; } tr.off .nm b { text-decoration: line-through; text-decoration-thickness: 1px; }
  tr.off .nm b { text-decoration: line-through; text-decoration-color: var(--faint); }
  tr.picked { background: var(--a-soft); }
  .iss { display: inline-grid; place-items: center; width: 24px; height: 24px; border: 0; background: none; border-radius: 6px; cursor: pointer; flex: none; padding: 0; }
  .iss.critical { color: var(--danger); } .iss.warn { color: var(--warn); } .iss.info { color: var(--info); }
  .iss:hover { background: var(--s3); }
  .acts { justify-content: flex-end; }
  @media (max-width: 1180px) { .hide-l { display: none; } }
  @media (max-width: 860px) { .hide-m { display: none; } }
</style>
