// Dev-only mock of game mods (Thunderstore, Factorio portal, Steam Workshop). Imported by mock.ts only.
import type * as A from './api';
import { ApiError } from './api';

export const NONE = Symbol('not a game-mods route');
export interface GCtx { uid: () => string; now: () => string; op: (kind: string, title: string, sid: string | null, stages: string[], opts?: { done?: () => void }) => A.Operation }

const HITS: Record<string, A.GameModHit[]> = {
  valheim: [
    { id: 'RandyKnapp-EpicLoot', title: 'Epic Loot', author: 'RandyKnapp', summary: 'Adds loot drops, magic items, and enchanting to Valheim.', downloads: 2321581, icon_url: null, url: 'https://thunderstore.io/c/valheim/p/RandyKnapp/EpicLoot/' },
    { id: 'ValheimModding-Jotunn', title: 'Jotunn', author: 'ValheimModding', summary: 'Valheim Library for adding custom content.', downloads: 9100000, icon_url: null, url: 'https://thunderstore.io/c/valheim/p/ValheimModding/Jotunn/' },
    { id: 'Smoothbrain-Backpacks', title: 'Backpacks', author: 'Smoothbrain', summary: 'Adds a backpack to your equipment.', downloads: 410000, icon_url: null, url: 'https://thunderstore.io/c/valheim/p/Smoothbrain/Backpacks/' },
  ],
  factorio: [
    { id: 'Krastorio2', title: 'Krastorio 2', author: 'raiguard', summary: 'An overhaul mod focusing on end-game technologies.', downloads: 1200000, icon_url: null, version: '2.1.4', url: 'https://mods.factorio.com/mod/Krastorio2' },
    { id: 'flib', title: 'Factorio Library', author: 'raiguard', summary: 'A library of reusable functions for mods.', downloads: 3400000, icon_url: null, version: '0.16.2', url: 'https://mods.factorio.com/mod/flib' },
    { id: 'AutoTrash', title: 'AutoTrash', author: 'Octav', summary: 'Automatically trash unwanted items.', downloads: 800000, icon_url: null, version: '5.0.1', url: 'https://mods.factorio.com/mod/AutoTrash' },
  ],
};
const state = new Map<string, { mods: A.GameMod[]; workshop: string | null; username: string | null; token: boolean }>();
const bad = (field: string, message: string) => new ApiError(422, 'validation', message, [{ path: field, message }]);

function of(s: A.Server) {
  let v = state.get(s.id);
  if (!v) {
    v = { mods: [], workshop: null, username: null, token: false };
    if (s.runtime === 'valheim') v.mods.push({ id: 'denikson-BepInExPack_Valheim', title: 'BepInExPack Valheim', version: '5.4.2202', enabled: true, auto: true, requires: [], url: 'https://thunderstore.io/c/valheim/p/denikson/BepInExPack_Valheim/', installed_at: new Date().toISOString() });
    state.set(s.id, v);
  }
  return v;
}

export async function handleGameMods(c: GCtx, s: A.Server, sub: string, method: string, b: Record<string, any>, qs: URLSearchParams): Promise<unknown> {
  if (!/^\/game-mods(\/|$)/.test(sub)) return NONE;
  const v = of(s);
  const running = s.state === 'running' || s.state === 'starting';
  const kind = s.runtime === 'valheim' ? 'thunderstore' : s.runtime === 'factorio' ? 'factorio' : 'workshop';
  const stopped = () => { if (running) throw new ApiError(409, 'server.running', 'stop the server before changing its mods'); };
  if (sub === '/game-mods' && method === 'GET') {
    const out: A.GameModsInfo = { kind, runtime: s.runtime, installed: v.mods, running };
    if (kind === 'thunderstore') out.bepinex = v.mods.some((m) => m.id === 'denikson-BepInExPack_Valheim');
    if (kind === 'factorio') out.credentials = { username: v.username, has_token: v.token };
    if (kind === 'workshop') {
      const field = s.runtime === 'gmod' ? 'collection' : 'map';
      out.workshop = { field, id: v.workshop, url: v.workshop ? `https://steamcommunity.com/sharedfiles/filedetails/?id=${v.workshop}` : null, has_api_key: false };
    }
    return out;
  }
  if (sub === '/game-mods/search') {
    const t = (qs.get('q') ?? '').toLowerCase();
    return { results: (HITS[s.runtime] ?? []).filter((h) => !t || h.title.toLowerCase().includes(t) || h.id.toLowerCase().includes(t)) };
  }
  if (sub === '/game-mods/updates') return { updates: v.mods.filter((m) => m.id === 'flib').map((m) => ({ id: m.id, installed: m.version, latest: '0.16.3' })) };
  if (sub === '/game-mods/credentials' && method === 'PUT') {
    v.username = b.username || null;
    if (!b.username) v.token = false; else if (b.token) v.token = true;
    return { ok: true };
  }
  if (sub === '/game-mods/workshop' && method === 'PUT') {
    const val = String(b.collection ?? b.map ?? '').trim();
    if (val && !/^\d+$/.test(val)) throw bad(s.runtime === 'gmod' ? 'collection' : 'map', 'must be digits only');
    stopped();
    v.workshop = val || null;
    return { ok: true, id: v.workshop };
  }
  if (sub === '/game-mods' && method === 'POST') {
    stopped();
    const hit = (HITS[s.runtime] ?? []).find((h) => h.id === b.id);
    if (!hit) throw bad('id', 'not a valid mod id for this game');
    if (kind === 'factorio' && !(v.username && v.token)) throw new ApiError(409, 'game_mods.credentials', 'Factorio downloads need your mod portal username and token; add them in the Mods tab');
    const stages = ['Resolving dependencies', `Installing ${hit.id}`];
    return { operation: c.op('gamemods.install', `Install ${hit.id} on ${s.name}`, s.id, stages, { done: () => {
      const had = v.mods.find((m) => m.id === hit.id);
      const entry: A.GameMod = { id: hit.id, title: hit.title, version: hit.version ?? '1.0.0', enabled: true, auto: false, requires: [], url: hit.url, installed_at: c.now() };
      if (had) Object.assign(had, entry); else v.mods.push(entry);
    } }) };
  }
  const m = sub.match(/^\/game-mods\/([^/]+)$/);
  if (m) {
    const id = decodeURIComponent(m[1]);
    const mod = v.mods.find((x) => x.id === id);
    if (!mod) throw new ApiError(404, 'not_found', 'mod not found');
    stopped();
    if (method === 'PATCH') { mod.enabled = !!b.enabled; return { ok: true }; }
    if (method === 'DELETE') {
      const by = v.mods.filter((x) => x.requires.includes(id)).map((x) => x.id);
      if (by.length) throw new ApiError(409, 'game_mods.required', `${mod.title} is required by ${by.join(', ')}; remove those first`);
      v.mods.splice(v.mods.indexOf(mod), 1);
      return;
    }
  }
  throw new ApiError(404, 'mock.not_found', `Mock has no route for ${method} ${sub}`);
}
