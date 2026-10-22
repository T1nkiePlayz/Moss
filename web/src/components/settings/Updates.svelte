<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../Icon.svelte';
  import { sysUpdate } from '../../lib/stores.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { dateTime } from '../../lib/format';
  import { checkUpdate, loadUpdate, startUpdate, STALL_TEXT, upd } from '../../lib/update.svelte';

  let root: HTMLElement;
  const busy = $derived(upd.phase === 'updating' || upd.phase === 'restarting' || upd.phase === 'stalled');
  onMount(() => { void loadUpdate(); });
  $effect(() => { if (sysUpdate.info) upd.info = sysUpdate.info; });
  // "Details" on the banner lands here.
  $effect(() => { if (upd.reveal) setTimeout(() => { root?.scrollIntoView({ block: 'start' }); root?.querySelector<HTMLElement>('#upd')?.focus(); }, 50); });
</script>

<section class="card" aria-labelledby="upd" bind:this={root} id="updates">
  <div class="card-title"><Icon name="download" size={14} /><span id="upd" tabindex="-1">Updates</span></div>
  {#if upd.info}
    {@const i = upd.info}
    <dl class="kv">
      <div><dt>Current version</dt><dd class="tnum">{i.current}</dd></div>
      <div><dt>Latest version</dt><dd class="tnum">{i.latest ?? i.current}</dd></div>
      <div><dt>Last checked</dt><dd>{i.checked_at ? dateTime(i.checked_at) : 'Never'}</dd></div>
    </dl>
    <p class="muted sm" aria-live="polite">{i.available ? `Moss ${i.latest} is available.` : 'Moss is up to date.'}</p>
    <div class="row wrap" style="margin-top:12px">
      <button class="btn" onclick={checkUpdate} disabled={upd.checking || busy}><Icon name="refresh" size={15} />{upd.checking ? 'Checking…' : 'Check now'}</button>
      {#if i.available && i.method !== 'manual'}
        <button class="btn primary" onclick={startUpdate} disabled={busy}><Icon name="download" size={15} />{busy ? 'Updating…' : `Update to ${i.latest}`}</button>
      {/if}
      {#if i.release_url}<a class="btn" href={i.release_url} target="_blank" rel="noopener noreferrer"><Icon name="external" size={15} />Release notes</a>{/if}
    </div>
    {#if i.available && i.method === 'manual'}
      <div class="banner info" style="margin-top:12px"><Icon name="info" size={16} />
        <div>
          {#if i.reason}<div>{i.reason}</div>{/if}
          {#if i.command}
            <div class="row" style="margin-top:8px;align-items:flex-start"><pre class="cmd"><code>{i.command}</code></pre><button class="btn sm" onclick={() => copyWithToast(i.command!, 'Command')}><Icon name="copy" size={14} />Copy</button></div>
          {/if}
        </div>
      </div>
    {/if}
    {#if upd.phase === 'restarting'}<p class="sm" role="status">Restarting Moss…</p>{/if}
    {#if upd.phase === 'stalled'}<p class="sm" role="alert">{STALL_TEXT}</p>{/if}
    {#if upd.phase === 'failed'}<p class="sm" role="alert">Update failed. {upd.error}</p>{/if}
  {:else}
    <p class="muted sm">Loading…</p>
  {/if}
</section>

<style>
  .kv { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; margin: 10px 0 8px; }
  dt { color: var(--muted); font-size: var(--fs-sm); } dd { margin: 2px 0 0; font-weight: 600; }
  .cmd { margin: 0; padding: 8px 10px; border-radius: var(--r-sm); background: var(--s1); border: 1px solid var(--line); overflow-x: auto; max-width: 100%; flex: 1; min-width: 0; }
  .cmd code { font-family: var(--mono, ui-monospace, monospace); font-size: var(--fs-sm); white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
