// API token display helpers (pure).
import type { TokenScope } from './api';
import { relTime } from './format';
export const expiryChoices: { days: number | null; label: string }[] = [
  { days: 7, label: '7 days' }, { days: 30, label: '30 days' }, { days: 90, label: '90 days' }, { days: 365, label: '1 year' }, { days: null, label: 'Never expires' },
];
export const scopeInfo: Record<TokenScope, { label: string; hint: string }> = {
  read: { label: 'Read only', hint: 'Can only make GET requests. Safe for dashboards and monitoring.' },
  full: { label: 'Full access', hint: 'Acts with your role and permissions. It still cannot manage tokens or users.' },
};
export interface Expiry { text: string; tone: 'ok' | 'warn' | 'danger' | '' ; expired: boolean }
/** "Expires in 12 days" / "Expired" / "Never expires"; warns inside the last 7 days. */
export function expiryStatus(iso: string | null, now = Date.now()): Expiry {
  if (!iso) return { text: 'Never expires', tone: '', expired: false };
  const t = Date.parse(iso);
  if (isNaN(t)) return { text: '—', tone: '', expired: false };
  const days = Math.ceil((t - now) / 86400000);
  if (t <= now) return { text: 'Expired', tone: 'danger', expired: true };
  if (days <= 1) return { text: 'Expires today', tone: 'warn', expired: false };
  return { text: `Expires in ${days} days`, tone: days <= 7 ? 'warn' : '', expired: false };
}
export const maskedPrefix = (p: string) => `${p}…`;
/** Compact "3 min. ago" for recent times, a plain date beyond a month (keeps table cells narrow). */
export function whenShort(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (isNaN(t)) return '—';
  return Math.abs(now - t) < 30 * 86400000 ? relTime(t, now) : new Date(t).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}
