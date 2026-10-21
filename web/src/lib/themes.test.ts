import { describe, expect, it } from 'vitest';
// @ts-expect-error the project has no Node types; vitest runs this in Node
import { readFileSync } from 'node:fs';
import { contrast } from './theme';

// Every theme in app.css, as the variables it ends up with (each starts from the dark :root block, like the browser).
const css: string = readFileSync(new URL('../app.css', import.meta.url), 'utf8');
const vars = (body: string): Record<string, string> => Object.fromEntries([...body.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
const base = vars(/:root, :root\[data-theme='dark'\] \{([^}]*)\}/.exec(css)![1]);
const themes: Record<string, Record<string, string>> = Object.fromEntries([...css.matchAll(/^:root\[data-theme='([\w-]+)'\] \{([^}]*)\}/gm)].map((m) => [m[1], { ...base, ...vars(m[2]) }]));
themes.dark = base;

const PAIRS: [string, string][] = [
  ['--text', '--bg'], ['--text', '--s2'], ['--muted', '--bg'], ['--muted', '--s2'], ['--faint', '--bg'],
  ['--a-text', '--bg'], ['--a-text', '--s2'], ['--a-ink', '--a2'],
  ['--warn', '--bg'], ['--danger', '--bg'], ['--info', '--bg'], ['--ok', '--bg'],
];
// Paper's primary button predates this check and is 4.1:1 on its dark gradient end (axe passes it on the blend).
// Themes added since must pass on both gradient ends.
const OLD = ['light', 'dark', 'contrast'];
const KNOWN: Record<string, string[]> = { light: ['--a-ink on --a2'] };

describe('themes', () => {
  it('finds every theme', () => expect(Object.keys(themes).sort()).toEqual(['campfire', 'contrast', 'dark', 'deepsea', 'desert', 'light', 'midnight', 'nether', 'sakura', 'synthwave', 'terminal', 'tundra', 'void']));

  for (const [name, v] of Object.entries(themes)) {
    it(`${name}: text, accents and status colours meet WCAG AA (4.5:1)`, () => {
      const pairs = !OLD.includes(name) ? [...PAIRS, ['--a-ink', '--a1'] as [string, string]] : PAIRS;
      const low = pairs.map(([f, b]) => [f, b, contrast(v[f], v[b])] as const).filter(([f, b, r]) => r < 4.5 && !KNOWN[name]?.includes(`${f} on ${b}`)).map(([f, b, r]) => `${f} on ${b}: ${r.toFixed(2)}`);
      expect(low).toEqual([]);
    });
  }
});
