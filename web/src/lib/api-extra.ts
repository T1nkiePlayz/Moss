// Calls used only by lazily loaded pages (game servers, maps, push); kept out of api.ts so the initial bundle stays small.
import { get, send, q, type GameModHit, type ProviderId, type ProviderInfo, type GameModsInfo, type GameUpdate, type OnlinePlayers, type Operation, type Server, type ServerMap } from './api';
const e = encodeURIComponent;

export interface ModpackHit { provider: ProviderId; project_id: string; slug: string; title: string; description: string; author: string; icon_url: string | null; downloads: number; game_versions?: string[]; loaders?: string[] }

export const apiExtra = {
  searchModpacks: (provider: ProviderId, text: string, offset = 0, limit = 20) => get<{ items: ModpackHit[]; total: number; next_offset: number | null }>(`/modpacks/search${q({ provider, q: text, offset, limit })}`, { ttl: 30000 }),
  providerList: () => get<{ providers: ProviderInfo[] }>('/content/providers', { ttl: 60000 }),
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

export interface PlayerList { id: string; name: string; kind: 'allow' | 'ban'; created_at?: string; entries: number; servers: number }
export interface PlayerListEntry { name: string; uuid: string | null; reason: string; added_by: string; added_at: string }
export interface ServerPlayerLists {
  lists: { id: string; name: string; kind: 'allow' | 'ban'; entries: number; subscribed: boolean }[];
  support: { allow: boolean; ban: boolean; note: string | null };
  pending: { allow: number; ban: number };
  allowlist_enabled: boolean | null;
}
export const playerLists = {
  list: () => get<{ items: PlayerList[] }>('/player-lists', { ttl: 0 }),
  create: (name: string, kind: 'allow' | 'ban') => send<PlayerList>('POST', '/player-lists', { name, kind }),
  remove: (id: string) => send('DELETE', `/player-lists/${e(id)}`),
  entries: (id: string) => get<{ items: PlayerListEntry[] }>(`/player-lists/${e(id)}/entries`, { ttl: 0 }),
  addEntry: (id: string, name: string, reason?: string) => send<PlayerListEntry>('POST', `/player-lists/${e(id)}/entries`, { name, reason }),
  removeEntry: (id: string, name: string) => send('DELETE', `/player-lists/${e(id)}/entries/${e(name)}`),
  forServer: (id: string) => get<ServerPlayerLists>(`/servers/${e(id)}/player-lists`, { ttl: 0 }),
  subscribe: (id: string, list_ids: string[]) => send<{ list_ids: string[] }>('PUT', `/servers/${e(id)}/player-lists`, { list_ids }),
};
