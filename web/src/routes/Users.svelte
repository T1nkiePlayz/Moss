<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import States from '../components/States.svelte';
  import { api, errMsg, type Role, type User } from '../lib/api';
  import { confirmDialog, toast, toastError } from '../lib/stores.svelte';
  import { auth } from '../lib/auth.svelte';
  import { relTime, initials } from '../lib/format';

  let users = $state<User[] | null>(null);
  let err = $state('');
  let email = $state('');
  let role = $state<Role>('user');
  let busy = $state(false);
  async function load() { err = ''; try { users = await api.users(); } catch (e) { err = errMsg(e); } }
  void load();

  async function invite(e: Event) {
    e.preventDefault(); busy = true;
    try { await api.addUser(email.trim(), role); toast('success', 'Invited', { body: `${email.trim()} can now sign in.` }); email = ''; load(); } catch (er) { toastError(er, 'Invite failed'); }
    busy = false;
  }
  async function setRole(u: User, r: Role) { try { await api.setRole(u.id, r); u.role = r; toast('success', 'Role updated'); } catch (e) { toastError(e); load(); } }
  async function del(u: User) {
    if (!(await confirmDialog({ title: `Remove ${u.email}?`, body: 'They will lose access immediately.', confirmLabel: 'Remove user', danger: true })).ok) return;
    try { await api.deleteUser(u.id); load(); } catch (e) { toastError(e, 'Could not remove'); }
  }
  const isOwner = $derived(auth.me?.role === 'owner');
</script>

<div class="page-head"><div><h1>Users</h1><p>Only invited emails can sign in to this instance.</p></div></div>

<form class="card row wrap" onsubmit={invite} style="margin-bottom:var(--gap)">
  <div class="field grow" style="min-width:220px"><label for="ie">Invite by email</label><input id="ie" class="input" type="email" required placeholder="friend@example.com" bind:value={email} /></div>
  <div class="field"><label for="ir">Role</label><select id="ir" class="select" style="width:130px" bind:value={role}><option value="user">User</option>{#if isOwner}<option value="admin">Admin</option>{/if}</select></div>
  <button class="btn primary" style="align-self:end" disabled={busy}><Icon name="plus" size={15} />Invite</button>
</form>

{#if err}<States kind="error" text={err} retry={load} />
{:else if !users}<States kind="loading" rows={3} />
{:else}
  <div class="table-wrap">
    <table>
      <thead><tr><th>User</th><th>Role</th><th>Added</th><th></th></tr></thead>
      <tbody>
        {#each users as u (u.id)}
          <tr>
            <td><div class="row"><span class="av">{initials(u.email)}</span><span>{u.email}{#if u.id === auth.me?.id}<span class="badge ok" style="margin-left:8px">You</span>{/if}</span></div></td>
            <td>
              {#if u.role === 'owner' || (u.role === 'admin' && !isOwner) || u.id === auth.me?.id}<span class="badge {u.role === 'owner' ? 'ok' : ''}">{u.role}</span>
              {:else}<select class="select sm" style="width:110px" aria-label="Role for {u.email}" value={u.role} onchange={(e) => setRole(u, e.currentTarget.value as Role)}><option value="user">user</option><option value="admin">admin</option></select>{/if}
            </td>
            <td class="muted">{relTime(u.created_at)}</td>
            <td class="num">{#if u.role !== 'owner' && u.id !== auth.me?.id && (isOwner || u.role === 'user')}<button class="btn ghost icon sm" aria-label="Remove {u.email}" onclick={() => del(u)}><Icon name="trash" size={14} /></button>{/if}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .av { width: 28px; height: 28px; border-radius: 50%; display: grid; place-items: center; background: var(--a-soft); color: var(--a-text); font-size: calc(11px * var(--ts, 1)); font-weight: 700; flex: none; }
</style>
