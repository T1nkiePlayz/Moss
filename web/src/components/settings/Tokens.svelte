<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import { apiOps, errMsg, type ApiToken, type CreatedToken, type TokenScope } from '../../lib/api';
  import { confirmDialog, toast, toastError } from '../../lib/stores.svelte';
  import { isAdmin } from '../../lib/auth.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { dateTime } from '../../lib/format';
  import { expiryChoices, expiryStatus, scopeInfo, maskedPrefix, whenShort } from '../../lib/tokens';

  let list = $state<ApiToken[] | null>(null);
  let err = $state('');
  let everyone = $state(false);
  let name = $state('');
  let scope = $state<TokenScope>('read');
  let days = $state<string>('30');
  let busy = $state(false);
  let created = $state<CreatedToken | null>(null);
  let formErr = $state('');

  async function load() { err = ''; try { list = await apiOps.tokens(everyone); } catch (e) { err = errMsg(e); } }
  $effect(() => { everyone; void load(); });

  async function create(e: Event) {
    e.preventDefault(); busy = true; formErr = '';
    try {
      created = await apiOps.createToken({ name: name.trim(), scope, ...(days ? { expires_in_days: Number(days) } : {}) });
      name = '';
      void load();
    } catch (er) { formErr = errMsg(er); }
    busy = false;
  }
  async function revoke(t: ApiToken) {
    const r = await confirmDialog({ title: `Revoke "${t.name}"?`, body: 'Anything using this token stops working immediately. This cannot be undone.', confirmLabel: 'Revoke token', danger: true });
    if (!r.ok) return;
    try { await apiOps.revokeToken(t.id); toast('info', 'Token revoked', { body: t.name }); if (created?.id === t.id) created = null; void load(); } catch (e) { toastError(e, 'Could not revoke'); }
  }
</script>

<section class="card" aria-labelledby="tk">
  <div class="card-title" id="tk"><Icon name="key" size={14} />API tokens</div>
  <p class="muted sm">Personal access tokens let scripts and tools call the Moss API as you (<code>Authorization: Bearer moss_pat_…</code>). They never grant access to token or user management.</p>

  {#if created}
    <div class="created" role="status">
      <div class="banner warn"><Icon name="alert" size={16} /><div><b>Copy your new token now.</b> It is shown only once and cannot be recovered. Store it in a password manager or secret store.</div></div>
      <div class="row g6 wrap" style="margin-top:10px">
        <label class="sr" for="new-token">New API token</label>
        <input id="new-token" class="input mono grow" readonly value={created.token} onfocus={(e) => e.currentTarget.select()} style="min-width:220px" />
        <button class="btn primary" onclick={() => copyWithToast(created!.token, 'Token')}><Icon name="copy" size={15} />Copy</button>
      </div>
      <div class="row between wrap" style="margin-top:10px"><span class="xs muted">{created.name} · {scopeInfo[created.scope].label} · {expiryStatus(created.expires_at).text}</span><button class="btn sm" onclick={() => (created = null)}>I have saved it</button></div>
    </div>
  {/if}

  <form class="form" onsubmit={create}>
    <div class="field"><label for="tn">Name</label><input id="tn" class="input" bind:value={name} required maxlength="64" placeholder="Grafana, backup script…" /></div>
    <div class="field"><label for="ts">Access</label><select id="ts" class="select" bind:value={scope} aria-describedby="ts-h"><option value="read">Read only</option><option value="full">Full access</option></select></div>
    <div class="field"><label for="te">Expires</label><select id="te" class="select" bind:value={days}>{#each expiryChoices as c}<option value={c.days === null ? '' : String(c.days)}>{c.label}</option>{/each}</select></div>
    <div class="field act"><button class="btn primary" disabled={busy || !name.trim()}><Icon name="plus" size={15} />{busy ? 'Creating…' : 'Create token'}</button></div>
    <span class="hint full" id="ts-h">{scopeInfo[scope].hint}</span>
  </form>
  <div aria-live="assertive">{#if formErr}<div class="banner danger" role="alert" style="margin-top:10px"><Icon name="alert" size={16} />{formErr}</div>{/if}</div>

  <div class="row between wrap list-head">
    <h3 class="sm">{everyone ? 'All tokens' : 'Your tokens'}</h3>
    {#if isAdmin()}<label class="row g6 sm" style="cursor:pointer"><input type="checkbox" class="check" bind:checked={everyone} />Show everyone's tokens</label>{/if}
  </div>
  {#if err}<States kind="error" text={err} retry={load} />
  {:else if !list}<States kind="loading" rows={2} />
  {:else if !list.length}<States kind="empty" icon="key" title="No API tokens" text="Create one above to automate Moss from scripts or monitoring." />
  {:else}
    <div class="table-wrap">
      <table>
        <thead><tr><th>Name</th><th>Access</th><th>Token</th><th>Last used</th><th>Expires</th><th></th></tr></thead>
        <tbody>
          {#each list as t (t.id)}
            {@const ex = expiryStatus(t.expires_at)}
            <tr>
              <td><b>{t.name}</b><div class="xs muted" title={dateTime(t.created_at)}>Created {whenShort(t.created_at)}</div></td>
              <td><span class="badge {t.scope === 'full' ? 'warn' : ''}">{scopeInfo[t.scope].label}</span></td>
              <td><code class="xs">{maskedPrefix(t.prefix)}</code></td>
              <td title={t.last_used ? dateTime(t.last_used) : ''}>{t.last_used ? whenShort(t.last_used) : 'Never'}</td>
              <td><span class="badge {ex.tone}" title={t.expires_at ? dateTime(t.expires_at) : ''}>{ex.text}</span></td>
              <td class="num"><button class="btn danger sm" onclick={() => revoke(t)} aria-label="Revoke token {t.name}"><Icon name="trash" size={13} />Revoke</button></td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .created { margin-top: 12px; padding: 12px; border: 1px solid var(--line2); border-radius: var(--r-sm); background: var(--s2); }
  .form { display: grid; grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) minmax(0, 1fr) auto; gap: 12px; align-items: start; margin: 16px 0 6px; }
  .act { padding-top: 22px; } .full { grid-column: 1 / -1; }
  .list-head { margin: 18px 0 10px; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  @media (max-width: 760px) { .form { grid-template-columns: minmax(0, 1fr); } .act { padding-top: 0; } .act .btn { width: 100%; } }
</style>
