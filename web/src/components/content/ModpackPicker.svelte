<script lang="ts">
  import Icon from '../Icon.svelte';
  import ModpackSummary from './ModpackSummary.svelte';
  import { api, ApiError, errMsg, type ModpackInspect, type ProviderId } from '../../lib/api';
  import { inspectModpackFile, type Upload } from '../../lib/upload';
  import { modpackFileProblem, parseModpackRef } from '../../lib/content';
  import { bytes } from '../../lib/format';

  // Upload a .mrpack/.zip (with progress, abortable) or look a pack up on Modrinth/CurseForge, then show what it contains.
  let { onresult }: { onresult: (m: ModpackInspect | null) => void } = $props();

  let tab = $state<'file' | ProviderId>('file');
  let ref = $state('');
  let busy = $state<'' | 'upload' | 'inspect'>('');
  let loaded = $state(0), total = $state(0);
  let err = $state('');
  let pack = $state<ModpackInspect | null>(null);
  let drag = $state(false);
  let up: Upload<ModpackInspect> | null = null;
  let input = $state<HTMLInputElement>();

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
    const r = parseModpackRef(ref, tab === 'file' ? 'modrinth' : tab);
    if (!r) { err = 'Enter a project id, slug or URL from Modrinth or CurseForge.'; return; }
    err = ''; busy = 'inspect';
    try { set(await api.inspectModpack(r)); } catch (e) { err = errMsg(e); }
    busy = '';
  }
  const pctDone = $derived(total ? Math.min(100, Math.round((loaded / total) * 100)) : 0);
  function onDrop(e: DragEvent) { e.preventDefault(); drag = false; void fromFile(e.dataTransfer?.files?.[0]); }
  function segKey(e: KeyboardEvent) {
    const order: ('file' | ProviderId)[] = ['file', 'modrinth', 'curseforge'];
    const d = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    tab = order[(order.indexOf(tab) + d + 3) % 3]; err = '';
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-t="${tab}"]`)?.focus();
  }
  const tabs: { id: 'file' | ProviderId; label: string }[] = [{ id: 'file', label: 'Upload file' }, { id: 'modrinth', label: 'Modrinth' }, { id: 'curseforge', label: 'CurseForge' }];
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
    {#if tab === 'file'}
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
  .seg { display: inline-flex; padding: 2px; gap: 2px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); width: fit-content; max-width: 100%; }
  .segb { border: 0; background: none; color: var(--muted); font: inherit; font-size: calc(13px * var(--ts, 1)); padding: 4px 12px; border-radius: 6px; cursor: pointer; }
  .segb[aria-checked='true'] { background: var(--a-soft); color: var(--a-text); font-weight: 600; }
  .segb:disabled { opacity: .5; cursor: not-allowed; }
  .segb:focus-visible { outline: 2px solid var(--a-text); outline-offset: 1px; }
  .drop { display: flex; flex-direction: column; align-items: center; gap: 6px; text-align: center; padding: 28px 16px; min-height: 176px; justify-content: center; border: 2px dashed var(--line2); border-radius: var(--r); background: color-mix(in srgb, var(--s1), transparent 40%); color: var(--muted); transition: border-color var(--t), background var(--t); }
  .drop.drag { border-color: var(--a1); background: var(--a-soft); }
  .prog { width: min(420px, 100%); display: flex; flex-direction: column; gap: 10px; align-items: stretch; text-align: left; color: var(--text); }
</style>
