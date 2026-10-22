<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import States from '../components/States.svelte';
  import Lazy from '../components/Lazy.svelte';
  import { api, ApiError, errMsg, type JavaInstall, type Settings as S } from '../lib/api';
import { apiOps } from '../lib/api-ops';
  import { confirmDialog, host, maxMemoryMb, prefs, resetA11y, setPref, toast, toastError, trackOp, type Prefs } from '../lib/stores.svelte';
  import { A11Y_DEFAULTS, TEXT_SIZES } from '../lib/a11y';
  import { can, isAdmin } from '../lib/auth.svelte';
  import { duration } from '../lib/format';

  const themes: [Prefs['theme'], string, string[]][] = [
    ['system', 'System', ['#090d0b', '#f5f6f1']], ['dark', 'Deep forest', ['#090d0b', '#1a241f']], ['light', 'Paper', ['#f5f6f1', '#ffffff']], ['contrast', 'High contrast', ['#000', '#fff']],
    ['nether', 'Nether', ['#130907', '#ff8a3d']], ['void', 'Void', ['#0c0913', '#d9a8ff']], ['deepsea', 'Deep sea', ['#061219', '#5fe0d4']],
    ['synthwave', 'Synthwave', ['#140a1f', '#ff6ec7']], ['terminal', 'Amber terminal', ['#0b0904', '#ffb000']], ['midnight', 'Midnight', ['#0b1020', '#9fb4ff']],
    ['campfire', 'Campfire', ['#15100b', '#ffb36b']], ['sakura', 'Cherry blossom', ['#fbf3f5', '#c2436f']], ['desert', 'Desert', ['#f6efe3', '#c0562f']], ['tundra', 'Tundra', ['#eef4f8', '#2a73a8']],
  ];
  type Opt<K extends keyof Prefs> = { key: K; label: string; hint: string; options: [Prefs[K], string][] };
  const a11yOpts: Opt<'motion' | 'focus' | 'links' | 'spacing' | 'effects' | 'contrast'>[] = [
    { key: 'motion', label: 'Motion', hint: 'Reduce turns off animations and transitions.', options: [['system', 'Follow system'], ['reduce', 'Reduce']] },
    { key: 'focus', label: 'Focus indicator', hint: 'Strong draws a thicker ring around whatever has keyboard focus, inputs included.', options: [['standard', 'Standard'], ['strong', 'Strong']] },
    { key: 'links', label: 'Underline links', hint: 'Links in text are underlined all the time, not only on hover.', options: [['hover', 'On hover'], ['always', 'Always']] },
    { key: 'spacing', label: 'Text spacing', hint: 'Wider adds line, letter and word spacing for easier reading.', options: [['standard', 'Standard'], ['wide', 'Wider']] },
    { key: 'effects', label: 'Transparency and effects', hint: 'Reduce removes the background grain, glows and see-through overlays.', options: [['standard', 'Standard'], ['reduce', 'Reduce']] },
    { key: 'contrast', label: 'System high contrast', hint: 'With the System theme, switch to High contrast when your device asks for more contrast.', options: [['system', 'Follow'], ['ignore', 'Ignore']] },
  ];
  const a11yChanged = $derived(prefs.motion !== 'system' || (Object.keys(A11Y_DEFAULTS) as (keyof typeof A11Y_DEFAULTS)[]).some((k) => prefs[k] !== A11Y_DEFAULTS[k]));
  // 'moss' is the theme's own accent: green for the four original themes, the second swatch colour for the others
  const ownAccent = $derived(themes.findIndex(([k]) => k === prefs.theme) > 3 ? themes.find(([k]) => k === prefs.theme)![2][1] : '#7fd36f');
  const accents: [Prefs['accent'], string, string][] = $derived([['moss', 'Theme accent', ownAccent], ['teal', 'Teal', '#4fd1c2'], ['amber', 'Amber', '#f2b04c'], ['violet', 'Violet', '#a98cf5']]);

  let st = $state<S | null>(null);
  let err = $state('');
  let saving = $state(false);
  let seed = '';
  const canAdmin = $derived(can('admin.settings'));
  async function load() { err = ''; try { st = await api.settings(); seed = JSON.stringify(st); } catch (e) { err = errMsg(e); } }
  void load();
  const dirty = $derived(!!st && JSON.stringify(st) !== seed);
  async function save(e: Event) {
    e.preventDefault(); if (!st) return; saving = true;
    try { st = await api.saveSettings(st); seed = JSON.stringify(st); toast('success', 'Settings saved'); } catch (er) { toastError(er, 'Could not save'); }
    saving = false;
  }

  let java = $state<JavaInstall[] | null>(null);
  let jErr = $state('');
  let major = $state(21);
  async function loadJava() { jErr = ''; try { java = (await api.java()).installs; } catch (e) { jErr = errMsg(e); } }
  void loadJava();
  let jUse = $state('');
  const jErrMsg = (e: unknown) => (e instanceof ApiError && e.code === 'java.in_use' ? e.message || 'A running server is using this Java version.' : '');
  async function installJava() {
    jUse = '';
    try { trackOp((await api.installJava(major)).operation); toast('info', `Installing Java ${major}`); } catch (e) { jUse = jErrMsg(e); if (!jUse) toastError(e, 'Install failed'); }
  }
  async function removeJava(j: JavaInstall) {
    jUse = '';
    const r = await confirmDialog({ title: `Remove Java ${j.major}?`, body: 'Deletes the copy Moss downloaded. Servers that need this version will not start until it is installed again.', confirmLabel: 'Remove Java', danger: true });
    if (!r.ok) return;
    try { await apiOps.removeJava(j.major); toast('info', `Removed Java ${j.major}`); void loadJava(); } catch (e) { jUse = jErrMsg(e); if (!jUse) toastError(e, 'Could not remove Java'); }
  }
  const loadBlueprints = () => import('../components/settings/Blueprints.svelte');
  const loadTheme = () => import('../components/settings/ThemeEditor.svelte');
  const loadNotif = () => import('../components/settings/Notifications.svelte');
  const loadTokens = () => import('../components/settings/Tokens.svelte');
  const loadDest = () => import('../components/settings/BackupDestinations.svelte');
  const loadHooks = () => import('../components/settings/Webhooks.svelte');
  const loadLists = () => import('../components/settings/PlayerLists.svelte');
  const loadStatus = () => import('../components/settings/StatusPages.svelte');
  const loadBundle = () => import('../components/settings/SupportBundle.svelte');
</script>

<div class="page-head"><div><h1>Settings</h1><p>Appearance, accessibility, instance defaults, notifications, backup destinations, API tokens and runtimes.</p></div></div>

<div class="col" style="gap:var(--gap)">
  <section class="card" aria-labelledby="ap">
    <div class="card-title" id="ap"><Icon name="sun" size={14} />Appearance</div>
    <div class="label" id="lt">Theme</div>
    <div class="themes" role="radiogroup" aria-labelledby="lt">
      {#each themes as [k, l, sw]}
        <button class="theme" role="radio" aria-checked={prefs.theme === k} onclick={() => setPref('theme', k)}>
          <span class="sw" style="background:linear-gradient(135deg,{sw[0]} 50%,{sw[1]} 50%)"></span>{l}
          {#if prefs.theme === k}<Icon name="check" size={14} />{/if}
        </button>
      {/each}
    </div>
    <div class="grid cols-2" style="margin-top:18px">
      <div><div class="label" id="la">Accent</div><div class="row g6" role="radiogroup" aria-labelledby="la">
        {#each accents as [k, l, c]}<button class="acc" role="radio" aria-checked={prefs.accent === k} aria-label={l} title={l} style="--c:{c}" onclick={() => setPref('accent', k)}>{#if prefs.accent === k}<Icon name="check" size={14} />{/if}</button>{/each}
      </div></div>
      <div><div class="label" id="ld">Density</div><div class="row g6" role="radiogroup" aria-labelledby="ld">
        {#each ['comfortable', 'compact'] as d}<button class="chip" role="radio" aria-checked={prefs.density === d} onclick={() => setPref('density', d as Prefs['density'])} style="text-transform:capitalize">{d}</button>{/each}
      </div></div>
    </div>
  </section>

  <section class="card" aria-labelledby="a11y">
    <div class="card-title" id="a11y"><Icon name="eye" size={14} />Accessibility</div>
    <p class="muted sm">Saved in this browser, like the appearance settings. Moss also follows your system's reduced-motion setting. The High contrast theme is under Appearance.</p>
    <div class="label" id="lts" style="margin-top:14px">Text size</div>
    <div class="row g6 wrap" role="radiogroup" aria-labelledby="lts" aria-describedby="lts-h">
      {#each TEXT_SIZES as n}<button class="chip" role="radio" aria-checked={Number(prefs.textSize) === n} onclick={() => setPref('textSize', n)}>{n}%</button>{/each}
    </div>
    <div class="hint" id="lts-h" style="margin-top:4px">Scales text across the app. Your browser's zoom works too, on top of this.</div>
    <div class="grid cols-2" style="margin-top:18px">
      {#each a11yOpts as o}
        <div>
          <div class="label" id="l-{o.key}">{o.label}</div>
          <div class="row g6 wrap" role="radiogroup" aria-labelledby="l-{o.key}" aria-describedby="h-{o.key}">
            {#each o.options as [v, l]}<button class="chip" role="radio" aria-checked={prefs[o.key] === v} onclick={() => setPref(o.key, v as never)}>{l}</button>{/each}
          </div>
          <div class="hint" id="h-{o.key}" style="margin-top:4px">{o.hint}</div>
        </div>
      {/each}
    </div>
    <div class="row" style="margin-top:18px"><button class="btn sm" disabled={!a11yChanged} onclick={() => { resetA11y(); toast('success', 'Accessibility settings reset'); }}>Reset accessibility settings</button></div>
  </section>

  <Lazy load={loadTheme} />
  {#if isAdmin()}<Lazy load={loadBlueprints} />{/if}

  <section class="card" aria-labelledby="in">
    <div class="card-title" id="in"><Icon name="sliders" size={14} />Instance</div>
    {#if err}<States kind="error" text={err} retry={load} />
    {:else if !st}<States kind="loading" rows={2} />
    {:else}
      <form class="grid cols-2" onsubmit={save}>
        <div class="field"><label for="dm">Default memory (MB)</label><input id="dm" class="input" type="number" min="512" max={Math.max(maxMemoryMb(), st.default_memory_mb)} step="256" bind:value={st.default_memory_mb} readonly={!canAdmin} /></div>
        <div class="field"><label for="bk">Backups to keep</label><input id="bk" class="input" type="number" min="1" bind:value={st.backup_keep} readonly={!canAdmin} /></div>
        <div class="field"><label for="mc">Modrinth contact</label><input id="mc" class="input" bind:value={st.modrinth_contact} readonly={!canAdmin} /><span class="hint">Sent in the User-Agent so Modrinth can reach you.</span></div>
        <div class="field"><label for="pb">Public base URL</label><input id="pb" class="input" bind:value={st.public_base_url} readonly={!canAdmin} /></div>
        <label class="row between" style="cursor:pointer;grid-column:1/-1"><span>Back up before risky changes<span class="hint" style="display:block">Takes a snapshot before restores and in-place updates.</span></span><input type="checkbox" class="switch" bind:checked={st.backup_before_change} disabled={!canAdmin} /></label>
        <div class="field"><span class="label">Data directory</span><code>{st.data_dir}</code></div>
        <div class="field"><span class="label">Listening on</span><code>{st.bind}</code></div>
        {#if canAdmin}<div class="row end" style="grid-column:1/-1"><button class="btn primary" disabled={!dirty || saving}>{saving ? 'Saving…' : 'Save settings'}</button></div>{:else}<p class="hint">Only admins can change instance settings.</p>{/if}
      </form>
    {/if}
  </section>

  {#if canAdmin}<Lazy load={loadNotif} />{/if}
  {#if canAdmin}<Lazy load={loadDest} />{/if}
  {#if canAdmin}<Lazy load={loadHooks} />{/if}
  {#if canAdmin}<Lazy load={loadLists} />{/if}
  {#if canAdmin}<Lazy load={loadStatus} />{/if}

  <section class="card" aria-labelledby="jv">
    <div class="card-title" id="jv"><Icon name="package" size={14} />Java installs</div>
    {#if jErr}<States kind="error" text={jErr} retry={loadJava} />
    {:else if !java}<States kind="loading" rows={2} />
    {:else}
      {#if !java.length}<p class="muted sm">No Java found. Install one to run servers.</p>
      {:else}<ul class="jl">{#each java as j}<li><b>Java {j.major}</b><span class="muted tnum">{j.version}</span><span class="badge {j.managed ? 'ok' : ''}">{j.managed ? 'Managed by Moss' : 'System'}</span><code class="xs ellipsis grow">{j.path}</code>{#if canAdmin && j.managed}<button class="btn danger sm" onclick={() => removeJava(j)} aria-label="Remove managed Java {j.major}"><Icon name="trash" size={13} />Remove</button>{/if}</li>{/each}</ul>{/if}
      <div aria-live="assertive">{#if jUse}<div class="banner danger" role="alert" style="margin-top:10px"><Icon name="alert" size={16} /><div><b>Java is in use</b><div class="muted">{jUse}</div><div class="muted">Stop those servers, then try again.</div></div></div>{/if}</div>
      {#if canAdmin}
        <div class="row" style="margin-top:14px"><select class="select" style="width:140px" aria-label="Java major version" bind:value={major}>{#each [8, 11, 17, 21, 25] as m}<option value={m}>Java {m}</option>{/each}</select><button class="btn" onclick={installJava}><Icon name="download" size={15} />Install</button></div>
      {/if}
    {/if}
  </section>

  <Lazy load={loadTokens} />
  {#if canAdmin}<Lazy load={loadBundle} />{/if}

  <section class="card" aria-labelledby="ab">
    <div class="card-title" id="ab"><Icon name="info" size={14} />About</div>
    <dl class="ab">
      <div><dt>Version</dt><dd>{host.status?.version ?? st?.version ?? '—'}</dd></div>
      <div><dt>Uptime</dt><dd class="tnum">{host.status ? duration(host.status.uptime_s) : '—'}</dd></div>
      <div><dt>Host</dt><dd>{host.status?.host.hostname ?? '—'}</dd></div>
      <div><dt>Operating system</dt><dd>{host.status?.host.os ?? '—'}</dd></div>
      <div><dt>Auth mode</dt><dd>{host.status?.auth_mode ?? '—'}</dd></div>
    </dl>
  </section>
</div>

<style>
  .themes { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin-top: 8px; }
  .theme { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 10px; background: var(--s2); border: 1px solid var(--line2); color: var(--text); cursor: pointer; font-weight: 550; font-size: var(--fs-sm); }
  .theme[aria-checked='true'] { border-color: var(--a2); background: var(--a-soft); }
  .sw { width: 26px; height: 26px; border-radius: 8px; border: 1px solid var(--line2); flex: none; }
  .acc { width: 30px; height: 30px; border-radius: 50%; background: var(--c); border: 2px solid var(--s1); box-shadow: 0 0 0 1px var(--line2); display: grid; place-items: center; color: #08130a; cursor: pointer; }
  .acc[aria-checked='true'] { box-shadow: 0 0 0 2px var(--c); }
  .label { margin-bottom: 8px; }
  .jl { list-style: none; margin: 0; padding: 0; } .jl li { display: flex; align-items: center; gap: 12px; padding: 8px 0; border-bottom: 1px solid var(--line); flex-wrap: wrap; } .jl li:last-child { border: 0; }
  .ab { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 14px; margin: 0; } .ab dt { font-size: var(--fs-xs); color: var(--muted); } .ab dd { margin: 2px 0 0; font-weight: 550; }
</style>
