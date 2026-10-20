<script lang="ts">
  import Icon from '../Icon.svelte';
  import Modal from '../Modal.svelte';
  import { api, errMsg, type Operation, type PackCheck, type PackInfo, type Server } from '../../lib/api';
  import { packChangeText, packDiffCounts } from '../../lib/content';
  import { relTime } from '../../lib/format';
  import { toast, toastError, trackOp } from '../../lib/stores.svelte';

  // Installed modpacks: version, pin, and the "check for a newer version" diff.
  let { server: s, canManage, reloadKey = 0, onoperation, ontest }: {
    server: Server; canManage: boolean; reloadKey?: number; onoperation: (o: Operation) => void; ontest: (install: PackInfo, token: string) => void;
  } = $props();

  let packs = $state<PackInfo[] | null>(null);
  let err = $state('');
  let busy = $state<string | null>(null);
  let checked = $state<{ pack: PackInfo; check: PackCheck } | null>(null);
  let applying = $state(false);

  async function load() { try { packs = (await api.modpacks(s.id)).packs; err = ''; } catch (e) { err = errMsg(e); packs = []; } }
  $effect(() => { s.id; reloadKey; void load(); });

  async function pin(p: PackInfo) {
    busy = p.id;
    try { await api.pinPack(s.id, p.id, !p.pinned); p.pinned = !p.pinned; toast('success', p.pinned ? `${p.name} is pinned` : `${p.name} is no longer pinned`, { body: p.pinned ? 'Update checks and update-all leave this pack and its files alone.' : undefined }); } catch (e) { toastError(e); }
    busy = null;
  }
  async function check(p: PackInfo) {
    busy = p.id;
    try {
      const c = await api.checkPack(s.id, p.id);
      if (c.pinned) toast('info', 'This pack is pinned', { body: c.message });
      else if (c.up_to_date) toast('success', `${p.name} is up to date`, { body: `Version ${c.current.version}` });
      else if (c.diff) checked = { pack: p, check: c };
    } catch (e) { toastError(e, 'Could not check for a pack update'); }
    busy = null;
  }
  async function apply() {
    if (!checked?.check.token) return;
    applying = true;
    try { const r = await api.updatePack(s.id, checked.pack.id, checked.check.token); trackOp(r.operation); onoperation(r.operation); checked = null; } catch (e) { toastError(e, 'Could not start the update'); }
    applying = false;
  }
  const d = $derived(checked?.check.diff ?? null);
  const counts = $derived(d ? packDiffCounts(d) : null);
</script>

{#if err}<div class="banner warn"><Icon name="alert" size={16} />{err}</div>{/if}
{#if packs?.length}
  <section class="card packs" aria-labelledby="packs-h">
    <h2 id="packs-h" class="h">Modpacks</h2>
    <ul>
      {#each packs as p (p.id)}
        <li>
          <div class="grow">
            <div class="row g6 wrap"><b>{p.name}</b><span class="badge tnum">{p.version || 'unknown version'}</span>{#if p.pinned}<span class="badge warn"><Icon name="pin" size={11} />Pinned</span>{/if}</div>
            <div class="xs muted">{p.loader || 'no loader'} · Minecraft {p.game_version} · {p.items} mods{p.source ? ` · from ${p.source.provider === 'curseforge' ? 'CurseForge' : 'Modrinth'}` : ' · imported from a file'} · installed {relTime(p.installed_at)}</div>
          </div>
          {#if canManage}
            <div class="row g6 wrap">
              <button class="btn sm" disabled={busy === p.id} aria-pressed={p.pinned} title="Keep this pack at its current version" onclick={() => pin(p)}>{p.pinned ? 'Unpin' : 'Pin'}</button>
              <button class="btn sm" class:spin={busy === p.id} disabled={busy === p.id || !p.can_check} title={p.can_check ? undefined : 'This pack was imported from a file, so Moss cannot look for newer versions by itself.'} onclick={() => check(p)}>Check for pack update</button>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

{#if checked && d && counts}
  <Modal title="{checked.pack.name}: {d.versions.from.version} to {d.versions.to.version}" onclose={() => (checked = null)} width={640}>
    <p class="sm muted">Nothing changes until you apply. Moss takes a backup first when that setting is on, and the change can be undone afterwards.</p>
    {#each d.blockers as b}<div class="banner danger" role="alert"><Icon name="x-circle" size={16} /><div>{b} <span class="xs">Use the Upgrade tab first.</span></div></div>{/each}
    {#each d.warnings as w}<div class="banner warn"><Icon name="alert" size={16} />{w}</div>{/each}
    <div class="sum" role="list" aria-label="Summary">
      <span role="listitem" class="badge ok">{counts.added} added</span><span role="listitem" class="badge info">{counts.updated} updated</span><span role="listitem" class="badge danger">{counts.removed} removed</span>
      <span role="listitem" class="badge">{counts.configNew + counts.configUpdated} config files</span>{#if counts.held}<span role="listitem" class="badge warn">{counts.held} held</span>{/if}
    </div>
    {#each [['Added', d.files.added], ['Updated', d.files.updated], ['Removed', d.files.removed], ['Held back', d.files.held]] as [label, list]}
      {#if (list as typeof d.files.added).length}
        <h4 class="sub">{label}</h4>
        <ul class="chg">{#each list as typeof d.files.added as c}<li><span class="grow ellipsis">{c.title}</span><span class="xs muted tnum">{packChangeText(c)}</span>{#if c.reason}<span class="badge">{c.reason}</span>{/if}</li>{/each}</ul>
      {/if}
    {/each}
    {#if d.overrides.added.length || d.overrides.updated.length}
      <h4 class="sub">Config files that will be written</h4>
      <ul class="chg">{#each d.overrides.added as p}<li><span class="grow mono xs ellipsis">{p}</span><span class="badge ok">new</span></li>{/each}{#each d.overrides.updated as p}<li><span class="grow mono xs ellipsis">{p}</span><span class="badge info">updated</span></li>{/each}</ul>
    {/if}
    {#if d.overrides.user_modified.length}
      <h4 class="sub">Files you changed (kept as they are)</h4>
      <ul class="chg">{#each d.overrides.user_modified as f}<li><span class="grow mono xs ellipsis" title={f.path}>{f.path}</span><span class="badge warn">your version stays</span></li>{/each}</ul>
    {/if}
    {#if !d.changes}<p class="sm muted">This version changes nothing on your server.</p>{/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (checked = null)}>Close</button>
      {#if checked && checked.check.token}
        <button class="btn" disabled={!d.can_apply || !d.changes || applying} onclick={() => { const c = checked!; checked = null; ontest(c.pack, c.check.token!); }}>Try update on a copy first</button>
        <button class="btn primary" disabled={!d.can_apply || !d.changes || applying} onclick={apply}>{applying ? 'Working…' : 'Update pack'}</button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .packs { margin-bottom: 16px; }
  .h { font-size: var(--fs-md, 1rem); margin: 0 0 8px; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .packs li { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; padding: 10px 12px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); }
  .sub { margin: 14px 0 6px; font-size: var(--fs-sm); color: var(--muted); }
  .sum { display: flex; gap: 6px; flex-wrap: wrap; margin: 10px 0; }
  .chg li { display: flex; gap: 10px; align-items: center; padding: 6px 10px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); }
</style>
