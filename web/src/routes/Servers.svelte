<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import ServerCard from '../components/ServerCard.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import States from '../components/States.svelte';
  import SavedViews from '../components/SavedViews.svelte';
  import { servers, prefs, setPref, refreshServers } from '../lib/stores.svelte';
  import { power } from '../lib/actions';
  import { duration, pct, bytes } from '../lib/format';
  import { serverHref } from '../lib/route';
  import type { Server, ServerState } from '../lib/api';

  let q = $state('');
  let filter = $state<'all' | 'running' | 'stopped' | 'attention'>('all');
  const groups: Record<string, ServerState[]> = { running: ['running', 'starting'], stopped: ['stopped', 'stopping'], attention: ['crashed', 'failed', 'needs_attention', 'installing'] };
  const list = $derived(servers.list.filter((s) => {
    if (filter !== 'all' && !groups[filter].includes(s.state)) return false;
    const t = q.trim().toLowerCase();
    return !t || [s.name, s.runtime, s.game_version, ...s.tags].some((x) => x.toLowerCase().includes(t));
  }));
  const count = (f: string) => (f === 'all' ? servers.list.length : servers.list.filter((s) => groups[f].includes(s.state)).length);
  async function toggle(s: Server) { await power(s, s.state === 'running' || s.state === 'starting' ? 'stop' : 'start'); }
</script>

<div class="page-head">
  <div><h1>Servers</h1><p>{servers.list.length} total · {count('running')} running</p></div>
  <a class="btn primary" href="#/servers/new"><Icon name="plus" size={16} />New server</a>
</div>

<div class="row wrap" style="margin-bottom:10px">
  <div class="grow" style="max-width:340px;position:relative;min-width:200px">
    <input class="input" style="padding-left:34px" type="search" placeholder="Search servers…" aria-label="Search servers" bind:value={q} />
    <span class="ico"><Icon name="search" size={15} /></span>
  </div>
  <div class="row g6 wrap" role="group" aria-label="Filter by status">
    {#each [['all', 'All'], ['running', 'Running'], ['stopped', 'Stopped'], ['attention', 'Attention']] as [k, l]}
      <button class="chip" aria-pressed={filter === k} onclick={() => (filter = k as typeof filter)}>{l}<span class="tnum faint">{count(k)}</span></button>
    {/each}
  </div>
  <div class="grow"></div>
  <div class="row g4" role="group" aria-label="View">
    <button class="btn icon sm {prefs.serversView === 'cards' ? '' : 'ghost'}" aria-pressed={prefs.serversView === 'cards'} aria-label="Card view" onclick={() => setPref('serversView', 'cards')}><Icon name="grid" size={15} /></button>
    <button class="btn icon sm {prefs.serversView === 'table' ? '' : 'ghost'}" aria-pressed={prefs.serversView === 'table'} aria-label="Table view" onclick={() => setPref('serversView', 'table')}><Icon name="list" size={15} /></button>
  </div>
</div>

<div style="margin-bottom:16px"><SavedViews scope="servers" filters={{ q, filter }} onapply={(f) => { q = typeof f.q === 'string' ? f.q : ''; filter = (['all', 'running', 'stopped', 'attention'].includes(f.filter as string) ? f.filter : 'all') as typeof filter; }} /></div>

{#if !servers.loaded}
  <div class="grid cols-auto">{#each [1, 2, 3, 4] as _}<div class="skel" style="height:190px"></div>{/each}</div>
{:else if servers.error && !servers.list.length}
  <States kind="error" text={servers.error} retry={refreshServers} />
{:else if !servers.list.length}
  <States kind="empty" icon="server" title="No servers yet" text="Pick a runtime, a version and go."><a class="btn primary" href="#/servers/new"><Icon name="plus" size={16} />Create server</a></States>
{:else if !list.length}
  <States kind="empty" icon="search" title="No matching servers" text="Try a different search or filter."><button class="btn" onclick={() => { q = ''; filter = 'all'; }}>Clear filters</button></States>
{:else if prefs.serversView === 'cards'}
  <div class="grid cols-auto">{#each list as s (s.id)}<ServerCard server={s} />{/each}</div>
{:else}
  <div class="table-wrap">
    <table>
      <thead><tr><th>Name</th><th>Status</th><th>Runtime</th><th class="num">Players</th><th class="num">CPU</th><th class="num">Memory</th><th class="num">Uptime</th><th>Tags</th><th></th></tr></thead>
      <tbody>
        {#each list as s (s.id)}
          <tr>
            <td><a href={serverHref(s.id)} onclick={(e) => e.stopPropagation()}><b>{s.name}</b></a></td>
            <td><StatusBadge state={s.state} detail={s.state_detail} /></td>
            <td class="muted">{s.runtime} {s.game_version}</td>
            <td class="num">{s.players ? `${s.players.online}/${s.players.max}` : '—'}</td>
            <td class="num">{pct(s.cpu_pct)}</td>
            <td class="num">{bytes(s.mem_bytes)}</td>
            <td class="num">{duration(s.uptime_s)}</td>
            <td>{#each s.tags as t}<span class="tag" style="margin-right:4px">{t}</span>{/each}</td>
            <td class="num"><button class="btn sm" aria-label="{s.state === 'running' ? 'Stop' : 'Start'} {s.name}" onclick={(e) => { e.stopPropagation(); void toggle(s); }}><Icon name={s.state === 'running' || s.state === 'starting' ? 'stop' : 'play'} size={12} />{s.state === 'running' || s.state === 'starting' ? 'Stop' : 'Start'}</button></td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .ico { position: absolute; left: 11px; top: 10px; color: var(--faint); pointer-events: none; display: grid; }
</style>
