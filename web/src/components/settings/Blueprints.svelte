<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import { api, errMsg, type Blueprint } from '../../lib/api';
  import { confirmDialog, toast, toastError } from '../../lib/stores.svelte';
  import { relTime } from '../../lib/format';

  let list = $state<Blueprint[] | null>(null);
  let err = $state('');
  let doc = $state('');
  let docErr = $state('');
  async function load() { err = ''; try { list = await api.blueprints(); } catch (e) { err = errMsg(e); } }
  $effect(() => { void load(); });

  async function exportOne(b: Blueprint) {
    try {
      const d = await api.exportBlueprint(b.id);
      const a = document.createElement('a');
      a.href = URL.createObjectURL(new Blob([JSON.stringify(d, null, 2)], { type: 'application/json' }));
      a.download = `${b.name.replace(/[^\w.-]+/g, '-').toLowerCase() || 'blueprint'}.moss-blueprint.json`; a.click(); URL.revokeObjectURL(a.href);
    } catch (e) { toastError(e, 'Could not export'); }
  }
  async function remove(b: Blueprint) {
    const r = await confirmDialog({ title: `Delete blueprint “${b.name}”?`, body: 'Servers already made from it are not affected.', confirmLabel: 'Delete blueprint', danger: true });
    if (!r) return;
    try { await api.deleteBlueprint(b.id); list = (list ?? []).filter((x) => x.id !== b.id); toast('info', 'Blueprint deleted'); } catch (e) { toastError(e, 'Could not delete'); }
  }
  async function doImport() {
    docErr = '';
    let parsed: unknown;
    try { parsed = JSON.parse(doc); } catch { docErr = 'That is not valid JSON.'; return; }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) { docErr = 'Expected a blueprint document.'; return; }
    try { const b = await api.importBlueprint(parsed as Record<string, unknown>); list = [b, ...(list ?? []).filter((x) => x.id !== b.id)]; doc = ''; toast('success', `Imported “${b.name}”`); } catch (e) { docErr = errMsg(e); }
  }
  function pick(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!f) return;
    if (f.size > 1_000_000) { docErr = 'That file is too large for a blueprint.'; return; }
    void f.text().then((t) => (doc = t));
  }
</script>

<section class="card" aria-labelledby="bp">
  <div class="card-title" id="bp"><Icon name="package" size={14} />Server blueprints</div>
  <p class="muted sm" style="margin:0 0 12px">A blueprint is a reusable recipe: runtime, version, memory, settings, content and schedules. Worlds, passwords, seeds and ports are never saved. Make one from a server's Settings tab, then pick it on the New server page.</p>
  {#if err}<States kind="error" text={err} retry={load} />
  {:else if !list}<States kind="loading" rows={2} />
  {:else if !list.length}<p class="muted sm">No blueprints yet.</p>
  {:else}
    <div class="table-wrap"><table>
      <caption class="sr">Saved blueprints</caption>
      <thead><tr><th>Name</th><th>Runtime</th><th class="num">Settings</th><th class="num">Content</th><th class="num">Schedules</th><th>Saved</th><th><span class="sr">Actions</span></th></tr></thead>
      <tbody>{#each list as b (b.id)}
        <tr><td>{b.name}</td><td>{b.runtime} {b.game_version}</td><td class="num tnum">{b.properties}</td><td class="num tnum">{b.content}</td><td class="num tnum">{b.schedules}</td><td>{relTime(b.created_at)}</td>
          <td class="row g4"><button class="btn sm" onclick={() => exportOne(b)} aria-label="Export {b.name}"><Icon name="download" size={13} />Export</button><button class="btn sm danger" onclick={() => remove(b)} aria-label="Delete {b.name}">Delete</button></td></tr>
      {/each}</tbody>
    </table></div>
  {/if}
  <details style="margin-top:14px">
    <summary style="cursor:pointer;font-weight:550">Import a blueprint</summary>
    <div class="field" style="margin-top:10px"><label for="bp-file">Blueprint file</label><input id="bp-file" class="input" type="file" accept="application/json,.json" onchange={pick} /></div>
    <div class="field" style="margin-top:8px"><label for="bp-doc">Or paste its JSON</label><textarea id="bp-doc" class="textarea mono" rows="5" spellcheck="false" bind:value={doc}></textarea>
      {#if docErr}<span class="hint" style="color:var(--danger)" role="alert">{docErr}</span>{/if}</div>
    <div style="margin-top:8px"><button class="btn sm primary" disabled={!doc.trim()} onclick={doImport}>Import</button></div>
  </details>
</section>
