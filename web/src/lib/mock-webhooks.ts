// Dev-only mock of event webhooks. Imported by mock.ts only.
import type * as A from './api';
import { ApiError } from './api';

export const NONE = Symbol('not a webhook route');
export interface WCtx { uid: () => string; now: () => string; servers: A.Server[] }

const TYPES: A.WebhookEventType[] = [
  ['server.started', 'A server finished starting and is ready'], ['server.stopped', 'A server stopped'], ['server.crashed', "A server's process died unexpectedly"],
  ['server.restarted', 'A server came back up shortly after going down'], ['backup.completed', 'A backup finished'], ['backup.failed', 'A backup or a remote copy of one failed'],
  ['operation.completed', 'Any long-running job finished'], ['operation.failed', 'Any long-running job failed'], ['player.joined', 'A player joined (polled every few seconds)'],
  ['player.left', 'A player left (same limits as player.joined)'], ['content.installed', 'A mod, plugin or modpack was installed'], ['update.available', 'Updates are available for installed content'],
  ['health.critical', 'A critical finding appeared in the health checks'],
].map(([type, description]) => ({ type, description }));

const bad = (field: string, message: string) => new ApiError(422, 'validation', message, [{ path: field, message }]);
const secret = () => `whsec_${Array.from({ length: 48 }, () => 'abcdefghijklmnopqrstuvwxyz0123456789'[Math.floor(Math.random() * 36)]).join('')}`;
const hooks: A.Webhook[] = [];
const log = new Map<string, A.WebhookDelivery[]>();
let seeded = false;

function seed(c: WCtx) {
  if (seeded) return; seeded = true;
  const h: A.Webhook = { id: 'wh-bot', name: 'Discord bot', url: 'https://bot.example.com/moss', events: ['server.crashed', 'backup.failed', 'player.joined'], server_ids: [], enabled: true, disabled_reason: null, consecutive_failures: 0, secret: 'set', pending: 0, last_delivery: null, created_at: '2026-01-05T10:00:00Z' };
  const d = (i: number, o: Partial<A.WebhookDelivery>): A.WebhookDelivery => ({ id: c.uid(), event_id: c.uid(), type: 'player.joined', test: false, state: 'delivered', attempts: 1, max_attempts: 8, created_at: new Date(Date.now() - i * 600000).toISOString(), finished_at: new Date(Date.now() - i * 600000).toISOString(), next_attempt_at: null, status_code: 204, latency_ms: 80 + i * 7, response: '', error: null, ...o });
  log.set(h.id, [d(1, {}), d(2, { type: 'server.crashed', state: 'dead', attempts: 8, status_code: 502, response: 'Bad gateway', error: 'the receiver answered HTTP 502' }), d(3, { type: 'backup.failed', status_code: 200, response: 'ok' })]);
  h.last_delivery = { state: 'delivered', at: log.get(h.id)![0].created_at, status_code: 204 };
  hooks.push(h);
}

function check(c: WCtx, b: A.WebhookInput, partial = false) {
  if (!partial || b.name !== undefined) if (!(b.name ?? '').trim()) throw bad('name', 'Give the webhook a name of up to 60 characters.');
  if (!partial || b.url !== undefined) {
    const u = (b.url ?? '').trim();
    if (!/^https:\/\//i.test(u)) throw bad('url', 'The address must start with https://.');
    if (/^https:\/\/(localhost|127\.|10\.|192\.168\.)/i.test(u)) throw bad('url', 'That is a private or local address. Moss only delivers to public addresses unless the administrator allows private webhooks.');
  }
  if (!partial || b.events !== undefined) if (!(b.events ?? []).length) throw bad('events', 'Choose at least one event.');
  for (const e of b.events ?? []) if (!TYPES.some((t) => t.type === e)) throw bad('events', `Unknown event ${e}.`);
  for (const s of b.server_ids ?? []) if (!c.servers.some((x) => x.id === s)) throw bad('server_ids', 'One of the chosen servers no longer exists.');
}
const find = (id: string) => hooks.find((h) => h.id === id) ?? (() => { throw new ApiError(404, 'not_found', 'webhook not found'); })();

export async function handleWebhooks(c: WCtx, method: string, p: string, b: Record<string, any>): Promise<unknown> {
  if (!p.startsWith('/webhooks')) return NONE;
  seed(c);
  let m: RegExpMatchArray | null;
  const M = (re: RegExp) => (m = p.match(re));
  if (p === '/webhooks' && method === 'GET') return { items: hooks.map((h) => ({ ...h, pending: (log.get(h.id) ?? []).filter((d) => d.state === 'pending').length })), event_types: TYPES };
  if (p === '/webhooks' && method === 'POST') {
    check(c, b);
    const h: A.Webhook = { id: c.uid(), name: String(b.name).trim(), url: String(b.url).trim(), events: b.events, server_ids: b.server_ids ?? [], enabled: b.enabled ?? true, disabled_reason: null, consecutive_failures: 0, secret: 'set', pending: 0, last_delivery: null, created_at: c.now() };
    hooks.push(h); log.set(h.id, []);
    return { ...h, secret: secret() };
  }
  if (M(/^\/webhooks\/([^/]+)$/)) {
    const h = find(m![1]);
    if (method === 'PATCH') {
      check(c, b, true);
      if (b.enabled === true && !h.enabled) { h.consecutive_failures = 0; h.disabled_reason = null; }
      for (const k of ['name', 'url', 'events', 'server_ids', 'enabled'] as const) if (b[k] !== undefined) (h as any)[k] = b[k];
      return h;
    }
    hooks.splice(hooks.indexOf(h), 1); log.delete(h.id);
    return;
  }
  if (M(/^\/webhooks\/([^/]+)\/rotate-secret$/)) { find(m![1]); return { secret: secret() }; }
  if (M(/^\/webhooks\/([^/]+)\/test$/)) {
    const h = find(m![1]);
    await new Promise((r) => setTimeout(r, 500));
    const fail = /fail|down/i.test(h.url);
    const d: A.WebhookDelivery = { id: c.uid(), event_id: c.uid(), type: 'test', test: true, state: fail ? 'dead' : 'delivered', attempts: 1, max_attempts: 8, created_at: c.now(), finished_at: c.now(), next_attempt_at: null,
      status_code: fail ? 500 : 204, latency_ms: fail ? 40 : 95, response: fail ? 'Internal Server Error' : '', error: fail ? 'the receiver answered HTTP 500' : null };
    log.set(h.id, [d, ...(log.get(h.id) ?? [])]);
    return { delivery: d };
  }
  if (M(/^\/webhooks\/([^/]+)\/deliveries$/)) { find(m![1]); return { items: log.get(m![1]) ?? [] }; }
  if (M(/^\/webhooks\/([^/]+)\/deliveries\/([^/]+)\/retry$/)) {
    const d = (log.get(find(m![1]).id) ?? []).find((x) => x.id === m![2]);
    if (!d || d.state !== 'dead' || d.test) throw new ApiError(409, 'webhook.not_retryable', 'Only deliveries that gave up can be sent again.');
    Object.assign(d, { state: 'delivered', attempts: 1, status_code: 204, error: null, response: '', finished_at: c.now() });
    return { delivery: d };
  }
  return NONE;
}
