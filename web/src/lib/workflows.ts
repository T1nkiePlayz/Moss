// Pure helpers for the Workflows page: action and condition choices, local checks, small formatting.
import type { Workflow, WorkflowAction, WorkflowCondition } from './api';

export const MAX_WAIT = 600;
export const MAX_ACTIONS = 20;
export const PLACEHOLDERS = ['{player}', '{server}', '{event}', '{online}', '{max}'];

export const ACTION_TYPES: { type: WorkflowAction['type']; label: string }[] = [
  { type: 'command', label: 'Send a console command' },
  { type: 'restart', label: 'Restart a server' },
  { type: 'start', label: 'Start a server' },
  { type: 'stop', label: 'Stop a server' },
  { type: 'backup', label: 'Take a backup' },
  { type: 'wait', label: 'Wait' },
  { type: 'notify', label: 'Send a notification' },
  { type: 'webhook', label: 'Call a web address' },
];

export const CONDITION_FIELDS = [
  { value: 'players.online', label: 'Players online', numeric: true },
  { value: 'players.max', label: 'Player limit', numeric: true },
  { value: 'player', label: 'Player name', numeric: false },
  { value: 'server', label: 'Server name', numeric: false },
  { value: 'detail', label: 'Any detail text', numeric: false },
];
export const CONDITION_OPS = [
  { value: '>=', label: 'is at least' }, { value: '>', label: 'is more than' }, { value: '<=', label: 'is at most' }, { value: '<', label: 'is less than' },
  { value: '==', label: 'is' }, { value: '!=', label: 'is not' }, { value: 'contains', label: 'contains' }, { value: 'not_contains', label: 'does not contain' },
];
const ORDERING = ['>', '>=', '<', '<='];

export function blankAction(type: WorkflowAction['type']): WorkflowAction {
  switch (type) {
    case 'command': return { type, server: 'trigger', command: '' };
    case 'wait': return { type, seconds: 10 };
    case 'notify': return { type, title: '', body: '' };
    case 'webhook': return { type, url: '' };
    default: return { type, server: 'trigger' };
  }
}

export const hasServer = (a: WorkflowAction): a is Extract<WorkflowAction, { server: string }> => 'server' in a;

/** A copy of `list` with the item at `i` moved one place up (-1) or down (1); unchanged at the ends. */
export function move<T>(list: T[], i: number, d: -1 | 1): T[] {
  const j = i + d;
  if (j < 0 || j >= list.length) return list;
  const out = [...list];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

/** The condition to send: numbers stay numbers for comparisons that need them. */
export function buildCondition(c: { on: boolean; field: string; op: string; value: string }): WorkflowCondition | null {
  if (!c.on) return null;
  const v = c.value.trim();
  const n = Number(v);
  const numeric = ORDERING.includes(c.op) || (CONDITION_FIELDS.find((f) => f.value === c.field)?.numeric && v !== '' && Number.isFinite(n));
  return { field: c.field, op: c.op, value: numeric && v !== '' && Number.isFinite(n) ? n : v };
}

/** Local checks that mirror the server's, so the form can say what is wrong before sending. Keys are field paths. */
export function workflowProblems(f: { name: string; trigger: string; actions: WorkflowAction[]; condition: WorkflowCondition | null }): Record<string, string> {
  const p: Record<string, string> = {};
  if (!f.name.trim()) p.name = 'Give the workflow a name.';
  if (!f.trigger) p.trigger = 'Choose what starts the workflow.';
  if (f.condition && f.condition.value === '') p.condition = 'Enter a value to compare with, or turn the condition off.';
  if (f.condition && ORDERING.includes(f.condition.op) && typeof f.condition.value !== 'number') p.condition = 'This comparison needs a number.';
  if (!f.actions.length) p.actions = 'Add at least one action.';
  if (f.actions.length > MAX_ACTIONS) p.actions = `At most ${MAX_ACTIONS} actions.`;
  f.actions.forEach((a, i) => {
    const k = `actions[${i}]`;
    if (a.type === 'command' && (!a.command.trim() || a.command.length > 200 || /[\r\n]/.test(a.command))) p[k] = 'Enter one line of up to 200 characters.';
    if (a.type === 'wait' && !(Number.isInteger(a.seconds) && a.seconds >= 1 && a.seconds <= MAX_WAIT)) p[k] = `Wait between 1 and ${MAX_WAIT} seconds.`;
    if (a.type === 'notify' && !a.title.trim()) p[k] = 'Give the notification a title.';
    if (a.type === 'webhook' && !/^https:\/\//i.test((a.url ?? '').trim())) p[k] = 'The address must start with https://.';
  });
  return p;
}

export function describeAction(a: WorkflowAction, name: (id: string) => string): string {
  const on = (s: string) => (s === 'trigger' ? 'the server in the event' : name(s));
  switch (a.type) {
    case 'start': return `Start ${on(a.server)}`;
    case 'stop': return `Stop ${on(a.server)}`;
    case 'restart': return `Restart ${on(a.server)}`;
    case 'backup': return `Back up ${on(a.server)}`;
    case 'command': return `Send “${a.command}” to ${on(a.server)}`;
    case 'wait': return `Wait ${a.seconds} s`;
    case 'notify': return `Notify “${a.title}”`;
    case 'webhook': return a.url ? 'Call a web address' : 'Deliver to a saved webhook';
  }
}

export function workflowStatus(w: Workflow): { tone: 'ok' | 'danger' | ''; label: string } {
  if (!w.enabled) return { tone: w.disabled_reason ? 'danger' : '', label: w.disabled_reason ? 'Switched off by Moss' : 'Off' };
  if (!w.last_run_at) return { tone: '', label: 'Never run' };
  return w.last_result === 'ok' ? { tone: 'ok', label: 'Last run succeeded' } : { tone: 'danger', label: 'Last run did not finish' };
}

export const scopeText = (w: Workflow, names: (id: string) => string) => (w.server_ids.length ? w.server_ids.map(names).join(', ') : 'any server you can manage');
