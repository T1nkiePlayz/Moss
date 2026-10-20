<script lang="ts">
  import Icon from '../Icon.svelte';
  import States from '../States.svelte';
  import { api, apiOps, errMsg, type AuditMode, type Settings } from '../../lib/api';
  import { toast, toastError } from '../../lib/stores.svelte';
  import { webhookKinds, defaultEvents } from '../../lib/notify';

  const MAX = 500;
  const audits: { id: AuditMode; label: string; hint: string }[] = [
    { id: 'redacted', label: 'Log commands, redact secrets', hint: 'Recommended. Passwords and tokens in commands such as /login are replaced.' },
    { id: 'all', label: 'Log every command as typed', hint: 'Full audit trail. Secrets typed in the console will be stored.' },
    { id: 'off', label: 'Do not log console commands', hint: 'Console input leaves no audit entry.' },
  ];

  let full = $state<Settings | null>(null);
  let err = $state('');
  let f = $state({ url: '', events: [] as string[], audit: 'redacted' as AuditMode, message: '' });
  let seed = $state('');
  let saving = $state(false);
  let testing = $state(false);
  let result = $state<{ ok: boolean; detail: string } | null>(null);

  const pick = (s: Settings) => ({ url: s.notify_webhook_url ?? '', events: [...(s.notify_events ?? defaultEvents)], audit: s.audit_console_commands ?? 'redacted', message: s.access_message ?? '' });
  async function load() {
    err = '';
    try { full = await api.settings(); f = pick(full); seed = JSON.stringify(f); } catch (e) { err = errMsg(e); }
  }
  void load();
  const dirty = $derived(JSON.stringify(f) !== seed);
  const urlDirty = $derived(!!full && f.url !== (full.notify_webhook_url ?? ''));
  const badUrl = $derived(!!f.url && !/^https:\/\//i.test(f.url.trim()));

  function toggle(kind: string, on: boolean) { f.events = on ? [...f.events, kind] : f.events.filter((x) => x !== kind); }
  async function save(e: Event) {
    e.preventDefault(); if (!full || badUrl) return;
    saving = true;
    try {
      full = await api.saveSettings({ ...full, notify_webhook_url: f.url.trim(), notify_events: f.events, audit_console_commands: f.audit, access_message: f.message });
      f = pick(full); seed = JSON.stringify(f); toast('success', 'Notification settings saved');
    } catch (er) { toastError(er, 'Could not save'); }
    saving = false;
  }
  async function test() {
    testing = true; result = null;
    try { const r = await apiOps.testWebhook(); result = { ok: r.delivered, detail: r.detail }; } catch (e) { result = { ok: false, detail: errMsg(e) }; }
    testing = false;
  }
</script>

<section class="card" aria-labelledby="nt">
  <div class="card-title" id="nt"><Icon name="bell" size={14} />Notifications and audit</div>
  {#if err}<States kind="error" text={err} retry={load} />
  {:else if !full}<States kind="loading" rows={3} />
  {:else}
    <form class="col" style="gap:20px" onsubmit={save}>
      <div class="field">
        <label for="wh">Webhook URL</label>
        <div class="row g6 wrap">
          <input id="wh" class="input grow" style="min-width:220px" type="url" inputmode="url" placeholder="https://discord.com/api/webhooks/…" bind:value={f.url} autocomplete="off" spellcheck="false" aria-describedby="wh-h" aria-invalid={badUrl ? 'true' : undefined} />
          <button type="button" class="btn" disabled={testing || !full.notify_webhook_url || urlDirty} onclick={test}><Icon name="zap" size={14} />{testing ? 'Sending…' : 'Send test'}</button>
        </div>
        <span class="hint" id="wh-h">HTTPS only. Discord webhooks get a rich embed; anything else receives JSON. Leave empty to turn webhooks off.{#if urlDirty} Save before testing.{/if}</span>
        {#if badUrl}<span class="err">The webhook URL must start with https://</span>{/if}
        <div aria-live="polite">
          {#if result}<div class="banner {result.ok ? 'info' : 'danger'}" role={result.ok ? 'status' : 'alert'}><Icon name={result.ok ? 'check-circle' : 'alert'} size={16} /><div><b>{result.ok ? 'Test message delivered' : 'Delivery failed'}</b><div class="muted">{result.detail}</div></div></div>{/if}
        </div>
      </div>

      <fieldset class="events">
        <legend class="label">Send these events to the webhook</legend>
        <div class="evgrid">
          {#each webhookKinds as k}
            <label class="ev"><input type="checkbox" class="check" checked={f.events.includes(k.kind)} onchange={(e) => toggle(k.kind, e.currentTarget.checked)} /><span>{k.label}<span class="hint" style="display:block">{k.hint}</span></span></label>
          {/each}
        </div>
        <button type="button" class="btn ghost sm" onclick={() => (f.events = [...defaultEvents])}>Reset to defaults</button>
      </fieldset>

      <div class="field">
        <label for="au">Console commands in the audit log</label>
        <select id="au" class="select" bind:value={f.audit} aria-describedby="au-h">{#each audits as a}<option value={a.id}>{a.label}</option>{/each}</select>
        <span class="hint" id="au-h">{audits.find((a) => a.id === f.audit)?.hint}</span>
      </div>

      <div class="field">
        <label for="am">Message for people who are not invited</label>
        <textarea id="am" class="textarea" rows="3" maxlength={MAX} bind:value={f.message} aria-describedby="am-h" placeholder="Ask Sam in #minecraft to add you."></textarea>
        <div class="row between"><span class="hint" id="am-h">Shown on the sign-in and "not invited" screens. Plain text.</span><span class="hint tnum" class:warn={f.message.length > MAX - 40}>{f.message.length} / {MAX}</span></div>
      </div>

      <div class="row end"><button class="btn" type="button" disabled={!dirty} onclick={() => { f = JSON.parse(seed); }}>Reset</button><button class="btn primary" disabled={!dirty || saving || badUrl}>{saving ? 'Saving…' : 'Save'}</button></div>
    </form>
  {/if}
</section>

<style>
  .events { border: 0; padding: 0; margin: 0; min-width: 0; }
  .events legend { padding: 0; margin-bottom: 8px; }
  .evgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr)); gap: 10px 18px; margin-bottom: 6px; }
  .ev { display: flex; gap: 10px; align-items: flex-start; cursor: pointer; font-size: var(--fs-sm); } .ev .check { margin-top: 2px; }
  .hint.warn { color: var(--warn); }
</style>
