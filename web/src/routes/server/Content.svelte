<script module lang="ts">
  let remembered: import('../../lib/api').ProviderId = 'modrinth';
</script>

<script lang="ts">
  import SavedViews from '../../components/SavedViews.svelte';
  import Icon from '../../components/Icon.svelte';
  import Modal from '../../components/Modal.svelte';
  import States from '../../components/States.svelte';
  import PlanModal from '../../components/PlanModal.svelte';
  import OpCard from '../../components/OpCard.svelte';
  import InstalledList from '../../components/content/InstalledList.svelte';
  import CompatPanel from '../../components/content/CompatPanel.svelte';
  import ItemDrawer from '../../components/content/ItemDrawer.svelte';
  import { api, download, errMsg, modpackExportPath, type ChangePlan, type ContentFinding, type ContentItem, type Hit, type Operation, type Project, type ProviderId, type ProviderInfo, type SearchResult, type Server } from '../../lib/api';
  import { normalizeItem, removalConflicts } from '../../lib/content';
  import { compact, bytes, relTime, num, initials } from '../../lib/format';
  import { onOp, ops, toast, toastError, trackOp, undoToast } from '../../lib/stores.svelte';
  import { can } from '../../lib/auth.svelte';

  let { server: s }: { server: Server } = $props();
  const kindName: Record<string, string> = { plugins: 'Plugins', mods: 'Mods', proxy_plugins: 'Proxy plugins', datapacks: 'Datapacks' };
  const label = $derived(kindName[s.content_kind] ?? 'Content');
  const canManage = $derived(can('content.manage'));

  let view = $state<'installed' | 'browse'>('installed');
  // installed
  let items = $state<ContentItem[] | null>(null);
  let iErr = $state('');
  let updates = $state<Record<string, { latest: string; version_id: string }>>({});
  let checking = $state(false);
  let toggling = $state<Record<string, boolean>>({});
  let drawerId = $state<string | null>(null);
  let reloadKey = $state(0);
  let busyFinding = $state<string | null>(null);
  let tracked = $state<string[]>([]);
  let importer = $state<import('svelte').Component<any> | null>(null);
  let exporting = $state(false);
  const drawer = $derived(items?.find((i) => i.id === drawerId) ?? null);
  const updateCount = $derived((items ?? []).filter((i) => updates[i.id] && !i.pinned).length);
  const identifying = $derived(tracked.some((id) => ops.list.find((o) => o.id === id && o.kind.includes('identify'))?.state === 'running' || ops.list.find((o) => o.id === id && o.kind.includes('identify'))?.state === 'queued'));
  // browse
  let providers = $state<ProviderInfo[]>([]);
  let chosen = $state<ProviderId>(remembered);
  const usable = (p: ProviderInfo) => p.available && p.supported;
  const avail = $derived(providers.filter((p) => p.available));
  const multi = $derived(avail.length > 1);
  const provider = $derived<ProviderId>(providers.find((p) => p.id === chosen && usable(p))?.id ?? providers.find(usable)?.id ?? 'modrinth');
  const pname = $derived(providers.find((p) => p.id === provider)?.name ?? 'Modrinth');
  $effect(() => { s.id; api.providers(s.id).then((r) => (providers = r.providers), () => (providers = [])); });
  function pick(id: ProviderId) {
    if (id === provider) return;
    chosen = remembered = id; q = ''; hits = []; res = null;
  }
  function segKey(e: KeyboardEvent) {
    const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const ok = providers.filter(usable), n = ok[(ok.findIndex((p) => p.id === provider) + d + ok.length) % ok.length];
    if (n) { pick(n.id); (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-p="${n.id}"]`)?.focus(); }
  }
  let q = $state('');
  let sort = $state('relevance');
  let res = $state<SearchResult | null>(null);
  let hits = $state<Hit[]>([]);
  let sErr = $state('');
  let searching = $state(false);
  let detail = $state<{ hit: Hit; project: Project | null; err: string } | null>(null);
  let plan = $state<null | { plan: ChangePlan; title: string; label: string; run: () => Promise<Operation | Operation[]> }>(null);
  let applying = $state(false);

  async function loadItems() { iErr = ''; try { items = (await api.content(s.id)).items.map(normalizeItem); reloadKey++; } catch (e) { iErr = errMsg(e); } }
  $effect(() => { s.id; items = null; updates = {}; tracked = []; drawerId = null; loadItems(); });

  // debounced search
  let timer: ReturnType<typeof setTimeout>;
  let seq = 0;
  async function search(offset = 0) {
    const my = ++seq;
    searching = true; sErr = '';
    try {
      const r = await api.search({ server_id: s.id, q: q.trim(), sort, offset, limit: 20, provider });
      if (my !== seq) return;
      res = r;
      hits = offset ? [...hits, ...r.hits] : r.hits;
    } catch (e) { if (my === seq) sErr = errMsg(e); }
    if (my === seq) searching = false;
  }
  $effect(() => { q; sort; provider; if (view !== 'browse') return; clearTimeout(timer); timer = setTimeout(() => search(0), q ? 300 : 0); return () => clearTimeout(timer); });

  // react to finished operations for this server
  $effect(() => onOp(async (o) => {
    if (o.server_id !== s.id || !/^(content|modpack|upgrade)/.test(o.kind) || o.state !== 'completed') return;
    await loadItems();
    try {
      const u = (await api.undoList(s.id))[0];
      if (u && Date.parse(u.created_at) >= Date.parse(o.created_at) - 2000) undoToast(u.title, u.id, loadItems);
    } catch { /* undo is best effort */ }
  }));

  const installedIds = $derived(new Set((items ?? []).filter((i) => i.provider === provider).map((i) => i.project_id).filter(Boolean)));

  function track(o: Operation | Operation[]) {
    for (const x of [o].flat()) { trackOp(x); tracked = [x.id, ...tracked].slice(0, 4); }
  }
  async function review(hit: { project_id: string; title: string }, version_id?: string) {
    try {
      const p = await api.plan(s.id, hit.project_id, version_id, provider);
      plan = { plan: p, title: `Install ${hit.title}`, label: 'Install', run: async () => (await api.install(s.id, hit.project_id, version_id, provider)).operation };
    } catch (e) { toastError(e, 'Could not build a plan'); }
  }
  function reviewRemove(its: ContentItem[]) {
    const what = its.length === 1 ? its[0].title : `${its.length} ${label.toLowerCase()}`;
    plan = {
      plan: {
        items: its.map((it) => ({ action: 'remove' as const, title: it.title, filename: it.filename, size: 0, reason: 'Data and config directories are kept.' })),
        warnings: s.state === 'running' ? ['The server is running; the add-on stays loaded until restart.'] : [], requires_restart: true,
        conflicts: removalConflicts(items ?? [], its),
      },
      title: `Remove ${what}`, label: 'Remove',
      run: async () => { const out: Operation[] = []; for (const it of its) out.push((await api.removeContent(s.id, it.id)).operation); return out; },
    };
  }
  function reviewUpdate(it: ContentItem) {
    const u = updates[it.id];
    plan = {
      plan: { items: [{ action: 'replace', title: it.title, filename: it.filename, size: 0, reason: u ? 'Newer compatible version found.' : 'Update to the latest compatible version.', version: u?.latest, from_version: it.version_number }], warnings: [], requires_restart: true },
      title: `Update ${it.title}`, label: 'Update', run: async () => (await api.updateContent(s.id, it.id)).operation,
    };
  }
  async function reviewUpdateAll() {
    try {
      const p = await api.updateAllPlan(s.id);
      if (!p.items.length) { toast('success', 'Everything is up to date'); updates = {}; return; }
      plan = { plan: p, title: `Update ${p.items.length} ${p.items.length === 1 ? 'item' : 'items'}`, label: 'Update all', run: async () => (await api.updateAll(s.id)).operation };
    } catch (e) { toastError(e, 'Could not build the update plan'); }
  }
  async function apply() {
    if (!plan) return;
    applying = true;
    try { track(await plan.run()); plan = null; detail = null; drawerId = null; view = 'installed'; toast('info', 'Working on it', { body: 'Progress is shown here and in the operations panel.' }); } catch (e) { toastError(e, 'Could not apply'); }
    applying = false;
  }
  async function check() {
    checking = true;
    try {
      const r = await api.checkUpdates(s.id);
      updates = Object.fromEntries(r.updates.map((u) => [u.item_id, { latest: u.latest, version_id: u.version_id }]));
      toast(r.updates.length ? 'info' : 'success', r.updates.length ? `${r.updates.length} update${r.updates.length > 1 ? 's' : ''} available` : 'Everything is up to date');
    } catch (e) { toastError(e, 'Update check failed'); }
    checking = false;
  }
  async function togglePin(it: ContentItem) {
    try { await api.pin(s.id, it.id, !it.pinned); it.pinned = !it.pinned; } catch (e) { toastError(e); }
  }
  // Optimistic enable/disable with rollback; returns whether the server says a restart is needed.
  async function toggleOne(it: ContentItem, enabled: boolean): Promise<{ ok: boolean; restart: boolean }> {
    const prev = it.enabled, id = it.id;
    it.enabled = enabled; toggling[id] = true;
    try {
      const r = await api.toggleContent(s.id, id, enabled);
      const i = items?.findIndex((x) => x.id === id) ?? -1;
      if (items && i >= 0 && r?.item) items[i] = normalizeItem({ ...items[i], ...r.item, enabled });
      return { ok: true, restart: !!r?.requires_restart };
    } catch (e) {
      it.enabled = prev;
      toastError(e, `Could not ${enabled ? 'enable' : 'disable'} ${it.title}`);
      return { ok: false, restart: false };
    } finally { delete toggling[id]; }
  }
  let refreshT: ReturnType<typeof setTimeout>;
  const refreshSoon = () => { clearTimeout(refreshT); refreshT = setTimeout(loadItems, 500); };
  $effect(() => () => clearTimeout(refreshT));
  const restartNote = (n: number) => { if (n && (s.state === 'running' || s.state === 'starting')) toast('info', 'Restart needed', { body: 'The change takes effect the next time the server starts.' }); };
  async function toggle(it: ContentItem, enabled: boolean) {
    const r = await toggleOne(it, enabled);
    if (r.ok) { restartNote(+r.restart); refreshSoon(); }
  }
  async function bulkToggle(its: ContentItem[], enabled: boolean) {
    const rs = await Promise.all(its.map((it) => toggleOne(it, enabled)));
    const ok = rs.filter((r) => r.ok).length;
    if (ok) { toast('success', `${enabled ? 'Enabled' : 'Disabled'} ${ok} ${ok === 1 ? 'item' : 'items'}`); restartNote(rs.filter((r) => r.restart).length); refreshSoon(); }
  }
  async function identify() {
    try { track((await api.identify(s.id)).operation); } catch (e) { toastError(e, 'Could not start identification'); }
  }
  async function openImport() {
    try { importer = (await import('../../components/content/ModpackImport.svelte')).default; } catch (e) { toastError(e, 'Could not load the importer'); }
  }
  async function exportPack() {
    exporting = true;
    try { await download(modpackExportPath(s.id), `${s.name.replace(/[^\w.-]+/g, '-')}.mrpack`); } catch (e) { toastError(e, 'Export failed'); }
    exporting = false;
  }
  // Compatibility finding actions
  async function runFinding(f: ContentFinding) {
    const a = f.action;
    if (!a) return;
    const it = items?.find((i) => i.id === (a.item ?? f.item));
    busyFinding = f.id;
    try {
      if (a.kind === 'toggle' && it) await toggle(it, !it.enabled);
      else if (a.kind === 'install' && a.project_id) await review({ project_id: a.project_id, title: f.title.replace(/^Missing (required )?(dependency )?/i, '') || a.project_id });
      else if (a.kind === 'remove' && it) reviewRemove([it]);
      else if (a.kind === 'update' && it) reviewUpdate(it);
      else if (a.kind === 'open' && it) drawerId = it.id;
      else if (a.kind === 'open' && a.project_id) window.open(`https://modrinth.com/project/${encodeURIComponent(a.project_id)}`, '_blank', 'noopener,noreferrer');
      else toast('warn', 'That item is no longer installed', { body: 'Re-check compatibility to refresh the list.' });
    } finally { busyFinding = null; }
  }
  async function openDetail(hit: Hit) {
    detail = { hit, project: null, err: '' };
    try { const p = await api.project(hit.project_id, s.id, provider); if (detail?.hit === hit) detail.project = p; } catch (e) { if (detail?.hit === hit) detail.err = errMsg(e); }
  }
</script>

<div class="row between wrap" style="margin-bottom:16px">
  <div class="row g6" role="group" aria-label="View">
    <button class="chip" aria-pressed={view === 'installed'} onclick={() => (view = 'installed')}>Installed{#if items}<span class="tnum faint">{items.length}</span>{/if}{#if updateCount}<span class="badge info" title="{updateCount} update{updateCount === 1 ? '' : 's'} available">{updateCount}<span class="sr"> updates available</span></span>{/if}</button>
    <button class="chip" aria-pressed={view === 'browse'} onclick={() => (view = 'browse')}><Icon name="search" size={14} />Browse{multi ? '' : ' ' + pname}</button>
  </div>
  {#if view === 'browse' && multi}
    <div class="seg" role="radiogroup" aria-label="Content provider" tabindex="-1" onkeydown={segKey}>
      {#each avail as p (p.id)}
        <button class="segb" role="radio" aria-checked={provider === p.id} tabindex={provider === p.id ? 0 : -1} data-p={p.id} disabled={!usable(p)} title={usable(p) ? undefined : (p.reason ?? 'Not available')} onclick={() => pick(p.id)}>{p.name}</button>
      {/each}
    </div>
  {/if}
  {#if view === 'installed' && canManage}
    <div class="row g6 wrap">
      {#if s.content_kind === 'mods'}
        <button class="btn" onclick={openImport}><Icon name="upload" size={15} />Import modpack</button>
        <button class="btn" disabled={exporting} onclick={exportPack}><Icon name="download" size={15} />{exporting ? 'Exporting…' : 'Export .mrpack'}</button>
      {/if}
      <button class="btn" class:spin={checking} disabled={checking} onclick={check}><Icon name="refresh" size={15} />{checking ? 'Checking…' : 'Check for updates'}</button>
      {#if updateCount}<button class="btn primary" onclick={reviewUpdateAll}>Review updates<span class="cnt tnum">{updateCount}</span></button>{/if}
    </div>
  {/if}
</div>

{#if view === 'installed'}
  {#if iErr}<States kind="error" text={iErr} retry={loadItems} />
  {:else if !items}<States kind="loading" rows={4} />
  {:else}
    <CompatPanel serverId={s.id} {items} {reloadKey} busy={busyFinding} onaction={runFinding} />
    {#each tracked as id (id)}
      {@const o = ops.list.find((x) => x.id === id)}
      {#if o}
        <div class="trk"><OpCard op={o} open={o.kind.includes('identify') || o.kind.includes('modpack')} />{#if o.state !== 'running' && o.state !== 'queued'}<button class="btn ghost icon sm x" aria-label="Dismiss" onclick={() => (tracked = tracked.filter((t) => t !== id))}><Icon name="x" size={14} /></button>{/if}</div>
      {/if}
    {/each}
    {#if !items.length}
      <States kind="empty" icon="puzzle" title="No {label.toLowerCase()} installed" text="Browse for {label.toLowerCase()} that match this server's version and loader{s.content_kind === 'mods' && canManage ? ', or import a modpack' : ''}."><button class="btn primary" onclick={() => (view = 'browse')}><Icon name="search" size={15} />Browse</button>{#if s.content_kind === 'mods' && canManage}<button class="btn" onclick={openImport}><Icon name="upload" size={15} />Import modpack</button>{/if}</States>
    {:else}
      <InstalledList {items} {updates} {canManage} {toggling} {identifying} onopen={(it) => (drawerId = it.id)} ontoggle={toggle} onbulktoggle={bulkToggle} onbulkremove={reviewRemove} onupdate={reviewUpdate} onpin={togglePin} onremove={(it) => reviewRemove([it])} onidentify={identify} />
    {/if}
  {/if}
{:else}
  <div class="row wrap" style="margin-bottom:12px">
    <div class="grow" style="max-width:420px;position:relative;min-width:200px">
      <input class="input" style="padding-left:34px" type="search" placeholder="Search {label.toLowerCase()}…" aria-label="Search {pname}" bind:value={q} />
      <span class="ico"><Icon name="search" size={15} /></span>
    </div>
    <select class="select" style="width:auto" aria-label="Sort" bind:value={sort}>
      <option value="relevance">Relevance</option><option value="downloads">Downloads</option><option value="updated">Recently updated</option><option value="follows">Follows</option>
    </select>
    <SavedViews scope="content" filters={{ q }} {sort} onapply={(f, so) => { q = typeof f.q === 'string' ? f.q : ''; sort = ['relevance', 'downloads', 'updated', 'follows'].includes(so) ? so : 'relevance'; }} />
    {#if res}<span class="xs muted">Showing {res.compat.kind.replace('_', ' ')} for <b>{res.compat.loader}</b> · {res.compat.game_version}</span>{/if}
  </div>
  {#if provider === 'curseforge' && s.content_kind !== 'datapacks'}<p class="xs muted" style="margin:-4px 0 12px">Results from CurseForge. CurseForge does not say whether a mod runs on servers, so check the project page.</p>{/if}
  {#if sErr && !hits.length}<States kind="error" text={sErr} retry={() => search(0)} />
  {:else if searching && !hits.length}<div class="grid cols-auto">{#each [1, 2, 3, 4, 5, 6] as _}<div class="skel" style="height:150px"></div>{/each}</div>
  {:else if !hits.length}<States kind="empty" icon="search" title="No results" text="Try a different search. Client-only projects are hidden." />
  {:else}
    <div class="grid cols-auto">
      {#each hits as h (h.project_id)}
        <article class="card hover hit">
          <div class="row" style="align-items:flex-start">
            {#if h.icon_url}<img class="ico2" src={h.icon_url} alt="" loading="lazy" />{:else}<div class="ico2 ph">{initials(h.title)}</div>{/if}
            <div class="grow"><h3 class="ellipsis">{h.title}</h3><div class="xs muted">by {h.author}</div></div>
            {#if multi}<span class="tag" title="Source: {pname}">{pname}</span>{/if}
          </div>
          <p class="muted sm desc">{h.description}</p>
          <div class="row g4 wrap">{#each h.categories.slice(0, 3) as c}<span class="tag">{c}</span>{/each}</div>
          <div class="row between" style="margin-top:auto">
            <span class="xs muted tnum" title="{num(h.downloads)} downloads"><Icon name="download" size={12} /> {compact(h.downloads)}</span>
            <div class="row g4">
              {#if installedIds.has(h.project_id)}<span class="badge ok"><Icon name="check" size={12} />Installed</span>{/if}
              <button class="btn sm" onclick={() => openDetail(h)}>Details</button>
              {#if canManage && !installedIds.has(h.project_id)}<button class="btn sm primary" onclick={() => review(h)}>Install</button>{/if}
            </div>
          </div>
        </article>
      {/each}
    </div>
    {#if res && hits.length < res.total}<div class="row" style="justify-content:center;margin-top:16px"><button class="btn" disabled={searching} onclick={() => search(hits.length)}>{searching ? 'Loading…' : `Load more (${res.total - hits.length})`}</button></div>{/if}
  {/if}
{/if}

{#if detail}
  <Modal title={detail.hit.title} side width={560} onclose={() => (detail = null)}>
    <div class="row" style="align-items:flex-start;margin-bottom:12px">
      {#if detail.hit.icon_url}<img class="ico2" src={detail.hit.icon_url} alt="" />{:else}<div class="ico2 ph">{initials(detail.hit.title)}</div>{/if}
      <div><div class="muted sm">by {detail.hit.author} · {compact(detail.hit.downloads)} downloads · updated {relTime(detail.hit.updated)}</div><p style="margin-top:4px">{detail.hit.description}</p></div>
    </div>
    {#if detail.err}<States kind="error" text={detail.err} />
    {:else if !detail.project}<States kind="loading" rows={4} />
    {:else}
      {@const p = detail.project}
      <div class="row g6 wrap" style="margin-bottom:14px"><span class="badge">{p.license}</span>
        {#each Object.entries(p.links).filter(([, v]) => v) as [k, v]}<a class="badge" href={v} target="_blank" rel="noreferrer noopener">{k}<Icon name="external" size={11} /></a>{/each}</div>
      {#if p.body}<div class="body-txt sm muted">{p.body}</div>{/if}
      <h3 style="margin:18px 0 8px">Compatible versions <span class="muted sm">({p.versions.length})</span></h3>
      {#if !p.versions.length}<p class="muted sm">No versions match this server's loader and game version.</p>{/if}
      <ul class="vers">
        {#each p.versions as v (v.id)}
          <li>
            <div class="grow">
              <div class="row g6"><b class="tnum">{v.version_number}</b><span class="badge {v.channel === 'release' ? 'ok' : v.channel === 'beta' ? 'warn' : 'danger'}">{v.channel}</span></div>
              <div class="xs muted">{relTime(v.date)} · {compact(v.downloads)} downloads · {v.files.find((f) => f.primary)?.filename ?? v.files[0]?.filename} ({bytes(v.files[0]?.size)})</div>
              {#if v.downloadable === false}<div class="xs warn-t">Author disabled automatic downloads — get it from the project page{#if p.links.page}&nbsp;(<a href={p.links.page} target="_blank" rel="noreferrer noopener">open</a>){/if} and upload it in Files</div>{/if}
              {#if v.warning}<div class="xs warn-t">{v.warning}</div>{/if}
              {#if v.dependencies.some((d) => d.kind === 'required')}<div class="xs faint">Requires {v.dependencies.filter((d) => d.kind === 'required').map((d) => d.project_id).join(', ')}</div>{/if}
            </div>
            {#if canManage}<button class="btn sm {v === p.versions[0] ? 'primary' : ''}" disabled={v.downloadable === false} onclick={() => review(detail!.hit, v.id)}>Install</button>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </Modal>
{/if}

{#if drawer}
  <ItemDrawer item={drawer} {canManage} latest={updates[drawer.id]?.latest ?? null} onclose={() => (drawerId = null)} onaction={(f) => { drawerId = null; void runFinding(f); }} ontoggle={(it) => toggle(it, !it.enabled)} onupdate={(it) => { drawerId = null; reviewUpdate(it); }} onremove={(it) => { drawerId = null; reviewRemove([it]); }} />
{/if}

{#if importer}
  {@const Importer = importer}
  <Importer server={s} onclose={() => (importer = null)} onstarted={(o: Operation) => { importer = null; track(o); view = 'installed'; }} />
{/if}

{#if plan}<PlanModal plan={plan.plan} title={plan.title} confirmLabel={plan.label} busy={applying} onconfirm={apply} oncancel={() => (plan = null)} />{/if}

<style>
  .cnt { display: inline-grid; place-items: center; min-width: 18px; height: 18px; padding: 0 5px; border-radius: 99px; background: var(--a-ink); color: var(--a-text); font-size: 11px; margin-left: 2px; }
  .trk { position: relative; margin-bottom: 12px; }
  .trk .x { position: absolute; bottom: 8px; right: 10px; }
  .seg { display: inline-flex; padding: 2px; gap: 2px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); }
  .segb { border: 0; background: none; color: var(--muted); font: inherit; font-size: 13px; padding: 4px 12px; border-radius: 6px; cursor: pointer; }
  .segb[aria-checked='true'] { background: var(--a-soft); color: var(--a-text); font-weight: 600; }
  .segb:disabled { opacity: 0.5; cursor: not-allowed; }
  .segb:focus-visible { outline: 2px solid var(--a-text); outline-offset: 1px; }
  .warn-t { color: var(--warn); }
  .ico { position: absolute; left: 11px; top: 10px; color: var(--faint); pointer-events: none; display: grid; }
  .hit { display: flex; flex-direction: column; gap: 10px; min-height: 170px; }
  .ico2 { width: 44px; height: 44px; border-radius: 10px; object-fit: cover; flex: none; background: var(--s3); }
  .ico2.ph { display: grid; place-items: center; font-weight: 700; color: var(--a-text); background: var(--a-soft); }
  .desc { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .vers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .vers li { display: flex; gap: 12px; align-items: center; padding: 10px 12px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); }
  .body-txt { white-space: pre-wrap; max-height: 200px; overflow: auto; padding: 12px; background: var(--bg); border-radius: var(--r-sm); }
</style>
