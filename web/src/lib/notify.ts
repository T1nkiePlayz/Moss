// Notification inbox helpers (pure).
import type { AppNotification, NotificationKind, Severity } from './api';

export const kindLabel: Record<NotificationKind, string> = {
  crash: 'Crash', crash_loop: 'Crash loop', backup_failed: 'Backup failed', disk_low: 'Low disk', operation_failed: 'Operation failed',
  restart_failed: 'Restart failed', update_available: 'Update available', security: 'Security', webhook_disabled: 'Webhook switched off', player_join: 'Player joined',
};
/** Event kinds an admin can subscribe the webhook to, in display order, with the contract's defaults. */
export const webhookKinds: { kind: NotificationKind; label: string; hint: string }[] = [
  { kind: 'crash', label: 'Server crashed', hint: 'A server exited unexpectedly.' },
  { kind: 'crash_loop', label: 'Crash loop', hint: 'Automatic restarts were paused after repeated crashes.' },
  { kind: 'backup_failed', label: 'Backup failed', hint: 'A manual or scheduled backup did not complete.' },
  { kind: 'disk_low', label: 'Disk space low', hint: 'The data volume is nearly full.' },
  { kind: 'restart_failed', label: 'Restart failed', hint: 'A scheduled or automatic restart could not complete.' },
  { kind: 'operation_failed', label: 'Operation failed', hint: 'Installs, updates and restores that fail.' },
  { kind: 'update_available', label: 'Update available', hint: 'A new Moss or server version is ready.' },
  { kind: 'webhook_disabled', label: 'Webhook switched off', hint: 'An event webhook failed too many times and was turned off.' },
  { kind: 'security', label: 'Security', hint: 'Sign-in and access events worth reviewing.' },
  { kind: 'player_join', label: 'Player joined', hint: 'Someone joins a server. Off by default; can be frequent.' },
];
export const defaultEvents: NotificationKind[] = ['crash', 'crash_loop', 'backup_failed', 'disk_low', 'restart_failed'];

const rank: Record<Severity, number> = { critical: 2, warn: 1, info: 0 };
export const severityIcon: Record<Severity, string> = { critical: 'x-circle', warn: 'alert', info: 'info' };
export const severityLabel: Record<Severity, string> = { critical: 'Critical', warn: 'Warning', info: 'Info' };
export const worst = (list: AppNotification[]): Severity => list.reduce<Severity>((a, n) => (rank[n.severity] > rank[a] ? n.severity : a), 'info');

export interface NotifGroup { label: string; items: AppNotification[] }
const dayStart = (t: number) => { const d = new Date(t); d.setHours(0, 0, 0, 0); return d.getTime(); };
export function dayLabel(t: number, now = Date.now()): string {
  const days = Math.round((dayStart(now) - dayStart(t)) / 86400000);
  if (days <= 0) return 'Today';
  if (days === 1) return 'Yesterday';
  return new Date(t).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
}
/** Newest first, grouped by calendar day (Today / Yesterday / date). Within a day, unread critical items float up. */
export function groupNotifications(items: AppNotification[], now = Date.now()): NotifGroup[] {
  const sorted = [...items].sort((a, b) => Date.parse(b.time) - Date.parse(a.time));
  const out: NotifGroup[] = [];
  for (const n of sorted) {
    const label = dayLabel(Date.parse(n.time), now);
    let g = out.find((x) => x.label === label);
    if (!g) out.push((g = { label, items: [] }));
    g.items.push(n);
  }
  for (const g of out) g.items.sort((a, b) => Number(a.read) - Number(b.read) || rank[b.severity] - rank[a.severity] || Date.parse(b.time) - Date.parse(a.time));
  return out;
}
/** Merge an incoming (live) notification into the list, newest first, replacing the same id. */
export function upsertNotification(list: AppNotification[], n: AppNotification, max = 100): AppNotification[] {
  return [n, ...list.filter((x) => x.id !== n.id)].sort((a, b) => Date.parse(b.time) - Date.parse(a.time)).slice(0, max);
}
export const badgeText = (n: number) => (n > 99 ? '99+' : String(n));
