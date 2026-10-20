<script lang="ts">
  import Icon from './Icon.svelte';
  import type { RestorePreview } from '../lib/api';
  import type { PreviewSummary } from '../lib/backupPreview';
  import { bytes, num } from '../lib/format';

  let { preview, sum }: { preview: RestorePreview; sum: PreviewSummary } = $props();
  const groupLabel = { added: 'Added', changed: 'Changed', removed: 'Removed' } as const;
</script>

<p class="head" class:warn={sum.destructive}>{sum.headline}</p>
<dl class="stats">
  {#each sum.rows as r}<div class="st {r.tone}"><dt>{r.label}</dt><dd class="tnum">{num(r.count)}</dd></div>{/each}
  {#if sum.showDelta}<div class="st"><dt>Size change</dt><dd class="tnum {sum.sizeTone}">{sum.sizeDelta}</dd></div>
  {:else}<div class="st"><dt>Backup size</dt><dd class="tnum">{bytes(preview.archive.size)}</dd></div>{/if}
</dl>
<p class="xs muted">Backup: {num(preview.archive.files)} files, {bytes(preview.archive.size)}{preview.current ? ` · Current: ${num(preview.current.files)} files, ${bytes(preview.current.size)}` : ''}</p>
{#each sum.rows.filter((r) => r.samples.length) as r (r.key)}
  <details>
    <summary>{groupLabel[r.key as 'added']} files ({num(r.count)})</summary>
    <ul class="paths mono xs">{#each r.samples as pth}<li>{pth}</li>{/each}</ul>
    {#if r.more}<p class="xs muted">…and {num(r.more)} more not listed.</p>{/if}
  </details>
{/each}
{#each preview.warnings as w}<div class="banner warn mt-s"><Icon name="alert" size={16} />{w}</div>{/each}

<style>
  .head { margin: 0 0 10px; font-weight: 550; } .head.warn { color: var(--warn); }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 8px; margin: 0 0 8px; }
  .st { padding: 8px 10px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); }
  .st dt { font-size: var(--fs-xs); color: var(--muted); } .st dd { margin: 2px 0 0; font-size: var(--fs-lg); font-weight: 650; }
  .st.ok dd, dd.ok { color: var(--a-text); } .st.warn dd { color: var(--warn); } .st.danger dd, dd.danger { color: var(--danger); }
  details { margin-top: 8px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); }
  summary { cursor: pointer; padding: 8px 12px; font-size: var(--fs-sm); font-weight: 550; }
  .paths { list-style: none; margin: 0; padding: 4px 12px 10px; max-height: 180px; overflow: auto; } .paths li { padding: 1px 0; overflow-wrap: anywhere; }
  details > p { padding: 0 12px 10px; margin: 0; }
</style>
