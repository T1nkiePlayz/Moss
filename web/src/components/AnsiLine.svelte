<script lang="ts">
  import { parseAnsi } from '../lib/ansi';
  // Renders one console line as text nodes in styled spans. Never uses innerHTML.
  let { line }: { line: string } = $props();
  const segs = $derived(parseAnsi(line));
</script>

{#each segs as s}{#if s.fg || s.bold || s.dim || s.italic || s.underline}<span style:color={s.fg} style:font-weight={s.bold ? 700 : undefined} style:opacity={s.dim ? 0.6 : undefined} style:font-style={s.italic ? 'italic' : undefined} style:text-decoration={s.underline ? 'underline' : undefined}>{s.text}</span>{:else}{s.text}{/if}{/each}
