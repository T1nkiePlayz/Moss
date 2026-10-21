import { applyCustomTheme, hasCustom, loadTheme, saveTheme, type CustomTheme } from './theme';
import { A11Y_DEFAULTS, a11yData, resolveTheme, type A11y } from './a11y';

// ---------- preferences ----------
export interface Prefs extends A11y {
  theme: 'system' | 'dark' | 'light' | 'contrast';
  accent: 'moss' | 'teal' | 'amber' | 'violet';
  density: 'comfortable' | 'compact';
  motion: 'system' | 'reduce';
  sidebar: 'full' | 'icons';
  serversView: 'cards' | 'table';
}
const defaults: Prefs = { theme: 'system', accent: 'moss', density: 'comfortable', motion: 'system', sidebar: 'full', serversView: 'cards', ...A11Y_DEFAULTS };
const loadPrefs = (): Prefs => { try { return { ...defaults, ...JSON.parse(localStorage.getItem('moss.prefs') || '{}') }; } catch { return { ...defaults }; } };
export const prefs = $state<Prefs>(loadPrefs());
/** The person's own colours (Settings > Appearance), kept in this browser; applied on top of light or dark. */
export const custom = $state<{ theme: CustomTheme | null }>({ theme: loadTheme() });
const mq = matchMedia('(prefers-color-scheme: light)');
const more = matchMedia('(prefers-contrast: more)');
export function applyPrefs() {
  const d = document.documentElement;
  d.dataset.theme = resolveTheme(prefs.theme, mq.matches, more.matches, prefs.contrast);
  d.dataset.accent = prefs.accent;
  d.dataset.density = prefs.density;
  d.dataset.motion = prefs.motion;
  Object.assign(d.dataset, a11yData(prefs));
  applyCustomTheme(d, d.dataset.theme === 'contrast' ? null : $state.snapshot(custom.theme), d.dataset.theme === 'light' ? 'light' : 'dark');
  try { localStorage.setItem('moss.prefs', JSON.stringify($state.snapshot(prefs))); } catch { /* storage unavailable */ }
}
for (const m of [mq, more]) m.addEventListener('change', () => prefs.theme === 'system' && applyPrefs());
/** Back to the default accessibility settings (Settings > Accessibility). */
export function resetA11y() { Object.assign(prefs, A11Y_DEFAULTS, { motion: 'system' }); applyPrefs(); }
export function setCustomTheme(t: CustomTheme | null) { custom.theme = t && hasCustom(t) ? t : null; saveTheme(custom.theme); applyPrefs(); }
export function setPref<K extends keyof Prefs>(k: K, v: Prefs[K]) { prefs[k] = v; applyPrefs(); }
export const cycleTheme = () => {
  const cur = document.documentElement.dataset.theme;
  setPref('theme', cur === 'dark' ? 'light' : 'dark');
};

