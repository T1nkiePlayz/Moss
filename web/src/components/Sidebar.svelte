<script lang="ts">
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { router } from '../lib/router.svelte';
  import { servers, prefs, setPref, activeOps } from '../lib/stores.svelte';
  import { isAdmin } from '../lib/auth.svelte';
  const collapsed = $derived(prefs.sidebar === 'icons');
  const nav = $derived([
    { href: '#/', name: 'overview', label: 'Overview', icon: 'home' },
    { href: '#/servers', name: 'servers', label: 'Servers', icon: 'server' },
    { href: '#/backups', name: 'backups', label: 'Backups', icon: 'archive' },
    { href: '#/activity', name: 'activity', label: 'Activity', icon: 'activity' },
    { href: '#/workflows', name: 'workflows', label: 'Workflows', icon: 'zap' },
    ...(isAdmin() ? [{ href: '#/users', name: 'users', label: 'Users', icon: 'users' }] : []),
    { href: '#/settings', name: 'settings', label: 'Settings', icon: 'settings' },
  ]);
  const cur = $derived(router.route.name === 'new' || router.route.name === 'server' ? 'servers' : router.route.name);
</script>

<aside class="side" class:collapsed aria-label="Sidebar">
  <a class="brand" href="#/" aria-label="Moss home"><Logo size={28} />{#if !collapsed}<span class="word">moss</span>{/if}</a>
  <nav aria-label="Primary">
    {#each nav as n}
      <a class="item" href={n.href} aria-current={cur === n.name ? 'page' : undefined} title={collapsed ? n.label : undefined}>
        <Icon name={n.icon} size={18} />{#if !collapsed}<span>{n.label}</span>{/if}
        {#if n.name === 'activity' && activeOps().length}<span class="pip">{activeOps().length}</span>{/if}
      </a>
    {/each}
  </nav>
  {#if servers.list.length}
    <div class="sect">{#if !collapsed}<span>Servers</span><a href="#/servers/new" class="add" aria-label="New server"><Icon name="plus" size={14} /></a>{/if}</div>
    <nav class="srv" aria-label="Servers">
      {#each servers.list.slice(0, 10) as s (s.id)}
        <a class="item sm" href="#/servers/{s.id}" title={s.name} aria-current={router.route.name === 'server' && router.route.params.id === s.id ? 'page' : undefined}>
          <span class="dot {s.state}" aria-hidden="true"></span>
          {#if !collapsed}<span class="ellipsis">{s.name}</span>{/if}
          <span class="sr">{s.state}</span>
        </a>
      {/each}
    </nav>
  {/if}
  <div class="grow"></div>
  <button class="item toggle" onclick={() => setPref('sidebar', collapsed ? 'full' : 'icons')} aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}>
    <Icon name="panel" size={18} />{#if !collapsed}<span>Collapse</span>{/if}
  </button>
</aside>

<style>
  .side { position: sticky; top: 0; height: 100vh; width: var(--side); flex: none; display: flex; flex-direction: column; gap: 4px; padding: 14px 10px; background: linear-gradient(180deg, var(--s1), var(--bg)); border-right: 1px solid var(--line); transition: width .18s var(--ease); overflow: hidden auto; }
  .side.collapsed { width: 64px; align-items: stretch; }
  .brand { display: flex; align-items: center; gap: 10px; padding: 4px 8px 14px; color: var(--text); text-decoration: none !important; }
  .collapsed .brand { justify-content: center; padding-inline: 0; }
  .word { font-size: 20px; font-weight: 700; letter-spacing: -.03em; }
  nav { display: flex; flex-direction: column; gap: 2px; }
  .item { position: relative; display: flex; align-items: center; gap: 12px; height: 36px; padding: 0 10px; border-radius: 10px; color: var(--muted); font-weight: 550; font-size: var(--fs-sm); text-decoration: none !important; background: none; border: 0; cursor: pointer; width: 100%; text-align: left; transition: background var(--t), color var(--t); }
  .collapsed .item { justify-content: center; padding: 0; }
  .item:hover { background: var(--hl); color: var(--text); }
  .item[aria-current='page'] { color: var(--text); background: var(--a-soft); }
  .item[aria-current='page']::before { content: ''; position: absolute; left: -10px; top: 8px; bottom: 8px; width: 3px; border-radius: 0 3px 3px 0; background: linear-gradient(var(--a1), var(--a2)); box-shadow: 0 0 12px var(--a-glow); }
  .item[aria-current='page'] :global(svg) { color: var(--a-text); }
  .item.sm { height: 30px; font-weight: 500; }
  .pip { margin-left: auto; min-width: 18px; height: 18px; border-radius: 9px; background: var(--a1); color: var(--a-ink); font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .collapsed .pip { position: absolute; top: 2px; right: 6px; margin: 0; }
  .sect { display: flex; align-items: center; justify-content: space-between; padding: 16px 12px 4px; font-size: 11px; text-transform: uppercase; letter-spacing: .09em; color: var(--faint); font-weight: 600; min-height: 34px; }
  .add { color: var(--muted); display: grid; } .add:hover { color: var(--a-text); }
  .toggle { margin-top: 8px; }
  @media (max-width: 860px) { .side { display: none; } }
</style>
