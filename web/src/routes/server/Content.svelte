<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import Modal from '../../components/Modal.svelte';
  import States from '../../components/States.svelte';
  import PlanModal from '../../components/PlanModal.svelte';
  import { api, errMsg, type ChangePlan, type ContentItem, type Hit, type Project, type SearchResult, type Server } from '../../lib/api';
  import { compact, bytes, relTime, num, initials } from '../../lib/format';
  import { onOp, toast, toastError, trackOp, undoToast } from '../../lib/stores.svelte';
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
  // browse
  let q = $state('');
  let sort = $state('relevance');
  let res = $state<SearchResult | null>(null);
  let hits = $state<Hit[]>([]);
  let sErr = $state('');
  let searching = $state(false);
  let detail = $state<{ hit: Hit; project: Project | null; err: string } | null>(null);
  let plan = $state<null | { plan: ChangePlan; title: string; label: string; run: () => Promise<{ operation: import('../../lib/api').Operation }> }>(null);
  let applying = $state(false);

  async function loadItems() { iErr = ''; try { items = (await api.content(s.id)).items; } catch (e) { iErr = errMsg(e); } }
  $effect(() => { s.id; items = null; loadItems(); });

  // debounced search
  let timer: ReturnType<typeof setTimeout>;
  let seq = 0;
  async function search(offset = 0) {
    const my = ++seq;
    searching = true; sErr = '';
    try {
      const r = await api.search({ server_id: s.id, q: q.trim(), sort, offset, limit: 20 });
      if (my !== seq) return;
      res = r;
      hits = offset ? [...hits, ...r.hits] : r.hits;
    } catch (e) { if (my === seq) sErr = errMsg(e); }
    if (my === seq) searching = false;
  }
  $effect(() => { q; sort; if (view !== 'browse') return; clearTimeout(timer); timer = setTimeout(() => search(0), q ? 300 : 0); return () => clearTimeout(timer); });

  // react to finished operations for this server
  $effect(() => onOp(async (o) => {
    if (o.server_id !== s.id || !o.kind.startsWith('content') || o.state !== 'completed') return;
    await loadItems();
    try {
      const u = (await api.undoList(s.id))[0];
      if (u && Date.parse(u.created_at) >= Date.parse(o.created_at) - 2000) undoToast(u.title, u.id, loadItems);
    } catch { /* undo is best effort */ }
  }));

  const installedIds = $derived(new Set((items ?? []).map((i) => i.project_id).filter(Boolean)));

  async function review(hit: { project_id: string; title: string }, version_id?: string) {
    try {
      const p = await api.plan(s.id, hit.project_id, version_id);
      plan = { plan: p, title: `Install ${hit.title}`, label: 'Install', run: () => api.install(s.id, hit.project_id, version_id) };
    } catch (e) { toastError(e, 'Could not build a plan'); }
  }
  function reviewRemove(it: ContentItem) {
    plan = {
      plan: { items: [{ action: 'remove', title: it.title, filename: it.filename, size: 0, reason: 'Data and config directories are kept.' }], warnings: s.state === 'running' ? ['The server is running; the add-on stays loaded until restart.'] : [], requires_restart: true },
      title: `Remove ${it.title}`, label: 'Remove', run: () => api.removeContent(s.id, it.id),
    };
  }
  function reviewUpdate(it: ContentItem) {
    const u = updates[it.id];
    plan = {
      plan: { items: [{ action: 'replace', title: it.title, filename: it.filename, size: 0, reason: `${it.version_number} → ${u.latest}` }], warnings: [], requires_restart: true },
      title: `Update ${it.title}`, label: 'Update', run: () => api.updateContent(s.id, it.id),
    };
  }
  async function apply() {
    if (!plan) return;
    applying = true;
    try { const r = await plan.run(); trackOp(r.operation); plan = null; detail = null; view = 'installed'; toast('info', 'Working on it', { body: 'Progress is in the operations panel.' }); } catch (e) { toastError(e, 'Could not apply'); }
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
  async function openDetail(hit: Hit) {
    detail = { hit, project: null, err: '' };
    try { const p = await api.project(hit.project_id, s.id); if (detail?.hit === hit) detail.project = p; } catch (e) { if (detail?.hit === hit) detail.err = errMsg(e); }
  }
</script>

<div class="row between wrap" style="margin-bottom:16px">
  <div class="row g6" role="group" aria-label="View">
    <button class="chip" aria-pressed={view === 'installed'} onclick={() => (view = 'installed')}>Installed{#if items}<span class="tnum faint">{items.length}</span>{/if}</button>
    <button class="chip" aria-pressed={view === 'browse'} onclick={() => (view = 'browse')}><Icon name="search" size={14} />Browse Modrinth</button>
  </div>
  {#if view === 'installed' && canManage}<button class="btn" disabled={checking} onclick={check}><Icon name="refresh" size={15} />{checking ? 'Checking…' : 'Check for updates'}</button>{/if}
</div>

{#if view === 'installed'}
  {#if iErr}<States kind="error" text={iErr} retry={loadItems} />
  {:else if !items}<States kind="loading" rows={4} />
  {:else if !items.length}
    <States kind="empty" icon="puzzle" title="No {label.toLowerCase()} installed" text="Browse Modrinth for {label.toLowerCase()} that match this server's version and loader."><button class="btn primary" onclick={() => (view = 'browse')}><Icon name="search" size={15} />Browse</button></States>
  {:else}
    <div class="table-wrap">
      <table>
        <thead><tr><th>Name</th><th>Source</th><th>Version</th><th>Installed</th><th></th></tr></thead>
        <tbody>
          {#each items as it (it.id)}
            <tr>
              <td><div class="ellipsis" style="max-width:260px"><b>{it.title}</b></div><div class="xs faint mono ellipsis" style="max-width:260px">{it.filename}</div></td>
              <td><span class="badge {it.source === 'moss' ? 'ok' : ''}" title={it.source === 'moss' ? 'Installed by Moss from Modrinth' : 'Dropped in manually; Moss cannot update it'}>{it.provider === 'modrinth' ? 'Modrinth' : 'Manual'}</span></td>
              <td class="tnum">{it.version_number ?? '—'}{#if updates[it.id]}<span class="badge info" style="margin-left:6px">→ {updates[it.id].latest}</span>{/if}</td>
              <td class="muted">{relTime(it.installed_at)}</td>
              <td class="num">
                <span class="row g4 end" style="justify-content:flex-end">
                  {#if canManage && it.provider === 'modrinth'}
                    <button class="btn ghost icon sm" aria-pressed={it.pinned} aria-label="{it.pinned ? 'Unpin' : 'Pin'} {it.title}" title={it.pinned ? 'Pinned: updates skip this' : 'Pin version'} onclick={() => togglePin(it)} style={it.pinned ? 'color:var(--a-text)' : ''}><Icon name="pin" size={14} /></button>
                  {/if}
                  {#if canManage && updates[it.id] && !it.pinned}<button class="btn sm primary" onclick={() => reviewUpdate(it)}>Update</button>{/if}
                  {#if canManage}<button class="btn ghost icon sm" aria-label="Remove {it.title}" onclick={() => reviewRemove(it)}><Icon name="trash" size={14} /></button>{/if}
                </span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{:else}
  <div class="row wrap" style="margin-bottom:12px">
    <div class="grow" style="max-width:420px;position:relative;min-width:200px">
      <input class="input" style="padding-left:34px" type="search" placeholder="Search {label.toLowerCase()}…" aria-label="Search Modrinth" bind:value={q} />
      <span class="ico"><Icon name="search" size={15} /></span>
    </div>
    <select class="select" style="width:auto" aria-label="Sort" bind:value={sort}>
      <option value="relevance">Relevance</option><option value="downloads">Downloads</option><option value="updated">Recently updated</option><option value="follows">Follows</option>
    </select>
    {#if res}<span class="xs muted">Showing {res.compat.kind.replace('_', ' ')} for <b>{res.compat.loader}</b> · {res.compat.game_version}</span>{/if}
  </div>
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
              {#if v.dependencies.some((d) => d.kind === 'required')}<div class="xs faint">Requires {v.dependencies.filter((d) => d.kind === 'required').map((d) => d.project_id).join(', ')}</div>{/if}
            </div>
            {#if canManage}<button class="btn sm {v === p.versions[0] ? 'primary' : ''}" onclick={() => review(detail!.hit, v.id)}>Install</button>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </Modal>
{/if}

{#if plan}<PlanModal plan={plan.plan} title={plan.title} confirmLabel={plan.label} busy={applying} onconfirm={apply} oncancel={() => (plan = null)} />{/if}

<style>
  .ico { position: absolute; left: 11px; top: 10px; color: var(--faint); pointer-events: none; display: grid; }
  .hit { display: flex; flex-direction: column; gap: 10px; min-height: 170px; }
  .ico2 { width: 44px; height: 44px; border-radius: 10px; object-fit: cover; flex: none; background: var(--s3); }
  .ico2.ph { display: grid; place-items: center; font-weight: 700; color: var(--a-text); background: var(--a-soft); }
  .desc { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .vers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .vers li { display: flex; gap: 12px; align-items: center; padding: 10px 12px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--s2); }
  .body-txt { white-space: pre-wrap; max-height: 200px; overflow: auto; padding: 12px; background: var(--bg); border-radius: var(--r-sm); }
</style>
