<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import { errMsg } from '../../lib/api';
  import { playerLists, type PlayerList, type PlayerListEntry } from '../../lib/api-extra';
  import { confirmDialog, toast, toastError } from '../../lib/stores.svelte';

  let items = $state<PlayerList[] | null>(null);
  let err = $state('');
  let newName = $state('');
  let newKind = $state<'allow' | 'ban'>('allow');
  let open = $state<string | null>(null);
  let rows = $state<PlayerListEntry[] | null>(null);
  let who = $state('');
  let why = $state('');

  async function load() { err = ''; try { items = (await playerLists.list()).items; } catch (e) { err = errMsg(e); } }
  void load();
  async function create() {
    try { await playerLists.create(newName.trim(), newKind); newName = ''; void load(); } catch (e) { toastError(e, 'Could not create the list'); }
  }
  async function show(l: PlayerList) {
    if (open === l.id) { open = null; return; }
    open = l.id; rows = null;
    try { rows = (await playerLists.entries(l.id)).items; } catch (e) { toastError(e, 'Could not load the list'); }
  }
  async function add(l: PlayerList) {
    try { await playerLists.addEntry(l.id, who.trim(), l.kind === 'ban' ? why.trim() : undefined); who = ''; why = ''; rows = (await playerLists.entries(l.id)).items; void load(); } catch (e) { toastError(e, 'Could not add the player'); }
  }
  async function drop(l: PlayerList, name: string) {
    try { await playerLists.removeEntry(l.id, name); rows = (await playerLists.entries(l.id)).items; void load(); } catch (e) { toastError(e, 'Could not remove the player'); }
  }
  async function remove(l: PlayerList) {
    const r = await confirmDialog({ title: `Delete ${l.name}?`, body: `${l.servers} server${l.servers === 1 ? '' : 's'} stop using it. Names Moss added to their whitelist or ban files are taken out again.`, confirmLabel: 'Delete list', danger: true });
    if (!r.ok) return;
    try { await playerLists.remove(l.id); toast('info', 'List deleted'); if (open === l.id) open = null; void load(); } catch (e) { toastError(e, 'Could not delete'); }
  }
</script>

<section class="card" aria-labelledby="pl">
  <div class="card-title" id="pl"><Icon name="users" size={14} />Player lists</div>
  <p class="muted sm">Keep one whitelist or ban list and use it on many servers. Choose which lists a server follows on its Players tab; Moss keeps the server's own files and commands in step. Entries you add by hand on a server are left alone. For a Velocity proxy, subscribe the servers behind it.</p>
  {#if err}<States kind="error" text={err} retry={load} />
  {:else if !items}<States kind="loading" rows={2} />
  {:else}
    {#if !items.length}<p class="muted sm">No lists yet.</p>{/if}
    <ul class="pl">
      {#each items as l (l.id)}
        <li>
          <div class="row g8 wrap">
            <b>{l.name}</b><span class="badge">{l.kind === 'allow' ? 'Whitelist' : 'Ban list'}</span>
            <span class="muted xs">{l.entries} player{l.entries === 1 ? '' : 's'} · {l.servers} server{l.servers === 1 ? '' : 's'}</span>
            <span class="grow"></span>
            <button class="btn sm" aria-expanded={open === l.id} onclick={() => show(l)}>{open === l.id ? 'Close' : 'Players'}</button>
            <button class="btn ghost icon sm" aria-label={`Delete ${l.name}`} onclick={() => remove(l)}><Icon name="trash" size={14} /></button>
          </div>
          {#if open === l.id}
            {#if !rows}<States kind="loading" rows={2} />
            {:else}
              <ul class="plain" style="margin:8px 0;padding:0;list-style:none">
                {#each rows as r (r.name)}
                  <li class="row between g8" style="padding:3px 0"><span>{r.name}{#if r.reason} <span class="muted xs">· {r.reason}</span>{/if}</span><button class="btn ghost sm" aria-label={`Remove ${r.name}`} onclick={() => drop(l, r.name)}>Remove</button></li>
                {:else}<li class="muted sm">Nobody on this list.</li>{/each}
              </ul>
              <form class="row g6 wrap" onsubmit={(e) => { e.preventDefault(); void add(l); }}>
                <label class="sr" for="pl-who">Player name</label><input id="pl-who" class="input" placeholder="Player name" bind:value={who} />
                {#if l.kind === 'ban'}<label class="sr" for="pl-why">Reason</label><input id="pl-why" class="input" placeholder="Reason (optional)" bind:value={why} />{/if}
                <button class="btn" disabled={!who.trim()}>Add</button>
              </form>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
    <form class="row g6 wrap" style="margin-top:12px" onsubmit={(e) => { e.preventDefault(); void create(); }}>
      <label class="sr" for="pl-name">List name</label><input id="pl-name" class="input" placeholder="List name" bind:value={newName} />
      <label class="sr" for="pl-kind">Kind</label>
      <select id="pl-kind" class="input" bind:value={newKind}><option value="allow">Whitelist</option><option value="ban">Ban list</option></select>
      <button class="btn primary" disabled={!newName.trim()}><Icon name="plus" size={15} />Create list</button>
    </form>
  {/if}
</section>

<style>
  .pl { list-style: none; margin: 12px 0 0; padding: 0; } .pl > li { padding: 12px 0; border-bottom: 1px solid var(--line); } .pl > li:last-child { border: 0; }
</style>
