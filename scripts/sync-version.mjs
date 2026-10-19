// Writes a release version (from a tag such as v1.2.3) into every manifest.
// Usage: node scripts/sync-version.mjs v1.2.3
import { readFileSync, writeFileSync } from "node:fs";

const version = (process.argv[2] ?? "").replace(/^v/, "");
if (!/^[0-9]+\.[0-9]+\.[0-9]+([-.+][0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`Invalid release version: ${process.argv[2]}`);
  process.exit(1);
}

const pkgPath = "web/package.json";
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
pkg.version = version;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");

const cargo = readFileSync("Cargo.toml", "utf8");
const next = cargo.replace(/(^\[package\][\s\S]*?^version\s*=\s*")[^"]+(")/m, `$1${version}$2`);
if (next === cargo && !cargo.includes(`version = "${version}"`)) {
  console.error("Could not find the [package] version in Cargo.toml");
  process.exit(1);
}
writeFileSync("Cargo.toml", next);

// keep Cargo.lock consistent so `cargo build --locked` keeps working
const lock = readFileSync("Cargo.lock", "utf8");
writeFileSync("Cargo.lock", lock.replace(/(name = "moss"\nversion = ")[^"]+(")/, `$1${version}$2`));
console.log(`Synced version ${version}`);
