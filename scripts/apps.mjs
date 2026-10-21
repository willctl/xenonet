// The app registry's data for the Rust (crates/xenonet-base/src/apps.json),
// from core/apps.js, which the JS and the page keep: its entries, as they
// are, in order.
//
//   node scripts/apps.mjs           writes crates/xenonet-base/src/apps.json
//   node scripts/apps.mjs --check   fails if it's out of date
import fs from "node:fs";
import { APPS } from "../core/apps.js";

const json = `${JSON.stringify(APPS, null, 1)}\n`;
const file = new URL("../crates/xenonet-base/src/apps.json", import.meta.url);
if (process.argv.includes("--check")) {
  if (!fs.existsSync(file) || fs.readFileSync(file, "utf8") !== json) {
    console.error("crates/xenonet-base/src/apps.json is out of date with core/apps.js: node scripts/apps.mjs");
    process.exit(1);
  }
} else {
  fs.writeFileSync(file, json);
  console.log(`wrote crates/xenonet-base/src/apps.json (${APPS.length} apps)`);
}
