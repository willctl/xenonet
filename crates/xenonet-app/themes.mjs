// The built-in themes' data for the Rust (src/themes.json), from
// core/themes.js, which the console page keeps: the object literals before
// its first function, as they are.
//
//   node crates/xenonet-app/themes.mjs           writes src/themes.json
//   node crates/xenonet-app/themes.mjs --check   fails if it's out of date
import fs from "node:fs";

const source = fs.readFileSync(new URL("../../core/themes.js", import.meta.url), "utf8");
const end = source.indexOf("\nconst isObject");
if (end < 0) throw new Error("core/themes.js has changed shape: its data no longer ends at isObject");
const data = `${source.slice(0, end)}\nexport { THEMES };\n`;
const { THEMES } = await import(`data:text/javascript,${encodeURIComponent(data)}`);
const json = `${JSON.stringify(THEMES, null, 1)}\n`;
const file = new URL("./src/themes.json", import.meta.url);
if (process.argv.includes("--check")) {
  if (!fs.existsSync(file) || fs.readFileSync(file, "utf8") !== json) {
    console.error("crates/xenonet-app/src/themes.json is out of date with core/themes.js: node crates/xenonet-app/themes.mjs");
    process.exit(1);
  }
} else {
  fs.writeFileSync(file, json);
  console.log(`wrote crates/xenonet-app/src/themes.json (${Object.keys(THEMES).length} themes)`);
}
