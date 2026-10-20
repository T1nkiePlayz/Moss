// Pure helpers for the Webhooks settings.
import type { Webhook, WebhookDelivery, WebhookInput } from './api';

/** Local checks that mirror the server's, so the form can explain a problem before sending. */
export function webhookProblems(f: WebhookInput): Record<string, string> {
  const p: Record<string, string> = {};
  if (!(f.name ?? '').trim()) p.name = 'Give the webhook a name.';
  const url = (f.url ?? '').trim();
  if (!url) p.url = 'Enter the address that should receive events.';
  else if (!/^https:\/\//i.test(url)) p.url = 'The address must start with https://.';
  else { try { const u = new URL(url); if (u.username || u.password) p.url = 'Do not put a user name or password in the address.'; } catch { p.url = 'That does not look like a full address.'; } }
  if (!(f.events ?? []).length) p.events = 'Choose at least one event.';
  return p;
}

export function deliveryBadge(d: WebhookDelivery): { tone: 'ok' | 'info' | 'danger'; label: string } {
  if (d.state === 'delivered') return { tone: 'ok', label: 'Delivered' };
  if (d.state === 'pending') return { tone: 'info', label: d.attempts ? `Retrying (${d.attempts}/${d.max_attempts})` : 'Queued' };
  return { tone: 'danger', label: d.test ? 'Test failed' : 'Gave up' };
}

export function webhookStatus(w: Webhook): { tone: 'ok' | 'danger' | ''; label: string } {
  if (!w.enabled) return { tone: w.disabled_reason ? 'danger' : '', label: w.disabled_reason ? 'Switched off after failures' : 'Off' };
  if (w.consecutive_failures > 0) return { tone: 'danger', label: `Failing (${w.consecutive_failures} in a row)` };
  return { tone: 'ok', label: 'On' };
}

export const scopeText = (w: Webhook, names: (id: string) => string) => (w.server_ids.length ? w.server_ids.map(names).join(', ') : 'All servers');
