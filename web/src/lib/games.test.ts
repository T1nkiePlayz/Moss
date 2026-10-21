import { beforeAll, describe, expect, it, vi } from 'vitest';
import { runtimeIcon } from './icons';
import type { Runtime, Server, Versions } from './api';

let handle: (m: string, p: string, b: unknown) => Promise<unknown>;
describe('game runtimes (mock backend)', () => {
  beforeAll(async () => {
    vi.stubGlobal('location', { search: '', hash: '', href: 'http://localhost/' });
    vi.stubGlobal('window', globalThis);
    handle = (await import('./mock')).handle;
  });
  it('lists the games alongside the Minecraft runtimes', async () => {
    const rts = (await handle('GET', '/runtimes', undefined)) as Runtime[];
    const games = rts.filter((r) => r.category === 'game');
    expect(games.map((g) => g.id)).toEqual(expect.arrayContaining(['bedrock', 'terraria', 'factorio', 'valheim', 'cs2', 'satisfactory']));
    expect(games.every((g) => g.default_port && g.protocol && g.platforms?.length)).toBe(true);
    expect(rts.some((r) => r.id === 'paper')).toBe(true);
    for (const g of games) expect(runtimeIcon(g)).not.toBe('cube');
    expect(runtimeIcon({ id: 'paper', category: 'plugins' })).toBe('cube');
  });

  it('seeds one server per showcased game with players and no Java UI', async () => {
    const list = (await handle('GET', '/servers', undefined)) as Server[];
    for (const rt of ['terraria', 'valheim', 'factorio', 'bedrock', 'cs2']) {
      const s = list.find((x) => x.runtime === rt);
      expect(s, rt).toBeTruthy();
      expect(s!.capabilities).not.toContain('java');
      expect(s!.capabilities).not.toContain('content');
    }
    expect(list.find((x) => x.runtime === 'cs2')!.players!.online).toBeGreaterThan(0);
    expect(list.find((x) => x.runtime === 'bedrock')!.capabilities).toContain('eula');
    expect(list.find((x) => x.runtime === 'paper')!.capabilities).toContain('java');
  });

  it('offers game versions', async () => {
    const f = (await handle('GET', '/runtimes/factorio/versions', undefined)) as Versions;
    expect(f.versions.some((v) => !v.stable)).toBe(true);
    const v = (await handle('GET', '/runtimes/valheim/versions', undefined)) as Versions;
    expect(v.recommended).toBe('latest');
  });
});
