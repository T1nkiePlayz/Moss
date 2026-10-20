<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import OpCard from '../components/OpCard.svelte';
  import States from '../components/States.svelte';
  import { api, ApiError, errMsg, type ImportDetection, type Runtime } from '../lib/api';
  import { ops, trackOp } from '../lib/stores.svelte';
  import { bytes } from '../lib/format';
  import { navigate } from '../lib/router.svelte';
  import { serverHref } from '../lib/route';
  import { isAdmin } from '../lib/auth.svelte';

  let mode = $state<'upload' | 'path'>('upload');
  let path = $state('');
  let file = $state<File | null>(null);
  let det = $state<ImportDetection | null>(null);
  let busy = $state(false);
  let err = $state('');
  let fieldErr = $state<Record<string, string>>({});
  let runtimes = $state<Runtime[]>([]);
  let f = $state({ name: '', runtime: '', game_version: '', memory_mb: 2048, port: null as number | null, eula: false });
  let created = $state<{ opId: string; serverId: string } | null>(null);
  const admin = isAdmin();
  $effect(() => { api.runtimes().then((r) => (runtimes = r.filter((x) => x.installable && x.id !== 'custom')), () => (runtimes = [])); });

  async function inspect() {
    busy = true; err = ''; det = null;
    try {
      const d = mode === 'path' ? await api.importInspectPath(path.trim()) : await api.importInspectFile(file!);
      det = d;
      f = { name: d.name.slice(0, 48), runtime: d.runtime ?? '', game_version: d.game_version ?? '', memory_mb: Math.max(1024, f.memory_mb), port: d.port, eula: false };
    } catch (e) { err = errMsg(e); }
    busy = false;
  }
  const nameOk = $derived(f.name.trim().length >= 1 && f.name.length <= 48);
  const ready = $derived(!!det && nameOk && !!f.runtime && !!f.game_version.trim() && f.eula && (f.port == null || (f.port >= 1024 && f.port <= 65535)));
  async function start() {
    if (!det) return;
    busy = true; err = ''; fieldErr = {};
    try {
      const r = await api.importServer({
        name: f.name.trim(), runtime: f.runtime, game_version: f.game_version.trim(), memory_mb: f.memory_mb, accept_eula: f.eula,
        ...(f.port ? { port: f.port } : {}),
        ...(mode === 'path' ? { path: path.trim() } : { token: det.token }),
      });
      trackOp(r.operation);
      created = { opId: r.operation.id, serverId: r.server.id };
    } catch (e) {
      if (e instanceof ApiError && e.fields.length) fieldErr = Object.fromEntries(e.fields.map((x) => [x.path, x.message]));
      err = errMsg(e);
    }
    busy = false;
  }
  const op = $derived(created ? ops.list.find((o) => o.id === created!.opId) : undefined);
  $effect(() => { if (op?.state === 'completed' && created) { const id = created.serverId; const t = setTimeout(() => navigate(serverHref(id)), 1200); return () => clearTimeout(t); } });
</script>

<div class="page-head"><div><h1>Import a server</h1><p>Bring in a server that already exists. Moss copies it, so the original stays untouched.</p></div></div>

{#if created}
  <div class="card form">
    <h2>{op?.state === 'completed' ? 'Server imported' : op?.state === 'failed' ? 'Import failed' : 'Importing…'}</h2>
    {#if op}<OpCard {op} open />{:else}<States kind="loading" rows={1} />{/if}
    <div class="row end"><a class="btn" href="#/servers">All servers</a><a class="btn primary" href={serverHref(created.serverId)}>Open server</a></div>
  </div>
{:else}
  <section class="card form col" style="gap:14px">
    <div class="row g6" role="group" aria-label="Where is the server">
      <button class="chip" aria-pressed={mode === 'upload'} onclick={() => { mode = 'upload'; det = null; err = ''; }}>Upload an archive</button>
      {#if admin}<button class="chip" aria-pressed={mode === 'path'} onclick={() => { mode = 'path'; det = null; err = ''; }}>Folder on this machine</button>{/if}
    </div>
    {#if mode === 'upload'}
      <div class="field"><label for="imp-file">Server archive</label>
        <input id="imp-file" class="input" type="file" accept=".zip,.tar.zst,.zst" onchange={(e) => { file = e.currentTarget.files?.[0] ?? null; det = null; }} />
        <span class="hint">A .zip or .tar.zst of the server folder, up to 512 MiB.</span></div>
    {:else}
      <div class="field"><label for="imp-path">Folder path</label>
        <input id="imp-path" class="input mono" placeholder="/srv/minecraft/survival" bind:value={path} oninput={() => (det = null)} spellcheck="false" />
        <span class="hint">An absolute path on the machine Moss runs on. Only administrators can use this.</span></div>
    {/if}
    {#if err && !det}<div class="banner danger" role="alert"><Icon name="alert" size={16} /><span>{err}</span></div>{/if}
    <div class="row"><button class="btn primary" disabled={busy || (mode === 'upload' ? !file : !path.trim().startsWith('/'))} onclick={inspect}>{busy && !det ? 'Looking…' : 'Look at it'}</button></div>
  </section>

  {#if det}
    <section class="card form col" style="gap:14px;margin-top:16px" aria-live="polite">
      <h2 style="margin:0">What Moss found</h2>
      <dl class="found">
        <div><dt>Runtime</dt><dd>{det.runtime ?? 'Not sure'}{det.game_version ? ` ${det.game_version}` : ''}{det.loader_version ? ` (loader ${det.loader_version})` : ''}</dd></div>
        <div><dt>Port</dt><dd class="tnum">{det.port ?? 'Not set'}</dd></div>
        <div><dt>Java</dt><dd>{det.java_major ? `Java ${det.java_major}${det.java_installed ? '' : ' (not installed yet, Moss will fetch it on first start)'}` : 'Not sure'}</dd></div>
        <div><dt>World</dt><dd>{det.has_world ? 'Included' : 'None found'}</dd></div>
        <div><dt>Size</dt><dd class="tnum">{bytes(det.size)}</dd></div>
      </dl>
      {#each det.notes as n}<div class="banner warn"><Icon name="alert" size={16} /><span>{n}</span></div>{/each}

      <div class="grid2">
        <div class="field"><label for="i-name">Name</label><input id="i-name" class="input" maxlength="48" bind:value={f.name} aria-invalid={!nameOk} />{#if fieldErr.name}<span class="err">{fieldErr.name}</span>{/if}</div>
        <div class="field"><label for="i-rt">Runtime</label>
          <select id="i-rt" class="select" bind:value={f.runtime}><option value="">Choose…</option>{#each runtimes as r (r.id)}<option value={r.id}>{r.name}</option>{/each}</select>{#if fieldErr.runtime}<span class="err">{fieldErr.runtime}</span>{/if}</div>
        <div class="field"><label for="i-ver">Minecraft version</label><input id="i-ver" class="input" bind:value={f.game_version} placeholder="1.21.4" />{#if fieldErr.game_version}<span class="err">{fieldErr.game_version}</span>{/if}</div>
        <div class="field"><label for="i-mem">Memory (MB)</label><input id="i-mem" class="input" type="number" min="512" step="256" bind:value={f.memory_mb} /></div>
        <div class="field"><label for="i-port">Port</label><input id="i-port" class="input" type="number" min="1024" max="65535" placeholder="Auto-assign" bind:value={f.port} />{#if fieldErr.port}<span class="err">{fieldErr.port}</span>{/if}</div>
      </div>
      <label class="row g6 eula"><input type="checkbox" class="check" bind:checked={f.eula} /><span class="sm">I accept the <a href="https://aka.ms/MinecraftEULA" target="_blank" rel="noreferrer noopener">Minecraft EULA</a>. This acceptance is recorded and audited.</span></label>
      {#if err}<div class="banner danger" role="alert"><Icon name="alert" size={16} /><span>{err}</span></div>{/if}
      <div class="row end"><button class="btn primary lg" disabled={busy || !ready} onclick={start}>{busy ? 'Importing…' : 'Import server'}</button></div>
    </section>
  {/if}
{/if}

<style>
  .found { display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 10px 20px; margin: 0; }
  .found dt { font-size: var(--fs-xs); color: var(--muted); }
  .found dd { margin: 2px 0 0; font-weight: 550; }
  .grid2 { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 14px; }
  .eula a { text-decoration: underline; }
  .eula { align-items: flex-start; padding: 12px; background: var(--s2); border-radius: var(--r-sm); border: 1px solid var(--line); }
</style>
