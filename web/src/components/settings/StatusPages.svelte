<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import Modal from '../Modal.svelte';
  import { apiOps, ApiError, errMsg, type StatusPage } from '../../lib/api';
  import { confirmDialog, servers, toast, toastError } from '../../lib/stores.svelte';
  import { copyWithToast } from '../../lib/clipboard';

  let list = $state<StatusPage[] | null>(null);
  let err = $state('');
  let form = $state<{ page: StatusPage | null } | null>(null);

  // form fields
  let f = $state({ title: '', slug: '', description: '', show_address: '', show_players: false, enabled: true, server_ids: [] as string[] });
  let busy = $state(false);
  let fieldErr = $state<Record<string, string>>({});
  let top = $state('');

  async function load() { err = ''; try { list = (await apiOps.statusPages()).items; } catch (e) { err = errMsg(e); } }
  void load();

  const url = (p: StatusPage) => `${location.origin}${p.path}`;
  const names = (ids: string[]) => ids.map((id) => servers.list.find((s) => s.id === id)?.name ?? 'a removed server');
  function open(page: StatusPage | null) {
    f = page
      ? { title: page.title, slug: page.slug, description: page.description, show_address: page.show_address ?? '', show_players: page.show_players, enabled: page.enabled, server_ids: [...page.server_ids] }
      : { title: '', slug: '', description: '', show_address: '', show_players: false, enabled: true, server_ids: [] };
    fieldErr = {}; top = ''; form = { page };
  }
  function move(i: number, d: number) {
    const j = i + d;
    if (j < 0 || j >= f.server_ids.length) return;
    const l = [...f.server_ids];
    [l[i], l[j]] = [l[j], l[i]];
    f.server_ids = l;
  }
  const pick = (id: string, on: boolean) => (f.server_ids = on ? [...f.server_ids.filter((x) => x !== id), id] : f.server_ids.filter((x) => x !== id));

  async function save(e: Event) {
    e.preventDefault(); fieldErr = {}; top = '';
    if (!f.title.trim()) { fieldErr.title = 'Give the page a title.'; return; }
    busy = true;
    const body = { title: f.title.trim(), description: f.description.trim(), show_address: f.show_address.trim() || null, show_players: f.show_players, enabled: f.enabled, server_ids: f.server_ids };
    try {
      if (form?.page) { await apiOps.patchStatusPage(form.page.id, { ...body, slug: f.slug.trim() || form.page.slug }); toast('success', 'Status page updated'); }
      else { await apiOps.addStatusPage({ ...body, slug: f.slug.trim() || undefined }); toast('success', 'Status page created'); }
      form = null; void load();
    } catch (er) {
      if (er instanceof ApiError && er.fields.length) { for (const x of er.fields) fieldErr[x.path] = x.message; } else top = errMsg(er);
    }
    busy = false;
  }
  async function toggle(p: StatusPage) {
    try { await apiOps.patchStatusPage(p.id, { enabled: !p.enabled }); void load(); } catch (e) { toastError(e, 'Could not change the status page'); }
  }
  async function remove(p: StatusPage) {
    const r = await confirmDialog({ title: `Delete ${p.title}?`, body: 'The public link stops working at once. The servers themselves are not touched.', confirmLabel: 'Delete page', danger: true });
    if (!r.ok) return;
    try { await apiOps.removeStatusPage(p.id); toast('info', 'Status page deleted'); void load(); } catch (e) { toastError(e, 'Could not delete'); }
  }
</script>

<section class="card" aria-labelledby="sp">
  <div class="card-title" id="sp"><Icon name="link" size={14} />Status pages</div>
  <p class="muted sm">A public page anyone can open without signing in, showing whether the servers you choose are online. Nothing is public until you make a page and turn it on. Visitors see only the title, the servers' names, game, version and state, uptime, the address you type, and player counts if you allow them.</p>

  {#if err}<States kind="error" text={err} retry={load} />
  {:else if !list}<States kind="loading" rows={2} />
  {:else}
    {#if !list.length}<p class="muted sm" style="margin-top:12px">No status pages yet.</p>{/if}
    <ul class="sl">
      {#each list as p (p.id)}
        <li>
          <div class="row g8 wrap">
            <b>{p.title}</b><span class="badge {p.enabled ? 'success' : ''}">{p.enabled ? 'Public' : 'Off'}</span>
            <span class="grow"></span>
            <button class="btn sm" onclick={() => copyWithToast(url(p), 'Link')}><Icon name="copy" size={13} />Copy link</button>
            <a class="btn sm" href={p.path} target="_blank" rel="noopener noreferrer"><Icon name="external" size={13} />Open</a>
            <button class="btn sm" onclick={() => toggle(p)}>{p.enabled ? 'Turn off' : 'Turn on'}</button>
            <button class="btn sm" onclick={() => open(p)}><Icon name="edit" size={13} />Edit</button>
            <button class="btn ghost icon sm" aria-label={`Delete ${p.title}`} onclick={() => remove(p)}><Icon name="trash" size={14} /></button>
          </div>
          <div class="xs muted mono ellipsis" title={url(p)}>{url(p)}</div>
          <div class="xs muted">{p.server_ids.length ? names(p.server_ids).join(', ') : 'No servers chosen'}{p.show_players ? ' · shows player counts' : ''}</div>
        </li>
      {/each}
    </ul>
    <div class="row" style="margin-top:12px"><button class="btn primary" onclick={() => open(null)}><Icon name="plus" size={15} />New status page</button></div>
  {/if}
</section>

{#if form}
  <Modal title={form.page ? 'Edit status page' : 'New status page'} onclose={() => (form = null)} width={620}>
    <form id="spf" class="col" onsubmit={save} novalidate>
      <div class="field"><label for="spt">Title</label><input id="spt" class="input" bind:value={f.title} maxlength="80" placeholder="Our servers" aria-invalid={!!fieldErr.title} data-autofocus />{#if fieldErr.title}<span class="err" role="alert">{fieldErr.title}</span>{/if}</div>
      <div class="field"><label for="spd">Description <span class="muted">(optional)</span></label><textarea id="spd" class="input" rows="2" bind:value={f.description} maxlength="500"></textarea>{#if fieldErr.description}<span class="err" role="alert">{fieldErr.description}</span>{/if}</div>
      <div class="field"><label for="spa">Public address <span class="muted">(optional)</span></label><input id="spa" class="input mono" bind:value={f.show_address} maxlength="100" placeholder="play.example.com" autocomplete="off" spellcheck="false" aria-describedby="spa-h" /><span class="hint" id="spa-h">Shown to visitors exactly as typed. Moss never shows the address it listens on.</span>{#if fieldErr.show_address}<span class="err" role="alert">{fieldErr.show_address}</span>{/if}</div>
      <div class="field"><label for="sps">Link name <span class="muted">(optional)</span></label><input id="sps" class="input mono" bind:value={f.slug} maxlength="40" placeholder={form.page ? '' : 'a random one is made for you'} autocomplete="off" spellcheck="false" aria-describedby="sps-h" aria-invalid={!!fieldErr.slug} /><span class="hint" id="sps-h">3 to 40 lowercase letters, digits or dashes. A short, guessable name lets anyone find the page.</span>{#if fieldErr.slug}<span class="err" role="alert">{fieldErr.slug}</span>{/if}</div>
      <label class="row g8"><input type="checkbox" class="check" bind:checked={f.show_players} /><span>Show how many players are online</span></label>
      <label class="row g8"><input type="checkbox" class="check" bind:checked={f.enabled} /><span>Public (anyone with the link can see it)</span></label>
      <fieldset class="evs">
        <legend class="label">Servers shown, in order</legend>
        {#if f.server_ids.length}
          <ol class="ord">
            {#each f.server_ids as id, i (id)}
              <li class="row g6"><span class="grow">{names([id])[0]}</span>
                <button type="button" class="btn ghost icon sm" aria-label="Move up" disabled={i === 0} onclick={() => move(i, -1)}>↑</button>
                <button type="button" class="btn ghost icon sm" aria-label="Move down" disabled={i === f.server_ids.length - 1} onclick={() => move(i, 1)}>↓</button>
              </li>
            {/each}
          </ol>
        {/if}
        <div class="col" style="margin-top:8px">{#each servers.list as s (s.id)}<label class="row g8"><input type="checkbox" class="check" checked={f.server_ids.includes(s.id)} onchange={(e) => pick(s.id, e.currentTarget.checked)} /><span>{s.name}</span></label>{/each}</div>
        {#if fieldErr.server_ids}<span class="err" role="alert">{fieldErr.server_ids}</span>{/if}
      </fieldset>
      {#if top}<div class="banner danger" role="alert"><Icon name="alert" size={16} />{top}</div>{/if}
    </form>
    {#snippet footer()}
      <button class="btn" type="button" onclick={() => (form = null)}>Cancel</button>
      <button class="btn primary" type="submit" form="spf" disabled={busy}>{busy ? 'Saving…' : form?.page ? 'Save' : 'Create page'}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .sl { list-style: none; margin: 12px 0 0; padding: 0; } .sl li { padding: 12px 0; border-bottom: 1px solid var(--line); } .sl li:last-child { border: 0; }
  .evs { border: 0; padding: 0; margin: 0; min-width: 0; }
  .ord { margin: 0 0 4px; padding: 0 0 0 20px; }
</style>
