<script lang="ts">
  import Icon from './Icon.svelte';
  import { servers, ui, prefs, setPref } from '../lib/stores.svelte';
  import { navigate } from '../lib/router.svelte';
  import { serverHref } from '../lib/route';
  import { fuzzy } from '../lib/fuzzy';
  import { power, backupNow, consequence } from '../lib/actions';
  import { can } from '../lib/auth.svelte';
  import { api } from '../lib/api';
  import { relTime, runtimeLabel } from '../lib/format';

  interface Item { id: string; title: string; hint: string; icon: string; group: string; run: () => void | Promise<void>; kw?: string }
  let q = $state('');
  let sel = $state(0);
  let input: HTMLInputElement;
  let list: HTMLElement;
  const prev = document.activeElement as HTMLElement | null;

  const items = $derived.by(() => {
    const out: Item[] = [
      { id: 'p-home', title: 'Overview', hint: 'Go to the dashboard', icon: 'home', group: 'Pages', run: () => navigate('/') },
      { id: 'p-servers', title: 'Servers', hint: 'Go to all servers', icon: 'server', group: 'Pages', run: () => navigate('/servers') },
      { id: 'p-new', title: 'New server', hint: 'Open the create-server wizard', icon: 'plus', group: 'Pages', run: () => navigate('/servers/new') },
      { id: 'p-backups', title: 'Backups', hint: 'All backups across servers', icon: 'archive', group: 'Pages', run: () => navigate('/backups') },
      { id: 'p-activity', title: 'Activity', hint: 'Operations and audit log', icon: 'activity', group: 'Pages', run: () => navigate('/activity') },
      { id: 'p-users', title: 'Users', hint: 'Invite and manage users', icon: 'users', group: 'Pages', run: () => navigate('/users') },
      { id: 'p-settings', title: 'Settings', hint: 'Appearance, instance, Java', icon: 'settings', group: 'Pages', run: () => navigate('/settings') },
      { id: 'a-theme', title: 'Toggle light / dark theme', hint: 'Switch appearance', icon: 'sun', group: 'Actions', kw: 'theme dark light', run: () => setPref('theme', document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark') },
      { id: 'a-density', title: prefs.density === 'compact' ? 'Use comfortable density' : 'Use compact density', hint: 'Change spacing', icon: 'list', group: 'Actions', run: () => setPref('density', prefs.density === 'compact' ? 'comfortable' : 'compact') },
    ];
    for (const s of servers.list) {
      const g = s.name;
      out.push({ id: `s-${s.id}`, title: s.name, hint: `${runtimeLabel(s)} · ${s.state}`, icon: 'cube', group: 'Servers', run: () => navigate(serverHref(s.id)) });
      const on = s.state === 'running' || s.state === 'starting';
      if (can('server.power')) {
        if (!on) out.push({ id: `a-start-${s.id}`, title: `Start ${g}`, hint: consequence.start, icon: 'play', group: 'Actions', kw: 'start run boot', run: () => void power(s, 'start') });
        else {
          out.push({ id: `a-stop-${s.id}`, title: `Stop ${g}`, hint: consequence.stop, icon: 'stop', group: 'Actions', kw: 'stop halt', run: () => void power(s, 'stop') });
          out.push({ id: `a-restart-${s.id}`, title: `Restart ${g}`, hint: consequence.restart, icon: 'refresh', group: 'Actions', kw: 'restart reboot', run: () => void power(s, 'restart') });
        }
      }
      if (s.capabilities.includes('console')) out.push({ id: `a-console-${s.id}`, title: `Open console: ${g}`, hint: 'Live log and commands', icon: 'terminal', group: 'Actions', kw: 'console logs', run: () => navigate(serverHref(s.id, 'console')) });
      if (s.capabilities.includes('backups') && can('backup.manage')) out.push({ id: `a-backup-${s.id}`, title: `Back up ${g} now`, hint: 'Creates a new backup in the background', icon: 'archive', group: 'Actions', kw: 'backup snapshot', run: () => void backupNow(s.id) });
    }
    return out;
  });
  // Backups, content and files come from the server-side search, after a short pause in typing.
  let remote = $state<Item[]>([]);
  let remoteFor = '';
  $effect(() => {
    const t = q.trim();
    if (t.length < 2) { remote = []; remoteFor = ''; return; }
    const h = setTimeout(async () => {
      try {
        const r = await api.searchAll(t, 5);
        if (t !== q.trim()) return;
        const sn = (id: string, n: string | null) => n ?? servers.list.find((x) => x.id === id)?.name ?? 'a server';
        remote = [
          ...r.backups.map((b): Item => ({ id: `r-b-${b.id}`, title: b.note || `Backup of ${b.server_name}`, hint: `Backup · ${b.server_name} · ${relTime(b.created_at)}`, icon: 'archive', group: 'Backups', run: () => navigate(serverHref(b.server_id, 'backups')) })),
          ...r.content.map((c): Item => ({ id: `r-c-${c.server_id}-${c.id}`, title: c.title, hint: `${c.kind} · ${sn(c.server_id, c.server_name)}`, icon: 'package', group: 'Content', run: () => navigate(serverHref(c.server_id, 'content')) })),
          ...r.files.map((f): Item => ({ id: `r-f-${f.server_id}-${f.path}`, title: f.path, hint: `File · ${f.server_name}`, icon: 'file', group: 'Files', run: () => navigate(serverHref(f.server_id, 'files')) })),
        ];
        remoteFor = t;
      } catch { remote = []; }
    }, 250);
    return () => clearTimeout(h);
  });
  const results = $derived.by(() => {
    if (!q.trim()) return items.filter((i) => i.group !== 'Actions' || i.id.startsWith('a-theme')).slice(0, 14);
    const local = items.map((i) => ({ i, s: fuzzy(q, i.title + ' ' + (i.kw ?? '')) })).filter((x) => x.s >= 0).sort((a, b) => b.s - a.s).slice(0, 12).map((x) => x.i);
    return remoteFor === q.trim() ? [...local, ...remote] : local;
  });
  $effect(() => { q; sel = 0; });
  $effect(() => { input.focus(); document.body.style.overflow = 'hidden'; return () => { document.body.style.overflow = ''; prev?.focus?.(); }; });
  $effect(() => { list?.querySelector('[aria-selected=true]')?.scrollIntoView({ block: 'nearest' }); sel; });

  function close() { ui.palette = false; }
  async function run(i: Item) { close(); await i.run(); }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); close(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); sel = (sel + 1) % Math.max(1, results.length); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); sel = (sel - 1 + results.length) % Math.max(1, results.length); }
    else if (e.key === 'Enter' && results[sel]) { e.preventDefault(); void run(results[sel]); }
    else if (e.key === 'Tab') e.preventDefault();
  }
</script>

<div class="ov" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="pal" role="dialog" aria-modal="true" aria-label="Command palette">
    <div class="in">
      <Icon name="search" size={18} />
      <input bind:this={input} bind:value={q} onkeydown={key} placeholder="Search pages, servers, backups, content and files…" role="combobox" aria-expanded="true" aria-controls="pal-list" aria-activedescendant={results[sel] ? 'pi-' + results[sel].id : undefined} aria-label="Search" autocomplete="off" spellcheck="false" />
      <kbd>esc</kbd>
    </div>
    <div class="list" id="pal-list" role="listbox" bind:this={list}>
      {#each results as r, i (r.id)}
        {#if i === 0 || results[i - 1].group !== r.group}<div class="grp xs faint" role="presentation">{r.group}</div>{/if}
        <div id="pi-{r.id}" class="it" role="option" tabindex="-1" aria-selected={i === sel} onmousemove={() => (sel = i)} onclick={() => run(r)} onkeydown={() => {}}>
          <span class="ico"><Icon name={r.icon} size={16} /></span>
          <div class="grow"><div class="ellipsis">{r.title}</div><div class="xs muted ellipsis">{r.hint}</div></div>
          {#if i === sel}<kbd>↵</kbd>{/if}
        </div>
      {:else}
        <div class="none muted sm">No matches for “{q}”.</div>
      {/each}
    </div>
    <div class="foot xs faint"><span><kbd>↑</kbd> <kbd>↓</kbd> navigate</span><span><kbd>↵</kbd> run</span><span class="grow"></span><span>Stop / restart ask for confirmation</span></div>
  </div>
</div>

<style>
  .ov { position: fixed; inset: 0; z-index: 70; background: var(--overlay); display: flex; justify-content: center; align-items: flex-start; padding: 12vh 16px 16px; animation: fade .12s; }
  .pal { width: min(620px, 100%); background: var(--s1); border: 1px solid var(--line2); border-radius: var(--r-lg); box-shadow: 0 30px 80px rgb(0 0 0 / .55), 0 1px 0 var(--hl) inset; overflow: hidden; animation: rise .16s var(--ease); display: flex; flex-direction: column; max-height: 70vh; }
  .in { display: flex; align-items: center; gap: 10px; padding: 0 16px; height: 54px; border-bottom: 1px solid var(--line); color: var(--muted); }
  .in input { flex: 1; background: none; border: 0; outline: 0; font-size: var(--fs-md); color: var(--text); height: 100%; min-width: 0; }
  .list { overflow: auto; padding: 6px; flex: 1; }
  .grp { padding: 8px 10px 4px; text-transform: uppercase; letter-spacing: .08em; }
  .it { display: flex; align-items: center; gap: 12px; padding: 8px 10px; border-radius: 10px; cursor: pointer; }
  .it[aria-selected='true'] { background: var(--a-soft); box-shadow: inset 2px 0 0 var(--a1); }
  .ico { width: 30px; height: 30px; display: grid; place-items: center; border-radius: 8px; background: var(--s3); color: var(--muted); flex: none; }
  .it[aria-selected='true'] .ico { color: var(--a-text); }
  .none { padding: 24px; text-align: center; }
  .foot { display: flex; gap: 14px; padding: 8px 14px; border-top: 1px solid var(--line); }
  @media (max-width: 600px) { .foot span:last-child { display: none; } .ov { padding-top: 8vh; } }
</style>
