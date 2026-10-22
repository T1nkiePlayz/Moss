<script lang="ts">
  import Icon from '../Icon.svelte';
  import ModpackSummary from './ModpackSummary.svelte';
  import { api, ApiError, errMsg, type ModpackInspect, type ProviderId } from '../../lib/api';
  import { inspectModpackFile, type Upload } from '../../lib/upload';
  import { modpackFileProblem, parseModpackRef } from '../../lib/content';
  import { bytes, compact } from '../../lib/format';
  import { apiExtra, type ModpackHit } from '../../lib/api-extra';

  // Upload a .mrpack/.zip (with progress, abortable) or look a pack up on Modrinth/CurseForge, then show what it contains.
  let { onresult }: { onresult: (m: ModpackInspect | null) => void } = $props();

  type Tab = 'search' | 'file' | ProviderId;
  let tab = $state<Tab>('search');
  let ref = $state('');
  let busy = $state<'' | 'upload' | 'inspect'>('');
  let loaded = $state(0), total = $state(0);
  let err = $state('');
  let pack = $state<ModpackInspect | null>(null);
  let drag = $state(false);
  let up: Upload<ModpackInspect> | null = null;
  let input = $state<HTMLInputElement>();

  // Search tab: debounced catalogue search; CurseForge is offered only when the instance (and session) can use it.
  let sp = $state<ProviderId>('modrinth'), sq = $state('');
  let hits = $state<ModpackHit[]>([]), nextOff = $state<number | null>(null), searching = $state(false), cfOk = $state(false), picked = $state('');
  let seq = 0;
  $effect(() => { void apiExtra.providerList().then((r) => { cfOk = !!r.providers.find((p) => p.id === 'curseforge')?.available; }).catch(() => {}); });
  async function runSearch(more = false) {
    const my = ++seq;
    searching = true; if (!more) err = '';
    try {
      const r = await apiExtra.searchModpacks(sp, sq.trim(), more ? (nextOff ?? 0) : 0);
      if (my !== seq) return;
      const seen = new Set(more ? hits.map((h) => h.project_id) : []);
      hits = more ? [...hits, ...r.items.filter((h) => !seen.has(h.project_id))] : r.items; nextOff = r.next_offset;
    } catch (e) { if (my === seq) { err = errMsg(e); if (!more) { hits = []; nextOff = null; } } }
    if (my === seq) searching = false;
  }
  $effect(() => {
    if (tab !== 'search') return;
    void sp; void sq;
    const t = setTimeout(() => void runSearch(), sq ? 350 : 0);
    return () => clearTimeout(t);
  });
  async function pick(h: ModpackHit) {
    err = ''; busy = 'inspect'; picked = h.project_id;
    try { set(await api.inspectModpack({ provider: h.provider, project_id: h.project_id })); } catch (e) { err = errMsg(e); }
    busy = ''; picked = '';
  }
  function set(m: ModpackInspect | null) { pack = m; onresult(m); }
  async function fromFile(f: File | undefined) {
    if (!f) return;
    const bad = modpackFileProblem(f);
    if (bad) { err = bad; return; }
    err = ''; busy = 'upload'; loaded = 0; total = f.size;
    up = inspectModpackFile(f, (l, t) => { loaded = l; total = t; if (l >= t) busy = 'inspect'; });
    try { set(await up.promise); } catch (e) { if (!(e instanceof ApiError && e.code === 'upload.aborted')) err = errMsg(e); }
    busy = ''; up = null;
  }
  async function fromRef() {
    const r = parseModpackRef(ref, tab === 'curseforge' ? 'curseforge' : 'modrinth');
    if (!r) { err = 'Enter a project id, slug or URL from Modrinth or CurseForge.'; return; }
    err = ''; busy = 'inspect';
    try { set(await api.inspectModpack(r)); } catch (e) { err = errMsg(e); }
    busy = '';
  }
  const pctDone = $derived(total ? Math.min(100, Math.round((loaded / total) * 100)) : 0);
  function onDrop(e: DragEvent) { e.preventDefault(); drag = false; void fromFile(e.dataTransfer?.files?.[0]); }
  function segKey(e: KeyboardEvent) {
    const order = tabs.map((t) => t.id);
    const d = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    tab = order[(order.indexOf(tab) + d + order.length) % order.length]; err = '';
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-t="${tab}"]`)?.focus();
  }
  function provKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    sp = sp === 'modrinth' ? 'curseforge' : 'modrinth';
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-p="${sp}"]`)?.focus();
  }
  const tabs: { id: Tab; label: string }[] = [{ id: 'search', label: 'Search' }, { id: 'file', label: 'Upload file' }, { id: 'modrinth', label: 'Modrinth link' }, { id: 'curseforge', label: 'CurseForge link' }];
</script>

{#if pack}
  <div class="col">
    <ModpackSummary {pack} />
    <div><button class="btn sm" onclick={() => { set(null); err = ''; }}><Icon name="chevron-left" size={13} />Choose a different pack</button></div>
  </div>
{:else}
  <div class="col">
    <div class="seg" role="radiogroup" aria-label="Modpack source" tabindex="-1" onkeydown={segKey}>
      {#each tabs as t}
        <button class="segb" role="radio" aria-checked={tab === t.id} tabindex={tab === t.id ? 0 : -1} data-t={t.id} disabled={!!busy} onclick={() => { tab = t.id; err = ''; }}>{t.label}</button>
      {/each}
    </div>
    {#if tab === 'search'}
      <div class="col" style="gap:10px">
        <div class="row" style="gap:8px;flex-wrap:wrap">
          <div class="field" style="flex:1;min-width:200px">
            <label for="mp-q">Search modpacks</label>
            <input id="mp-q" class="input" type="search" bind:value={sq} disabled={!!busy} placeholder="Fabulously Optimized, RLCraft…" autocomplete="off" spellcheck="false" />
          </div>
          {#if cfOk}
            <div class="seg" role="radiogroup" aria-label="Catalogue" tabindex="-1" style="align-self:flex-end" onkeydown={provKey}>
              {#each [['modrinth', 'Modrinth'], ['curseforge', 'CurseForge']] as [id, label]}
                <button class="segb" role="radio" aria-checked={sp === id} tabindex={sp === id ? 0 : -1} data-p={id} disabled={!!busy} onclick={() => (sp = id as ProviderId)}>{label}</button>
              {/each}
            </div>
          {/if}
        </div>
        {#if !cfOk}<span class="hint">CurseForge modpacks appear here when CurseForge is configured and you are signed in.</span>{/if}
        <ul class="hits" aria-busy={searching} aria-label="Modpack results">
          {#each hits as h (h.provider + h.project_id)}
            <li>
              <button class="hit" disabled={!!busy} onclick={() => void pick(h)} aria-label="Use {h.title} by {h.author}">
                {#if h.icon_url}<img src={h.icon_url} alt="" width="40" height="40" loading="lazy" referrerpolicy="no-referrer" />{:else}<span class="ph"><Icon name="package" size={20} /></span>{/if}
                <span class="meta">
                  <span class="row" style="gap:8px;flex-wrap:wrap"><b>{h.title}</b><span class="xs muted">by {h.author}</span><span class="xs faint tnum">{compact(h.downloads)} downloads</span></span>
                  <span class="sm muted desc">{h.description}</span>
                  {#if h.game_versions?.length || h.loaders?.length}<span class="xs faint">{[...(h.loaders ?? []), ...(h.game_versions ?? [])].join(' · ')}</span>{/if}
                </span>
                {#if busy && picked === h.project_id}<span class="xs muted">Reading the pack…</span>{/if}
              </button>
            </li>
          {/each}
        </ul>
        {#if searching}<div class="skel" style="height:56px"></div>{:else if !hits.length && !err}<p class="muted sm">No modpacks found.</p>{/if}
        {#if !searching && nextOff != null}<div><button class="btn sm" disabled={!!busy} onclick={() => void runSearch(true)}>Load more</button></div>{/if}
      </div>
    {:else if tab === 'file'}
      <div class="drop" class:drag role="group" aria-label="Upload a modpack" ondragover={(e) => { e.preventDefault(); drag = true; }} ondragleave={() => (drag = false)} ondrop={onDrop}>
        {#if busy}
          <div class="prog">
            <div class="row between sm"><span>{busy === 'upload' ? 'Uploading…' : 'Reading the pack…'}</span><span class="tnum">{busy === 'upload' ? `${bytes(loaded)} / ${bytes(total)} · ${pctDone}%` : ''}</span></div>
            <div class="bar" class:indet={busy === 'inspect'} role="progressbar" aria-label="Upload progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow={busy === 'upload' ? pctDone : undefined}><i style={busy === 'upload' ? `width:${Math.max(2, pctDone)}%` : ''}></i></div>
            {#if busy === 'upload'}<div><button class="btn sm danger" onclick={() => up?.abort()}>Cancel upload</button></div>{/if}
          </div>
        {:else}
          <Icon name="upload" size={26} />
          <p><b>Drop a modpack here</b></p>
          <p class="xs muted">.mrpack (Modrinth) or .zip (CurseForge), up to 512 MiB</p>
          <button class="btn" onclick={() => input?.click()}>Choose file…</button>
        {/if}
        <input bind:this={input} class="sr" type="file" accept=".mrpack,.zip" tabindex="-1" aria-hidden="true" onchange={(e) => { void fromFile(e.currentTarget.files?.[0]); e.currentTarget.value = ''; }} />
      </div>
    {:else}
      <form class="col" onsubmit={(e) => { e.preventDefault(); void fromRef(); }}>
        <div class="field">
          <label for="mp-ref">{tab === 'modrinth' ? 'Modrinth' : 'CurseForge'} modpack id or URL</label>
          <input id="mp-ref" class="input" bind:value={ref} disabled={!!busy} placeholder={tab === 'modrinth' ? 'https://modrinth.com/modpack/fabulously-optimized' : 'https://www.curseforge.com/minecraft/modpacks/…'} autocomplete="off" spellcheck="false" aria-invalid={!!err} aria-describedby="mp-err" />
          <span class="hint">Paste a project or version link, or just the id. A version link picks that exact version; otherwise the latest is used.</span>
        </div>
        <div><button class="btn primary" type="submit" disabled={!!busy || !ref.trim()}>{busy ? 'Inspecting…' : 'Inspect modpack'}</button></div>
      </form>
    {/if}
    <div id="mp-err" aria-live="polite">{#if err}<div class="banner danger" role="alert"><Icon name="x-circle" size={16} />{err}</div>{/if}</div>
  </div>
{/if}

<style>
  .hits { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; max-height: 420px; overflow-y: auto; }
  .hit { display: flex; gap: 12px; align-items: center; width: 100%; text-align: left; font: inherit; color: var(--text); padding: 8px 10px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s1); cursor: pointer; transition: border-color var(--t), background var(--t); }
  .hit:hover:not(:disabled) { border-color: var(--a1); background: var(--a-soft); }
  .hit:focus-visible { outline: 2px solid var(--a-text); outline-offset: 1px; }
  .hit:disabled { opacity: .6; cursor: not-allowed; }
  .hit img, .ph { width: 40px; height: 40px; border-radius: 8px; flex: none; object-fit: cover; background: var(--s2); display: grid; place-items: center; color: var(--muted); }
  .meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .desc { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  @media (prefers-reduced-motion: reduce) { .hit { transition: none; } }
  .seg { display: inline-flex; padding: 2px; gap: 2px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); width: fit-content; max-width: 100%; }
  .segb { border: 0; background: none; color: var(--muted); font: inherit; font-size: calc(13px * var(--ts, 1)); padding: 4px 12px; border-radius: 6px; cursor: pointer; }
  .segb[aria-checked='true'] { background: var(--a-soft); color: var(--a-text); font-weight: 600; }
  .segb:disabled { opacity: .5; cursor: not-allowed; }
  .segb:focus-visible { outline: 2px solid var(--a-text); outline-offset: 1px; }
  .drop { display: flex; flex-direction: column; align-items: center; gap: 6px; text-align: center; padding: 28px 16px; min-height: 176px; justify-content: center; border: 2px dashed var(--line2); border-radius: var(--r); background: color-mix(in srgb, var(--s1), transparent 40%); color: var(--muted); transition: border-color var(--t), background var(--t); }
  .drop.drag { border-color: var(--a1); background: var(--a-soft); }
  .prog { width: min(420px, 100%); display: flex; flex-direction: column; gap: 10px; align-items: stretch; text-align: left; color: var(--text); }
</style>
