<script lang="ts">
  import Modal from './Modal.svelte';
  let { title, label, value = '', confirmLabel = 'Save', hint, onsubmit, oncancel }: {
    title: string; label: string; value?: string; confirmLabel?: string; hint?: string; onsubmit: (v: string) => void | Promise<void>; oncancel: () => void;
  } = $props();
  // svelte-ignore state_referenced_locally
  let v = $state(value);
  let busy = $state(false);
  async function go(e: Event) { e.preventDefault(); if (!v.trim()) return; busy = true; try { await onsubmit(v.trim()); } finally { busy = false; } }
</script>

<Modal {title} onclose={oncancel} width={420}>
  <form id="pm" onsubmit={go}>
    <div class="field"><label for="pm-in">{label}</label><input id="pm-in" class="input" bind:value={v} data-autofocus autocomplete="off" />{#if hint}<span class="hint">{hint}</span>{/if}</div>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={oncancel}>Cancel</button>
    <button class="btn primary" form="pm" disabled={busy || !v.trim()}>{confirmLabel}</button>
  {/snippet}
</Modal>
