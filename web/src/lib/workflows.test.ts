import { describe, expect, it } from 'vitest';
import type { Workflow, WorkflowAction } from './api';
import { blankAction, buildCondition, describeAction, move, workflowProblems, workflowStatus } from './workflows';

const w = (o: Partial<Workflow> = {}): Workflow => ({ id: 'w', name: 'n', enabled: true, trigger: 'server.started', server_ids: [], condition: null, actions: [], created_by: 'u', created_at: '', last_run_at: null, last_result: null, disabled_reason: null, ...o });
const ok = { name: 'Greet', trigger: 'player.joined', condition: null, actions: [{ type: 'command', server: 'trigger', command: 'say hi {player}' } as WorkflowAction] };

describe('workflow form checks', () => {
  it('accepts a complete form', () => expect(workflowProblems(ok)).toEqual({}));
  it('says what is missing', () => {
    expect(Object.keys(workflowProblems({ name: ' ', trigger: '', condition: null, actions: [] }))).toEqual(['name', 'trigger', 'actions']);
  });
  it('checks each action', () => {
    const p = workflowProblems({ ...ok, actions: [blankAction('command'), { type: 'wait', seconds: 601 }, { type: 'notify', title: '', body: '' }, { type: 'webhook', url: 'http://x.test' }, blankAction('restart')] });
    expect(Object.keys(p)).toEqual(['actions[0]', 'actions[1]', 'actions[2]', 'actions[3]']);
    expect(workflowProblems({ ...ok, actions: [{ type: 'command', server: 'trigger', command: 'a\nb' }] })['actions[0]']).toBeTruthy();
  });
  it('checks the condition', () => {
    expect(workflowProblems({ ...ok, condition: { field: 'player', op: '==', value: '' } }).condition).toMatch(/value/);
    expect(workflowProblems({ ...ok, condition: { field: 'players.online', op: '>=', value: 'x' } }).condition).toMatch(/number/);
    expect(workflowProblems({ ...ok, condition: { field: 'players.online', op: '>=', value: 3 } })).toEqual({});
  });
});

describe('condition building', () => {
  it('sends nothing when off', () => expect(buildCondition({ on: false, field: 'player', op: '==', value: 'x' })).toBeNull());
  it('keeps numbers numeric where they matter', () => {
    expect(buildCondition({ on: true, field: 'players.online', op: '>=', value: ' 5 ' })).toEqual({ field: 'players.online', op: '>=', value: 5 });
    expect(buildCondition({ on: true, field: 'player', op: '==', value: '5' })).toEqual({ field: 'player', op: '==', value: '5' });
    expect(buildCondition({ on: true, field: 'players.online', op: '>=', value: 'many' })).toEqual({ field: 'players.online', op: '>=', value: 'many' });
  });
});

describe('actions', () => {
  it('reorders without touching the original', () => {
    const l = ['a', 'b', 'c'];
    expect(move(l, 1, -1)).toEqual(['b', 'a', 'c']);
    expect(move(l, 1, 1)).toEqual(['a', 'c', 'b']);
    expect(move(l, 0, -1)).toBe(l);
    expect(move(l, 2, 1)).toBe(l);
    expect(l).toEqual(['a', 'b', 'c']);
  });
  it('describes steps in words', () => {
    const name = (id: string) => (id === 's1' ? 'Survival' : 'gone');
    expect(describeAction({ type: 'restart', server: 'trigger' }, name)).toBe('Restart the server in the event');
    expect(describeAction({ type: 'backup', server: 's1' }, name)).toBe('Back up Survival');
    expect(describeAction({ type: 'wait', seconds: 30 }, name)).toBe('Wait 30 s');
  });
});

describe('status', () => {
  it('names the states', () => {
    expect(workflowStatus(w()).label).toBe('Never run');
    expect(workflowStatus(w({ last_run_at: 'x', last_result: 'ok' })).tone).toBe('ok');
    expect(workflowStatus(w({ last_run_at: 'x', last_result: 'step 1 failed' })).tone).toBe('danger');
    expect(workflowStatus(w({ enabled: false })).label).toBe('Off');
    expect(workflowStatus(w({ enabled: false, disabled_reason: 'lost access' })).tone).toBe('danger');
  });
});
