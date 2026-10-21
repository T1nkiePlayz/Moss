// Custom colour themes: a small set of tokens per scheme, expanded to the full CSS variable set, with a WCAG AA checker.
export type Scheme = 'light' | 'dark';
export const TOKENS = [
  { key: 'bg', label: 'Background' },
  { key: 'surface', label: 'Cards and panels' },
  { key: 'text', label: 'Text' },
  { key: 'accent', label: 'Accent' },
  { key: 'ok', label: 'Success' },
  { key: 'warn', label: 'Warning' },
  { key: 'danger', label: 'Error' },
  { key: 'info', label: 'Information' },
] as const;
export type TokenKey = (typeof TOKENS)[number]['key'];
export type Colors = Record<TokenKey, string>;
export interface CustomTheme { light?: Partial<Colors>; dark?: Partial<Colors> }

/** The built-in colours (mirrors app.css) so the editor starts from what you see. */
export const DEFAULTS: Record<Scheme, Colors> = {
  light: { bg: '#f5f6f1', surface: '#ffffff', text: '#14241a', accent: '#206b33', ok: '#206b33', warn: '#8a5000', danger: '#c2321f', info: '#075d75' },
  dark: { bg: '#090d0b', surface: '#141c18', text: '#e6efe9', accent: '#9be37d', ok: '#9be37d', warn: '#f2b04c', danger: '#ff6b5e', info: '#5cc8e8' },
};

const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i;
export const isHex = (v: unknown): v is string => typeof v === 'string' && HEX.test(v);
export function normHex(v: string): string {
  const h = v.trim().toLowerCase();
  return h.length === 4 ? '#' + [...h.slice(1)].map((c) => c + c).join('') : h;
}
export const rgb = (hex: string): [number, number, number] => { const h = normHex(hex).slice(1); return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as [number, number, number]; };
const hex2 = (n: number) => Math.round(Math.max(0, Math.min(255, n))).toString(16).padStart(2, '0');
export const toHex = ([r, g, b]: number[]) => `#${hex2(r)}${hex2(g)}${hex2(b)}`;

export function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}
export const mix = (a: string, b: string, t: number) => { const [p, q] = [rgb(a), rgb(b)]; return toHex(p.map((v, i) => v + (q[i] - v) * t)); };
const alpha = (hex: string, a: number) => { const [r, g, b] = rgb(hex); return `rgb(${r} ${g} ${b} / ${a})`; };
/** Black or white, whichever reads better on `bg`. */
export const inkOn = (bg: string) => (contrast(bg, '#ffffff') >= contrast(bg, '#000000') ? '#ffffff' : '#000000');

export interface Check { id: string; label: string; fg: string; bg: string; ratio: number; need: number; ok: boolean }
/** WCAG 2.2 AA (unchanged since 2.1): 4.5:1 for normal text. Every colour that is used as text, or as the fill behind button text, is checked. */
export function checks(c: Colors): Check[] {
  const rows: [string, string, string, string][] = [
    ['text-bg', 'Text on background', c.text, c.bg],
    ['text-surface', 'Text on cards', c.text, c.surface],
    ['accent-bg', 'Accent text on background', c.accent, c.bg],
    ['accent-surface', 'Accent text on cards', c.accent, c.surface],
    ['button', 'Button label on accent', inkOn(c.accent), c.accent],
    ['ok', 'Success on background', c.ok, c.bg],
    ['warn', 'Warning on background', c.warn, c.bg],
    ['danger', 'Error on background', c.danger, c.bg],
    ['info', 'Information on background', c.info, c.bg],
  ];
  return rows.map(([id, label, fg, bg]) => { const ratio = contrast(fg, bg); return { id, label, fg, bg, ratio, need: 4.5, ok: ratio >= 4.5 }; });
}

/** Full CSS variable set for one scheme, derived from the eight editable colours. */
export function cssVars(c: Colors, scheme: Scheme): Record<string, string> {
  const dark = scheme === 'dark';
  const soft = (x: string) => alpha(x, dark ? 0.12 : 0.1);
  return {
    '--bg': c.bg, '--s1': mix(c.bg, c.surface, 0.5), '--s2': c.surface, '--s3': mix(c.surface, c.text, dark ? 0.07 : 0.05),
    '--text': c.text, '--muted': mix(c.text, c.bg, dark ? 0.35 : 0.3), '--faint': mix(c.text, c.bg, dark ? 0.3 : 0.25),
    '--line': alpha(c.text, dark ? 0.1 : 0.12), '--line2': alpha(c.text, dark ? 0.16 : 0.22),
    '--a1': c.accent, '--a2': c.accent, '--a-text': c.accent, '--a-ink': inkOn(c.accent), '--a-soft': soft(c.accent), '--a-glow': alpha(c.accent, 0.3),
    '--ok': c.ok, '--warn': c.warn, '--danger': c.danger, '--info': c.info,
    '--warn-soft': soft(c.warn), '--danger-soft': soft(c.danger), '--info-soft': soft(c.info),
  };
}
/** Resolved colours for a scheme: the custom value where set and valid, else the default. */
export function resolve(t: CustomTheme | null | undefined, scheme: Scheme): Colors {
  const out = { ...DEFAULTS[scheme] };
  for (const k of Object.keys(out) as TokenKey[]) { const v = t?.[scheme]?.[k]; if (isHex(v)) out[k] = normHex(v); }
  return out;
}
export const hasCustom = (t: CustomTheme | null | undefined) => !!t && (['light', 'dark'] as Scheme[]).some((s) => Object.keys(t[s] ?? {}).length > 0);
/** Only the colours that differ from the defaults. */
export function minimal(c: Colors, scheme: Scheme): Partial<Colors> {
  return Object.fromEntries((Object.keys(c) as TokenKey[]).filter((k) => normHex(c[k]) !== DEFAULTS[scheme][k]).map((k) => [k, normHex(c[k])]));
}

export function applyCustomTheme(el: HTMLElement, t: CustomTheme | null | undefined, scheme: Scheme) {
  for (const k of Object.keys(cssVars(DEFAULTS[scheme], scheme))) el.style.removeProperty(k);
  if (!hasCustom(t) || !Object.keys(t?.[scheme] ?? {}).length) return;
  for (const [k, v] of Object.entries(cssVars(resolve(t, scheme), scheme))) el.style.setProperty(k, v);
}

// ---- JSON import / export ----
export function exportTheme(t: CustomTheme): string {
  return JSON.stringify({ format: 'moss-theme', version: 1, light: t.light ?? {}, dark: t.dark ?? {} }, null, 2);
}
export function parseTheme(text: string): { ok: true; theme: CustomTheme } | { ok: false; error: string } {
  let j: unknown;
  try { j = JSON.parse(text); } catch { return { ok: false, error: 'This is not valid JSON.' }; }
  const o = j as Record<string, unknown>;
  if (!o || typeof o !== 'object' || o.format !== 'moss-theme') return { ok: false, error: 'This file is not a Moss theme (the "format" is missing).' };
  const theme: CustomTheme = {};
  for (const s of ['light', 'dark'] as Scheme[]) {
    const part = o[s];
    if (part === undefined) continue;
    if (!part || typeof part !== 'object' || Array.isArray(part)) return { ok: false, error: `"${s}" must be an object of colours.` };
    const out: Partial<Colors> = {};
    for (const [k, v] of Object.entries(part as Record<string, unknown>)) {
      if (!TOKENS.some((t) => t.key === k)) return { ok: false, error: `"${k}" is not a colour Moss knows.` };
      if (!isHex(v)) return { ok: false, error: `${s}.${k} must be a colour like #1a2b3c.` };
      out[k as TokenKey] = normHex(v);
    }
    theme[s] = out;
  }
  return { ok: true, theme };
}

const KEY = 'moss.theme';
export function loadTheme(): CustomTheme | null {
  try { const r = parseTheme(localStorage.getItem(KEY) ?? ''); return r.ok ? r.theme : null; } catch { return null; }
}
export function saveTheme(t: CustomTheme | null) {
  try { if (t && hasCustom(t)) localStorage.setItem(KEY, exportTheme(t)); else localStorage.removeItem(KEY); } catch { /* storage unavailable */ }
}
