<script lang="ts">
  import { filterZones, zoneLabel } from '../lib/tz';
  let { value = $bindable(), zones, id, describedby }: { value: string; zones: string[]; id: string; describedby?: string } = $props();
  let q = $state('');
  let open = $state(false);
  let active = $state(0);
  let input: HTMLInputElement | undefined = $state();
  const shown = $derived(filterZones(zones, q, 50));
  const lid = $derived(`${id}-list`);

  function pick(z: string) { value = z; open = false; q = ''; }
  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { e.preventDefault(); open = true; active = Math.min(shown.length - 1, active + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); active = Math.max(0, active - 1); }
    else if (e.key === 'Enter' && open && shown[active]) { e.preventDefault(); pick(shown[active]); }
    else if (e.key === 'Escape' && open) { e.stopPropagation(); open = false; q = ''; }
  }
  $effect(() => { shown; active = 0; });
  $effect(() => { if (open && input) document.getElementById(`${id}-o${active}`)?.scrollIntoView({ block: 'nearest' }); });
</script>

<div class="tz">
  <input {id} bind:this={input} class="input" role="combobox" aria-expanded={open} aria-controls={lid} aria-autocomplete="list" aria-activedescendant={open && shown.length ? `${id}-o${active}` : undefined} aria-describedby={describedby}
    autocomplete="off" spellcheck="false" placeholder="Search time zones…"
    value={open ? q : zoneLabel(value)} oninput={(e) => { q = e.currentTarget.value; open = true; }}
    onfocus={(e) => { open = true; q = ''; e.currentTarget.select(); }} onblur={() => { open = false; q = ''; }} onkeydown={key} />
  {#if open}
    <ul class="list" id={lid} role="listbox" aria-label="Time zones">
      {#each shown as z, i (z)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li id="{id}-o{i}" role="option" aria-selected={z === value} class:act={i === active} class:cur={z === value} onmousedown={(e) => { e.preventDefault(); pick(z); }} onmousemove={() => (active = i)}>{zoneLabel(z)}</li>
      {:else}
        <li class="none" role="option" aria-selected="false" aria-disabled="true">No matching zone</li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .tz { position: relative; }
  .list { position: absolute; z-index: 20; left: 0; right: 0; top: calc(100% + 4px); max-height: 240px; overflow: auto; margin: 0; padding: 4px; list-style: none; background: var(--s2); border: 1px solid var(--line2); border-radius: var(--r-sm); box-shadow: 0 12px 32px rgb(0 0 0 / .4); }
  li { padding: 6px 10px; border-radius: 6px; font-size: var(--fs-sm); cursor: pointer; }
  li.act { background: var(--hl); outline: 1px solid var(--line2); } li.cur { color: var(--a-text); font-weight: 600; } li.none { color: var(--muted); cursor: default; }
</style>
