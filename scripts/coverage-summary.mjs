#!/usr/bin/env node
// Markdown line-coverage summary per source directory, from an lcov file.
//   node scripts/coverage-summary.mjs [lcov.info]
// Prints the table; when GITHUB_STEP_SUMMARY is set it is appended there too. No dependencies.
import { appendFileSync, readFileSync } from "node:fs";
import { relative, resolve, sep } from "node:path";

const file = process.argv[2] ?? "lcov.info";
const root = process.cwd();
const dirs = new Map(); // directory -> { found, hit }
let current = null;

for (const line of readFileSync(file, "utf8").split(/\r?\n/)) {
  if (line.startsWith("SF:")) {
    const rel = relative(root, resolve(root, line.slice(3))).split(sep).join("/");
    // src/games/foo.rs -> src/games; src/main.rs -> src; tests and anything outside the repo are skipped
    const parts = rel.split("/");
    current = parts[0] === "src" ? (parts.length > 2 ? `src/${parts[1]}` : "src") : null;
    if (current && !dirs.has(current)) dirs.set(current, { found: 0, hit: 0 });
  } else if (current && line.startsWith("DA:")) {
    const d = dirs.get(current);
    d.found++;
    if (Number(line.slice(3).split(",")[1]) > 0) d.hit++;
  } else if (line === "end_of_record") {
    current = null;
  }
}

const pct = (h, f) => (f ? (100 * h / f).toFixed(1) : "n/a");
const rows = [...dirs].sort(([a], [b]) => a.localeCompare(b));
const total = rows.reduce((t, [, d]) => ({ found: t.found + d.found, hit: t.hit + d.hit }), { found: 0, hit: 0 });
let md = "### Line coverage by directory\n\n| Directory | Lines | Covered | Coverage |\n| --- | ---: | ---: | ---: |\n";
for (const [name, d] of rows) md += `| ${name} | ${d.found} | ${d.hit} | ${pct(d.hit, d.found)}% |\n`;
md += `| **Total (src)** | ${total.found} | ${total.hit} | **${pct(total.hit, total.found)}%** |\n`;

console.log(md);
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, md + "\n");
