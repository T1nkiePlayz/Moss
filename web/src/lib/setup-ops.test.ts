import { describe, expect, it } from 'vitest';
import { setupOps } from './setup-ops';
import type { Operation } from './api';

const op = (id: string, kind: string, created_at: string, server_id = 's1', state = 'running') => ({ id, kind, created_at, server_id, state }) as Operation;

describe('setupOps', () => {
  it('keeps only this server setup ops, newest first, later lists win', () => {
    const r = setupOps('s1', [op('a', 'server.install', '2026-01-01T00:00:00Z'), op('b', 'content.install', '2026-01-03T00:00:00Z'), op('c', 'server.import', '2026-01-02T00:00:00Z'), op('d', 'server.install', '2026-01-04T00:00:00Z', 's2'), op('a', 'server.install', '2026-01-01T00:00:00Z', 's1', 'completed')]);
    expect(r.map((o) => o.id)).toEqual(['c', 'a']);
    expect(r[1].state).toBe('completed');
  });
});
