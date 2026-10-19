<script lang="ts">
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { router, navigate } from '../lib/router.svelte';
  import { serverById, ui, activeOps, prefs, setPref, cycleTheme } from '../lib/stores.svelte';
  import { auth, signOut } from '../lib/auth.svelte';
  import { initials } from '../lib/format';

  const crumbs = $derived.by(() => {
    const r = router.route;
    const c: { label: string; href?: string }[] = [];
    const names: Record<string, string> = { overview: 'Overview', servers: 'Servers', backups: 'Backups', activity: 'Activity', users: 'Users', settings: 'Settings', notfound: 'Not found' };
    if (r.name === 'new') c.push({ label: 'Servers', href: '#/servers' }, { label: 'New server' });
    else if (r.name === 'server') {
      c.push({ label: 'Servers', href: '#/servers' });
      const s = serverById(r.params.id);
      c.push(r.params.tab === 'overview' ? { label: s?.name ?? '…' } : { label: s?.name ?? '…', href: `#/servers/${r.params.id}` });
      if (r.params.tab !== 'overview') c.push({ label: r.params.tab[0].toUpperCase() + r.params.tab.slice(1) });
    } else c.push({ label: names[r.name] ?? '' });
    return c;
  });
  const n = $derived(activeOps().length);
  const dark = $derived(document.documentElement.dataset.theme !== 'light');
  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
  let menu: HTMLDivElement | undefined = $state();
  $effect(() => {
    if (!ui.userMenu) return;
    const off = (e: MouseEvent) => { if (menu && !menu.contains(e.target as Node)) ui.userMenu = false; };
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && (ui.userMenu = false);
    document.addEventListener('mousedown', off);
    document.addEventListener('keydown', esc);
    return () => { document.removeEventListener('mousedown', off); document.removeEventListener('keydown', esc); };
  });
</script>

<header class="top">
  <a class="mlogo" href="#/" aria-label="Moss home"><Logo size={26} /></a>
  <nav class="crumbs" aria-label="Breadcrumb">
    <ol>
      {#each crumbs as c, i}
        <li class:last={i === crumbs.length - 1}>
          {#if i > 0}<Icon name="chevron-right" size={13} />{/if}
          {#if c.href}<a href={c.href}>{c.label}</a>{:else}<span aria-current={i === crumbs.length - 1 ? 'page' : undefined}>{c.label}</span>{/if}
        </li>
      {/each}
    </ol>
  </nav>
  <button class="search" onclick={() => (ui.palette = true)} aria-label="Open command palette">
    <Icon name="search" size={15} /><span class="ph">Search or run a command…</span><span class="keys"><kbd>{mac ? '⌘' : 'Ctrl'}</kbd><kbd>K</kbd></span>
  </button>
  <button class="btn ghost icon msearch" onclick={() => (ui.palette = true)} aria-label="Search"><Icon name="search" /></button>
  <button class="btn ghost icon ops" onclick={() => (ui.ops = true)} aria-label="Operations{n ? `, ${n} running` : ''}">
    <Icon name="layers" />{#if n}<span class="badge-n" aria-hidden="true">{n}</span>{/if}
  </button>
  <button class="btn ghost icon" onclick={cycleTheme} aria-label="Toggle theme">
    <Icon name={dark ? 'sun' : 'moon'} />
  </button>
  <div class="um" bind:this={menu}>
    <button class="av" onclick={() => (ui.userMenu = !ui.userMenu)} aria-haspopup="menu" aria-expanded={ui.userMenu} aria-label="User menu">{initials(auth.email || 'dev')}</button>
    {#if ui.userMenu}
      <div class="pop" role="menu">
        <div class="who"><div class="ellipsis"><b>{auth.email || 'Local session'}</b></div><div class="xs muted">{auth.me?.role ?? 'owner'}{auth.config?.mode === 'dev' ? ' · dev mode' : ''}</div></div>
        <button role="menuitem" class="mi" onclick={() => { ui.userMenu = false; navigate('/settings'); }}><Icon name="settings" size={16} />Settings</button>
        <button role="menuitem" class="mi" onclick={() => { setPref('sidebar', prefs.sidebar === 'full' ? 'icons' : 'full'); ui.userMenu = false; }}><Icon name="panel" size={16} />Toggle sidebar</button>
        {#if auth.config?.mode === 'supabase'}<button role="menuitem" class="mi" onclick={() => { ui.userMenu = false; signOut(); }}><Icon name="logout" size={16} />Sign out</button>{/if}
      </div>
    {/if}
  </div>
</header>

<style>
  .top { position: sticky; top: 0; z-index: 30; height: var(--top); display: flex; align-items: center; gap: 10px; padding: 0 20px; background: color-mix(in srgb, var(--bg), transparent 8%); border-bottom: 1px solid var(--line); }
  .mlogo { display: none; }
  ol { list-style: none; display: flex; align-items: center; gap: 4px; margin: 0; padding: 0; font-size: var(--fs-sm); color: var(--muted); min-width: 0; }
  li { display: flex; align-items: center; gap: 4px; min-width: 0; } li.last { color: var(--text); font-weight: 600; min-width: 0; }
  li span, li a { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .crumbs { min-width: 0; flex: 1 1 auto; }
  .search { display: flex; align-items: center; gap: 8px; height: 34px; margin-left: 16px; padding: 0 8px 0 12px; width: 340px; flex: 0 1 340px; min-width: 150px; background: var(--s2); border: 1px solid var(--line2); border-radius: 10px; color: var(--muted); cursor: pointer; transition: border-color var(--t); }
  .search:hover { border-color: var(--faint); }
  .ph { flex: 1; text-align: left; font-size: var(--fs-sm); overflow: hidden; white-space: nowrap; text-overflow: ellipsis; } .keys { display: flex; gap: 3px; }
  .msearch { display: none; }
  .ops { position: relative; }
  .badge-n { position: absolute; top: 1px; right: 1px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--a1); color: var(--a-ink); font-size: 10px; font-weight: 700; display: grid; place-items: center; box-shadow: 0 0 0 2px var(--bg); animation: pulse 2.2s infinite; }
  .um { position: relative; }
  .av { width: 32px; height: 32px; border-radius: 50%; border: 1px solid var(--line2); background: linear-gradient(135deg, var(--s3), var(--s2)); color: var(--a-text); font-weight: 700; font-size: 12px; cursor: pointer; }
  .pop { position: absolute; right: 0; top: 40px; width: 240px; padding: 6px; background: var(--s2); border: 1px solid var(--line2); border-radius: var(--r); box-shadow: 0 16px 40px rgb(0 0 0 / .45); animation: rise .14s var(--ease); z-index: 5; }
  .who { padding: 8px 10px 10px; border-bottom: 1px solid var(--line); margin-bottom: 4px; }
  .mi { display: flex; align-items: center; gap: 10px; width: 100%; height: 34px; padding: 0 10px; border: 0; background: none; border-radius: 8px; color: var(--text); cursor: pointer; font-size: var(--fs-sm); }
  .mi:hover { background: var(--hl); }
  @media (max-width: 860px) {
    .top { padding: 0 12px; gap: 6px; } .mlogo { display: grid; } .search { display: none; } .msearch { display: inline-flex; }
    .crumbs li:not(.last) { display: none; } li.last :global(svg) { display: none; }
  }
</style>
