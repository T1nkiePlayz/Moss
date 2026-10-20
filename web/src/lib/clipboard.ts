import { toast } from './stores.svelte';
/** Copy text; falls back to a hidden textarea where the async clipboard API is unavailable (plain http). */
export async function copyText(text: string): Promise<boolean> {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* fall through */ }
  try {
    const t = document.createElement('textarea');
    t.value = text; t.setAttribute('readonly', ''); t.style.cssText = 'position:fixed;top:-100px;opacity:0';
    document.body.appendChild(t); t.select();
    const ok = document.execCommand('copy');
    t.remove();
    return ok;
  } catch { return false; }
}
export async function copyWithToast(text: string, what: string) {
  if (await copyText(text)) toast('success', `${what} copied`, { timeout: 2200 });
  else toast('warn', 'Could not copy', { body: 'Select the text and copy it manually.' });
}
