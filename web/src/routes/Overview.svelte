<script lang="ts">
  import Gauge from '../components/Gauge.svelte';
  import Icon from '../components/Icon.svelte';
  import ServerCard from '../components/ServerCard.svelte';
  import HealthCards from '../components/HealthCards.svelte';
  import OpCard from '../components/OpCard.svelte';
  import States from '../components/States.svelte';
  import { host, servers, health, ops, refreshServers, refreshHealth } from '../lib/stores.svelte';
  import { bytes, duration, num } from '../lib/format';
  const st = $derived(host.status);
  const diskUsed = $derived(st ? (1 - st.host.disk_free / st.host.disk_total) * 100 : 0);
  const recent = $derived(ops.list.slice(0, 4));
  const crit = $derived(health.list.filter((f) => f.severity !== 'info').length);
</script>

<div class="page-head">
  <div>
    <h1>Overview</h1>
    <p>{#if st}{st.host.hostname} · up {duration(st.uptime_s)} · Moss {st.version}{:else}Loading host…{/if}</p>
  </div>
  <a class="btn primary" href="#/servers/new"><Icon name="plus" size={16} />New server</a>
</div>

<section class="card" aria-label="Host resources">
  <div class="card-title"><Icon name="cpu" size={14} />Host</div>
  {#if !st && host.error}
    <States kind="error" text={host.error} retry={() => location.reload()} />
  {:else if !st}
    <div class="grid cols-3">{#each [1, 2, 3] as _}<div class="skel" style="height:92px"></div>{/each}</div>
  {:else}
    <div class="grid cols-3 g">
      <Gauge value={st.host.cpu_pct} label="CPU" icon="cpu" sub="Load {st.host.load1.toFixed(2)}" history={host.cpu} />
      <Gauge value={(st.host.mem_used / st.host.mem_total) * 100} label="Memory" icon="ram" sub="{bytes(st.host.mem_used)} / {bytes(st.host.mem_total)}" history={host.mem} />
      <Gauge value={diskUsed} label="Disk" icon="disk" sub="{bytes(st.host.disk_free)} free of {bytes(st.host.disk_total)}" warnAt={80} dangerAt={92} />
    </div>
  {/if}
</section>

<div class="grid kpis mt">
  <div class="card kpi"><div class="muted sm">Servers</div><div class="big tnum">{st ? num(st.counts.servers) : '—'}</div></div>
  <div class="card kpi"><div class="muted sm">Running</div><div class="big tnum" style="color:var(--a-text)">{st ? num(st.counts.running) : '—'}</div></div>
  <div class="card kpi"><div class="muted sm">Players online</div><div class="big tnum">{st ? num(st.counts.players) : '—'}</div></div>
  <div class="card kpi"><div class="muted sm">Needs attention</div><div class="big tnum" style="color:{crit ? 'var(--warn)' : 'inherit'}">{health.loaded ? crit : '—'}</div></div>
</div>

<div class="row between mt" style="margin-top:28px;margin-bottom:12px"><h2>Servers</h2><a class="sm" href="#/servers">View all</a></div>
{#if !servers.loaded}
  <div class="grid cols-auto">{#each [1, 2, 3] as _}<div class="skel" style="height:190px"></div>{/each}</div>
{:else if servers.error && !servers.list.length}
  <States kind="error" text={servers.error} retry={refreshServers} />
{:else if !servers.list.length}
  <States kind="empty" icon="server" title="No servers yet" text="Create your first Minecraft server in a few clicks."><a class="btn primary" href="#/servers/new"><Icon name="plus" size={16} />Create server</a></States>
{:else}
  <div class="grid cols-auto">{#each servers.list.slice(0, 6) as s (s.id)}<ServerCard server={s} />{/each}</div>
{/if}

<div class="grid cols-2" style="margin-top:28px;align-items:start">
  <section aria-label="Health">
    <div class="row between" style="margin-bottom:12px"><h2><Icon name="heart" size={16} /> Health</h2><button class="btn ghost sm" onclick={refreshHealth}><Icon name="refresh" size={14} />Re-check</button></div>
    <HealthCards findings={health.list} loaded={health.loaded} error={health.error} retry={refreshHealth} />
  </section>
  <section aria-label="Recent operations">
    <div class="row between" style="margin-bottom:12px"><h2><Icon name="layers" size={16} /> Recent operations</h2><a class="sm" href="#/activity">Activity</a></div>
    {#if !ops.loaded}<States kind="loading" rows={2} />
    {:else if !recent.length}<States kind="empty" icon="layers" title="Quiet so far" text="Installs, backups and restores will appear here." />
    {:else}<div class="col">{#each recent as o (o.id)}<OpCard op={o} />{/each}</div>{/if}
  </section>
</div>

<style>
  h2 { display: flex; align-items: center; gap: 8px; }
  .g { gap: 24px; }
  .kpis { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .kpi { padding: 14px 16px; text-align: left; } .big { font-size: 28px; font-weight: 650; letter-spacing: -.02em; line-height: 1.2; }
  @media (max-width: 760px) { .kpis { grid-template-columns: repeat(2, 1fr); } }
</style>
