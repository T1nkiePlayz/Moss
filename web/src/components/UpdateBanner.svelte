<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { sysUpdate } from '../lib/stores.svelte';
  import { ops } from '../lib/stores.svelte';
  import { BANNER_MS, markBannerShown, pausableTimer, shouldShowBanner } from '../lib/update';
  import { dismissUpdateError, loadUpdate, startUpdate, STALL_TEXT, upd } from '../lib/update.svelte';

  let visible = $state(false);
  let fading = $state(false);
  let timer: ReturnType<typeof pausableTimer> | undefined;
  let fadeId: ReturnType<typeof setTimeout> | undefined;

  onMount(() => { void loadUpdate(); return () => { timer?.stop(); clearTimeout(fadeId); }; });
  // A new version announced over SSE.
  $effect(() => { if (sysUpdate.info) upd.info = sysUpdate.info; });
  $effect(() => {
    const i = upd.info;
    if (i && upd.phase === 'idle' && !visible && shouldShowBanner(i.latest, i.available)) {
      markBannerShown(i.latest!);
      fading = false; visible = true;
      timer?.stop();
      timer = pausableTimer(BANNER_MS, hide);
    }
  });
  function hide() {
    timer?.stop();
    fading = true;
    clearTimeout(fadeId);
    fadeId = setTimeout(() => { visible = false; fading = false; }, 400);
  }
  function details() { hide(); location.hash = '#/settings'; upd.reveal++; }
  function update() { hide(); void startUpdate(); }
  const op = $derived(ops.list.find((o) => o.kind === 'moss.update' && (o.state === 'queued' || o.state === 'running')));
</script>

{#if upd.phase !== 'idle'}
  <div class="bar-wrap"><div class="upd" class:bad={upd.phase === 'failed' || upd.phase === 'stalled'} role="status" aria-live="polite">
    <Icon name={upd.phase === 'failed' || upd.phase === 'stalled' ? 'alert' : 'refresh'} size={16} />
    <div class="txt">
      {#if upd.phase === 'updating'}<b>Updating Moss…</b> <span class="muted">{op?.stage ?? 'Waiting for the operation'}</span>
      {:else if upd.phase === 'restarting'}<b>Restarting Moss…</b> <span class="muted">This page reloads when the new version is up.</span>
      {:else if upd.phase === 'stalled'}{STALL_TEXT}
      {:else}<b>Update failed.</b> <span class="muted">{upd.error}</span>{/if}
    </div>
    {#if upd.phase === 'failed'}<button class="btn ghost icon sm" aria-label="Dismiss" onclick={dismissUpdateError}><Icon name="x" size={14} /></button>{/if}
  </div></div>
{:else if visible && upd.info?.available}
  <div class="bar-wrap">
    <div class="upd" class:fade={fading} role="status" aria-live="polite" aria-label="Moss update"
      onmouseenter={() => timer?.pause()} onmouseleave={() => timer?.resume()} onfocusin={() => timer?.pause()} onfocusout={() => timer?.resume()}>
      <Icon name="download" size={16} />
      <div class="txt"><b>Moss {upd.info.latest} is available</b></div>
      <div class="acts">
        {#if upd.info.method !== 'manual'}<button class="btn primary sm" onclick={update}>Update</button>{/if}
        <button class="btn sm" onclick={details}>Details</button>
        <button class="btn ghost icon sm" aria-label="Dismiss update notice" onclick={hide}><Icon name="x" size={14} /></button>
      </div>
    </div>
  </div>
{/if}

<style>
  .bar-wrap { position: fixed; z-index: 70; left: 0; right: 0; bottom: 16px; display: flex; justify-content: center; padding: 0 16px; pointer-events: none; }
  .upd { pointer-events: auto; display: flex; align-items: center; gap: 12px; max-width: min(560px, 100%); padding: 10px 10px 10px 14px; border-radius: var(--r); background: var(--s3); border: 1px solid var(--line2); box-shadow: var(--shadow, 0 8px 28px rgb(0 0 0 / .3)); font-size: var(--fs-sm); transition: opacity .35s var(--ease), transform .35s var(--ease); }
  .upd > :global(svg) { flex: none; color: var(--info); }
  .upd.bad { border-color: color-mix(in srgb, var(--danger), transparent 55%); } .upd.bad > :global(svg) { color: var(--danger); }
  .upd.fade { opacity: 0; transform: translateY(8px); }
  .txt { min-width: 0; overflow-wrap: anywhere; }
  .acts { display: flex; align-items: center; gap: 6px; flex: none; }
  @media (max-width: 760px) {
    .bar-wrap { bottom: 76px; }
    .upd { flex-wrap: wrap; }
    .txt { flex: 1 1 100%; }
  }
</style>
