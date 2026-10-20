<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import DestinationForm from './DestinationForm.svelte';
  import { apiOps, ApiError, errMsg, type BackupDestination, type DestinationTest, type ServerDestination } from '../../lib/api';
  import { confirmDialog, servers, toast, toastError } from '../../lib/stores.svelte';
  import { keepText } from '../../lib/remote';

  let list = $state<BackupDestination[] | null>(null);
  let err = $state('');
  let editing = $state<BackupDestination | null>(null);
  let adding = $state(false);
  let results = $state<Record<string, DestinationTest | 'running'>>({});
  let choice = $state<Record<string, ServerDestination>>({});

  async function load() {
    err = '';
    try {
      list = await apiOps.destinations();
      const all = await Promise.all(servers.list.map((s) => apiOps.serverDestination(s.id).then((d) => [s.id, d] as const).catch(() => null)));
      choice = Object.fromEntries(all.filter((x) => x !== null));
    } catch (e) { err = errMsg(e); }
  }
  void load();
  const where = (d: BackupDestination) => (d.kind === 'folder' ? d.path : `${d.bucket}${d.prefix ? '/' + d.prefix : ''} · ${d.endpoint}`);

  async function test(d: BackupDestination) {
    results[d.id] = 'running';
    try { results[d.id] = await apiOps.testDestination(d.id); } catch (e) { delete results[d.id]; toastError(e, 'Could not run the test'); }
  }
  async function remove(d: BackupDestination) {
    const n = d.stored_backups ?? 0;
    const r = await confirmDialog({ title: `Remove ${d.name}?`, body: n ? `${n} backup${n === 1 ? ' is' : 's are'} stored there. The files stay where they are, but Moss forgets them, and backups that exist only there disappear from the list.` : 'Moss stops copying backups there. Nothing already stored is deleted.', confirmLabel: 'Remove destination', danger: true });
    if (!r.ok) return;
    try { await apiOps.removeDestination(d.id, n > 0); toast('info', 'Destination removed'); void load(); }
    catch (e) { if (e instanceof ApiError && e.code === 'destination.in_use') void load(); toastError(e, 'Could not remove'); }
  }
  async function assign(sid: string, v: string) {
    try { choice[sid] = await apiOps.setServerDestination(sid, v); } catch (e) { toastError(e, 'Could not change the destination'); void load(); }
  }
  const done = () => { adding = false; editing = null; void load(); };
</script>

<section class="card" aria-labelledby="bd">
  <div class="card-title" id="bd"><Icon name="archive" size={14} />Backup destinations</div>
  <p class="muted sm">Copy every finished backup to another disk or to cloud storage, so a dead drive does not take your worlds with it. Each copy is checked after it is written.</p>
  {#if err}<States kind="error" text={err} retry={load} />
  {:else if !list}<States kind="loading" rows={2} />
  {:else}
    {#if !list.length}<p class="muted sm" style="margin-top:12px">No destinations yet. Backups stay on this machine only.</p>{/if}
    <ul class="dl">
      {#each list as d (d.id)}
        {@const r = results[d.id]}
        <li>
          <div class="row g8 wrap">
            <Icon name={d.kind === 'folder' ? 'folder' : 'upload'} size={16} /><b>{d.name}</b>
            {#if d.is_default}<span class="badge ok">Default</span>{/if}
            <span class="badge">{d.kind === 'folder' ? 'Folder' : 'S3'}</span>
            <span class="grow"></span>
            <button class="btn sm" onclick={() => test(d)} disabled={r === 'running'}>{r === 'running' ? 'Testing…' : 'Test'}</button>
            <button class="btn sm" onclick={() => (editing = d)}><Icon name="edit" size={13} />Edit</button>
            <button class="btn ghost icon sm" aria-label={`Remove ${d.name}`} onclick={() => remove(d)}><Icon name="trash" size={14} /></button>
          </div>
          <div class="xs muted mono ellipsis" title={where(d)}>{where(d)}</div>
          <div class="xs muted">{keepText(d.keep_local, 'local backup')} on this machine · {keepText(d.keep_remote, 'copy')} here · {d.stored_backups ?? 0} stored{d.kind === 's3' ? ` · keys ${d.access_key === 'set' && d.secret_key === 'set' ? 'saved' : 'missing'}` : ''}</div>
          {#if r && r !== 'running'}
            <div class="banner {r.ok ? 'ok' : 'danger'}" role="status" style="margin-top:8px"><Icon name={r.ok ? 'check' : 'alert'} size={16} />
              <div><b>{r.ok ? 'Everything worked' : 'The test failed'}</b><ul class="steps">{#each r.steps as s}<li class:bad={!s.ok}>{s.ok ? '✓' : '✗'} {s.step}{s.detail ? `: ${s.detail}` : ''}</li>{/each}</ul></div></div>
          {/if}
        </li>
      {/each}
    </ul>
    <div class="row" style="margin-top:12px"><button class="btn primary" onclick={() => (adding = true)}><Icon name="plus" size={15} />Add destination</button></div>

    {#if list.length && servers.list.length}
      <h3 class="sm sh">Per server</h3>
      <div class="table-wrap"><table>
        <thead><tr><th>Server</th><th>Backups go to</th></tr></thead>
        <tbody>
          {#each servers.list as s (s.id)}
            {@const c = choice[s.id]}
            <tr><td>{s.name}</td><td>
              <label class="sr" for={`sd-${s.id}`}>Destination for {s.name}</label>
              <select id={`sd-${s.id}`} class="select" value={c?.destination_id ?? 'default'} disabled={!c} onchange={(e) => assign(s.id, e.currentTarget.value)}>
                <option value="default">Default{c?.destination_id === 'default' ? c.effective ? ` (${c.effective.name})` : ' (none)' : ''}</option>
                <option value="none">Keep on this machine only</option>
                {#each list as d}<option value={d.id}>{d.name}</option>{/each}
              </select></td></tr>
          {/each}
        </tbody>
      </table></div>
    {/if}
  {/if}
</section>

{#if adding}<DestinationForm onclose={() => (adding = false)} ondone={done} />{/if}
{#if editing}<DestinationForm dest={editing} onclose={() => (editing = null)} ondone={done} />{/if}

<style>
  .dl { list-style: none; margin: 12px 0 0; padding: 0; } .dl li { padding: 12px 0; border-bottom: 1px solid var(--line); } .dl li:last-child { border: 0; }
  .sh { margin: 22px 0 8px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); }
  .steps { margin: 6px 0 0; padding: 0; list-style: none; font-size: var(--fs-sm); } .steps li.bad { color: var(--danger); }
  .banner.ok { background: var(--a-soft); border-color: color-mix(in srgb, var(--a2), transparent 60%); }
</style>
