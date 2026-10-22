import type { Operation } from './api';

/** Operation kinds that make up a server's setup log. */
export const SETUP_KINDS = ['server.install', 'server.import'];

/** Setup operations of one server from possibly overlapping lists (later lists win), newest first. */
export function setupOps(serverId: string, lists: Operation[]): Operation[] {
  const by = new Map<string, Operation>();
  for (const o of lists) if (o.server_id === serverId && SETUP_KINDS.includes(o.kind)) by.set(o.id, o);
  return [...by.values()].sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at));
}
