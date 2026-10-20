// Reinstall / repair (POST /servers/{id}/actions/reinstall), with the right amount of confirmation.
import { apiOps, type Server } from './api';
import { confirmDialog, toast, toastError, trackOp } from './stores.svelte';

/** Reinstall is allowed when the install failed or the server is stopped. */
export const canReinstall = (s: Pick<Server, 'state'>) => s.state === 'failed' || s.state === 'stopped';

export async function reinstall(s: Server, keepData: boolean, skipConfirm = false): Promise<boolean> {
  if (!skipConfirm) {
    const r = await confirmDialog(keepData
      ? { title: `Repair the server jar of ${s.name}?`, body: 'Downloads the server jar and loader files again. Your world, configuration and add-ons are kept.', confirmLabel: 'Repair jar' }
      : { title: `Reinstall ${s.name}?`, body: 'Downloads and sets up the server again from scratch. Files in the server folder, including worlds, can be replaced. Take a backup first if you need them.', typed: s.name, confirmLabel: 'Reinstall server', danger: true });
    if (!r.ok) return false;
  }
  try {
    trackOp((await apiOps.reinstall(s.id, keepData)).operation);
    toast('info', keepData ? 'Repair started' : 'Reinstall started', { body: s.name });
    return true;
  } catch (e) { toastError(e, keepData ? 'Could not repair the server' : 'Could not reinstall the server'); return false; }
}
