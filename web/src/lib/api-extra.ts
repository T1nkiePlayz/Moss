// Calls used only by lazily loaded pages (game servers, maps, push); kept out of api.ts so the initial bundle stays small.
import { get, send, q, type GameModHit, type GameModsInfo, type GameUpdate, type OnlinePlayers, type Operation, type Server, type ServerMap } from './api';
const e = encodeURIComponent;

export const apiExtra = {
  pushKey: () => get<{ key: string }>('/push/key', { ttl: 0 }),
  pushSubscribe: (b: PushSubscriptionJSON) => send('POST', '/push/subscriptions', b),
  pushUnsubscribe: (endpoint: string) => send('DELETE', '/push/subscriptions', { endpoint }),
  map: (id: string) => get<ServerMap>(`/servers/${e(id)}/map`, { ttl: 0 }),
  onlinePlayers: (id: string) => get<OnlinePlayers>(`/servers/${e(id)}/players/online`),
  kickPlayer: (id: string, name: string, reason?: string) => send('POST', `/servers/${e(id)}/players/${e(name)}/kick`, { reason }),
  banPlayer: (id: string, name: string, reason?: string) => send('POST', `/servers/${e(id)}/players/${e(name)}/ban`, { reason }),
  gameUpdate: (id: string, refresh = false) => get<GameUpdate>(`/servers/${e(id)}/game-update${(refresh ? '?refresh=true' : '')}`, { ttl: 0 }),
  startGameUpdate: (id: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/game-update`),
};

export const gameMods = {
  info: (id: string) => get<GameModsInfo>(`/servers/${e(id)}/game-mods`, { ttl: 0 }),
  search: (id: string, text: string) => get<{ results: GameModHit[] }>(`/servers/${e(id)}/game-mods/search${q({ q: text })}`, { ttl: 30000 }),
  updates: (id: string) => get<{ updates: { id: string; installed: string; latest: string }[] }>(`/servers/${e(id)}/game-mods/updates`, { ttl: 0 }),
  install: (id: string, mod: string, version?: string) => send<{ operation: Operation }>('POST', `/servers/${e(id)}/game-mods`, { id: mod, version }),
  setEnabled: (id: string, mod: string, enabled: boolean) => send('PATCH', `/servers/${e(id)}/game-mods/${e(mod)}`, { enabled }),
  remove: (id: string, mod: string) => send('DELETE', `/servers/${e(id)}/game-mods/${e(mod)}`),
  setWorkshop: (id: string, b: { collection?: string; map?: string }) => send('PUT', `/servers/${e(id)}/game-mods/workshop`, b),
  setCredentials: (id: string, username: string, token?: string) => send('PUT', `/servers/${e(id)}/game-mods/credentials`, { username, token }),
};

export const isGameServer = (s: Pick<Server, 'runtime' | 'capabilities'>) => !s.capabilities.includes('java') && s.runtime !== 'custom' && s.runtime !== 'sim';
