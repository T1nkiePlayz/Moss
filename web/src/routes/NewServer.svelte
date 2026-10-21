<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import States from '../components/States.svelte';
  import OpCard from '../components/OpCard.svelte';
  import Lazy from '../components/Lazy.svelte';
  import BlueprintStart from '../components/BlueprintStart.svelte';
  import { api, ApiError, errMsg, type Runtime, type Versions, type JavaInstall, type ModpackInspect } from '../lib/api';
  import { host, ops, trackOp, toastError } from '../lib/stores.svelte';
  import { bytes } from '../lib/format';
  import { runtimeIcon } from '../lib/runtime-icons';
  import { navigate } from '../lib/router.svelte';
  import { serverHref } from '../lib/route';

  let step = $state(1);
  let runtimes = $state<Runtime[] | null>(null);
  let rtErr = $state('');
  let runtime = $state<Runtime | null>(null);
  let versions = $state<Versions | null>(null);
  let vErr = $state('');
  let vFilter = $state<'stable' | 'all'>('stable');
  let version = $state('');
  let loaderVer = $state('');
  // "From modpack" path: pick/inspect a pack first; its runtime, game version and loader then come from the pack.
  let fromPack = $state(false);
  let pack = $state<ModpackInspect | null>(null);
  let packReady = $state(false);
  let packErr = $state('');
  let name = $state('');
  let memory = $state(2048);
  let port = $state<number | null>(null);
  let command = $state('java -jar server.jar nogui');
  let eula = $state(false);
  let java = $state<JavaInstall[] | null>(null);
  let busy = $state(false);
  let created = $state<{ opId: string; serverId: string } | null>(null);
  let fieldErr = $state<Record<string, string>>({});
  let formErr = $state('');

  const catLabel: Record<string, string> = { official: 'Official', plugins: 'Plugin servers', modded: 'Modded', proxy: 'Proxies', custom: 'Custom', game: 'Other games' };
  const kindLabel: Record<string, string> = { plugins: 'Plugins', mods: 'Mods', proxy_plugins: 'Proxy plugins', datapacks: 'Datapacks', none: 'No add-ons' };

  async function loadRt() {
    rtErr = '';
    try { runtimes = await api.runtimes(); } catch (e) { rtErr = errMsg(e); }
  }
  void loadRt();

  // One entry per id; an installable entry wins over a "planned" placeholder of the same runtime.
  const usableRuntimes = $derived.by(() => {
    const m = new Map<string, Runtime>();
    for (const r of runtimes ?? []) { const o = m.get(r.id); if (!o || (!o.installable && r.installable)) m.set(r.id, r); }
    return [...m.values()];
  });
  // Minecraft runtimes grouped by kind, then the other games, then the custom command.
  const mcGroups = $derived.by(() => {
    const g: Record<string, Runtime[]> = {};
    for (const r of usableRuntimes) if (r.category !== 'game' && r.category !== 'custom') (g[r.category] ??= []).push(r);
    return Object.entries(g);
  });
  const gameList = $derived(usableRuntimes.filter((r) => r.category === 'game'));
  const customList = $derived(usableRuntimes.filter((r) => r.category === 'custom'));
  const isGame = $derived(runtime?.category === 'game');
  const platformNote = (r: Runtime) => (r.platforms ?? []).map((p) => (p === 'macos' ? 'macOS' : 'Linux')).join(' + ');
  const trackNote = (r: Runtime) => (r.player_tracking === 'none' ? 'no player counts' : 'player counts');
  const loaders = $derived(versions?.loader_versions?.[version] ?? []);
  $effect(() => { const l = loaders; if (!fromPack) loaderVer = l.find((x) => x.recommended)?.id ?? l[0]?.id ?? ''; });

  function startPack() { fromPack = true; pack = null; packReady = false; packErr = ''; runtime = null; versions = null; version = ''; step = 2; }
  function onPack(m: ModpackInspect | null) {
    pack = m; packErr = '';
    if (!m) return;
    const r = usableRuntimes.find((x) => x.id === m.loader.id);
    if (!r || !r.installable) { packErr = `This pack needs ${m.loader.id}, which Moss cannot install yet.`; return; }
    runtime = r; version = m.game_version; loaderVer = m.loader.version ?? '';
    name = m.name.slice(0, 48); memory = Math.max(memory, 4096);
  }

  async function pick(r: Runtime) {
    if (!r.installable) return;
    fromPack = false; pack = null; packReady = false;
    runtime = r;
    versions = null; vErr = ''; version = '';
    if (r.id !== 'custom') {
      api.versions(r.id).then((v) => { versions = v; version = v.recommended; }).catch((e) => (vErr = errMsg(e)));
    }
    if (!name) name = r.name === 'Custom command' ? 'Custom server' : `My ${r.name} server`;
    memory = r.category === 'game' ? 2048 : r.id === 'velocity' ? 512 : Math.max(memory, 2048);
    step = 2;
  }

  const shown = $derived((versions?.versions ?? []).filter((v) => vFilter === 'all' || v.stable));
  const needsEula = $derived(!!runtime && (runtime.category === 'game' ? !!runtime.needs_eula : runtime.id !== 'velocity' && runtime.id !== 'custom'));
  const hasUnstable = $derived((versions?.versions ?? []).some((v) => !v.stable));
  const maxMem = $derived(Math.max(4096, Math.floor(((host.status?.host.mem_total ?? 16e9) / 1048576) * 0.9 / 256) * 256));
  // Rough Java requirement by game version (informational; the backend decides the real value).
  const javaNeed = $derived.by(() => {
    if (runtime?.id === 'velocity') return 17;
    const m = version.match(/^1\.(\d+)(?:\.(\d+))?/);
    if (!m) return 21;
    const minor = +m[1], patch = +(m[2] ?? 0);
    return minor > 20 || (minor === 20 && patch >= 5) ? 21 : minor >= 18 ? 17 : minor === 17 ? 16 : 8;
  });
  const javaOk = $derived(java?.some((j) => j.major >= javaNeed));
  const nameOk = $derived(name.trim().length >= 1 && name.length <= 48);
  const portOk = $derived(port == null || (port >= 1024 && port <= 65535));
  const canNext2 = $derived(nameOk && portOk && (runtime?.id === 'custom' ? command.trim().length > 0 : !!version) && (!fromPack || !!pack));

  async function toReview() {
    step = 3;
    api.java().then((j) => (java = j.installs)).catch(() => (java = []));
  }
  async function installJava() {
    try { trackOp((await api.installJava(javaNeed)).operation); } catch (e) { toastError(e); }
  }

  async function create() {
    if (!runtime) return;
    busy = true; fieldErr = {}; formErr = '';
    try {
      const r = await api.createServer({
        name: name.trim(), runtime: runtime.id, game_version: runtime.id === 'custom' ? 'custom' : version, memory_mb: isGame ? 2048 : memory,
        port: port ?? undefined, accept_eula: needsEula ? eula : false,
        ...((fromPack || loaders.length) && loaderVer ? { loader_version: loaderVer } : {}),
        ...(fromPack && pack ? { modpack_token: pack.token } : {}),
        ...(runtime.id === 'custom' ? { custom: { command: command.trim().split(/\s+/) } } : {}),
      });
      trackOp(r.operation);
      created = { opId: r.operation.id, serverId: r.server.id };
      step = 4;
    } catch (e) {
      if (e instanceof ApiError && e.fields.length) { fieldErr = Object.fromEntries(e.fields.map((f) => [f.path, f.message])); step = 2; }
      formErr = errMsg(e);
    }
    busy = false;
  }
  const op = $derived(created ? ops.list.find((o) => o.id === created!.opId) : undefined);
  $effect(() => { if (op?.state === 'completed' && created) { const id = created.serverId; const t = setTimeout(() => navigate(serverHref(id)), 1200); return () => clearTimeout(t); } });
  const steps = ['Runtime', 'Version & details', 'Review'];
</script>

<div class="page-head"><div><h1>New server</h1><p>Create a server in three steps.</p></div></div>


<ol class="steps" aria-label="Progress">
  {#each steps as s, i}
    <li class:on={step === i + 1} class:done={step > i + 1 || step === 4}>
      <span class="n">{#if step > i + 1 || step === 4}<Icon name="check" size={13} />{:else}{i + 1}{/if}</span><span class="lb">{s}</span>
    </li>
  {/each}
</ol>

{#snippet rtCard(r: Runtime)}
  {@const off = !r.installable}
  {@const tier = r.tier === 'planned' && r.installable ? 'experimental' : r.tier}
  <button class="card hover rt" class:off class:sel={runtime?.id === r.id} disabled={off} onclick={() => pick(r)} aria-label="{r.name}, {tier}">
    <div class="row between" style="align-items:flex-start">
      <div class="rico"><Icon name={runtimeIcon(r)} size={20} /></div>
      <span class="badge {tier === 'stable' ? 'ok' : tier === 'experimental' ? 'warn' : ''}">{tier === 'planned' ? 'Planned' : tier === 'stable' ? 'Stable' : 'Experimental'}</span>
    </div>
    <h3>{r.name}</h3>
    <p class="muted sm">{r.description}</p>
    {#if r.category === 'game'}
      <div class="xs faint">{platformNote(r)} · {r.default_port}/{r.protocol} · {trackNote(r)}{off ? ' · not supported on this host' : ''}</div>
    {:else}<div class="xs faint">{kindLabel[r.content_kind] ?? ''}</div>{/if}
  </button>
{/snippet}

{#if step === 1}
  {#if rtErr}<States kind="error" text={rtErr} retry={loadRt} />
  {:else if !runtimes}<States kind="loading" rows={3} />
  {:else}
    <BlueprintStart oncreated={(r) => { trackOp(r.operation); created = { opId: r.operation.id, serverId: r.server.id }; step = 4; }} />
    <h3 class="cat">Start from a modpack</h3>
    <div class="grid cols-auto" style="--gap:12px">
      <button class="card hover rt" onclick={startPack}>
        <div class="row between" style="align-items:flex-start"><div class="rico"><Icon name="package" size={20} /></div><span class="badge info">New</span></div>
        <h3>From modpack</h3>
        <p class="muted sm">Upload a .mrpack or CurseForge .zip, or paste a Modrinth/CurseForge link. Moss sets up the right loader, version and mods.</p>
        <div class="xs faint">Mods</div>
      </button>
    </div>
    <h2 class="sect">Minecraft</h2>
    {#each mcGroups as [cat, list]}
      <h3 class="cat">{catLabel[cat] ?? cat}</h3>
      <div class="grid cols-auto" style="--gap:12px">{#each list as r (r.id)}{@render rtCard(r)}{/each}</div>
    {/each}
    {#if gameList.length}
      <h2 class="sect">Other games</h2>
      <div class="grid cols-auto" style="--gap:12px">{#each gameList as r (r.id)}{@render rtCard(r)}{/each}</div>
    {/if}
    {#if customList.length}
      <h2 class="sect">Custom</h2>
      <div class="grid cols-auto" style="--gap:12px">{#each customList as r (r.id)}{@render rtCard(r)}{/each}</div>
    {/if}
  {/if}
{:else if step === 2 && fromPack && !packReady}
  <div class="card form" style="max-width:760px">
    <div class="row between"><h2>From modpack</h2><button class="btn ghost sm" onclick={() => { fromPack = false; pack = null; step = 1; }}><Icon name="chevron-left" size={14} />Change</button></div>
    <Lazy load={() => import('../components/content/ModpackPicker.svelte')} props={{ onresult: onPack }} />
    {#if packErr}<div class="banner danger" role="alert"><Icon name="x-circle" size={16} />{packErr}</div>{/if}
    <div class="row end"><button class="btn primary" disabled={!pack || !!packErr} onclick={() => (packReady = true)}>Continue<Icon name="chevron-right" size={15} /></button></div>
  </div>
{:else if step === 2 && runtime}
  <div class="card form">
    <div class="row between"><h2>{runtime.name}</h2><button class="btn ghost sm" onclick={() => (fromPack ? (packReady = false) : (step = 1))}><Icon name="chevron-left" size={14} />{fromPack ? 'Change pack' : 'Change'}</button></div>
    {#if fromPack && pack}
      <div class="banner" style="align-items:center"><Icon name="package" size={18} /><div class="grow"><b>{pack.name}</b> {pack.version}<div class="xs muted">{pack.loader.id}{pack.loader.version ? ' ' + pack.loader.version : ''} · Minecraft {pack.game_version} · {pack.files.server} server mods</div></div></div>
      {#if pack.files.unavailable}<div class="banner warn"><Icon name="alert" size={16} />{pack.files.unavailable} file{pack.files.unavailable === 1 ? '' : 's'} cannot be downloaded automatically; you can add {pack.files.unavailable === 1 ? 'it' : 'them'} in Files afterwards.</div>{/if}
    {:else if runtime.id !== 'custom'}
      <div class="field">
        <div class="row between wrap">
          <label for="ver">Version</label>
          <div class="row g6" role="group" aria-label="Version channel">
            <button class="chip" aria-pressed={vFilter === 'stable'} onclick={() => (vFilter = 'stable')}>Stable</button>
            {#if !isGame || hasUnstable}<button class="chip" aria-pressed={vFilter === 'all'} onclick={() => (vFilter = 'all')}>{isGame ? 'Include experimental' : 'Include snapshots'}</button>{/if}
          </div>
        </div>
        {#if vErr}<States kind="error" text={vErr} retry={() => pick(runtime!)} />
        {:else if !versions}<div class="skel" style="height:36px"></div>
        {:else}
          <select id="ver" class="select" bind:value={version}>
            {#each shown as v}<option value={v.id}>{v.id}{v.id === versions.recommended ? ' (recommended)' : ''}{v.stable ? '' : isGame ? ' (experimental)' : ' (snapshot)'}</option>{/each}
          </select>
          {#if version === versions.recommended}<span class="badge ok" style="width:fit-content"><Icon name="star" size={12} />Recommended</span>{/if}
        {/if}
        {#if fieldErr.game_version}<span class="err">{fieldErr.game_version}</span>{/if}
      </div>
      {#if loaders.length}
        <div class="field">
          <label for="lver">{runtime.name} version</label>
          <select id="lver" class="select" bind:value={loaderVer}>
            {#each loaders as l (l.id)}<option value={l.id}>{l.id}{l.recommended ? ' (recommended)' : ''}</option>{/each}
          </select>
          {#if fieldErr.loader_version}<span class="err">{fieldErr.loader_version}</span>{/if}
        </div>
      {/if}
    {:else}
      <div class="field"><label for="cmd">Command</label><textarea id="cmd" class="textarea mono" rows="2" bind:value={command}></textarea><span class="hint">Runs in the server directory. Split on spaces.</span></div>
    {/if}
    <div class="field">
      <label for="nm">Name</label>
      <input id="nm" class="input" maxlength="48" bind:value={name} aria-invalid={!nameOk || !!fieldErr.name} />
      {#if fieldErr.name}<span class="err">{fieldErr.name}</span>{/if}
    </div>
    {#if !isGame}<div class="field">
      <div class="row between"><label for="mem">Memory</label><span class="tnum"><b>{memory >= 1024 ? (memory / 1024).toFixed(memory % 1024 ? 2 : 0) + ' GB' : memory + ' MB'}</b></span></div>
      <input id="mem" type="range" min="512" max={maxMem} step="256" bind:value={memory} />
      <span class="hint">Host has {host.status ? bytes(host.status.host.mem_total) : '…'} in total. Leave room for the OS and other servers.</span>
      {#if fieldErr.memory_mb}<span class="err">{fieldErr.memory_mb}</span>{/if}
    </div>{/if}
    <div class="field">
      <label for="pt">Port <span class="faint">(optional)</span></label>
      <input id="pt" class="input" type="number" min="1024" max="65535" placeholder={runtime.default_port ? `Auto-assign (starts at ${runtime.default_port})` : 'Auto-assign'} bind:value={port} aria-invalid={!portOk || !!fieldErr.port} />
      {#if runtime.category === 'game'}<span class="hint">Players connect on {runtime.default_port} ({runtime.protocol?.toUpperCase()}) by default; forward that port on your router.</span>{/if}
      {#if fieldErr.port}<span class="err">{fieldErr.port}</span>{/if}
    </div>
    {#if isGame}<div class="banner" style="align-items:center"><Icon name="info" size={16} /><div class="grow sm">{runtime.description} Edit the game's own config from the Files tab after installing.</div></div>{/if}
    {#if formErr && step === 2}<div class="banner danger" role="alert">{formErr}</div>{/if}
    <div class="row end"><button class="btn" onclick={() => (step = 1)}>Back</button><button class="btn primary" disabled={!canNext2} onclick={toReview}>Review<Icon name="chevron-right" size={15} /></button></div>
  </div>
{:else if step === 3 && runtime}
  <div class="card form">
    <h2>Review</h2>
    <dl class="sum">
      <dt>Name</dt><dd>{name}</dd>
      <dt>Runtime</dt><dd>{runtime.name}{runtime.id !== 'custom' ? ' ' + version : ''}{loaderVer && runtime.id !== 'custom' ? ` · loader ${loaderVer}` : ''}</dd>
      {#if fromPack && pack}<dt>Modpack</dt><dd>{pack.name} {pack.version}<div class="xs muted">{pack.files.server} server mods will be downloaded and installed</div></dd>{/if}
      {#if !isGame}<dt>Memory</dt><dd class="tnum">{memory} MB</dd>{/if}
      <dt>Port</dt><dd class="tnum">{port ?? 'Auto'}{isGame ? ` (${runtime.protocol?.toUpperCase()})` : ''}</dd>
      {#if !isGame}<dt>Add-ons</dt><dd>{kindLabel[runtime.content_kind]}</dd>{/if}
    </dl>
    {#if isGame}<div class="banner" style="align-items:center"><Icon name="download" size={16} /><div class="grow sm">Moss downloads the server from its official source when you create it{runtime.player_tracking === 'none' ? '' : '; player counts appear on the dashboard once it is running'}.</div></div>{/if}
    {#if runtime.id !== 'custom' && !isGame}
      {#if !java}<div class="skel" style="height:44px"></div>
      {:else if javaOk}<div class="banner" style="align-items:center"><span style="color:var(--a-text)"><Icon name="check-circle" size={18} /></span>Java {javaNeed}+ is installed ({java.map((j) => j.major).join(', ')}).</div>
      {:else}<div class="banner warn" style="align-items:center"><Icon name="alert" size={18} /><div class="grow">This version needs Java {javaNeed}+ and none is installed. You can create the server now, but it will not start until Java is available.</div><button class="btn sm" onclick={installJava}>Install Java {javaNeed}</button></div>{/if}
    {/if}
    {#if needsEula}
      <label class="row eula"><input type="checkbox" class="check" bind:checked={eula} /><span class="sm">I accept the <a href="https://aka.ms/MinecraftEULA" target="_blank" rel="noreferrer noopener">Minecraft EULA <Icon name="external" size={12} /></a>. This acceptance is recorded and audited.</span></label>
    {/if}
    {#if formErr}<div class="banner danger" role="alert">{formErr}</div>{/if}
    <div class="row end"><button class="btn" onclick={() => (step = 2)}>Back</button><button class="btn primary lg" disabled={busy || (needsEula && !eula)} onclick={create}>{busy ? 'Creating…' : 'Create server'}</button></div>
  </div>
{:else if step === 4 && created}
  <div class="card form">
    <h2>{op?.state === 'completed' ? 'Server ready' : op?.state === 'failed' ? 'Install failed' : 'Setting things up…'}</h2>
    {#if op}<OpCard {op} open />{:else}<States kind="loading" rows={1} />{/if}
    <div class="row end">
      <a class="btn" href="#/servers">All servers</a>
      <a class="btn primary" href={serverHref(created.serverId)}>Open server</a>
    </div>
  </div>
{/if}

<style>
  .steps { list-style: none; display: flex; gap: 8px; padding: 0; margin: 0 0 24px; flex-wrap: wrap; }
  .steps li { display: flex; align-items: center; gap: 8px; color: var(--faint); font-size: var(--fs-sm); font-weight: 600; padding-right: 14px; }
  .steps li:not(:last-child)::after { content: ''; width: 28px; height: 1px; background: var(--line2); margin-left: 6px; }
  .n { width: 24px; height: 24px; border-radius: 50%; display: grid; place-items: center; border: 1px solid var(--line2); font-size: 12px; }
  .on { color: var(--text); } .on .n { background: linear-gradient(135deg, var(--a1), var(--a2)); color: var(--a-ink); border-color: transparent; box-shadow: 0 0 14px var(--a-glow); }
  .done .n { color: var(--a-text); border-color: var(--a2); }
  @media (max-width: 560px) { .lb { display: none; } .on .lb { display: inline; } }
  .cat { margin: 22px 0 10px; font-size: var(--fs-sm); text-transform: uppercase; letter-spacing: .08em; color: var(--muted); }
  .cat:first-of-type { margin-top: 0; }
  .sect { margin: 28px 0 6px; font-size: var(--fs-lg, 1.05rem); }
  .rt { text-align: left; display: flex; flex-direction: column; gap: 6px; cursor: pointer; color: inherit; font: inherit; }
  .rt.sel { border-color: var(--a2); }
  .rt.off { opacity: .5; cursor: not-allowed; }
  .rico { width: 38px; height: 38px; border-radius: 10px; display: grid; place-items: center; background: var(--a-soft); color: var(--a-text); }
  .form { max-width: 640px; display: flex; flex-direction: column; gap: 18px; }
  .sum { display: grid; grid-template-columns: 110px 1fr; gap: 8px 16px; margin: 0; } .sum dt { color: var(--muted); } .sum dd { margin: 0; font-weight: 550; }
  .eula { gap: 10px; align-items: flex-start; padding: 12px; background: var(--s2); border-radius: var(--r-sm); border: 1px solid var(--line); cursor: pointer; }
</style>
