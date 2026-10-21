<script lang="ts">
  import Sidebar from './components/Sidebar.svelte';
  import Topbar from './components/Topbar.svelte';
  import Icon from './components/Icon.svelte';
  import Lazy from './components/Lazy.svelte';
  import OpsDrawer from './components/OpsDrawer.svelte';
  import States from './components/States.svelte';
  import Overview from './routes/Overview.svelte';
  import { isAdmin } from './lib/auth.svelte';
  import { router } from './lib/router.svelte';
  import { startLive, stopLive, poll, refreshHost, refreshHealth, ui, serverById } from './lib/stores.svelte';

  const routes = {
    servers: () => import('./routes/Servers.svelte'),
    new: () => import('./routes/NewServer.svelte'),
    import: () => import('./routes/ImportServer.svelte'),
    server: () => import('./routes/Server.svelte'),
    backups: () => import('./routes/AllBackups.svelte'),
    activity: () => import('./routes/Activity.svelte'),
    workflows: () => import('./routes/Workflows.svelte'),
    users: () => import('./routes/Users.svelte'),
    settings: () => import('./routes/Settings.svelte'),
    notfound: () => import('./routes/NotFound.svelte'),
  } as const;

  $effect(() => {
    startLive();
    const stops = [poll(refreshHost, 5000), poll(refreshHealth, 30000)];
    return () => { stops.forEach((s) => s()); stopLive(); };
  });

  $effect(() => {
    const r = router.route;
    const base = r.name === 'server' ? serverById(r.params.id)?.name ?? 'Server' : r.name === 'overview' ? 'Overview' : r.name === 'new' ? 'New server' : r.name[0].toUpperCase() + r.name.slice(1);
    document.title = `${base} · Moss`;
  });

  function keys(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); ui.palette = !ui.palette; }
  }

  const tabs = [
    { href: '#/', name: 'overview', label: 'Overview', icon: 'home' },
    { href: '#/servers', name: 'servers', label: 'Servers', icon: 'server' },
    { href: '#/backups', name: 'backups', label: 'Backups', icon: 'archive' },
    { href: '#/activity', name: 'activity', label: 'Activity', icon: 'activity' },
    { href: '#/settings', name: 'settings', label: 'Settings', icon: 'settings' },
  ];
  const cur = $derived(router.route.name === 'new' || router.route.name === 'import' || router.route.name === 'server' ? 'servers' : router.route.name);
  const guard = $derived(router.route.name === 'users' && !isAdmin());
</script>

<svelte:window onkeydown={keys} />

  <a href="#main" class="skip" onclick={(e) => { e.preventDefault(); document.getElementById('main')?.focus(); }}>Skip to content</a>
  <div class="shell">
    <Sidebar />
    <div class="col-main">
      <Topbar />
      <main id="main" tabindex="-1">
        <div class="page">
          {#if guard}
            <States kind="empty" icon="lock" title="Admins only" text="You don't have permission to view this page." />
          {:else if router.route.name === 'overview'}
            <Overview />
          {:else}
            {#key router.route.name === 'server' ? 'server' : router.route.name}
              <Lazy load={routes[router.route.name as keyof typeof routes]} props={{ route: router.route }} />
            {/key}
          {/if}
        </div>
      </main>
    </div>
    <nav class="tabbar" aria-label="Primary mobile">
      {#each tabs as t}
        <a href={t.href} aria-current={cur === t.name ? 'page' : undefined}><Icon name={t.icon} size={20} /><span>{t.label}</span></a>
      {/each}
    </nav>
  </div>
<OpsDrawer />
{#if ui.palette}{#await import('./components/Palette.svelte') then m}{@const P = m.default}<P />{/await}{/if}

<style>
  .shell { display: flex; min-height: 100vh; }
  .col-main { flex: 1; min-width: 0; display: flex; flex-direction: column; overflow-x: clip; }
  main { flex: 1; outline: none; }
  .page { max-width: 1280px; margin: 0 auto; padding: 24px 24px 48px; width: 100%; }
  .skip { position: fixed; left: 12px; top: -60px; z-index: 200; background: var(--a1); color: var(--a-ink); padding: 8px 14px; border-radius: 8px; font-weight: 600; transition: top .15s; }
  .skip:focus { top: 12px; }
  .tabbar { display: none; }
  @media (max-width: 860px) {
    .page { padding: 16px 14px 96px; }
    .tabbar { position: fixed; z-index: 40; left: 0; right: 0; bottom: 0; display: grid; grid-template-columns: repeat(5, 1fr); padding: 6px 4px calc(6px + env(safe-area-inset-bottom)); background: color-mix(in srgb, var(--s1), transparent 4%); border-top: 1px solid var(--line2); }
    .tabbar a { display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 6px 0; font-size: 10.5px; font-weight: 600; color: var(--muted); text-decoration: none; border-radius: 10px; }
    .tabbar a[aria-current='page'] { color: var(--a-text); }
    .tabbar a[aria-current='page'] :global(svg) { filter: drop-shadow(0 0 6px var(--a-glow)); }
  }
</style>
