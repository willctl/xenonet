// xenonet connect: adds XenoNet to Claude Code's, Codex's and Grok Build's
// settings, so their sessions can see and message each other (core/engine/connect.js, through
// the running XenoNet, which knows how its agents' tools should run). It shows
// what will change and asks first.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline/promises";
import { c, print, json, Fail } from "../lib/out.js";
import { need, get, post, nameOf, candidates } from "../../core/client.js";
import { createConnect } from "../../core/engine/connect.js";
import { appsWith } from "../../core/apps.js";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

export const help = `Adds XenoNet to Claude Code's, Codex's and Grok Build's own settings: the
xenonet MCP server (tools so each session can see the others, message them
and claim files) and hooks (XenoNet says who else is working when a session
starts, warns before an edit to a file another session has in hand, and
passes on messages). It shows what will change and asks first. xenonet
disconnect takes it all out again. While XenoNet's plugin for Claude Code is
installed, Claude Code is left to it, and connecting takes out connect's own
entries there.

  xenonet connect [claude] [codex] [grok]
                                   each that's here unless you name them
  --yes                            don't ask
  --json                           the connection's state as JSON, changing nothing
  --here --port <n>                for a XenoNet whose service runs on another machine
                                   and answers on this one through a forwarded port
                                   (a VM, say): the entries go in this computer's
                                   settings and run through its own launcher, and
                                   only that XenoNet's are changed`;

// The apps connect goes through, as the app registry has them.
const CONNECTED = appsWith("connect").map((a) => a.id);

// Names as a sentence lists them: "A, B and C".
const andList = (names) => (names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names.at(-1)}` : names.join(""));

// The apps to connect: the ones named, or every one that's here (an app
// that may not be, Grok Build, says in the preview whether it is).
const chosen = (names, preview) => (names.length ? names.map((n) => (n === "claude-code" ? "claude" : n)) : CONNECTED.filter((id) => preview.apps.find((a) => a.app === id)?.installed !== false));

// How each change shows: its mark, color and what happens to it.
const LOOK = { present: ["✓", "green", "already in place"], missing: ["+", "yellow", "to add"], different: ["~", "yellow", "to update"], remove: ["-", "red", "to take out"] };

export function render(preview, { apps = CONNECTED } = {}) {
  for (const a of preview.apps.filter((x) => apps.includes(x.app))) {
    print(c.bold(a.name));
    for (const ch of a.changes) {
      const [mark, color, say] = LOOK[ch.state] ?? ["?", "gray", ch.state];
      print(`  ${c[color](mark)} ${ch.what} ${c.dim(`(${say})`)}`);
      print(c.dim(`      ${ch.where}`));
      print(c.dim(`      ${ch.detail}`));
    }
    for (const b of a.broken ?? []) print(`  ${c.red("✗")} ${b}`);
    for (const n of a.notes ?? []) print(c.dim(`  ${n}`));
    print();
  }
}

export async function confirm(question, flags) {
  if (flags.yes) return true;
  if (!process.stdin.isTTY) throw new Fail(`${question} Run it again with --yes to go ahead.`);
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return /^y(es)?$/i.test((await rl.question(`${question} ${c.dim("[y/N]")} `)).trim());
  } finally {
    rl.close();
  }
}

// The XenoNet on that port runs elsewhere, so it can't write this computer's
// settings: they're written here, as disconnect writes them with no XenoNet
// running, with this computer's launcher and that XenoNet's port.
async function here(names, flags) {
  if (!flags.port) throw new Fail("--here needs --port: the port the other XenoNet answers on here", 2);
  const port = candidates(flags)[0];
  const local = createConnect({ port, root: ROOT });
  const preview = await local.preview();
  if (flags.json) return json(preview);
  const apps = chosen(names, preview);
  const todo = preview.apps.filter((a) => apps.includes(a.app) && (a.changes.some((ch) => ch.state !== "present") || a.broken?.length));
  const who = `the XenoNet on port ${port}`;
  render(preview, { apps });
  if (!todo.length) return print(c.green(`Already connected to ${who}.`));
  if (!(await confirm(`Connect ${andList(todo.map((a) => a.name))} here to ${who}?`, flags))) return print(c.dim("Nothing changed."));
  const after = await local.apply(todo.map((a) => a.app));
  const left = after.apps.filter((a) => apps.includes(a.app)).flatMap((a) => a.changes.filter((ch) => ch.state !== "present").map((ch) => `${a.name}: ${ch.what}`));
  if (left.length) throw new Fail(`some changes didn't take: ${left.join("; ")}. xenonet doctor says more.`);
  print(`${c.green("✓")} Connected ${andList(todo.map((a) => a.name))} to ${who}. While nothing answers there, its hooks stay out of the way.`);
}

export async function run(names, flags) {
  const bad = names.map((n) => (n === "claude-code" ? "claude" : n)).filter((a) => !CONNECTED.includes(a));
  if (bad.length) throw new Fail(`connect works with ${andList(CONNECTED)}, not ${bad.join(", ")}`, 2);
  if (flags.here) return here(names, flags);
  const s = await need(flags);
  const preview = await get(s, "/api/engine/connect");
  if (flags.json) return json(preview);
  const apps = chosen(names, preview);
  if (preview.demo) throw new Fail(`${nameOf(s)} shows made-up data, so it can't connect Claude Code and Codex. Use XenoNet or XenoNet Dev.`);
  const todo = preview.apps.filter((a) => apps.includes(a.app) && (a.changes.some((ch) => ch.state !== "present") || a.broken?.length));
  if (!todo.length) {
    render(preview, { apps });
    return print(c.green(`Already connected to ${nameOf(s)}.`));
  }
  render(preview, { apps });
  const who = andList(todo.map((a) => a.name));
  if (!(await confirm(`Connect ${who} to ${nameOf(s)}?`, flags))) return print(c.dim("Nothing changed."));
  const after = await post(s, "/api/engine/connect", { apps: todo.map((a) => a.app) });
  const left = after.apps.filter((a) => apps.includes(a.app)).flatMap((a) => a.changes.filter((ch) => ch.state !== "present").map((ch) => `${a.name}: ${ch.what}`));
  if (left.length) throw new Fail(`some changes didn't take: ${left.join("; ")}. xenonet doctor says more.`);
  print(`${c.green("✓")} Connected ${who} to ${nameOf(s)}. New sessions get XenoNet's tools and hooks; sessions already open pick them up when restarted.`);
  if (todo.some((a) => a.app === "codex")) print(c.dim("  Codex asks you to trust new hooks once: run /hooks in Codex and trust XenoNet's."));
}
