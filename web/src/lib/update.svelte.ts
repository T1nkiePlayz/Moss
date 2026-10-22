// Shared state and flow for Moss self-updates (loaded lazily with the banner / Settings section).
import { api, type UpdateInfo } from './api';
import { sysUpdate as sysApi } from './api-extra';
import { confirmDialog, toastError } from './feedback.svelte';
import { onOp, servers, trackOp } from './stores.svelte';

export type UpdatePhase = 'idle' | 'updating' | 'restarting' | 'stalled' | 'failed';
export const upd = $state<{ info: UpdateInfo | null; phase: UpdatePhase; error: string; checking: boolean; reveal: number }>({ info: null, phase: 'idle', error: '', checking: false, reveal: 0 });

export const STALL_MS = 180000;
export const STALL_TEXT = 'Moss has not come back on the new version yet. Check the server log (with the systemd helper: `journalctl -u moss-update`).';

export async function loadUpdate() { try { upd.info = await sysApi.info(); } catch { /* admin-only and optional: stay silent */ } return upd.info; }
export async function checkUpdate() {
  upd.checking = true;
  try { upd.info = await sysApi.check(); } catch (e) { toastError(e, 'Could not check for updates'); }
  upd.checking = false;
}

let timer: ReturnType<typeof setInterval> | undefined;
let unsub: (() => void) | undefined;
function finish() { clearInterval(timer); timer = undefined; unsub?.(); unsub = undefined; }

/** Ask for confirmation, start the update, then watch the operation and the version until Moss is back. */
export async function startUpdate() {
  const info = upd.info;
  if (!info?.latest || upd.phase === 'updating' || upd.phase === 'restarting') return;
  const { ok } = await confirmDialog({ title: `Update to ${info.latest}?`, body: `Moss will download, verify and install ${info.latest}, then restart. Game servers keep running.`, confirmLabel: 'Update and restart' });
  if (!ok) return;
  let before = info.current;
  try { before = (await api.status()).version; } catch { /* keep the version from the update info */ }
  let id = '';
  try {
    const r = await sysApi.start();
    id = r.operation.id;
    trackOp(r.operation);
  } catch (e) { toastError(e, 'Could not start the update'); return; }
  upd.phase = 'updating'; upd.error = '';
  let since = 0;
  const restarting = () => { if (upd.phase === 'updating') { upd.phase = 'restarting'; since = Date.now(); } };
  const handle = (o: { id: string; state: string; error: string | null }) => {
    if (o.id !== id) return;
    if (o.state === 'completed' || o.state === 'interrupted') restarting();
    else if (o.state === 'failed' || o.state === 'cancelled') { finish(); upd.phase = 'failed'; upd.error = o.error ?? 'The update did not complete.'; }
  };
  unsub = onOp(handle);
  timer = setInterval(async () => {
    if (!servers.live) restarting();
    try {
      const s = await api.status();
      if (s.version !== before) { finish(); location.reload(); }
    } catch { restarting(); }
    if (upd.phase === 'restarting' && Date.now() - since > STALL_MS) upd.phase = 'stalled';
  }, 2000);
}

export function dismissUpdateError() { if (upd.phase === 'failed') upd.phase = 'idle'; }
