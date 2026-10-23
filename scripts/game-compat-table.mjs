#!/usr/bin/env node
// Renders the nightly game matrix results as a Markdown compatibility table (game x steps).
// Usage: node scripts/game-compat-table.mjs <dir-or-file.jsonl>... > game-compat.md
// Input is the JSON lines tests/games_e2e.rs writes: {"game","step","ok","seconds","error"}. A line with
// step "skipped" (written by the workflow for games that do not fit a hosted runner) marks every step skipped.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const STEPS = ["install", "start", "players", "backup", "stop", "no_orphan"];

function files(p) {
  if (!statSync(p).isDirectory()) return [p];
  return readdirSync(p).flatMap((f) => files(join(p, f)));
}

const rows = new Map(); // game -> { step -> result }
for (const arg of process.argv.slice(2)) {
  for (const f of files(arg).filter((f) => f.endsWith(".jsonl") || f.endsWith(".json"))) {
    for (const line of readFileSync(f, "utf8").split("\n")) {
      if (!line.trim()) continue;
      let r;
      try { r = JSON.parse(line); } catch { continue; }
      if (!r.game || !r.step) continue;
      if (!rows.has(r.game)) rows.set(r.game, {});
      rows.get(r.game)[r.step] = r;
    }
  }
}

const fmt = (s) => (s >= 3600 ? `${Math.floor(s / 3600)}h${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m` : s >= 60 ? `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s` : `${s}s`);
const esc = (s) => String(s ?? "").replace(/\s+/g, " ").replace(/\|/g, "\\|").slice(0, 160);

const out = ["| Game | " + STEPS.join(" | ") + " |", "|---|" + STEPS.map(() => "---").join("|") + "|"];
const notes = [];
for (const game of [...rows.keys()].sort()) {
  const r = rows.get(game);
  const skipped = r.skipped;
  let failed = false;
  const cells = STEPS.map((step) => {
    if (skipped) return "⏭";
    const x = r[step];
    if (!x) return failed ? "·" : "❔"; // never reached after an earlier failure, or never recorded
    if (!x.ok) { failed = true; notes.push(`- **${game}** ${step}: ${esc(x.error)}`); return `❌ ${fmt(x.seconds)}`; }
    return `✅ ${fmt(x.seconds)}`;
  });
  if (skipped) notes.push(`- **${game}**: ${esc(skipped.error || "skipped")}`);
  out.push(`| ${game} | ${cells.join(" | ")} |`);
}
if (rows.size === 0) out.push("| _no results_ |" + STEPS.map(() => "").join("|") + "|");
out.push("", "✅ passed, ❌ failed, ⏭ skipped, · not reached after an earlier failure, ❔ no result recorded. Times are wall-clock for the step.");
if (notes.length) out.push("", "### Details", "", ...notes);
console.log(out.join("\n"));
