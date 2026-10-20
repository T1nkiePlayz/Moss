import { describe, expect, it } from 'vitest';
import { checks, contrast, cssVars, DEFAULTS, exportTheme, inkOn, minimal, normHex, parseTheme, resolve } from './theme';

describe('contrast', () => {
  it('matches the WCAG reference values', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 1);
    expect(contrast('#777777', '#ffffff')).toBeCloseTo(4.48, 1);
    expect(contrast('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
  });
  it('picks a readable ink colour', () => {
    expect(inkOn('#ffffff')).toBe('#000000');
    expect(inkOn('#06150a')).toBe('#ffffff');
  });
});

describe('defaults', () => {
  it('pass AA in both schemes (axe must keep passing with the built-in colours)', () => {
    for (const s of ['light', 'dark'] as const) for (const c of checks(DEFAULTS[s])) expect(c.ok, `${s} ${c.label} ${c.ratio.toFixed(2)}`).toBe(true);
  });
  it('flag a failing pair', () => {
    const bad = checks({ ...DEFAULTS.light, text: '#aaaaaa' });
    expect(bad.filter((c) => !c.ok).map((c) => c.id)).toEqual(['text-bg', 'text-surface']);
  });
});

describe('theme data', () => {
  it('expands to the full variable set and normalises short hex', () => {
    expect(normHex('#ABC')).toBe('#aabbcc');
    const v = cssVars(DEFAULTS.dark, 'dark');
    expect(v['--bg']).toBe('#090d0b');
    expect(v['--a-ink']).toBe('#000000');
    expect(Object.keys(v)).toContain('--danger-soft');
  });
  it('round-trips through JSON and keeps only changes', () => {
    const t = { light: { accent: '#336699' }, dark: {} };
    const r = parseTheme(exportTheme(t));
    expect(r.ok && r.theme.light?.accent).toBe('#336699');
    expect(resolve(t, 'light').accent).toBe('#336699');
    expect(resolve(t, 'dark')).toEqual(DEFAULTS.dark);
    expect(minimal({ ...DEFAULTS.light, accent: '#336699' }, 'light')).toEqual({ accent: '#336699' });
  });
  it('rejects bad imports', () => {
    for (const s of ['nope', '{}', '{"format":"moss-theme","light":{"accent":"red"}}', '{"format":"moss-theme","light":{"x":"#fff"}}', '{"format":"moss-theme","dark":[]}']) expect(parseTheme(s).ok, s).toBe(false);
    expect(parseTheme('{"format":"moss-theme","version":1,"light":{"accent":"#fff"}}').ok).toBe(true);
  });
});
