// What the docs say about the xenonet command and the assistant's MCP server
// must agree with the binary: its command table
// (crates/xenonet-cli/src/commands/mod.rs) and the assistant's tools, as
// the MCP contract's golden file has them.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
const rust = read("crates/xenonet-cli/src/commands/mod.rs");

// { name: hidden } for every command in the binary's table.
const commands = () => {
  const out = {};
  for (const m of rust.matchAll(/\bcmd\(\s*"([a-z]+)"/g)) out[m[1]] = false;
  for (const m of rust.matchAll(/Command \{ name: "([a-z]+)".*?hidden: (true|false) \}/g)) out[m[1]] = m[2] === "true";
  return out;
};
const aliases = () => Object.fromEntries([...rust.match(/ALIASES: &\[\(&str, &str\)\] = &\[(.*?)\];/s)[1].matchAll(/\("([a-z]+)", "([a-z]+)"\)/g)].map((m) => [m[1], m[2]]));

test("docs/guide/cli.md lists every shown command, and only commands and aliases", () => {
  const doc = read("docs/guide/cli.md");
  const all = commands();
  const alias = aliases();
  assert.ok(Object.keys(all).length > 20, "the command table was read");
  const rows = [...doc.matchAll(/^\| `xenonet(?: ([a-z]+))?/gm)].map((m) => m[1]).filter(Boolean);
  const known = new Set([...Object.keys(all), ...Object.keys(alias)]);
  for (const name of rows) assert.ok(known.has(name), `cli.md has a row for "xenonet ${name}", which isn't a command`);
  const listed = new Set(rows.map((n) => alias[n] ?? n));
  for (const [name, hidden] of Object.entries(all)) {
    if (!hidden) assert.ok(listed.has(name) || doc.includes(`\`xenonet ${name}`), `cli.md doesn't list xenonet ${name}`);
  }
  assert.match(doc, /^## Exit codes$/m);
});

test("the remote actors' guide counts the tools the assistant server has", () => {
  const golden = JSON.parse(read("crates/xenonet-mcp/tests/golden/the-assistant-for-a-grant.json"));
  const listing = golden.answers.find((a) => Array.isArray(a.result?.tools));
  const tools = listing.result.tools.map((t) => t.name);
  const words = { four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10 };
  const guide = read("docs/guide/cloud-assistants.md");
  const counts = [...guide.matchAll(/\b(four|five|six|seven|eight|nine|ten) tools\b/g)].map((m) => words[m[1]]);
  assert.ok(counts.length >= 2);
  for (const n of counts) assert.equal(n, tools.length);
  for (const tool of tools) assert.ok(guide.includes(`\`${tool}\``), `${tool} isn't in the guide's table`);
});
