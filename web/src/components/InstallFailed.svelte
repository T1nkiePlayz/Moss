<script lang="ts">
  import Icon from './Icon.svelte';
  import type { Server } from '../lib/api';
  import { ui } from '../lib/stores.svelte';
  import { reinstall } from '../lib/lifecycle';
  import { can } from '../lib/auth.svelte';
  import { serverHref } from '../lib/route';
  let { server: s }: { server: Server } = $props();
  let busy = $state(false);
  async function retry() { busy = true; await reinstall(s, false, true); busy = false; }
</script>

<section class="fail" role="alert" aria-label="Installation failed">
  <div class="ic"><Icon name="x-circle" size={26} /></div>
  <div class="grow">
    <h2>Installation failed</h2>
    <p class="muted">{s.state_detail || 'The server could not be installed. Check the operation log for the reason.'}</p>
  </div>
  <div class="row g6 wrap">
    {#if can('config.write')}<button class="btn primary lg" disabled={busy} onclick={retry}><Icon name="refresh" size={16} />{busy ? 'Starting…' : 'Retry install'}</button>{/if}
    <button class="btn" onclick={() => (ui.ops = true)}><Icon name="layers" size={15} />View log</button>
    <a class="btn ghost" href={serverHref(s.id, 'settings')}>Settings</a>
  </div>
</section>

<style>
  .fail { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; padding: 16px 18px; margin-bottom: var(--gap); border-radius: var(--r); background: var(--danger-soft); border: 1px solid color-mix(in srgb, var(--danger), transparent 50%); }
  .ic { color: var(--danger); display: grid; }
  h2 { font-size: var(--fs-md); } p { margin: 2px 0 0; overflow-wrap: anywhere; }
  .grow { min-width: 220px; }
</style>
