<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import WebhookForm from './WebhookForm.svelte';
  import { errMsg, type Webhook, type WebhookDelivery, type WebhookEventType } from '../../lib/api';
import { apiOps } from '../../lib/api-ops';
  import { confirmDialog, servers, toast, toastError } from '../../lib/stores.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { dateTime } from '../../lib/format';
  import { deliveryBadge, scopeText, webhookStatus } from '../../lib/webhooks';

  let list = $state<Webhook[] | null>(null);
  let types = $state<WebhookEventType[]>([]);
  let err = $state('');
  let adding = $state(false);
  let editing = $state<Webhook | null>(null);
  let secret = $state<{ name: string; value: string } | null>(null);
  let open = $state<string | null>(null);
  let log = $state<WebhookDelivery[] | null>(null);
  let testing = $state<string | null>(null);

  async function load() { err = ''; try { const r = await apiOps.webhooks(); list = r.items; types = r.event_types; } catch (e) { err = errMsg(e); } }
  void load();
  async function showLog(w: Webhook) {
    if (open === w.id) { open = null; return; }
    open = w.id; log = null;
    try { log = (await apiOps.webhookDeliveries(w.id)).items; } catch (e) { toastError(e, 'Could not load the delivery log'); }
  }
  async function reloadLog(id: string) { if (open === id) { try { log = (await apiOps.webhookDeliveries(id)).items; } catch { /* keep the old log */ } } }
  async function test(w: Webhook) {
    testing = w.id;
    try {
      const { delivery } = await apiOps.testWebhookEndpoint(w.id);
      if (delivery.state === 'delivered') toast('success', 'Test event delivered', { body: `${w.name} answered HTTP ${delivery.status_code}.` });
      else toast('error', 'The test event failed', { body: delivery.error ?? 'No answer.' });
      void reloadLog(w.id);
    } catch (e) { toastError(e, 'Could not send the test event'); }
    testing = null;
  }
  async function toggle(w: Webhook) {
    try { await apiOps.patchWebhook(w.id, { enabled: !w.enabled }); void load(); } catch (e) { toastError(e, 'Could not change the webhook'); }
  }
  async function newSecret(w: Webhook) {
    const r = await confirmDialog({ title: `Make a new secret for ${w.name}?`, body: 'The old secret stops working at once. Update the receiver with the new one, or it will reject Moss\'s events.', confirmLabel: 'Make new secret' });
    if (!r.ok) return;
    try { secret = { name: w.name, value: (await apiOps.rotateWebhookSecret(w.id)).secret }; } catch (e) { toastError(e, 'Could not make a new secret'); }
  }
  async function remove(w: Webhook) {
    const r = await confirmDialog({ title: `Remove ${w.name}?`, body: 'Moss stops sending events there and forgets its delivery log. Events still waiting to be delivered are dropped.', confirmLabel: 'Remove webhook', danger: true });
    if (!r.ok) return;
    try { await apiOps.removeWebhook(w.id); toast('info', 'Webhook removed'); if (open === w.id) open = null; void load(); } catch (e) { toastError(e, 'Could not remove'); }
  }
  async function resend(w: Webhook, d: WebhookDelivery) {
    try { await apiOps.retryWebhookDelivery(w.id, d.id); toast('info', 'Queued again'); void reloadLog(w.id); void load(); } catch (e) { toastError(e, 'Could not send again'); }
  }
  const done = (s?: string, h?: Webhook) => { adding = false; editing = null; if (s && h) secret = { name: h.name, value: s }; void load(); };
  const names = (id: string) => servers.list.find((s) => s.id === id)?.name ?? 'a removed server';
</script>

<section class="card" aria-labelledby="wh">
  <div class="card-title" id="wh"><Icon name="zap" size={14} />Webhooks</div>
  <p class="muted sm">Let other programs react to what happens here: a server crashed, a backup finished, a player joined. Moss sends each event as signed JSON to the HTTPS address you choose, and tries again if it is not answered.</p>

  {#if secret}
    <div class="created" role="status">
      <div class="banner warn"><Icon name="alert" size={16} /><div><b>Copy the signing secret for {secret.name} now.</b> It is shown only once. Your receiver uses it to check that events really come from Moss.</div></div>
      <div class="row g6 wrap" style="margin-top:10px">
        <label class="sr" for="wh-secret">Signing secret</label>
        <input id="wh-secret" class="input mono grow" readonly value={secret.value} onfocus={(e) => e.currentTarget.select()} style="min-width:220px" />
        <button class="btn primary" onclick={() => copyWithToast(secret!.value, 'Secret')}><Icon name="copy" size={15} />Copy</button>
        <button class="btn" onclick={() => (secret = null)}>I have saved it</button>
      </div>
    </div>
  {/if}

  {#if err}<States kind="error" text={err} retry={load} />
  {:else if !list}<States kind="loading" rows={2} />
  {:else}
    {#if !list.length}<p class="muted sm" style="margin-top:12px">No webhooks yet.</p>{/if}
    <ul class="wl">
      {#each list as w (w.id)}
        {@const s = webhookStatus(w)}
        <li>
          <div class="row g8 wrap">
            <b>{w.name}</b><span class="badge {s.tone}">{s.label}</span>
            {#if w.pending}<span class="badge">{w.pending} waiting</span>{/if}
            <span class="grow"></span>
            <button class="btn sm" onclick={() => test(w)} disabled={testing === w.id}>{testing === w.id ? 'Sending…' : 'Send test event'}</button>
            <button class="btn sm" onclick={() => toggle(w)}>{w.enabled ? 'Turn off' : 'Turn on'}</button>
            <button class="btn sm" onclick={() => (editing = w)}><Icon name="edit" size={13} />Edit</button>
            <button class="btn ghost icon sm" aria-label={`Remove ${w.name}`} onclick={() => remove(w)}><Icon name="trash" size={14} /></button>
          </div>
          <div class="xs muted mono ellipsis" title={w.url}>{w.url}</div>
          <div class="xs muted">{w.events.length} event{w.events.length === 1 ? '' : 's'}: {w.events.join(', ')} · {scopeText(w, names)}</div>
          {#if w.disabled_reason}<div class="banner danger" role="status" style="margin-top:8px"><Icon name="alert" size={16} />{w.disabled_reason} Fix the receiver, then turn it on again.</div>{/if}
          <div class="row g8 wrap" style="margin-top:6px">
            <button class="btn ghost sm" aria-expanded={open === w.id} onclick={() => showLog(w)}>{open === w.id ? 'Hide' : 'Show'} delivery log</button>
            <button class="btn ghost sm" onclick={() => newSecret(w)}>New secret</button>
          </div>
          {#if open === w.id}
            {#if !log}<States kind="loading" rows={2} />
            {:else if !log.length}<p class="muted sm">Nothing has been sent yet.</p>
            {:else}
              <div class="table-wrap"><table>
                <caption class="sr">Recent deliveries for {w.name}</caption>
                <thead><tr><th>When</th><th>Event</th><th>Result</th><th>Answer</th><th><span class="sr">Actions</span></th></tr></thead>
                <tbody>
                  {#each log as d (d.id)}
                    {@const b = deliveryBadge(d)}
                    <tr>
                      <td class="tnum">{dateTime(d.created_at)}</td><td><code>{d.type}</code></td>
                      <td><span class="badge {b.tone}">{b.label}</span></td>
                      <td class="xs" title={d.error ?? d.response ?? ''}>{d.status_code ?? '—'}{d.latency_ms != null ? ` · ${d.latency_ms} ms` : ''}{d.error ? ` · ${d.error}` : ''}{d.response ? ` · ${d.response.slice(0, 80)}` : ''}</td>
                      <td>{#if d.state === 'dead' && !d.test}<button class="btn sm" onclick={() => resend(w, d)}>Send again</button>{/if}</td>
                    </tr>
                  {/each}
                </tbody>
              </table></div>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
    <div class="row" style="margin-top:12px"><button class="btn primary" onclick={() => (adding = true)}><Icon name="plus" size={15} />Add webhook</button></div>
  {/if}
</section>

{#if adding}<WebhookForm {types} onclose={() => (adding = false)} ondone={done} />{/if}
{#if editing}<WebhookForm hook={editing} {types} onclose={() => (editing = null)} ondone={done} />{/if}

<style>
  .wl { list-style: none; margin: 12px 0 0; padding: 0; } .wl li { padding: 12px 0; border-bottom: 1px solid var(--line); } .wl li:last-child { border: 0; }
</style>
