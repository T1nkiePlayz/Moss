<script lang="ts">
  import Icon from './Icon.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import PulseStrip from './PulseStrip.svelte';
  import { api, type Server, type TimelineEvent } from '../lib/api';
  import { duration, pct, bytes, runtimeLabel } from '../lib/format';
  import { power } from '../lib/actions';
  import { serverHref } from '../lib/route';
  let { server: s }: { server: Server } = $props();
  let events = $state<TimelineEvent[] | null>(null);
  $effect(() => {
    const id = s.id;
    api.timeline(id).then((e) => (events = e)).catch(() => (events = []));
  });
  let busy = $state(false);
  const active = $derived(s.state === 'running' || s.state === 'starting');
  async function toggle(e: Event) {
    e.preventDefault(); e.stopPropagation();
    busy = true;
    await power(s, active ? 'stop' : 'start');
    busy = false;
  }
</script>

<a class="card hover sc" href={serverHref(s.id)} class:crashed={s.state === 'crashed' || s.state === 'failed'}>
  <div class="row between" style="align-items:flex-start">
    <div class="grow">
      <h3 class="ellipsis">{s.name}</h3>
      <div class="xs muted ellipsis">{runtimeLabel(s)} · :{s.port}</div>
    </div>
    <StatusBadge state={s.state} detail={s.state_detail} />
  </div>
  <div class="stats tnum sm">
    <div title="Players"><Icon name="users" size={14} /><span>{s.players ? (s.players.max ? `${s.players.online}/${s.players.max}` : `${s.players.online}`) : '—'}</span></div>
    <div title="Uptime"><Icon name="clock" size={14} /><span>{duration(s.uptime_s)}</span></div>
    <div title="CPU"><Icon name="cpu" size={14} /><span>{pct(s.cpu_pct)}</span></div>
    <div title="Memory"><Icon name="ram" size={14} /><span>{bytes(s.mem_bytes, 1)}</span></div>
  </div>
  <PulseStrip {events} running={s.state === 'running'} compact />
  <div class="row between">
    <div class="row g4 wrap">{#each s.tags.slice(0, 3) as t}<span class="tag">{t}</span>{/each}</div>
    <button class="btn sm {active ? '' : 'primary'}" disabled={busy || s.state === 'installing' || s.state === 'stopping'} onclick={toggle} aria-label="{active ? 'Stop' : 'Start'} {s.name}">
      <Icon name={active ? 'stop' : 'play'} size={13} />{active ? 'Stop' : 'Start'}
    </button>
  </div>
</a>

<style>
  .sc { display: flex; flex-direction: column; gap: 14px; color: inherit; text-decoration: none !important; }
  .sc.crashed { border-color: color-mix(in srgb, var(--danger), transparent 60%); }
  .stats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 6px; }
  .stats div { display: flex; align-items: center; gap: 5px; color: var(--muted); min-width: 0; }
  .stats span { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 420px) { .stats { grid-template-columns: repeat(2, 1fr); } }
</style>
