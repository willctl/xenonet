// The program that collects text inside a sandbox (COLLECT_TEXT), for the
// Rust (src/collect_text.js): core/integrations/reviews.js makes it from
// its own functions' source, which runs on the sandbox's Node, so the
// binary carries it as text rather than asking a Node of yours for it.
//
//   node crates/xenonet-integrations/collect-text.mjs           writes it
//   node crates/xenonet-integrations/collect-text.mjs --check   fails if it's out of date
import fs from "node:fs";
import { COLLECT_TEXT } from "../../core/integrations/reviews.js";

const file = new URL("./src/collect_text.js", import.meta.url);
if (process.argv.includes("--check")) {
  if (!fs.existsSync(file) || fs.readFileSync(file, "utf8") !== COLLECT_TEXT) {
    console.error("crates/xenonet-integrations/src/collect_text.js is out of date with core/integrations/reviews.js: node crates/xenonet-integrations/collect-text.mjs");
    process.exit(1);
  }
} else {
  fs.writeFileSync(file, COLLECT_TEXT);
  console.log(`wrote crates/xenonet-integrations/src/collect_text.js (${COLLECT_TEXT.length} characters)`);
}
