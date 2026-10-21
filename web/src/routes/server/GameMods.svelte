<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import States from '../../components/States.svelte';
  import OpCard from '../../components/OpCard.svelte';
  import { errMsg, gameMods, type GameModHit, type GameModsInfo, type Operation, type Server } from '../../lib/api';
  import { compact } from '../../lib/format';
  import { onOp, ops, toast, toastError, trackOp } from '../../lib/stores.svelte';
  import { can } from '../../lib/auth.svelte';

  let { server: s }: { server: Server } = $props();
  const canManage = $derived(can('content.manage'));
  const canConfig = $derived(can('config.write'));

  let info = $state<GameModsInfo | null>(null);
  let err = $state('');
  let tracked = $state<string[]>([]);
  let updates = $state<Record<string, string>>({});
  let checking = $state(false);
  let q = $state('');
  let hits = $state<GameModHit[] | null>(null);
  let sErr = $state('');
  let searching = $state(false);
  let wid = $state('');
  let user = $state('');
  let token = $state('');
  let busy = $state('');

  async function load() {
    err = '';
    try {
      info = await gameMods.info(s.id);
      wid = info.workshop?.id ?? '';
      user = info.credentials?.username ?? '';
    } catch (e) { err = errMsg(e); }
  }
  $effect(() => { s.id; info = null; tracked = []; updates = {}; hits = null; load(); });

  $effect(() => onOp((o) => { if (o.server_id === s.id && o.kind === 'gamemods.install' && (o.state === 'completed' || o.state === 'failed')) load(); }));

  let timer: ReturnType<typeof setTimeout>;
  let seq = 0;
  $effect(() => {
    q; const i = info;
    if (!i || i.kind === 'workshop') return;
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const my = ++seq;
      searching = true; sErr = '';
      try { const r = await gameMods.search(s.id, q.trim()); if (my === seq) hits = r.results; } catch (e) { if (my === seq) sErr = errMsg(e); }
      if (my === seq) searching = false;
    }, q ? 300 : 0);
    return () => clearTimeout(timer);
  });

  const locked = $derived(!!info?.running);
  const installedIds = $derived(new Set((info?.installed ?? []).map((m) => m.id)));
  const needsCreds = $derived(info?.kind === 'factorio' && !info.credentials?.has_token);

  function track(o: Operation) { trackOp(o); tracked = [o.id, ...tracked].slice(0, 3); }

  async function run(key: string, f: () => Promise<unknown>, ok?: string) {
    busy = key;
    try { await f(); if (ok) toast('success', ok); await load(); } catch (e) { toastError(e, 'That did not work'); }
    busy = '';
  }
  const install = (id: string) => run('i:' + id, async () => track((await gameMods.install(s.id, id)).operation));
  const saveWorkshop = () => run('w', () => gameMods.setWorkshop(s.id, info!.workshop!.field === 'collection' ? { collection: wid.trim() } : { map: wid.trim() }), 'Saved. It takes effect the next time the server starts.');
  const saveCreds = () => run('c', async () => { await gameMods.setCredentials(s.id, user.trim(), token.trim() || undefined); token = ''; }, 'Credentials saved');
  async function check() {
    checking = true;
    try { updates = Object.fromEntries((await gameMods.updates(s.id)).updates.map((u) => [u.id, u.latest])); toast('info', Object.keys(updates).length ? 'Updates available' : 'Everything is up to date'); } catch (e) { toastError(e, 'Could not check for updates'); }
    checking = false;
  }
  const appId = $derived(s.runtime === 'gmod' ? 4020 : 730);
</script>

{#if err}<States kind="error" text={err} retry={load} />
{:else if !info}<States kind="loading" rows={4} />
{:else if info.kind === 'workshop'}
  {@const w = info.workshop!}
  <section class="card col" style="gap:14px">
    <div class="card-title">Steam Workshop {w.field === 'collection' ? 'collection' : 'map'}</div>
    <p class="hint">
      {#if w.field === 'collection'}The server mounts this Workshop collection when it starts (<code>+host_workshop_collection</code>).{:else}The server starts on this Workshop map instead of de_dust2 (<code>+host_workshop_map</code>).{/if}
      Steam downloads the content itself, so the first start can take a while. A Steam Web API key (<code>steam_api_key</code> in moss-game.cfg) is optional.
    </p>
    {#if locked}<p class="hint"><Icon name="alert" size={14} /> Stop the server to change this.</p>{/if}
    <div class="field">
      <label for="wid">{w.field === 'collection' ? 'Collection' : 'Map'} ID</label>
      <div class="row" style="gap:8px">
        <input id="wid" class="input" inputmode="numeric" pattern="[0-9]*" placeholder="e.g. 3070284539" bind:value={wid} readonly={!canManage || locked} />
        {#if canManage}<button class="btn primary" disabled={locked || busy === 'w' || wid.trim() === (w.id ?? '')} onclick={saveWorkshop}>Save</button>{/if}
      </div>
      <span class="hint">The number at the end of the item's Steam page address. Clear it to go back to the default.</span>
    </div>
    <div class="row" style="gap:12px">
      {#if w.url}<a class="btn" href={w.url} target="_blank" rel="noopener noreferrer"><Icon name="external" size={15} />Open on Steam</a>{/if}
      <a class="btn ghost" href="https://steamcommunity.com/app/{appId}/workshop/" target="_blank" rel="noopener noreferrer">Browse the Workshop</a>
    </div>
  </section>
{:else}
  {#if locked}<div class="card" role="status"><Icon name="alert" size={14} /> The server is running. Stop it to install, remove or toggle mods; changes load on the next start.</div>{/if}
  {#if info.kind === 'thunderstore' && !info.bepinex}<p class="hint">Installing a mod also installs BepInEx (the mod loader), and Moss then starts Valheim through it. Mods come from Thunderstore and are not checksum-verified.</p>{/if}

  {#if needsCreds}
    <section class="card col" style="gap:12px">
      <div class="card-title">Factorio mod portal login</div>
      <p class="hint">The portal only serves downloads to logged-in players. Enter your factorio.com username and your token (from <a href="https://factorio.com/profile" target="_blank" rel="noopener noreferrer">your profile</a>). They are stored in moss-game.cfg on this host and never shown again.</p>
      <div class="row" style="gap:8px;flex-wrap:wrap">
        <input class="input" aria-label="Username" placeholder="Username" bind:value={user} autocomplete="off" />
        <input class="input" aria-label="Token" type="password" placeholder="Token" bind:value={token} autocomplete="off" />
        <button class="btn primary" disabled={!canConfig || busy === 'c' || !user.trim() || !token.trim()} onclick={saveCreds}>Save</button>
      </div>
      {#if !canConfig}<span class="hint">You need permission to edit configuration to save these.</span>{/if}
    </section>
  {/if}

  {#each tracked as id (id)}
    {@const o = ops.list.find((x) => x.id === id)}
    {#if o}<OpCard op={o} open />{/if}
  {/each}

  <section class="col" style="gap:10px">
    <div class="row" style="justify-content:space-between">
      <div class="card-title">Installed ({info.installed.length})</div>
      <button class="btn" class:spin={checking} disabled={checking || !info.installed.length} onclick={check}><Icon name="refresh" size={15} />{checking ? 'Checking…' : 'Check for updates'}</button>
    </div>
    {#if !info.installed.length}
      <States kind="empty" icon="puzzle" title="No mods installed" text="Search below to add some." />
    {:else}
      <ul class="list">
        {#each info.installed as m (m.id)}
          <li class="row" style="gap:10px;justify-content:space-between;padding:8px 0">
            <div class="col" style="gap:2px;min-width:0">
              <a href={m.url} target="_blank" rel="noopener noreferrer"><strong>{m.title}</strong></a>
              <span class="hint">{m.id} · {m.version}{m.auto ? ' · dependency' : ''}{m.enabled ? '' : ' · disabled'}</span>
            </div>
            <div class="row" style="gap:6px">
              {#if updates[m.id]}<button class="btn sm primary" disabled={!canManage || locked || !!busy} onclick={() => install(m.id)}>Update to {updates[m.id]}</button>{/if}
              <button class="btn sm" disabled={!canManage || locked || !!busy || m.id === 'denikson-BepInExPack_Valheim'} onclick={() => run('t:' + m.id, () => gameMods.setEnabled(s.id, m.id, !m.enabled))}>{m.enabled ? 'Disable' : 'Enable'}</button>
              <button class="btn sm ghost" aria-label="Remove {m.title}" disabled={!canManage || locked || !!busy} onclick={() => run('r:' + m.id, () => gameMods.remove(s.id, m.id), 'Removed')}><Icon name="trash" size={14} /></button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="col" style="gap:10px">
    <div class="card-title">Browse {info.kind === 'thunderstore' ? 'Thunderstore' : 'the Factorio mod portal'}</div>
    <input class="input" type="search" aria-label="Search mods" placeholder="Search mods" bind:value={q} />
    {#if sErr}<States kind="error" text={sErr} />
    {:else if searching && !hits}<States kind="loading" rows={3} />
    {:else if hits && !hits.length}<States kind="empty" icon="search" title="No matches" text="Try a different search." />
    {:else if hits}
      <ul class="list">
        {#each hits as h (h.id)}
          <li class="row" style="gap:10px;justify-content:space-between;padding:8px 0">
            <div class="col" style="gap:2px;min-width:0">
              <a href={h.url} target="_blank" rel="noopener noreferrer"><strong>{h.title}</strong></a>
              <span class="hint">{h.author ?? ''}{h.downloads != null ? ` · ${compact(h.downloads)} downloads` : ''}{h.version ? ` · ${h.version}` : ''}</span>
              {#if h.summary}<span>{h.summary}</span>{/if}
            </div>
            <button class="btn sm primary" disabled={!canManage || locked || !!busy || needsCreds} title={needsCreds ? 'Add your mod portal login first' : ''} onclick={() => install(h.id)}>{installedIds.has(h.id) ? 'Reinstall' : 'Install'}</button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}
