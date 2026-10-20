#!/usr/bin/env node
// Bundle-size budget for the web UI. Run after `npm run build` in web/.
// The "initial" JavaScript is what index.html loads before anything is shown: its script tag plus every
// modulepreload. Every other chunk is reported so growth is visible.
// Two numbers: TARGET_KB is the goal (40). BUDGET_KB is the limit CI enforces today. The Svelte runtime alone is
// about 20 kB gzipped and the app shell, router, stores and API client add about 30 kB, so the goal needs a
// smaller shell, not just lazy routes. Lower BUDGET_KB whenever the build gets smaller; never raise it quietly.
// Usage: node scripts/bundle-budget.mjs [path-to-dist] [budget-kb]
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = resolve(process.argv[2] ?? join(root, 'web', 'dist'));
const TARGET_KB = 40;
const budgetKb = Number(process.argv[3] ?? 54);
if (!existsSync(join(dist, 'index.html'))) { console.error(`No build found in ${dist}. Run "npm run build" in web/ first.`); process.exit(2); }

const gz = (file) => gzipSync(readFileSync(file), { level: 9 }).length;
const kb = (n) => (n / 1024).toFixed(2).padStart(7) + ' kB';
const html = readFileSync(join(dist, 'index.html'), 'utf8');
const initial = new Set();
for (const m of html.matchAll(/<(?:script|link)\b[^>]*>/g)) {
  const tag = m[0];
  if (!/type="module"|rel="modulepreload"/.test(tag)) continue;
  const src = /(?:src|href)="([^"]+\.js)"/.exec(tag)?.[1];
  if (src) initial.add(src.replace(/^\.?\//, '').replace(/^\/+/, ''));
}

let total = 0;
console.log('Initial JavaScript (gzipped):');
for (const f of [...initial].sort()) { const n = gz(join(dist, f)); total += n; console.log(`  ${kb(n)}  ${f}`); }
console.log(`  ${kb(total)}  total, budget ${budgetKb} kB\n`);

const assets = join(dist, 'assets');
const rest = readdirSync(assets).filter((f) => f.endsWith('.js') && !initial.has(`assets/${f}`))
  .map((f) => [f, gz(join(assets, f))]).sort((a, b) => b[1] - a[1]);
console.log('Other chunks, loaded per route or on demand (gzipped):');
for (const [f, n] of rest) console.log(`  ${kb(n)}  assets/${f}`);

if (initial.size === 0) { console.error('\nFAIL: no initial scripts found in index.html.'); process.exit(1); }
if (total > budgetKb * 1024) { console.error(`\nFAIL: initial JavaScript is ${(total / 1024).toFixed(2)} kB gzipped, over the ${budgetKb} kB budget.`); process.exit(1); }
if (total > TARGET_KB * 1024) console.log(`\nNote: ${((total - TARGET_KB * 1024) / 1024).toFixed(2)} kB above the ${TARGET_KB} kB goal.`);
console.log(`\nOK: initial JavaScript is ${(total / 1024).toFixed(2)} kB gzipped, within the ${budgetKb} kB budget.`);
