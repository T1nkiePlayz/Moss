// Dev-only mock of event workflows. Imported by mock.ts only.
import type * as A from './api';
import { ApiError } from './api';
import { TYPES } from './mock-webhooks';
import { workflowProblems } from './workflows';

export const NONE = Symbol('not a workflow route');
export interface WfCtx { uid: () => string; now: () => string; servers: A.Server[] }

const bad = (field: string, message: string) => new ApiError(422, 'validation', message, [{ path: field, message }]);
const list: A.Workflow[] = [];
const runs = new Map<string, A.WorkflowRun[]>();
let seeded = false;

function seed(c: WfCtx) {
  if (seeded) return; seeded = true;
  const ago = (min: number) => new Date(Date.now() - min * 60000).toISOString();
  const wf = (o: Partial<A.Workflow> & Pick<A.Workflow, 'id' | 'name' | 'trigger' | 'actions'>): A.Workflow => ({ enabled: true, server_ids: [], condition: null, created_by: 'dev', created_at: '2026-02-01T10:00:00Z', last_run_at: null, last_result: null, disabled_reason: null, ...o });
  list.push(
    wf({ id: 'wf-welcome', name: 'Welcome new players', trigger: 'player.joined', actions: [{ type: 'command', server: 'trigger', command: 'say Welcome {player}! {online}/{max} online.' }], last_run_at: ago(12), last_result: 'ok' }),
    wf({ id: 'wf-crash', name: 'Restart after a crash', trigger: 'server.crashed', actions: [{ type: 'notify', title: '{server} crashed, restarting', body: '' }, { type: 'wait', seconds: 30 }, { type: 'restart', server: 'trigger' }], last_run_at: ago(300), last_result: 'ok' }),
    wf({ id: 'wf-backup', name: 'Back up after every stop', trigger: 'server.stopped', actions: [{ type: 'backup', server: 'trigger' }], last_run_at: ago(1500), last_result: 'step 1 (backup): not enough free disk space' }),
    wf({ id: 'wf-health', name: 'Tell the team about critical findings', trigger: 'health.critical', enabled: false, disabled_reason: 'The creator no longer has the access this workflow needs: missing permission server.power (server.power on server s1)', actions: [{ type: 'notify', title: 'Critical: {event}', body: '{server}' }, { type: 'webhook', url: 'https://example.com/moss-alerts' }], last_run_at: ago(4000), last_result: 'creator lost access; workflow disabled' }),
  );
  const ev = (type: string, name: string, data: Record<string, unknown> = {}): A.WorkflowRun['event'] => ({ type, server: { id: c.servers[0]?.id ?? 's1', name }, data });
  runs.set('wf-welcome', [0, 1, 2].map((i) => ({ id: c.uid(), at: ago(12 + i * 40), event: ev('player.joined', c.servers[0]?.name ?? 'Survival', { player: ['Steve', 'Alex', 'Sam'][i], online: i + 2, max: 20 }), ok: true, log: ['step 1 ok: sent a command to ' + (c.servers[0]?.name ?? 'Survival')] })));
  runs.set('wf-crash', [{ id: c.uid(), at: ago(300), event: ev('server.crashed', 'Modded', { detail: 'exit code 1' }), ok: true, log: ['step 1 ok: notification sent', 'step 2 ok: waited 30s', 'step 3 ok: restarting Modded'] }]);
  runs.set('wf-backup', [{ id: c.uid(), at: ago(1500), event: ev('server.stopped', 'Survival'), ok: false, log: ['failed: step 1 (backup): not enough free disk space'] }]);
  runs.set('wf-health', []);
}

const find = (id: string) => list.find((w) => w.id === id) ?? (() => { throw new ApiError(404, 'not_found', 'workflow not found'); })();

function check(c: WfCtx, b: A.WorkflowInput, cur?: A.Workflow) {
  const f = { name: b.name ?? cur?.name ?? '', trigger: b.trigger ?? cur?.trigger ?? '', actions: b.actions ?? cur?.actions ?? [], condition: b.condition === undefined ? cur?.condition ?? null : b.condition };
  const p = workflowProblems(f);
  const first = Object.entries(p)[0];
  if (first) throw bad(first[0], first[1]);
  if (!TYPES.some((t) => t.type === f.trigger)) throw bad('trigger', 'Choose one of the known events.');
  for (const s of b.server_ids ?? []) if (!c.servers.some((x) => x.id === s)) throw bad('server_ids', 'One of the chosen servers no longer exists.');
}

export async function handleWorkflows(c: WfCtx, method: string, p: string, b: Record<string, any>): Promise<unknown> {
  if (!p.startsWith('/workflows')) return NONE;
  seed(c);
  let m: RegExpMatchArray | null;
  const M = (re: RegExp) => (m = p.match(re));
  if (p === '/workflows' && method === 'GET') return { items: list, event_types: TYPES };
  if (p === '/workflows' && method === 'POST') {
    check(c, b);
    const w: A.Workflow = { id: c.uid(), name: String(b.name).trim(), enabled: b.enabled ?? true, trigger: b.trigger, server_ids: b.server_ids ?? [], condition: b.condition ?? null, actions: b.actions, created_by: 'dev', created_at: c.now(), last_run_at: null, last_result: null, disabled_reason: null };
    list.push(w); runs.set(w.id, []);
    return w;
  }
  if (M(/^\/workflows\/([^/]+)$/)) {
    const w = find(m![1]);
    if (method === 'PATCH') {
      check(c, b, w);
      if (b.enabled === true) w.disabled_reason = null;
      for (const k of ['name', 'enabled', 'trigger', 'server_ids', 'condition', 'actions'] as const) if (b[k] !== undefined) (w as any)[k] = b[k];
      return w;
    }
    if (method === 'DELETE') { list.splice(list.indexOf(w), 1); runs.delete(w.id); return; }
    return w;
  }
  if (M(/^\/workflows\/([^/]+)\/runs$/)) { find(m![1]); return { items: runs.get(m![1]) ?? [] }; }
  if (M(/^\/workflows\/([^/]+)\/test$/)) {
    const w = find(m![1]);
    await new Promise((r) => setTimeout(r, 250));
    const srv = c.servers.find((s) => s.id === (b.server_id ?? w.server_ids[0])) ?? c.servers[0];
    const name = (id: string) => (id === 'trigger' ? srv?.name ?? 'Sample server' : c.servers.find((s) => s.id === id)?.name ?? null);
    const steps: A.WorkflowStep[] = w.actions.map((a, i) => {
      const target = 'server' in a ? name(a.server) : null;
      const problem = 'server' in a && !target ? 'server not found' : null;
      const text = a.type === 'command' ? `Send to ${target}: ${a.command.replace('{player}', 'Steve').replace('{server}', target ?? '').replace('{online}', '3').replace('{max}', '20')}`
        : a.type === 'wait' ? `Wait ${a.seconds} seconds` : a.type === 'notify' ? `Notify: ${a.title.replace('{server}', target ?? srv?.name ?? '')}` : a.type === 'webhook' ? 'Call a webhook address' : `${a.type[0].toUpperCase()}${a.type.slice(1)} ${target}`;
      return { index: i + 1, action: a.type, description: text, server: target, allowed: !problem, problem };
    });
    return { event: { type: w.trigger, server: srv ? { id: srv.id, name: srv.name } : null, data: { player: 'Steve', online: 3, max: 20 } }, trigger_matches: true, condition_holds: true, would_run: steps.every((s) => s.allowed), executed: false, steps } satisfies A.WorkflowPlan;
  }
  return NONE;
}
