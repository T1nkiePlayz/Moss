<script lang="ts">
  import Icon from './Icon.svelte';
  import { api, type SavedView } from '../lib/api';
  import { toast, toastError } from '../lib/stores.svelte';

  let { scope, filters, sort = '', onapply }: { scope: SavedView['scope']; filters: Record<string, unknown>; sort?: string; onapply: (filters: Record<string, unknown>, sort: string) => void } = $props();
  let views = $state<SavedView[]>([]);
  let adding = $state(false);
  let name = $state('');
  let input = $state<HTMLInputElement>();
  let active = $state('');

  async function load() { try { views = await api.views(scope); } catch { views = []; } }
  $effect(() => { scope; void load(); });
  const dirty = $derived(Object.values(filters).some((v) => v !== '' && v !== 'all' && v != null) || (sort !== '' && sort !== 'relevance'));

  function apply(v: SavedView) { active = v.id; onapply(v.filters, v.sort); }
  async function save() {
    const n = name.trim();
    if (!n) return;
    try {
      const v = await api.saveView({ scope, name: n, filters, sort });
      views = [...views.filter((x) => x.id !== v.id && x.name !== v.name), v]; active = v.id; adding = false; name = '';
      toast('success', `Saved view “${v.name}”`);
    } catch (e) { toastError(e, 'Could not save the view'); }
  }
  async function remove(v: SavedView) {
    try { await api.deleteView(v.id); views = views.filter((x) => x.id !== v.id); if (active === v.id) active = ''; toast('info', `Removed view “${v.name}”`); } catch (e) { toastError(e, 'Could not remove the view'); }
  }
  $effect(() => { if (adding) input?.focus(); });
</script>

<div class="row g6 wrap" role="group" aria-label="Saved views">
  {#each views as v (v.id)}
    <span class="chip view" class:on={active === v.id}>
      <button class="vbtn" aria-pressed={active === v.id} onclick={() => apply(v)}>{v.name}</button>
      <button class="x" aria-label="Remove view {v.name}" onclick={() => remove(v)}><Icon name="x" size={12} /></button>
    </span>
  {/each}
  {#if adding}
    <form class="row g4" onsubmit={(e) => { e.preventDefault(); void save(); }}>
      <input bind:this={input} class="input sm" style="width:150px" aria-label="Name for this view" placeholder="Name this view" maxlength="40" bind:value={name} onkeydown={(e) => e.key === 'Escape' && (adding = false)} />
      <button class="btn sm primary" type="submit" disabled={!name.trim()}>Save</button>
      <button class="btn sm ghost" type="button" onclick={() => (adding = false)}>Cancel</button>
    </form>
  {:else}
    <button class="btn sm ghost" disabled={!dirty} title={dirty ? 'Remember the current search and filters' : 'Set a search or filter first'} onclick={() => (adding = true)}><Icon name="plus" size={13} />Save view</button>
  {/if}
</div>

<style>
  .view { padding: 0; gap: 0; }
  .view.on { border-color: var(--a1); background: var(--a-soft); }
  .vbtn { all: unset; cursor: pointer; padding: 5px 4px 5px 12px; border-radius: 99px 0 0 99px; }
  .vbtn:focus-visible, .x:focus-visible { outline: 2px solid var(--a1); outline-offset: 1px; }
  .x { all: unset; cursor: pointer; padding: 5px 9px 5px 4px; display: grid; place-items: center; color: var(--muted); border-radius: 0 99px 99px 0; }
  .x:hover { color: var(--text); }
</style>
