<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import States from '../../components/States.svelte';
  import BackupList from '../../components/BackupList.svelte';
  import TimezoneSelect from '../../components/TimezoneSelect.svelte';
  import { api, apiOps, errMsg, type Backup, type Schedule, type Server } from '../../lib/api';
  import { backupNow } from '../../lib/actions';
  import { confirmDialog, onOp, toast, toastError } from '../../lib/stores.svelte';
  import { can } from '../../lib/auth.svelte';
  import { relTime } from '../../lib/format';
  import { browserZone, dailyWording, dstNote, formatInZone, nextDailyRun, nextRunDisplay, offsetMinutes, zoneLabel } from '../../lib/tz';

  let { server: s }: { server: Server } = $props();
  const manage = $derived(can('backup.manage'));
  let list = $state<Backup[] | null>(null);
  let err = $state('');
  let note = $state('');
  let sched = $state<Schedule[] | null>(null);
  let sErr = $state('');

  async function load() { err = ''; try { list = await api.backups(s.id); } catch (e) { err = errMsg(e); } }
  async function loadSched() { sErr = ''; try { sched = await api.schedules(s.id); } catch (e) { sErr = errMsg(e); } }
  $effect(() => { s.id; list = sched = null; load(); loadSched(); });
  $effect(() => onOp((o) => { if (o.server_id === s.id && o.kind.startsWith('backup') && o.state !== 'running' && o.state !== 'queued') load(); }));

  async function create(e: Event) { e.preventDefault(); await backupNow(s.id, note.trim() || undefined); note = ''; }

  // schedule form
  let f = $state({ kind: 'backup' as Schedule['kind'], command: '', mode: 'daily' as 'daily' | 'every', time: '04:00', every: 180, keep: 7, adding: false, tz: browserZone() });
  let zones = $state<string[]>([]);
  $effect(() => {
    if (!f.adding || zones.length) return;
    apiOps.timezones().then((r) => (zones = r.zones)).catch(() => { try { zones = Intl.supportedValuesOf('timeZone'); } catch { zones = ['UTC']; } });
  });
  // The zone list may omit the browser's own name (e.g. "Etc/UTC"); keep it selectable.
  const zoneList = $derived(zones.includes(f.tz) ? zones : [f.tz, ...zones]);
  const here = browserZone();
  const preview = $derived.by(() => {
    const t = f.mode === 'daily' ? nextDailyRun(f.time, f.tz) : null;
    return t === null ? null : { zone: formatInZone(t, f.tz), local: formatInZone(t, here), same: offsetMinutes(f.tz, t) === offsetMinutes(here, t) };
  });
  const when = (x: Schedule) => (x.daily_at ? dailyWording(x) : x.every_minutes ? (x.every_minutes % 60 === 0 ? `Every ${x.every_minutes / 60} h` : `Every ${x.every_minutes} min`) : '—');
  async function addSched(e: Event) {
    e.preventDefault();
    try {
      await api.createSchedule(s.id, {
        kind: f.kind, command: f.kind === 'command' ? f.command : null,
        daily_at: f.mode === 'daily' ? f.time : null, every_minutes: f.mode === 'every' ? f.every : null,
        tz: f.tz, enabled: true, keep: f.kind === 'backup' ? f.keep : null,
      });
      f.adding = false; toast('success', 'Schedule added'); loadSched();
    } catch (er) { toastError(er, 'Could not add schedule'); }
  }
  async function toggle(x: Schedule) { try { await api.patchSchedule(x.id, { enabled: !x.enabled }); x.enabled = !x.enabled; } catch (e) { toastError(e); } }
  async function run(x: Schedule) { try { await api.runSchedule(x.id); toast('info', 'Schedule triggered'); } catch (e) { toastError(e); } }
  async function del(x: Schedule) {
    if (!(await confirmDialog({ title: 'Delete schedule?', body: `${x.kind} · ${when(x)}`, confirmLabel: 'Delete', danger: true })).ok) return;
    try { await api.deleteSchedule(x.id); loadSched(); } catch (e) { toastError(e); }
  }
</script>

<div class="row between wrap" style="margin-bottom:16px">
  <h2>Backups</h2>
  {#if manage}
    <form class="row g6" onsubmit={create}>
      <input class="input" style="width:220px" placeholder="Note (optional)" aria-label="Backup note" bind:value={note} />
      <button class="btn primary"><Icon name="plus" size={15} />Back up now</button>
    </form>
  {/if}
</div>

{#if err}<States kind="error" text={err} retry={load} />
{:else if !list}<States kind="loading" rows={3} />
{:else if !list.length}<States kind="empty" icon="archive" title="No backups yet" text="Create one now or set up a schedule below." />
{:else}<BackupList backups={list} onchange={load} />{/if}

<section class="card mt" style="margin-top:28px">
  <div class="card-title"><Icon name="clock" size={14} />Schedules <span class="grow"></span>{#if manage}<button class="btn sm" style="text-transform:none;letter-spacing:0" onclick={() => (f.adding = !f.adding)}><Icon name="plus" size={13} />Add schedule</button>{/if}</div>
  {#if f.adding}
    <form class="sf" onsubmit={addSched}>
      <div class="field"><label for="sk">Action</label><select id="sk" class="select" bind:value={f.kind}><option value="backup">Backup</option><option value="restart">Restart</option><option value="command">Run command</option><option value="start">Start</option><option value="stop">Stop</option></select></div>
      {#if f.kind === 'command'}<div class="field"><label for="sc">Command</label><input id="sc" class="input mono" required bind:value={f.command} placeholder="say Restarting soon" /></div>{/if}
      <div class="field"><label for="sm">Repeat</label><select id="sm" class="select" bind:value={f.mode}><option value="daily">Daily at a time</option><option value="every">Every N minutes</option></select></div>
      {#if f.mode === 'daily'}
        <div class="field"><label for="st">Time of day</label><input id="st" class="input" type="time" required bind:value={f.time} aria-describedby="st-h" /></div>
        <div class="field"><label for="stz">Time zone</label><TimezoneSelect id="stz" zones={zoneList} bind:value={f.tz} describedby="st-h" /></div>
      {:else}<div class="field"><label for="se">Minutes</label><input id="se" class="input" type="number" min="5" required bind:value={f.every} /></div>{/if}
      {#if f.kind === 'backup'}<div class="field"><label for="sg">Keep last</label><input id="sg" class="input" type="number" min="1" bind:value={f.keep} /></div>{/if}
      {#if f.mode === 'daily'}
        <div class="full xs muted" id="st-h" aria-live="polite">
          Runs every day at <b>{f.time || '…'}</b> {zoneLabel(f.tz)}.
          {#if preview}Next run: <b>{preview.zone}</b>{#if !preview.same} · <b>{preview.local}</b> for you ({zoneLabel(here)}){/if}.{/if}
          {#if dstNote(f.tz)}<div>{dstNote(f.tz)}</div>{/if}
        </div>
      {/if}
      <div class="row end" style="align-self:end"><button type="button" class="btn" onclick={() => (f.adding = false)}>Cancel</button><button class="btn primary">Add</button></div>
    </form>
  {/if}
  {#if sErr}<States kind="error" text={sErr} retry={loadSched} />
  {:else if !sched}<States kind="loading" rows={1} />
  {:else if !sched.length && !f.adding}<p class="muted sm">No schedules. Automatic backups protect you from bad updates and griefing.</p>
  {:else if sched.length}
    <ul class="sl">
      {#each sched as x (x.id)}
        {@const nr = x.enabled ? nextRunDisplay(x) : null}
        <li>
          <input type="checkbox" class="switch" checked={x.enabled} disabled={!manage} aria-label="Enable schedule" onchange={() => toggle(x)} />
          <div class="grow">
            <div><b style="text-transform:capitalize">{x.kind}</b>{#if x.command} <code>{x.command}</code>{/if} · {when(x)}{#if x.keep} · keep {x.keep}{/if}</div>
            <div class="xs muted">Last run {x.last_run ? relTime(x.last_run) : 'never'}{x.last_status ? ` (${x.last_status})` : ''} · next {x.enabled ? relTime(x.next_run) : 'paused'}</div>
            {#if nr}<div class="xs muted" title="Next run in the schedule's zone and in your zone">{nr.inZone}{#if !nr.same} · <span style="color:var(--text)">{nr.local}</span> your time{/if}</div>{/if}
          </div>
          {#if manage}<button class="btn sm" onclick={() => run(x)}><Icon name="play" size={12} />Run now</button><button class="btn ghost icon sm" aria-label="Delete schedule" onclick={() => del(x)}><Icon name="trash" size={14} /></button>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .full { grid-column: 1 / -1; }
  .sf { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; padding: 14px; margin-bottom: 14px; background: var(--s2); border: 1px solid var(--line); border-radius: var(--r-sm); }
  .sl { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .sl li { display: flex; align-items: center; gap: 14px; padding: 10px 0; border-bottom: 1px solid var(--line); } .sl li:last-child { border: 0; }
</style>
