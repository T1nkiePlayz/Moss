<script lang="ts">
  import States from '../../components/States.svelte';
  import { api, errMsg, type Grant, type Server, type User } from '../../lib/api';
  import { toast, toastError } from '../../lib/stores.svelte';
  import { isAdmin } from '../../lib/auth.svelte';

  let { server: s }: { server: Server } = $props();
  let users = $state<User[] | null>(null);
  let grants = $state<Record<string, string>>({});
  let orig = '';
  let err = $state('');
  let saving = $state(false);
  const bundles = [['none', 'No access'], ['viewer', 'Viewer'], ['operator', 'Operator'], ['manager', 'Manager']];

  async function load() {
    err = '';
    try {
      const [u, g] = await Promise.all([api.users(), api.access(s.id)]);
      users = u;
      grants = Object.fromEntries(g.map((x: Grant) => [x.user_id, x.bundle]));
      orig = JSON.stringify(grants);
    } catch (e) { err = errMsg(e); }
  }
  $effect(() => { s.id; users = null; load(); });
  const dirty = $derived(JSON.stringify(grants) !== orig);

  async function save() {
    saving = true;
    try {
      await api.saveAccess(s.id, Object.entries(grants).filter(([, b]) => b && b !== 'none').map(([user_id, bundle]) => ({ user_id, bundle })));
      orig = JSON.stringify(grants);
      toast('success', 'Access updated');
    } catch (e) { toastError(e, 'Could not save access'); }
    saving = false;
  }
</script>

{#if !isAdmin()}
  <States kind="empty" icon="lock" title="Admins only" text="Only owners and admins can change who can access this server." />
{:else if err}<States kind="error" text={err} retry={load} />
{:else if !users}<States kind="loading" rows={3} />
{:else}
  <div class="grid cols-2" style="align-items:start">
    <section class="card">
      <div class="card-title">Who can use {s.name}</div>
      {#each users.filter((u) => u.role === 'user') as u (u.id)}
        <div class="row between ur">
          <div class="ellipsis grow">{u.email}</div>
          <select class="select" style="width:150px" aria-label="Access for {u.email}" bind:value={grants[u.id]}>
            {#each bundles as [v, l]}<option value={v} selected={(grants[u.id] ?? 'none') === v}>{l}</option>{/each}
          </select>
        </div>
      {:else}
        <p class="muted sm">No regular users yet. Owners and admins already have access to every server. Invite people on the <a href="#/users">Users</a> page.</p>
      {/each}
      <div class="row end" style="margin-top:14px"><button class="btn primary" disabled={!dirty || saving} onclick={save}>{saving ? 'Saving…' : 'Save access'}</button></div>
    </section>
    <section class="card">
      <div class="card-title">What the roles mean</div>
      <dl class="def">
        <dt>Viewer</dt><dd>See status, players and the console output.</dd>
        <dt>Operator</dt><dd>Viewer, plus start/stop, console commands and backups.</dd>
        <dt>Manager</dt><dd>Operator, plus files, config, add-ons and deleting the server.</dd>
      </dl>
    </section>
  </div>
{/if}

<style>
  .ur { padding: 8px 0; border-bottom: 1px solid var(--line); } .ur:last-of-type { border: 0; }
  .def { margin: 0; display: grid; grid-template-columns: 90px 1fr; gap: 10px 14px; } .def dt { font-weight: 600; } .def dd { margin: 0; color: var(--muted); }
</style>
