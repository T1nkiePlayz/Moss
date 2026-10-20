<script lang="ts">
  import type { MetricRange } from '../lib/api';
  import { ranges } from '../lib/metrics';
  let { value = $bindable(), label = 'Time range' }: { value: MetricRange; label?: string } = $props();
  function key(e: KeyboardEvent) {
    const i = ranges.findIndex((r) => r.id === value);
    const n = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? i + 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? i - 1 : i;
    if (n === i) return;
    e.preventDefault();
    value = ranges[(n + ranges.length) % ranges.length].id;
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
  }
</script>

<!-- svelte-ignore a11y_interactive_supports_focus -->
<div class="row g4 rs" role="radiogroup" aria-label={label} onkeydown={key}>
  {#each ranges as r (r.id)}
    <button class="chip sm" role="radio" aria-checked={value === r.id} tabindex={value === r.id ? 0 : -1} onclick={() => (value = r.id)}>{r.label}</button>
  {/each}
</div>

<style>
  .chip { height: 26px; padding: 0 10px; font-size: var(--fs-xs); }
  .chip[aria-checked='true'] { background: var(--a-soft); color: var(--a-text); border-color: color-mix(in srgb, var(--a2), transparent 55%); }
</style>
