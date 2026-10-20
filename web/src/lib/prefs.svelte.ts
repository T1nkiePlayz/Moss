import { applyCustomTheme, hasCustom, loadTheme, saveTheme, type CustomTheme } from './theme';

// ---------- preferences ----------
export interface Prefs {
  theme: 'system' | 'dark' | 'light' | 'contrast';
  accent: 'moss' | 'teal' | 'amber' | 'violet';
  density: 'comfortable' | 'compact';
  motion: 'system' | 'reduce';
  sidebar: 'full' | 'icons';
  serversView: 'cards' | 'table';
}
const defaults: Prefs = { theme: 'system', accent: 'moss', density: 'comfortable', motion: 'system', sidebar: 'full', serversView: 'cards' };
const loadPrefs = (): Prefs => { try { return { ...defaults, ...JSON.parse(localStorage.getItem('moss.prefs') || '{}') }; } catch { return { ...defaults }; } };
export const prefs = $state<Prefs>(loadPrefs());
/** The person's own colours (Settings > Appearance), kept in this browser; applied on top of light or dark. */
export const custom = $state<{ theme: CustomTheme | null }>({ theme: loadTheme() });
const mq = matchMedia('(prefers-color-scheme: light)');
export function applyPrefs() {
  const d = document.documentElement;
  d.dataset.theme = prefs.theme === 'system' ? (mq.matches ? 'light' : 'dark') : prefs.theme;
  d.dataset.accent = prefs.accent;
  d.dataset.density = prefs.density;
  d.dataset.motion = prefs.motion;
  applyCustomTheme(d, d.dataset.theme === 'contrast' ? null : $state.snapshot(custom.theme), d.dataset.theme === 'light' ? 'light' : 'dark');
  try { localStorage.setItem('moss.prefs', JSON.stringify($state.snapshot(prefs))); } catch { /* storage unavailable */ }
}
mq.addEventListener('change', () => prefs.theme === 'system' && applyPrefs());
export function setCustomTheme(t: CustomTheme | null) { custom.theme = t && hasCustom(t) ? t : null; saveTheme(custom.theme); applyPrefs(); }
export function setPref<K extends keyof Prefs>(k: K, v: Prefs[K]) { prefs[k] = v; applyPrefs(); }
export const cycleTheme = () => {
  const cur = document.documentElement.dataset.theme;
  setPref('theme', cur === 'dark' ? 'light' : 'dark');
};

