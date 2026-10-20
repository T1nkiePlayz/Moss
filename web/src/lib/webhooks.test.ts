import { describe, expect, it } from 'vitest';
import type { Webhook, WebhookDelivery } from './api';
import { deliveryBadge, scopeText, webhookProblems, webhookStatus } from './webhooks';

const w = (o: Partial<Webhook> = {}): Webhook => ({ id: 'w', name: 'n', url: 'https://x.test', events: ['server.started'], server_ids: [], enabled: true, disabled_reason: null, consecutive_failures: 0, secret: 'set', pending: 0, last_delivery: null, created_at: '', ...o });
const d = (o: Partial<WebhookDelivery> = {}): WebhookDelivery => ({ id: 'd', event_id: 'e', type: 't', test: false, state: 'delivered', attempts: 1, max_attempts: 8, created_at: '', finished_at: null, next_attempt_at: null, status_code: 200, latency_ms: 5, response: null, error: null, ...o });

describe('webhook form checks', () => {
  it('accepts a complete form', () => expect(webhookProblems({ name: 'Bot', url: 'https://example.com/hook', events: ['server.crashed'] })).toEqual({}));
  it('explains what is missing', () => {
    expect(Object.keys(webhookProblems({}))).toEqual(['name', 'url', 'events']);
    expect(webhookProblems({ name: 'a', url: 'http://example.com', events: ['x'] }).url).toMatch(/https/);
    expect(webhookProblems({ name: 'a', url: 'https://u:p@example.com', events: ['x'] }).url).toMatch(/user name/);
    expect(webhookProblems({ name: 'a', url: 'https://', events: ['x'] }).url).toBeTruthy();
  });
});

describe('badges', () => {
  it('names delivery states', () => {
    expect(deliveryBadge(d()).label).toBe('Delivered');
    expect(deliveryBadge(d({ state: 'pending', attempts: 2 }))).toEqual({ tone: 'info', label: 'Retrying (2/8)' });
    expect(deliveryBadge(d({ state: 'pending', attempts: 0 })).label).toBe('Queued');
    expect(deliveryBadge(d({ state: 'dead' })).label).toBe('Gave up');
    expect(deliveryBadge(d({ state: 'dead', test: true })).label).toBe('Test failed');
  });
  it('names webhook states', () => {
    expect(webhookStatus(w()).label).toBe('On');
    expect(webhookStatus(w({ consecutive_failures: 3 })).tone).toBe('danger');
    expect(webhookStatus(w({ enabled: false, disabled_reason: 'too many' })).label).toMatch(/after failures/);
    expect(webhookStatus(w({ enabled: false })).label).toBe('Off');
  });
  it('describes scope', () => {
    expect(scopeText(w(), (i) => i)).toBe('All servers');
    expect(scopeText(w({ server_ids: ['a', 'b'] }), (i) => i.toUpperCase())).toBe('A, B');
  });
});
