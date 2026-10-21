<script lang="ts">
  import { runtimeLabel } from '../../lib/format';
  import Icon from '../../components/Icon.svelte';
  import LimitsForm from '../../components/LimitsForm.svelte';
  import IsolationForm from '../../components/IsolationForm.svelte';
  import { canReinstall, reinstall } from '../../lib/lifecycle';
  import { api, apiOps, ApiError, errMsg, isGameServer, type GameUpdate, type Server } from '../../lib/api';
  import { confirmDialog, toast, toastError, trackOp } from '../../lib/stores.svelte';
  import { can, isAdmin } from '../../lib/auth.svelte';
  import { navigate } from '../../lib/router.svelte';

  let { server: s }: { server: Server } = $props();
  const canEdit = $derived(can('config.write'));
  const canDelete = $derived(can('server.delete'));
  const showReinstall = $derived(canEdit && canReinstall(s));

  const showUpdates = $derived(isGameServer(s));
  let upd = $state<GameUpdate | null>(null);
  let updErr = $state('');
  let checking = $state(false);
  async function checkUpdate(refresh = false) {
    checking = true; updErr = '';
    try { upd = await apiOps.gameUpdate(s.id, refresh); } catch (e) { upd = null; updErr = errMsg(e); } finally { checking = false; }
  }
  $effect(() => { if (showUpdates) { s.id; checkUpdate(); } });
  async function runUpdate() {
    const r = await confirmDialog({ title: `Update ${s.name}?`, body: 'Takes a backup first, then stops the server, installs the new version and starts it again if it was running. Worlds and settings are kept.', confirmLabel: 'Update server' });
    if (!r.ok) return;
    try { trackOp((await apiOps.startGameUpdate(s.id)).operation); toast('info', 'Update started', { body: s.name }); } catch (e) { toastError(e, 'Could not start the update'); }
  }

  let f = $state({ name: '', memory_mb: 0, port: 0, flags_profile: 'balanced' as Server['flags_profile'], autostart: false, rp_enabled: false, rp_max: 3, tags: '', notes: '' });
  let seed = '';
  const fromServer = () => ({ name: s.name, memory_mb: s.memory_mb, port: s.port, flags_profile: s.flags_profile, autostart: s.autostart, rp_enabled: s.restart_policy.enabled, rp_max: s.restart_policy.max_per_10m, tags: s.tags.join(', '), notes: s.notes });
  $effect(() => { if (seed !== s.id) { seed = s.id; f = fromServer(); } });
  let saving = $state(false);
  let errs = $state<Record<string, string>>({});
  const dirty = $derived(JSON.stringify(f) !== JSON.stringify(fromServer()));
  const needsRestart = $derived((f.memory_mb !== s.memory_mb || f.port !== s.port || f.flags_profile !== s.flags_profile) && (s.state === 'running' || s.state === 'starting'));

  async function save(e: Event) {
    e.preventDefault();
    saving = true; errs = {};
    try {
      await api.patchServer(s.id, { name: f.name.trim(), memory_mb: f.memory_mb, port: f.port, flags_profile: f.flags_profile, autostart: f.autostart, restart_policy: { enabled: f.rp_enabled, max_per_10m: f.rp_max }, tags: f.tags.split(',').map((t) => t.trim()).filter(Boolean), notes: f.notes });
      toast('success', 'Settings saved', { body: needsRestart ? 'Restart the server to apply memory/port changes.' : undefined });
    } catch (er) {
      if (er instanceof ApiError) errs = Object.fromEntries(er.fields.map((x) => [x.path, x.message]));
      toastError(er, 'Could not save');
    }
    saving = false;
  }

  let bpName = $state('');
  let bpBusy = $state(false);
  async function saveBlueprint() {
    bpBusy = true;
    try { const b = await api.saveBlueprint(s.id, bpName.trim() || `${s.name} blueprint`); toast('success', `Saved blueprint “${b.name}”`, { body: 'Find it under Settings, or start a new server from it.' }); bpName = ''; } catch (er) { toastError(er, 'Could not save the blueprint'); }
    bpBusy = false;
  }

  async function del() {
    const r = await confirmDialog({
      title: `Delete ${s.name}?`, body: 'The server is removed from Moss. This cannot be undone.', typed: s.name, checkbox: 'Also delete world data and all files on disk', confirmLabel: 'Delete server', danger: true,
      consequence: s.state === 'running' ? 'The server is running and will be stopped first.' : undefined,
    });
    if (!r.ok) return;
    try { await api.deleteServer(s.id, r.checked); toast('info', `Deleted ${s.name}`, { body: r.checked ? 'World data removed.' : 'Files were kept on disk.' }); navigate('/servers'); } catch (e) { toastError(e, 'Delete failed'); }
  }
</script>

<form class="grid cols-2" style="align-items:start" onsubmit={save}>
  <section class="card col" style="gap:16px">
    <div class="card-title">General</div>
    <div class="field"><label for="n">Name</label><input id="n" class="input" bind:value={f.name} maxlength="48" required readonly={!canEdit} />{#if errs.name}<span class="err">{errs.name}</span>{/if}</div>
    {#if s.capabilities.includes('java')}<div class="field">
      <div class="row between"><label for="m">Memory</label><b class="tnum sm">{f.memory_mb} MB</b></div>
      <input id="m" type="range" min="512" max="32768" step="256" bind:value={f.memory_mb} disabled={!canEdit} />{#if errs.memory_mb}<span class="err">{errs.memory_mb}</span>{/if}
    </div>{/if}
    <div class="field"><label for="p">Port</label><input id="p" class="input" type="number" min="1024" max="65535" bind:value={f.port} readonly={!canEdit} />{#if errs.port}<span class="err">{errs.port}</span>{/if}</div>
    {#if s.capabilities.includes('java')}<div class="field"><label for="fl">JVM flags profile</label>
      <select id="fl" class="select" bind:value={f.flags_profile} disabled={!canEdit}><option value="balanced">Balanced (Aikar-style tuned flags)</option><option value="minimal">Minimal (just -Xms/-Xmx)</option></select></div>{/if}
    {#if needsRestart}<div class="banner warn"><Icon name="refresh" size={16} />Memory, port and flags apply on the next restart.</div>{/if}
  </section>

  <section class="card col" style="gap:16px">
    <div class="card-title">Behaviour</div>
    <label class="row between" style="cursor:pointer"><span>Start automatically with Moss<span class="hint" style="display:block">Brings this server up after the host reboots.</span></span><input type="checkbox" class="switch" bind:checked={f.autostart} disabled={!canEdit} /></label>
    <label class="row between" style="cursor:pointer"><span>Restart after a crash<span class="hint" style="display:block">Stops retrying after too many crashes in 10 minutes.</span></span><input type="checkbox" class="switch" bind:checked={f.rp_enabled} disabled={!canEdit} /></label>
    {#if f.rp_enabled}<div class="field"><label for="rp">Max restarts per 10 minutes</label><input id="rp" class="input" type="number" min="1" max="20" bind:value={f.rp_max} readonly={!canEdit} /></div>{/if}
    <div class="field"><label for="t">Tags</label><input id="t" class="input" bind:value={f.tags} placeholder="friends, main" readonly={!canEdit} /><span class="hint">Comma separated.</span></div>
    <div class="field"><label for="nt">Notes</label><textarea id="nt" class="textarea" rows="3" bind:value={f.notes} readonly={!canEdit}></textarea></div>
  </section>

  <section class="card" style="grid-column:1/-1">
    <div class="card-title">Runtime</div>
    <dl class="rt">
      <div><dt>Runtime</dt><dd>{runtimeLabel(s)}{s.loader_version ? ` (loader ${s.loader_version})` : ''}</dd></div>
      {#if s.capabilities.includes('java')}<div><dt>Java</dt><dd>Java {s.java_major}</dd></div>{/if}
      <div><dt>Process</dt><dd class="tnum">{s.pid ? `PID ${s.pid}` : 'not running'}</dd></div>
      {#if s.capabilities.includes('eula')}<div><dt>EULA</dt><dd>{s.eula_accepted ? 'Accepted' : 'Not accepted'}</dd></div>{/if}
      <div><dt>Server ID</dt><dd><code class="xs">{s.id}</code></dd></div>
    </dl>
  </section>

  {#if showUpdates}
    <section class="card" style="grid-column:1/-1" aria-label="Updates">
      <div class="card-title"><Icon name="activity" size={14} />Updates</div>
      {#if updErr}<p class="xs muted">{updErr}</p>
      {:else if !upd}<p class="xs muted">Checking for updates…</p>
      {:else}
        <p>{#if upd.available}<b>Update available:</b> {upd.installed} → {upd.latest}{:else}Up to date{#if upd.installed} ({upd.installed}){/if}.{/if}</p>
        <p class="xs muted">Checked {upd.checked_at} via {upd.source}.</p>
      {/if}
      <div class="row" style="margin-top:8px">
        <button class="btn" type="button" disabled={checking} onclick={() => checkUpdate(true)}>{checking ? 'Checking…' : 'Check now'}</button>
        {#if canEdit && upd?.available}<button class="btn primary" type="button" onclick={runUpdate}>Update now</button>{/if}
      </div>
    </section>
  {/if}

  {#if canEdit}
    <div class="row end" style="grid-column:1/-1"><button class="btn" type="button" disabled={!dirty} onclick={() => (f = fromServer())}>Reset</button><button class="btn primary" disabled={!dirty || saving}>{saving ? 'Saving…' : 'Save changes'}</button></div>
  {/if}
</form>

<div style="margin-top:var(--gap)"><LimitsForm server={s} /></div>

<div style="margin-top:var(--gap)"><IsolationForm server={s} /></div>
{#if isAdmin()}
  <section class="card" style="margin-top:24px">
    <div class="card-title"><Icon name="package" size={14} />Save as blueprint</div>
    <p class="sm muted" style="margin:0 0 10px">Keep this server's runtime, version, memory, settings, content and schedules as a recipe for new servers. The world, seed, port and anything secret are left out.</p>
    <div class="row g6 wrap"><label class="sr" for="bpname">Blueprint name</label><input id="bpname" class="input" style="max-width:280px" maxlength="60" placeholder="{s.name} blueprint" bind:value={bpName} /><button class="btn" disabled={bpBusy} onclick={saveBlueprint}>{bpBusy ? 'Saving…' : 'Save blueprint'}</button></div>
  </section>
{/if}

{#if canDelete || showReinstall}
  <section class="card danger-zone" style="margin-top:24px">
    <div class="card-title" style="color:var(--danger)"><Icon name="alert" size={14} />Danger zone</div>
    {#if showReinstall}
      <div class="row between wrap dz">
        <div><b>Repair server jar</b><div class="sm muted">Downloads the server jar and loader files again. World, config and add-ons are kept.</div></div>
        <button class="btn" onclick={() => reinstall(s, true)}><Icon name="refresh" size={15} />Repair jar…</button>
      </div>
      <div class="row between wrap dz">
        <div><b>Reinstall server</b><div class="sm muted">Sets the server up again from scratch. Files in the server folder, including worlds, can be replaced.</div></div>
        <button class="btn danger" onclick={() => reinstall(s, false)}><Icon name="refresh" size={15} />Reinstall…</button>
      </div>
    {/if}
    {#if canDelete}<div class="row between wrap dz">
      <div><b>Delete this server</b><div class="sm muted">Removes it from Moss. You choose whether the world data is deleted too.</div></div>
      <button class="btn danger" onclick={del}><Icon name="trash" size={15} />Delete server…</button>
    </div>{/if}
  </section>
{/if}

<style>
  .dz + .dz { margin-top: 14px; padding-top: 14px; border-top: 1px solid var(--line); }
  .danger-zone { border-color: color-mix(in srgb, var(--danger), transparent 60%); }
  .rt { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 14px; margin: 0; } .rt dt { font-size: var(--fs-xs); color: var(--muted); } .rt dd { margin: 2px 0 0; font-weight: 550; }
  @media (max-width: 860px) { form { grid-template-columns: minmax(0, 1fr) !important; } section[style*='grid-column'] { grid-column: auto !important; } }
</style>
