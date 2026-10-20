// Refreshes docs/openapi.frozen.json (the snapshot tests/api_compat.rs compares against) from docs/openapi.json.
// This is a deliberate act: it prints what changed, refuses to drop routes unless --allow-breaking is given, and
// should be part of a reviewed change that also updates docs/api-policy.md's changelog when anything is removed.
// Usage: node scripts/api-freeze.mjs [--allow-breaking]
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const [cur, frozen] = ["docs/openapi.json", "docs/openapi.frozen.json"];
const ops = (spec) => new Set(Object.entries(spec.paths).flatMap(([p, item]) => Object.keys(item).map((m) => `${m.toUpperCase()} ${p}`)));
const current = JSON.parse(readFileSync(cur, "utf8"));
const before = existsSync(frozen) ? ops(JSON.parse(readFileSync(frozen, "utf8"))) : new Set();
const after = ops(current);
const added = [...after].filter((o) => !before.has(o));
const removed = [...before].filter((o) => !after.has(o));
if (removed.length && !process.argv.includes("--allow-breaking")) {
  console.error(`Refusing to freeze: these operations were removed (a breaking change, see docs/api-policy.md):\n  ${removed.join("\n  ")}\nRe-run with --allow-breaking if that is intended.`);
  process.exit(1);
}
writeFileSync(frozen, JSON.stringify(current, null, 2) + "\n");
console.log(`Froze ${after.size} operations (${added.length} added, ${removed.length} removed) into ${frozen}.`);
for (const o of added) console.log(`  + ${o}`);
for (const o of removed) console.log(`  - ${o}`);
console.log("Run `cargo test --test api_compat` and commit the snapshot together with the change that needs it.");
