// Shared user-level actions (used by cards, tabs, command palette, health cards).
import { api, type Finding, type Server } from './api';
import { confirmDialog, toast, toastError, trackOp, serverById } from './stores.svelte';
import { navigate } from './router.svelte';
import { serverHref } from './route';

export type PowerAction = 'start' | 'stop' | 'restart' | 'kill';

export const consequence: Record<PowerAction, string> = {
  start: 'Starts the server process.',
  stop: 'Sends a graceful stop; players are disconnected after world save.',
  restart: 'Stops, then starts the server. Players are disconnected for about a minute.',
  kill: 'Force-kills the process. Unsaved world data may be lost.',
};

export async function power(s: Server, a: PowerAction, skipConfirm = false): Promise<boolean> {
  if (a !== 'start' && !skipConfirm) {
    const online = s.players?.online ?? 0;
    const r = await confirmDialog({
      title: `${a === 'kill' ? 'Force kill' : a[0].toUpperCase() + a.slice(1)} ${s.name}?`,
      body: consequence[a],
      consequence: online ? `${online} player${online === 1 ? ' is' : 's are'} online right now.` : undefined,
      confirmLabel: a === 'kill' ? 'Kill process' : a[0].toUpperCase() + a.slice(1),
      danger: a !== 'restart',
    });
    if (!r.ok) return false;
  }
  try {
    await api.action(s.id, a);
    return true;
  } catch (e) {
    toastError(e, `Could not ${a} ${s.name}`);
    return false;
  }
}

export async function backupNow(serverId: string, note?: string) {
  try {
    const r = await api.createBackup(serverId, note);
    trackOp(r.operation);
    toast('info', 'Backup started', { body: serverById(serverId)?.name });
  } catch (e) { toastError(e, 'Backup failed to start'); }
}

export async function runFinding(f: Finding) {
  const a = f.action;
  if (!a) return;
  try {
    if (a.kind === 'start' && f.server_id) await power(serverById(f.server_id)!, 'start');
    else if (a.kind === 'backup' && f.server_id) await backupNow(f.server_id);
    else if (a.kind === 'open_tab' && f.server_id) navigate(serverHref(f.server_id, a.tab ?? 'overview'));
    else if (a.kind === 'accept_eula' && f.server_id) {
      const r = await confirmDialog({ title: 'Accept the Minecraft EULA?', body: 'This records your acceptance of https://aka.ms/MinecraftEULA for this server (audited).', confirmLabel: 'I accept' });
      if (r.ok) { await api.patchServer(f.server_id, { accept_eula: true }); toast('success', 'EULA accepted'); }
    } else if (a.kind === 'install_java') {
      const major = a.major ?? parseInt(f.title.match(/Java\s+(\d+)/i)?.[1] ?? '21', 10);
      const r = await api.installJava(major);
      trackOp(r.operation);
    }
  } catch (e) { toastError(e, a.label); }
}
