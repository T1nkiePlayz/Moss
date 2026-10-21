<script lang="ts">
  import Icon from '../Icon.svelte';
  import { api, errMsg } from '../../lib/api';
  import { checks, cssVars, DEFAULTS, exportTheme, hasCustom, isHex, minimal, normHex, parseTheme, resolve, TOKENS, type Colors, type CustomTheme, type Scheme, type TokenKey } from '../../lib/theme';
  import { custom, setCustomTheme, toast, toastError } from '../../lib/stores.svelte';

  let scheme = $state<Scheme>(document.documentElement.dataset.scheme === 'light' ? 'light' : 'dark');
  let draft = $state<Record<Scheme, Colors>>({ light: resolve(custom.theme, 'light'), dark: resolve(custom.theme, 'dark') });
  let json = $state('');
  let jsonErr = $state('');
  let busy = $state(false);

  const cur = $derived(draft[scheme]);
  const results = $derived(checks(cur));
  const failing = $derived((['light', 'dark'] as Scheme[]).flatMap((s) => checks(draft[s]).filter((c) => !c.ok).map((c) => `${s === 'light' ? 'Light' : 'Dark'}: ${c.label}`)));
  const asTheme = (): CustomTheme => ({ light: minimal(draft.light, 'light'), dark: minimal(draft.dark, 'dark') });
  const changed = $derived(JSON.stringify(asTheme()) !== JSON.stringify(custom.theme ?? { light: {}, dark: {} }) && !(custom.theme === null && !hasCustom(asTheme())));
  const vars = $derived(Object.entries(cssVars(cur, scheme)).map(([k, v]) => `${k}:${v}`).join(';'));

  function setColor(k: TokenKey, v: string) { if (isHex(v)) draft[scheme][k] = normHex(v); }
  function apply() { setCustomTheme(asTheme()); toast('success', hasCustom(asTheme()) ? 'Colours applied' : 'Back to the standard colours'); }
  function reset(all: boolean) { draft[scheme] = { ...DEFAULTS[scheme] }; if (all) draft[scheme === 'light' ? 'dark' : 'light'] = { ...DEFAULTS[scheme === 'light' ? 'dark' : 'light'] }; }

  function doExport() { json = exportTheme(asTheme()); jsonErr = ''; }
  function download() {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([exportTheme(asTheme())], { type: 'application/json' }));
    a.download = 'moss-theme.json'; a.click(); URL.revokeObjectURL(a.href);
  }
  function doImport() {
    const r = parseTheme(json);
    if (!r.ok) { jsonErr = r.error; return; }
    jsonErr = '';
    draft = { light: resolve(r.theme, 'light'), dark: resolve(r.theme, 'dark') };
    toast('info', 'Colours loaded. Check them below, then apply.');
  }

  let note = $state('');
  async function saveAccount() {
    busy = true; note = '';
    try { const cur = (await api.prefsGet()).data; await api.prefsSave({ ...cur, theme: asTheme() }); note = 'Saved to your account. Other browsers can load it from here.'; } catch (e) { toastError(e, 'Could not save to your account'); }
    busy = false;
  }
  async function loadAccount() {
    busy = true; note = '';
    try {
      const d = (await api.prefsGet()).data.theme;
      const r = parseTheme(JSON.stringify({ format: 'moss-theme', ...(d as object) }));
      if (!d || !r.ok) note = 'Your account has no saved colours yet.';
      else { draft = { light: resolve(r.theme, 'light'), dark: resolve(r.theme, 'dark') }; note = 'Loaded from your account. Apply to use it here.'; }
    } catch (e) { note = errMsg(e); }
    busy = false;
  }
</script>

<section class="card" aria-labelledby="te">
  <div class="card-title" id="te"><Icon name="sliders" size={14} />Custom colours</div>
  <p class="muted sm" style="margin:0 0 12px">Change the main colours of the Deep forest (dark) and Paper (light) themes, and System, which uses them. Text must stay readable (WCAG AA), so colours that are too faint cannot be applied. Your colours are kept in this browser{#if custom.theme} and are in use{/if}.</p>

  <div class="row g6" role="radiogroup" aria-label="Which look to edit" style="margin-bottom:14px">
    {#each [['light', 'Light'], ['dark', 'Dark']] as [k, l]}<button class="chip" role="radio" aria-checked={scheme === k} onclick={() => (scheme = k as Scheme)}>{l}</button>{/each}
  </div>

  <div class="te-grid">
    <div class="te-fields">
      {#each TOKENS as t (t.key)}
        <div class="te-row">
          <label for="tc-{t.key}">{t.label}</label>
          <input type="color" class="swatch" id="tc-{t.key}" value={cur[t.key]} oninput={(e) => setColor(t.key, e.currentTarget.value)} />
          <input class="input sm mono" aria-label="{t.label} as hex" value={cur[t.key]} maxlength="7" spellcheck="false" aria-invalid={!isHex(cur[t.key])}
            onchange={(e) => { const v = e.currentTarget.value.trim(); if (isHex(v.startsWith('#') ? v : '#' + v)) setColor(t.key, v.startsWith('#') ? v : '#' + v); else e.currentTarget.value = cur[t.key]; }} />
        </div>
      {/each}
      <div class="row g6 wrap" style="margin-top:6px">
        <button class="btn sm" onclick={() => reset(false)}>Reset {scheme} colours</button>
        <button class="btn sm ghost" onclick={() => reset(true)}>Reset both</button>
      </div>
    </div>

    <div class="te-prev" style={vars} aria-label="Live preview" role="group">
      <div class="prev-bg">
        <div class="prev-card">
          <b>Survival SMP</b>
          <span class="prev-muted">Paper 1.21.4 · 3 players</span>
          <div class="row g6 wrap" style="margin-top:8px">
            <span class="prev-btn">Start</span>
            <span class="prev-badge" style="color:var(--ok)">Running</span>
            <span class="prev-badge" style="color:var(--warn)">Update</span>
            <span class="prev-badge" style="color:var(--danger)">Crashed</span>
            <span class="prev-badge" style="color:var(--info)">Info</span>
          </div>
          <span style="color:var(--a-text);font-weight:600;margin-top:6px">Open console</span>
        </div>
      </div>
      <div class="xs faint" style="margin-top:6px">Preview of the {scheme} look. Nothing changes until you apply.</div>
    </div>
  </div>

  <div class="table-wrap" style="margin-top:16px">
    <table>
      <caption class="sr">Contrast of the {scheme} colours against WCAG AA (4.5 to 1)</caption>
      <thead><tr><th>Pair</th><th class="num">Ratio</th><th>AA</th></tr></thead>
      <tbody>
        {#each results as c (c.id)}
          <tr><td><span class="dotc" style="background:{c.fg}"></span> on <span class="dotc" style="background:{c.bg}"></span> {c.label}</td><td class="num tnum">{c.ratio.toFixed(2)} : 1</td>
            <td>{#if c.ok}<span class="badge ok"><Icon name="check" size={12} />Pass</span>{:else}<span class="badge danger"><Icon name="alert" size={12} />Too faint</span>{/if}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>

  {#if failing.length}<div class="banner warn" role="alert" style="margin-top:12px"><Icon name="alert" size={16} /><span>Can't apply yet. These need more contrast: {failing.join(', ')}.</span></div>{/if}

  <div class="row g6 wrap" style="margin-top:14px">
    <button class="btn primary" disabled={!!failing.length || !changed} onclick={apply}><Icon name="check" size={15} />Apply colours</button>
    {#if custom.theme}<button class="btn" onclick={() => { setCustomTheme(null); reset(true); toast('info', 'Back to the standard colours'); }}>Use standard colours</button>{/if}
    <span class="grow"></span>
    <button class="btn sm" disabled={busy} onclick={saveAccount}>Save to my account</button>
    <button class="btn sm" disabled={busy} onclick={loadAccount}>Load from my account</button>
  </div>
  <div aria-live="polite" class="xs muted" style="margin-top:6px">{note}</div>

  <details style="margin-top:14px">
    <summary style="cursor:pointer;font-weight:550">Share or import as JSON</summary>
    <div class="field" style="margin-top:10px">
      <label for="te-json">Theme JSON</label>
      <textarea id="te-json" class="textarea mono" rows="6" spellcheck="false" bind:value={json} placeholder="Export your colours here, or paste a theme to import."></textarea>
      {#if jsonErr}<span class="hint" style="color:var(--danger)" role="alert">{jsonErr}</span>{/if}
    </div>
    <div class="row g6 wrap" style="margin-top:8px">
      <button class="btn sm" onclick={doExport}>Show my colours as JSON</button>
      <button class="btn sm" onclick={download}><Icon name="download" size={14} />Download</button>
      <button class="btn sm" disabled={!json.trim()} onclick={doImport}>Load pasted colours</button>
    </div>
  </details>
</section>

<style>
  .te-grid { display: grid; grid-template-columns: minmax(240px, 1fr) minmax(240px, 1fr); gap: 20px; align-items: start; }
  @media (max-width: 760px) { .te-grid { grid-template-columns: 1fr; } }
  .te-row { display: grid; grid-template-columns: 1fr 44px 96px; gap: 10px; align-items: center; padding: 5px 0; }
  .te-row label { font-size: var(--fs-sm); font-weight: 550; color: var(--muted); }
  .swatch { width: 44px; height: 30px; padding: 0; border: 1px solid var(--line2); border-radius: 8px; background: none; cursor: pointer; }
  .prev-bg { background: var(--bg); color: var(--text); padding: 16px; border-radius: var(--r); border: 1px solid var(--line2); }
  .prev-card { background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); padding: 14px; display: flex; flex-direction: column; gap: 2px; }
  .prev-muted { color: var(--muted); font-size: var(--fs-sm); }
  .prev-btn { background: var(--a1); color: var(--a-ink); padding: 5px 14px; border-radius: 8px; font-weight: 600; font-size: var(--fs-sm); }
  .prev-badge { background: var(--s3); padding: 2px 9px; border-radius: 99px; font-size: var(--fs-xs); font-weight: 600; }
  .dotc { display: inline-block; width: 12px; height: 12px; border-radius: 3px; border: 1px solid var(--line2); vertical-align: -1px; }
</style>
