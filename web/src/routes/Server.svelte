<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import States from '../components/States.svelte';
  import Lazy from '../components/Lazy.svelte';
  import Overview from './server/Overview.svelte';
  import { api, errMsg, type Server } from '../lib/api';
  import { servers, serverById } from '../lib/stores.svelte';
  import { power } from '../lib/actions';
  import { isAdmin } from '../lib/auth.svelte';
  import { serverHref, type Route } from '../lib/route';

  let { route }: { route: Route } = $props();
  const id = $derived(route.params.id);
  const tab = $derived(route.params.tab);
  const s = $derived(serverById(id));

  // Deep link before the list arrives (or a server outside the list): fetch directly.
  let fetched = $state<{ id: string; server?: Server; err?: string } | null>(null);
  $effect(() => {
    const want = id;
    if (serverById(want) || !servers.loaded) return;
    api.server(want).then((server) => (fetched = { id: want, server })).catch((e) => (fetched = { id: want, err: errMsg(e) }));
  });
  const missing = $derived(!s && servers.loaded && fetched?.id === id ? fetched.err ?? 'Server not found' : '');

  const kindName: Record<string, string> = { plugins: 'Plugins', mods: 'Mods', proxy_plugins: 'Proxy plugins', datapacks: 'Datapacks' };
  const tabs = $derived.by(() => {
    if (!s) return [];
    const c = s.capabilities;
    const t: { id: string; label: string; icon: string }[] = [{ id: 'overview', label: 'Overview', icon: 'home' }];
    if (c.includes('console')) t.push({ id: 'console', label: 'Console', icon: 'terminal' });
    if (c.includes('files')) t.push({ id: 'files', label: 'Files', icon: 'folder' });
    if (c.includes('properties')) t.push({ id: 'config', label: 'Config', icon: 'sliders' });
    if (c.includes('content') && s.content_kind !== 'none') t.push({ id: 'content', label: kindName[s.content_kind] ?? 'Content', icon: 'puzzle' });
    if (s.runtime !== 'custom' || s.content_kind !== 'none') t.push({ id: 'upgrade', label: 'Upgrade', icon: 'arrow-up' });
    if (c.includes('backups')) t.push({ id: 'backups', label: 'Backups', icon: 'archive' });
    if (isAdmin()) t.push({ id: 'access', label: 'Access', icon: 'shield' });
    t.push({ id: 'settings', label: 'Settings', icon: 'settings' });
    return t;
  });
  const loaders = {
    console: () => import('./server/Console.svelte'),
    files: () => import('./server/Files.svelte'),
    config: () => import('./server/Config.svelte'),
    content: () => import('./server/Content.svelte'),
    upgrade: () => import('./server/Upgrade.svelte'),
    backups: () => import('./server/Backups.svelte'),
    access: () => import('./server/Access.svelte'),
    settings: () => import('./server/Settings.svelte'),
  } as const;
  const on = $derived(!!s && (s.state === 'running' || s.state === 'starting'));
  
</script>

{#if missing}
  <States kind="error" title="Server unavailable" text={missing} retry={() => (fetched = null)}><a class="btn" href="#/servers">All servers</a></States>
{:else if !s}
  <States kind="loading" rows={4} />
{:else}
  <div class="page-head" style="padding-bottom:12px">
    <div class="row g16" style="min-width:0">
      <div style="min-width:0">
        <div class="row wrap g6"><h1 class="ellipsis">{s.name}</h1><StatusBadge state={s.state} detail={s.state_detail} /></div>
        <p class="ellipsis">{s.runtime} {s.game_version}{s.loader_version ? ` · loader ${s.loader_version}` : ''} · Java {s.java_major} · {s.memory_mb} MB</p>
      </div>
    </div>
    <div class="row g6 wrap">
      {#if on}
        <button class="btn" onclick={() => power(s, 'restart')}><Icon name="refresh" size={15} />Restart</button>
        <button class="btn danger" onclick={() => power(s, 'stop')}><Icon name="stop" size={14} />Stop</button>
      {:else}
        <button class="btn primary" disabled={s.state === 'installing' || s.state === 'stopping'} onclick={() => power(s, 'start')}><Icon name="play" size={14} />Start</button>
      {/if}
    </div>
  </div>

  <div class="tabs" role="tablist" aria-label="Server sections">
    {#each tabs as t}
      <a class="tab" role="tab" href={serverHref(s.id, t.id)} aria-selected={tab === t.id}><Icon name={t.icon} size={15} />{t.label}</a>
    {/each}
  </div>

  <div role="tabpanel" aria-label={tab}>
    {#if tab === 'overview'}
      <Overview server={s} />
    {:else if tabs.some((t) => t.id === tab)}
      {#key tab + s.id}
        <Lazy load={loaders[tab as keyof typeof loaders]} props={{ server: s }} />
      {/key}
    {:else}
      <States kind="empty" icon="lock" title="Not available" text="This server doesn't support that section, or you don't have access to it."><a class="btn" href={serverHref(s.id)}>Back to overview</a></States>
    {/if}
  </div>
{/if}
