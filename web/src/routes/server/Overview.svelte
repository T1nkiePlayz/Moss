<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import MetricsPanel from '../../components/MetricsPanel.svelte';
  import PulseStrip from '../../components/PulseStrip.svelte';
  import HealthCards from '../../components/HealthCards.svelte';
  import AnsiLine from '../../components/AnsiLine.svelte';
  import States from '../../components/States.svelte';
  import StatusBadge from '../../components/StatusBadge.svelte';
  import { api, apiOps, errMsg, type Server, type MetricSample, type TimelineEvent, type Finding, type UndoEntry } from '../../lib/api';
  import type { ChartDef } from '../../lib/metrics';
  import { poll, toast, toastError, confirmDialog } from '../../lib/stores.svelte';
  import { power, consequence } from '../../lib/actions';
  import { duration, bytes, relTime, pct } from '../../lib/format';
  import { levelOf } from '../../lib/ansi';
  import { serverHref } from '../../lib/route';

  let { server: s }: { server: Server } = $props();
  const id = $derived(s.id);

  let events = $state<TimelineEvent[] | null>(null);
  let findings = $state<Finding[]>([]);
  let fLoaded = $state(false);
  let tail = $state<string[]>([]);
  let undo = $state<UndoEntry[] | null>(null);
  let undoErr = $state('');

  $effect(() => {
    const sid = id;
    events = null; fLoaded = false;
    const stop = poll(async () => {
      if (s.capabilities.includes('console')) api.backlog(sid, 8).then((b) => (tail = b.lines.slice(-8))).catch(() => {});
    }, 5000);
    api.timeline(sid).then((e) => (events = e)).catch(() => (events = []));
    api.health(sid).then((f) => (findings = f)).catch(() => {}).finally(() => (fLoaded = true));
    loadUndo();
    return stop;
  });
  async function loadUndo() { try { undo = await api.undoList(id); undoErr = ''; } catch (e) { undoErr = errMsg(e); } }
  async function revert(u: UndoEntry) {
    const r = await confirmDialog({ title: `Undo "${u.title}"?`, body: 'Restores these files to how they were before the change.', consequence: u.files.slice(0, 5).join(', '), confirmLabel: 'Revert' });
    if (!r.ok) return;
    try { await api.revert(u.id); toast('success', 'Reverted', { body: u.title }); loadUndo(); } catch (e) { toastError(e, 'Undo failed'); }
  }

  const active = $derived(s.state === 'running' || s.state === 'starting');
  const busy = $derived(s.state === 'installing' || s.state === 'stopping' || s.state === 'starting');
  const addr = $derived(`${location.hostname}:${s.port}`);
  async function copy() {
    try { await navigator.clipboard.writeText(addr); toast('success', 'Address copied', { body: addr, timeout: 2200 }); } catch { toast('warn', 'Could not copy', { body: addr }); }
  }
  const memMax = $derived(s.memory_mb * 1048576);
  const defs = $derived<ChartDef[]>([
    { id: 'cpu', label: 'CPU', icon: 'cpu', avg: 'cpu_avg', peak: 'cpu_max', max: 100, format: (v) => v.toFixed(0) + '%' },
    { id: 'mem', label: 'Memory', icon: 'ram', avg: 'mem_avg', peak: 'mem_max', max: s.capabilities.includes('java') ? memMax : undefined, color: 'var(--info)', format: (v) => bytes(v) },
    { id: 'players', label: 'Players', icon: 'users', avg: 'players_max', peak: 'players_max', color: 'var(--warn)', format: (v) => Math.round(v) + ' online', show: (m) => s.capabilities.includes('players') && m.some((x) => (x.players_max ?? 0) > 0) },
  ]);
  // Servers without /metrics (older backend): the last hour comes from the raw stats ring buffer.
  const fallback = async (): Promise<MetricSample[]> => (await api.stats(s.id)).samples.map((x) => ({ t: x.t, cpu_avg: x.cpu, cpu_max: x.cpu, mem_avg: x.mem, mem_max: x.mem, players_max: x.players ?? 0 }));
</script>

<div class="grid hero-grid">
  <section class="card hero" class:on={s.state === 'running'} class:bad={s.state === 'crashed' || s.state === 'failed'} aria-label="Status">
    <div class="row between wrap g16">
      <div class="row g16">
        <div class="orb"><span class="dot {s.state}" style="width:14px;height:14px"></span></div>
        <div>
          <div class="row g6"><StatusBadge state={s.state} /><span class="muted sm">since {relTime(s.state_since)}</span></div>
          <div class="big tnum">{s.state === 'running' ? duration(s.uptime_s) : s.state === 'crashed' ? 'Crashed' : s.state === 'stopped' ? 'Offline' : '…'}</div>
          {#if s.state_detail}<div class="sm" style="color:var(--warn)">{s.state_detail}</div>{/if}
        </div>
      </div>
      <div class="row g6 wrap">
        {#if !active}
          <button class="btn primary lg" disabled={busy} onclick={() => power(s, 'start')} title={consequence.start}><Icon name="play" size={16} />Start</button>
        {:else}
          <button class="btn lg" disabled={busy && s.state !== 'starting'} onclick={() => power(s, 'restart')} title={consequence.restart}><Icon name="refresh" size={16} />Restart</button>
          <button class="btn danger lg" disabled={s.state === 'stopping'} onclick={() => power(s, 'stop')} title={consequence.stop}><Icon name="stop" size={15} />Stop</button>
        {/if}
        {#if s.state === 'stopping' || s.state === 'starting'}<button class="btn ghost lg" onclick={() => power(s, 'kill')}>Force kill</button>{/if}
      </div>
    </div>
    <div class="facts">
      <div><span class="muted xs">Address</span><button class="btn ghost sm copy mono" onclick={copy} aria-label="Copy address {addr}">{addr}<Icon name="copy" size={13} /></button></div>
      <div><span class="muted xs">Players</span><b class="tnum">{s.players ? (s.players.max ? `${s.players.online} / ${s.players.max}` : `${s.players.online}`) : '—'}</b></div>
      <div><span class="muted xs">CPU</span><b class="tnum">{pct(s.cpu_pct)}</b></div>
      <div><span class="muted xs">Memory</span><b class="tnum">{bytes(s.mem_bytes)}{#if s.capabilities.includes('java')} <span class="faint">/ {s.memory_mb} MB</span>{/if}</b></div>
    </div>
    {#if s.motd}<div class="motd mono sm" aria-label="Message of the day">{s.motd}</div>{/if}
  </section>

  <section class="card" aria-label="Players">
    <div class="card-title"><Icon name="users" size={14} />Players online</div>
    {#if s.players?.sample?.length}
      <ul class="plist">{#each s.players.sample as p}<li><span class="av">{p[0]?.toUpperCase()}</span>{p}</li>{/each}</ul>
    {:else}
      <p class="muted sm">{s.state === 'running' ? (s.players?.online ? `${s.players.online} online (names unavailable).` : 'Nobody is online.') : 'Server is not running.'}</p>
    {/if}
  </section>
</div>

<div class="mt">
  {#key s.id}<MetricsPanel title="Resources" icon="activity" load={(r) => apiOps.metrics(s.id, r)} {defs} {fallback} />{/key}
</div>

<section class="card mt" aria-label="Pulse timeline">
  <div class="card-title"><Icon name="activity" size={14} />Pulse · last 24 hours</div>
  <PulseStrip {events} running={s.state === 'running'} />
  <div class="row g16 wrap xs muted" style="margin-top:10px">
    <span><i class="sq" style="background:var(--a2)"></i>Up</span><span><i class="sq" style="background:var(--danger)"></i>Crash</span><span><i class="sq" style="background:var(--warn)"></i>Restart</span><span><i class="sq" style="background:var(--info)"></i>Backup / change</span>
  </div>
</section>

<div class="grid cols-2 mt" style="align-items:start">
  <section aria-label="Health">
    <h2 class="sec"><Icon name="heart" size={16} />Health</h2>
    <HealthCards {findings} loaded={fLoaded} showServer={false} />
  </section>
  <section aria-label="Undo history">
    <h2 class="sec"><Icon name="history" size={16} />Undo history</h2>
    {#if undoErr}<States kind="error" text={undoErr} retry={loadUndo} />
    {:else if !undo}<States kind="loading" rows={2} />
    {:else if !undo.length}<States kind="empty" icon="undo" title="Nothing to undo" text="Config edits, file saves and add-on changes can be reverted from here." />
    {:else}
      <ul class="undo">
        {#each undo as u (u.id)}
          <li><div class="grow"><div class="ellipsis"><b>{u.title}</b></div><div class="xs muted ellipsis">{relTime(u.created_at)} · {u.files.join(', ')}</div></div><button class="btn sm" onclick={() => revert(u)}><Icon name="undo" size={13} />Undo</button></li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

{#if s.capabilities.includes('console')}
  <section class="card mt" aria-label="Console tail">
    <div class="card-title"><Icon name="terminal" size={14} />Console <span class="grow"></span><a class="btn sm" href={serverHref(s.id, 'console')} style="text-transform:none;letter-spacing:0">Open console<Icon name="chevron-right" size={13} /></a></div>
    {#if tail.length}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -- keyboard users must be able to scroll this region -->
      <div class="tail mono xs" tabindex="0" role="log" aria-label="Recent console output">{#each tail as l}<div class="l {levelOf(l)}"><AnsiLine line={l} /></div>{/each}</div>
    {:else}<p class="muted sm">No recent output.</p>{/if}
  </section>
{/if}

<style>
  .hero-grid { grid-template-columns: minmax(0, 2fr) minmax(220px, 1fr); }
  @media (max-width: 860px) { .hero-grid { grid-template-columns: minmax(0, 1fr); } }
  .hero { padding: 20px; overflow: hidden; }
  .hero.on::before { content: ''; position: absolute; inset: -40% -10% auto -10%; height: 200px; background: radial-gradient(closest-side, var(--a-soft), transparent); pointer-events: none; }
  .hero.bad { border-color: color-mix(in srgb, var(--danger), transparent 55%); }
  .hero.bad::before { content: ''; position: absolute; inset: -40% -10% auto -10%; height: 200px; background: radial-gradient(closest-side, var(--danger-soft), transparent); pointer-events: none; }
  .orb { width: 52px; height: 52px; border-radius: 16px; display: grid; place-items: center; background: var(--s3); border: 1px solid var(--line2); flex: none; }
  .big { font-size: 30px; font-weight: 650; letter-spacing: -.02em; line-height: 1.2; }
  .facts { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 14px; margin-top: 20px; padding-top: 16px; border-top: 1px solid var(--line); position: relative; }
  .facts > div { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .copy { padding: 0 6px; margin-left: -6px; width: fit-content; max-width: 100%; color: var(--text); }
  .motd { margin-top: 14px; padding: 8px 12px; background: var(--bg); border-radius: 8px; color: var(--muted); border: 1px solid var(--line); position: relative; }
  .plist { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 8px; }
  .plist li { display: flex; align-items: center; gap: 10px; }
  .av { width: 26px; height: 26px; border-radius: 7px; background: var(--a-soft); color: var(--a-text); display: grid; place-items: center; font-weight: 700; font-size: 12px; }
  .sec { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
  .sq { display: inline-block; width: 10px; height: 10px; border-radius: 3px; margin-right: 6px; vertical-align: -1px; }
  .undo { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 8px; }
  .undo li { display: flex; gap: 12px; align-items: center; padding: 10px 12px; background: var(--s1); border: 1px solid var(--line); border-radius: var(--r-sm); }
  .tail { background: #070a08; color: #d7e3db; border-radius: 8px; padding: 10px 12px; overflow: auto; }
  .l { white-space: pre; line-height: 1.6; } .l.warn { color: #f2b04c; } .l.error { color: #ff7d71; } .l.debug { color: #8a9d92; }
</style>
