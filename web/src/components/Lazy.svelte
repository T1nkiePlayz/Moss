<script lang="ts">
  import type { Component } from 'svelte';
  import States from './States.svelte';
  let { load, props = {} }: { load: () => Promise<{ default: Component<any> }>; props?: Record<string, unknown> } = $props();
  const p = $derived(load());
</script>

{#await p}
  <States kind="loading" rows={4} />
{:then m}
  {@const C = m.default}
  <C {...props} />
{:catch}
  <States kind="error" title="Could not load this page" text="The app may have been updated. Reload to get the latest version." retry={() => location.reload()} />
{/await}
