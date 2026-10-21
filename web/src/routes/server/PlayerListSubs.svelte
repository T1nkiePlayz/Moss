<script lang="ts">
  import States from '../../components/States.svelte';
  import { errMsg, type Server } from '../../lib/api';
  import { playerLists, type ServerPlayerLists } from '../../lib/api-extra';
  import { toastError } from '../../lib/stores.svelte';
  import { auth } from '../../lib/auth.svelte';

  let { server: s }: { server: Server } = $props();
  let data = $state<ServerPlayerLists | null>(null);
  let err = $state('');
  const admin = $derived(auth.me?.role === 'owner' || auth.me?.role === 'admin');
  async function load() { const id = s.id; try { const r = await playerLists.forServer(id); if (id === s.id) { data = r; err = ''; } } catch (e) { err = errMsg(e); } }
  $effect(() => { s.id; data = null; void load(); });
  const usable = (k: 'allow' | 'ban') => !!data?.support[k];
  async function toggle(id: string, on: boolean) {
    if (!data) return;
    const ids = data.lists.filter((l) => (l.id === id ? on : l.subscribed)).map((l) => l.id);
    try { await playerLists.subscribe(s.id, ids); await load(); } catch (e) { toastError(e, 'Could not change the lists'); }
  }
</script>

{#if err}<p class="muted sm">{err}</p>
{:else if !data}<States kind="loading" rows={1} />
{:else if data.lists.length || data.support.note}
  <section class="card" style="margin-bottom:14px">
    <div class="card-title">Shared player lists</div>
    {#if data.support.note}<p class="muted sm" style="margin:0 0 8px">{data.support.note}</p>{/if}
    {#if data.allowlist_enabled === false && data.lists.some((l) => l.subscribed && l.kind === 'allow')}
      <p class="sm" role="status" style="margin:0 0 8px">The whitelist is switched off on this server (white-list / allow-list is false in server.properties), so the allow list is not enforced. Moss does not change that setting.</p>
    {/if}
    {#each data.lists as l (l.id)}
      <label class="row g8" style="padding:3px 0">
        <input type="checkbox" checked={l.subscribed} disabled={!admin || !usable(l.kind)} onchange={(e) => toggle(l.id, e.currentTarget.checked)} />
        <span>{l.name}</span><span class="muted xs">{l.kind === 'allow' ? 'whitelist' : 'ban list'} · {l.entries} player{l.entries === 1 ? '' : 's'}{usable(l.kind) ? '' : ' · not supported here'}</span>
      </label>
    {/each}
    {#if data.pending.allow + data.pending.ban > 0}<p class="muted xs" style="margin:8px 0 0">{data.pending.allow + data.pending.ban} name(s) not applied yet (waiting for the server to run, or for Mojang to answer).</p>{/if}
  </section>
{/if}
