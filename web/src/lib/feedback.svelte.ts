// Toasts, the confirm dialog and UI flags: small and shared, so sign-in code can use them without the app's stores.
import { api, errMsg } from './api';

// ---------- toasts ----------
export interface Toast { id: number; kind: 'info' | 'success' | 'warn' | 'error'; title: string; body?: string; action?: { label: string; run: () => void }; timeout: number }
export const toasts = $state<{ list: Toast[] }>({ list: [] });
let tid = 0;
export function toast(kind: Toast['kind'], title: string, o: { body?: string; action?: Toast['action']; timeout?: number } = {}) {
  const t: Toast = { id: ++tid, kind, title, body: o.body, action: o.action, timeout: o.timeout ?? (o.action ? 9000 : kind === 'error' ? 8000 : 4000) };
  toasts.list.push(t);
  if (toasts.list.length > 5) toasts.list.shift();
  setTimeout(() => dismissToast(t.id), t.timeout);
}
export const dismissToast = (id: number) => { toasts.list = toasts.list.filter((t) => t.id !== id); };
export const toastError = (e: unknown, title = 'Something went wrong') => toast('error', title, { body: errMsg(e) });

/** Toast with an Undo button wired to POST /undo/{id}/revert. */
export function undoToast(title: string, undoId: string, after?: () => void) {
  toast('success', title, {
    action: {
      label: 'Undo',
      run: async () => {
        try { await api.revert(undoId); toast('info', 'Reverted'); after?.(); } catch (e) { toastError(e, 'Undo failed'); }
      },
    },
  });
}

// ---------- confirm dialog ----------
export interface ConfirmReq {
  title: string; body?: string; confirmLabel?: string; danger?: boolean; typed?: string; checkbox?: string; consequence?: string;
}
export const dialog = $state<{ req: (ConfirmReq & { resolve: (r: { ok: boolean; checked: boolean }) => void }) | null }>({ req: null });
export function confirmDialog(req: ConfirmReq) {
  return new Promise<{ ok: boolean; checked: boolean }>((resolve) => { dialog.req = { ...req, resolve }; });
}

// ---------- ui flags ----------
export const ui = $state({ palette: false, ops: false, drawer: false, userMenu: false });

