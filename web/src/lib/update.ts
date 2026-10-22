// Pure helpers for the "Moss update available" banner (lazy chunk; tested in update.test.ts).
export const BANNER_MS = 15000;
const key = (v: string) => `moss.update.seen.${v}`;

/** True when the banner should appear: an update exists, and this tab has not shown that version yet. */
export function shouldShowBanner(latest: string | null, available: boolean, store: Pick<Storage, 'getItem'> | null = safeStore()): boolean {
  if (!available || !latest) return false;
  try { return store?.getItem(key(latest)) !== '1'; } catch { return true; }
}
export function markBannerShown(latest: string, store: Pick<Storage, 'setItem'> | null = safeStore()) {
  try { store?.setItem(key(latest), '1'); } catch { /* storage blocked: the banner may reappear, which is harmless */ }
}
function safeStore(): Storage | null { try { return sessionStorage; } catch { return null; } }

/** A countdown that can be paused (hover/focus) and resumed with the remaining time. */
export function pausableTimer(ms: number, done: () => void) {
  let left = ms, started = 0, id: ReturnType<typeof setTimeout> | undefined, over = false;
  const run = () => { started = Date.now(); id = setTimeout(() => { over = true; id = undefined; done(); }, left); };
  run();
  return {
    pause() { if (id === undefined) return; clearTimeout(id); id = undefined; left = Math.max(0, left - (Date.now() - started)); },
    resume() { if (id !== undefined || over) return; run(); },
    stop() { clearTimeout(id); id = undefined; over = true; },
  };
}
