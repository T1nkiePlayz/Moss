<script lang="ts">
  import { ICONS } from '../lib/icons';
  let { name, size = 18, label }: { name: string; size?: number; label?: string } = $props();
  const filled = $derived(name === 'play' || name === 'stop');
</script>

<svg
  width={size} height={size} viewBox="0 0 24 24" fill={filled ? 'currentColor' : 'none'} stroke="currentColor"
  stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
  role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : 'true'} focusable="false"
><path d={ICONS[name] ?? ICONS.info} /></svg>
