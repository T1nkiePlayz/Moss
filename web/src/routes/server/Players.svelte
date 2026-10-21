<script lang="ts">
  import States from '../../components/States.svelte';
  import PlayerListSubs from './PlayerListSubs.svelte';
  import { apiExtra } from '../../lib/api-extra';
  import { apiOps, errMsg, type AnalyticsDays, type OnlinePlayers, type PlayerAnalytics, type Server } from '../../lib/api';
  import { confirmDialog, poll, toast, toastError } from '../../lib/stores.svelte';
  import { can } from '../../lib/auth.svelte';
  import { bars, heatmap, weekdays } from '../../lib/analytics';
  import { dateTime, duration, num, relTime } from '../../lib/format';

  let { server: s }: { server: Server } = $props();
  const choices: { id: AnalyticsDays; label: string }[] = [{ id: 7, label: '7 days' }, { id: 30, label: '30 days' }, { id: 90, label: '90 days' }];
  let days = $state<AnalyticsDays>(7);
  let data = $state<PlayerAnalytics | null>(null);
  let err = $state('');
  let seq = 0;

  async function load() {
    const my = ++seq, d = days, id = s.id;
    try {
      const r = await apiOps.analytics(id, d);
      if (my === seq) { data = r; err = ''; }
    } catch (e) { if (my === seq) err = errMsg(e); }
  }
  $effect(() => { days; s.id; data = null; err = ''; return poll(load, 60000); });

  let live = $state<OnlinePlayers | null>(null);
  let liveErr = $state('');
  const running = $derived(s.state === 'running');
  const canAct = $derived(can('server.console.write'));
  async function loadLive() {
    const id = s.id;
    try { const r = await apiExtra.onlinePlayers(id); if (id === s.id) { live = r; liveErr = ''; } } catch (e) { if (id === s.id) liveErr = errMsg(e); }
  }
  $effect(() => { s.id; running; live = null; liveErr = ''; if (!running) return; return poll(loadLive, 15000); });
  async function act(name: string, ban: boolean) {
    const verb = ban ? 'Ban' : 'Kick';
    if (!(await confirmDialog({ title: `${verb} ${name}?`, body: ban ? 'They will not be able to rejoin until unbanned.' : 'They are disconnected and may rejoin.', confirmLabel: verb, danger: true })).ok) return;
    try { await (ban ? apiExtra.banPlayer : apiExtra.kickPlayer)(s.id, name); toast('success', `${verb === 'Ban' ? 'Banned' : 'Kicked'} ${name}`); loadLive(); } catch (e) { toastError(e, `Could not ${verb.toLowerCase()} ${name}`); }
  }

  const pts = $derived(data ? bars(data.hourly, data.days) : []);
  const top = $derived(Math.max(1, ...pts.map((b) => b.peak)));
  const heat = $derived(data ? heatmap(data.hourly) : { cells: [], max: 0 });
  const empty = $derived(!!data && data.hourly.length === 0 && data.sessions === 0);
  const BW = 6, H = 120;
  let hover = $state<number | null>(null);
  const label = (t: number) => (days === 7 ? dateTime(new Date(t).toISOString()) : new Date(t).toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' }));
  function move(e: PointerEvent) {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    hover = Math.max(0, Math.min(pts.length - 1, Math.floor(((e.clientX - r.left) / r.width) * pts.length)));
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); hover = Math.max(0, Math.min(pts.length - 1, (hover ?? pts.length - 1) + (e.key === 'ArrowLeft' ? -1 : 1))); }
    else if (e.key === 'Escape') hover = null;
  }
  const hh = (c: number) => `${String(c).padStart(2, '0')}:00`;
  const summary = $derived(data ? `${data.unique_players} unique players, busiest ${data.peak.count} at once` : '');
</script>

<PlayerListSubs server={s} />

{#if running}
  <section class="card" style="margin-bottom:14px">
    <div class="card-title">Online now{live ? ` (${live.online})` : ''}</div>
    {#if liveErr}<p class="muted sm" style="margin:0">{liveErr}</p>
    {:else if !live}<States kind="loading" rows={2} />
    {:else if live.players.length === 0}<p class="muted sm" style="margin:0">{live.online > 0 ? `${live.online} connected; this game only reports a count.` : 'Nobody is online.'}</p>
    {:else}
      <ul class="plain" style="margin:0;padding:0;list-style:none">
        {#each live.players as p (p.name)}
          <li class="row between g8" style="padding:4px 0"><span>{p.name}{#if p.id} <span class="muted xs mono">{p.id}</span>{/if}</span>
            {#if canAct && live.can_kick}<span class="row g4"><button class="btn sm" onclick={() => act(p.name, false)}>Kick</button>{#if live.can_ban}<button class="btn sm danger" onclick={() => act(p.name, true)}>Ban</button>{/if}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<div class="row between wrap g12" style="margin-bottom:14px">
  <p class="muted sm" style="margin:0">Who played {s.name} and when, from the player list the server reports.</p>
  <div class="row g4" role="group" aria-label="Time range">
    {#each choices as c (c.id)}<button class="chip sm" aria-pressed={days === c.id} onclick={() => (days = c.id)}>{c.label}</button>{/each}
  </div>
</div>

{#if err}<States kind="error" text={err} retry={load} />
{:else if !data}<States kind="loading" rows={4} />
{:else if empty}
  <States kind="empty" icon="users" title="No player activity yet" text="Analytics are recorded while the server is running. Come back once players have joined." />
{:else}
  <div class="grid kpis">
    <div class="card kpi"><div class="muted sm">Unique players</div><div class="big tnum">{num(data.unique_players)}</div></div>
    <div class="card kpi"><div class="muted sm">Peak at once</div><div class="big tnum">{num(data.peak.count)}</div><div class="xs muted">{data.peak.at ? dateTime(data.peak.at) : '—'}</div></div>
    <div class="card kpi"><div class="muted sm">Total playtime</div><div class="big tnum">{duration(data.total_play_seconds)}</div></div>
    <div class="card kpi"><div class="muted sm">Average session</div><div class="big tnum">{data.sessions ? duration(data.avg_session_seconds) : '—'}</div><div class="xs muted">{num(data.sessions)} sessions</div></div>
  </div>

  <section class="card mt">
    <div class="card-title">Players online, {days === 7 ? 'by hour' : 'by day'}</div>
    <div class="rc">
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <svg viewBox="0 0 {Math.max(1, pts.length) * BW} {H}" preserveAspectRatio="none" role="img" tabindex="0" aria-label="Players online, {days === 7 ? 'hourly' : 'daily'} bars. {summary}" onpointermove={move} onpointerleave={() => (hover = null)} onkeydown={key} onblur={() => (hover = null)}>
        {#each [0.25, 0.5, 0.75] as g}<line x1="0" x2={pts.length * BW} y1={H * g} y2={H * g} stroke="var(--line)" vector-effect="non-scaling-stroke" />{/each}
        {#each pts as b, i}
          <rect x={i * BW + 0.6} y={H - (b.peak / top) * (H - 4)} width={BW - 1.2} height={(b.peak / top) * (H - 4)} fill="var(--a1)" opacity={hover === i ? 0.6 : 0.3} />
          <rect x={i * BW + 0.6} y={H - (b.avg / top) * (H - 4)} width={BW - 1.2} height={(b.avg / top) * (H - 4)} fill="var(--a1)" opacity={hover === i ? 1 : 0.85} />
        {/each}
      </svg>
      {#if hover !== null && pts[hover]}
        <div class="tip tnum" class:r={hover > pts.length * 0.6} style="left:{((hover + 0.5) / pts.length) * 100}%" role="status">
          <b>peak {pts[hover].peak}</b> · avg {pts[hover].avg.toFixed(1)}<span class="muted"> · {label(pts[hover].t)}</span>
        </div>
      {/if}
    </div>
    <p class="xs muted" style="margin:8px 0 0">Solid bars are the average, pale bars the peak. Gaps are times the server was off.</p>
  </section>

  <div class="grid cols-2 mt" style="align-items:start">
    <section class="card">
      <div class="card-title">Busiest times</div>
      <div class="heat" role="img" aria-label="Average players online by weekday and hour of day, in your local time">
        {#each heat.cells as row, r}
          <div class="hl xs muted">{weekdays[r]}</div>
          {#each row as v, c}
            <div class="hc" title="{weekdays[r]} {hh(c)}: {v === null ? 'no data' : v.toFixed(1) + ' players on average'}" style="background:{v === null ? 'var(--s2)' : `color-mix(in srgb, var(--a1) ${Math.round(8 + (heat.max ? v / heat.max : 0) * 80)}%, var(--s2))`}"></div>
          {/each}
        {/each}
        <div></div>{#each Array(24) as _, c}<div class="hx xs muted">{c % 6 === 0 ? c : ''}</div>{/each}
      </div>
      <p class="xs muted" style="margin:8px 0 0">Darker means more players on average. Hours are in your local time.</p>
    </section>

    <section class="card">
      <div class="card-title">Top players</div>
      {#if data.top_players.length}
        <div class="table-wrap">
          <table>
            <thead><tr><th>Player</th><th class="num">Playtime</th><th class="num">Sessions</th><th>Last seen</th></tr></thead>
            <tbody>
              {#each data.top_players as p (p.name)}
                <tr>
                  <td>{p.name}{#if p.online}<span class="badge ok" style="margin-left:8px">Online</span>{/if}</td>
                  <td class="num">{duration(p.play_seconds)}</td>
                  <td class="num">{num(p.sessions)}</td>
                  <td class="muted">{p.online ? 'now' : relTime(p.last_seen)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="muted sm">No named players recorded in this period.</p>
      {/if}
    </section>
  </div>

  {#if data.recent_sessions.length}
    <section class="card mt">
      <div class="card-title">Recent sessions</div>
      <div class="table-wrap">
        <table>
          <thead><tr><th>Player</th><th>Joined</th><th>Left</th><th class="num">Length</th></tr></thead>
          <tbody>
            {#each data.recent_sessions as r}
              <tr><td>{r.name}</td><td class="muted">{dateTime(r.joined_at)}</td><td class="muted">{r.left_at ? dateTime(r.left_at) : 'Still online'}</td><td class="num">{duration(r.seconds)}</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  {/if}

  <p class="xs muted mt-s">Player names come from the server's status reply, which for Minecraft lists at most 12 names at a time. With more players online, counts stay exact but some joins and leaves are not recorded. Servers that hide their player list only record counts.</p>
{/if}

<style>
  .kpis { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .kpi { padding: 14px 16px; } .big { font-size: calc(28px * var(--ts, 1)); font-weight: 650; letter-spacing: -.02em; line-height: 1.2; }
  @media (max-width: 760px) { .kpis { grid-template-columns: repeat(2, 1fr); } }
  .rc { position: relative; width: 100%; height: 120px; margin-top: 22px; }
  svg { width: 100%; height: 100%; display: block; overflow: visible; touch-action: pan-y; border-radius: 4px; }
  .tip { position: absolute; top: -28px; transform: translateX(-50%); background: var(--s3); border: 1px solid var(--line2); padding: 1px 8px; border-radius: 6px; font-size: calc(11px * var(--ts, 1)); white-space: nowrap; pointer-events: none; z-index: 2; }
  .tip.r { transform: translateX(-100%); }
  .heat { display: grid; grid-template-columns: 34px repeat(24, minmax(0, 1fr)); gap: 2px; align-items: center; }
  .hc { height: 18px; border-radius: 3px; } .hl { line-height: 18px; } .hx { text-align: left; line-height: 14px; }
</style>
