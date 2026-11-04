// What the docs and the two xenonet commands (cli/, the JS one, and
// crates/xenonet-cli, the binary) say about the same things must agree.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { COMMANDS, ALIASES } from "../cli/lib/commands.js";
import { TOOLS } from "../core/schema/assistant.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
const rust = read("crates/xenonet-cli/src/commands/mod.rs");

const rustCommands = () => [...rust.matchAll(/(?:\bcmd\(\s*|Command \{ name: )"([a-z]+)"/g)].map((m) => m[1]);

test("the JS command's table names the same commands and aliases as the binary's", () => {
  assert.deepEqual(Object.keys(COMMANDS).sort(), rustCommands().sort());
  const aliases = [...rust.match(/ALIASES: &\[\(&str, &str\)\] = &\[(.*?)\];/s)[1].matchAll(/\("([a-z]+)", "([a-z]+)"\)/g)].map((m) => [m[1], m[2]]);
  assert.deepEqual(Object.entries(ALIASES).sort(), aliases.sort());
});

test("every command the JS table marks native exists in the binary alone", () => {
  for (const [name, cmd] of Object.entries(COMMANDS)) {
    assert.equal(!!cmd.native, !fs.existsSync(path.join(root, "cli", "commands", `${name}.js`)), `${name}: native commands have no cli/commands file, and the others do`);
  }
});

test("docs/guide/cli.md lists every shown command, and only commands and aliases", () => {
  const doc = read("docs/guide/cli.md");
  const rows = [...doc.matchAll(/^\| `xenonet(?: ([a-z]+))?/gm)].map((m) => m[1]).filter(Boolean);
  const known = new Set([...Object.keys(COMMANDS), ...Object.keys(ALIASES)]);
  for (const name of rows) assert.ok(known.has(name), `cli.md has a row for "xenonet ${name}", which isn't a command`);
  const listed = new Set(rows.map((n) => ALIASES[n] ?? n));
  const hidden = Object.entries(COMMANDS).filter(([, c]) => c.hidden).map(([n]) => n);
  for (const name of Object.keys(COMMANDS)) {
    if (!hidden.includes(name)) assert.ok(listed.has(name) || doc.includes(`\`xenonet ${name}`), `cli.md doesn't list xenonet ${name}`);
  }
});

test("docs/guide/cli.md says what the exit codes mean, and --exit-code is only drift's", () => {
  const doc = read("docs/guide/cli.md");
  assert.match(doc, /^## Exit codes$/m);
  for (const name of Object.keys(COMMANDS)) {
    if (name === "drift" || name === "completion") continue;
    const file = path.join(root, "cli", "commands", `${name}.js`);
    if (fs.existsSync(file)) assert.ok(!/exit-code|exitCode/.test(fs.readFileSync(file, "utf8")), `${name} takes --exit-code, so cli.md must say so`);
  }
});

test("the remote actors' guide counts the tools the assistant server has", () => {
  const words = { four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10 };
  const guide = read("docs/guide/cloud-assistants.md");
  const counts = [...guide.matchAll(/\b(four|five|six|seven|eight|nine|ten) tools\b/g)].map((m) => words[m[1]]);
  assert.ok(counts.length >= 2);
  for (const n of counts) assert.equal(n, TOOLS.length);
  for (const tool of TOOLS) assert.ok(guide.includes(`\`${tool}\``), `${tool} isn't in the guide's table`);
  assert.match(read("mcp/assistant.js"), new RegExp(`has ${Object.keys(words).find((k) => words[k] === TOOLS.length)} tools`));
});

test("native commands are handed to the binary, with its exit code, or say it isn't built", async () => {
  const { delegate, nativeBinary } = await import("../cli/lib/native.js");
  assert.equal(delegate(["-e", "process.exit(3)"], { bin: process.execPath }), 3);
  assert.throws(() => delegate(["net"], { bin: null }), /npm run rust:bin/);
  assert.equal(nativeBinary({ env: { XENONET_CLI_BIN: "/x/xenonet" } }), "/x/xenonet");
});
