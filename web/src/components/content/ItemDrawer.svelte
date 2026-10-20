<script lang="ts">
  import Modal from '../Modal.svelte';
  import Icon from '../Icon.svelte';
  import FindingList from './FindingList.svelte';
  import { pageUrl, sourceBadge } from '../../lib/content';
  import { dateTime } from '../../lib/format';
  import type { ContentFinding, ContentItem } from '../../lib/api';

  let { item, canManage, latest = null, onclose, onaction, ontoggle, onupdate, onremove }: {
    item: ContentItem; canManage: boolean; latest?: string | null; onclose: () => void; onaction: (f: ContentFinding) => void;
    ontoggle: (it: ContentItem) => void; onupdate: (it: ContentItem) => void; onremove: (it: ContentItem) => void;
  } = $props();
  const m = $derived(item.meta);
  const sb = $derived(sourceBadge(item));
  const url = $derived(pageUrl(item));
</script>

<Modal title={item.title} side width={520} {onclose}>
  <div class="row g6 wrap" style="margin-bottom:12px">
    <span class="badge {sb.tone}" title={sb.hint}>{sb.label}</span>
    {#if !item.enabled}<span class="badge warn">Disabled</span>{/if}
    {#if item.pinned}<span class="badge info"><Icon name="pin" size={11} />Pinned</span>{/if}
    {#if m}<span class="badge">{m.loader}</span><span class="badge">{m.environment === 'both' ? 'client + server' : m.environment + ' side'}</span>{/if}
  </div>
  {#if item.issues.length}
    <h3 class="h">Issues</h3>
    <FindingList findings={item.issues} open {onaction} />
  {/if}
  <h3 class="h">Details</h3>
  <dl class="dl">
    <dt>Version</dt><dd class="tnum">{item.version_number ?? m?.version ?? '—'}{#if latest}<span class="badge info" style="margin-left:6px">→ {latest}</span>{/if}</dd>
    <dt>File</dt><dd class="mono">{item.filename}</dd>
    <dt>Installed</dt><dd>{dateTime(item.installed_at)}</dd>
    {#if m}
      <dt>Id</dt><dd class="mono">{m.id}</dd>
      {#if m.authors.length}<dt>Authors</dt><dd>{m.authors.join(', ')}</dd>{/if}
    {/if}
    {#if item.sha512}<dt>SHA-512</dt><dd class="mono hash" title={item.sha512}>{item.sha512}</dd>{/if}
  </dl>
  {#if m?.description}<p class="sm muted" style="margin-top:10px">{m.description}</p>{/if}
  {#if !m}<p class="sm muted" style="margin-top:10px">No metadata could be read from this file.</p>{/if}
  {#if m && (m.depends.length || m.breaks.length)}
    <h3 class="h">Dependencies</h3>
    <ul class="deps">
      {#each m.depends as d}<li><code>{d.id}</code>{#if d.range}<span class="xs faint mono">{d.range}</span>{/if}<span class="badge {d.required ? '' : 'info'}">{d.required ? 'required' : 'optional'}</span></li>{/each}
      {#each m.breaks as b}<li><code>{b}</code><span class="badge danger">breaks</span></li>{/each}
    </ul>
  {/if}
  {#snippet footer()}
    {#if url}<a class="btn" href={url} target="_blank" rel="noreferrer noopener">Open page<Icon name="external" size={13} /></a>{/if}
    {#if canManage}
      <button class="btn" onclick={() => ontoggle(item)}>{item.enabled ? 'Disable' : 'Enable'}</button>
      {#if latest && !item.pinned}<button class="btn primary" onclick={() => onupdate(item)}>Update</button>{/if}
      <button class="btn danger" onclick={() => onremove(item)}>Remove</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .h { margin: 16px 0 8px; font-size: var(--fs-sm); text-transform: uppercase; letter-spacing: .06em; color: var(--muted); }
  .h:first-of-type { margin-top: 0; }
  .dl { display: grid; grid-template-columns: 92px minmax(0, 1fr); gap: 6px 12px; margin: 0; font-size: var(--fs-sm); }
  .dl dt { color: var(--muted); } .dl dd { margin: 0; overflow-wrap: anywhere; }
  .hash { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; font-size: var(--fs-xs); }
  .deps { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .deps li { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
</style>
