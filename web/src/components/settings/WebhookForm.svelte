<script lang="ts">
  import { untrack } from 'svelte';
  import Modal from '../Modal.svelte';
  import Icon from '../Icon.svelte';
  import { ApiError, errMsg, type Webhook, type WebhookEventType } from '../../lib/api';
import { apiOps } from '../../lib/api-ops';
  import { webhookProblems } from '../../lib/webhooks';
  import { servers, toast } from '../../lib/stores.svelte';

  let { hook = null, types, onclose, ondone }: { hook?: Webhook | null; types: WebhookEventType[]; onclose: () => void; ondone: (secret?: string, h?: Webhook) => void } = $props();
  const h0 = untrack(() => hook);
  const editing = !!h0;
  let f = $state({ name: h0?.name ?? '', url: h0?.url ?? '', events: [...(h0?.events ?? [])], server_ids: [...(h0?.server_ids ?? [])] });
  let scope = $state<'all' | 'some'>(h0?.server_ids.length ? 'some' : 'all');
  let busy = $state(false);
  let shown = $state(false);
  let server = $state<Record<string, string>>({});
  let top = $state('');
  const local = $derived(shown ? webhookProblems(f) : {});
  const msg = (k: string) => local[k] || server[k] || '';
  const toggle = (list: string[], v: string, on: boolean) => (on ? [...list.filter((x) => x !== v), v] : list.filter((x) => x !== v));

  async function save(e: Event) {
    e.preventDefault(); shown = true; server = {}; top = '';
    if (Object.keys(webhookProblems(f)).length) return;
    if (scope === 'some' && !f.server_ids.length) { server.server_ids = 'Choose at least one server, or pick all servers.'; return; }
    busy = true;
    const body = { name: f.name.trim(), url: f.url.trim(), events: f.events, server_ids: scope === 'all' ? [] : f.server_ids };
    try {
      if (hook) { await apiOps.patchWebhook(hook.id, body); toast('success', 'Webhook updated'); ondone(); }
      else { const w = await apiOps.addWebhook(body); ondone(w.secret, w); }
    } catch (er) {
      if (er instanceof ApiError && er.fields.length) { for (const x of er.fields) server[x.path] = x.message; } else top = errMsg(er);
    }
    busy = false;
  }
</script>

<Modal title={editing ? 'Edit webhook' : 'Add a webhook'} {onclose} width={620}>
  <form id="whf" class="col" onsubmit={save} novalidate>
    <div class="field"><label for="wn">Name</label><input id="wn" class="input" bind:value={f.name} maxlength="60" placeholder="Discord bot, dashboard…" aria-invalid={!!msg('name')} data-autofocus />{#if msg('name')}<span class="err" role="alert">{msg('name')}</span>{/if}</div>
    <div class="field"><label for="wu">Address</label><input id="wu" class="input mono" type="url" bind:value={f.url} placeholder="https://example.com/moss-events" autocomplete="off" spellcheck="false" aria-describedby="wu-h" aria-invalid={!!msg('url')} />
      <span class="hint" id="wu-h">Moss sends signed JSON to this HTTPS address. It must be reachable from this machine.</span>{#if msg('url')}<span class="err" role="alert">{msg('url')}</span>{/if}</div>
    <fieldset class="evs" aria-describedby={msg('events') ? 'we-e' : undefined}>
      <legend class="label">Events</legend>
      {#each types as t}
        <label class="ev"><input type="checkbox" class="check" checked={f.events.includes(t.type)} onchange={(e) => (f.events = toggle(f.events, t.type, e.currentTarget.checked))} /><span><code>{t.type}</code><span class="hint" style="display:block">{t.description}</span></span></label>
      {/each}
      {#if msg('events')}<span class="err" id="we-e" role="alert">{msg('events')}</span>{/if}
    </fieldset>
    <fieldset class="evs">
      <legend class="label">Servers</legend>
      <div class="row g6" role="radiogroup" aria-label="Which servers">
        <button type="button" class="chip" role="radio" aria-checked={scope === 'all'} onclick={() => (scope = 'all')}>All servers</button>
        <button type="button" class="chip" role="radio" aria-checked={scope === 'some'} onclick={() => (scope = 'some')}>Only some</button>
      </div>
      {#if scope === 'some'}
        <div class="col" style="margin-top:8px">{#each servers.list as s (s.id)}<label class="row g8"><input type="checkbox" class="check" checked={f.server_ids.includes(s.id)} onchange={(e) => (f.server_ids = toggle(f.server_ids, s.id, e.currentTarget.checked))} /><span>{s.name}</span></label>{/each}</div>
        <span class="hint">Events that do not belong to one server (such as update notices) are not sent to a webhook limited to some servers.</span>
      {/if}
      {#if msg('server_ids')}<span class="err" role="alert">{msg('server_ids')}</span>{/if}
    </fieldset>
    {#if top}<div class="banner danger" role="alert"><Icon name="alert" size={16} />{top}</div>{/if}
  </form>
  {#snippet footer()}
    <button class="btn" type="button" onclick={onclose}>Cancel</button>
    <button class="btn primary" type="submit" form="whf" disabled={busy}>{busy ? 'Saving…' : editing ? 'Save changes' : 'Add webhook'}</button>
  {/snippet}
</Modal>

<style>
  .evs { border: 0; padding: 0; margin: 0; min-width: 0; }
  .ev { display: flex; gap: 10px; align-items: flex-start; padding: 5px 0; cursor: pointer; }
</style>
