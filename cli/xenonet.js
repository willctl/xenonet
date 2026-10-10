#!/usr/bin/env node
// The xenonet command. Each command lives in commands/ and is loaded only when
// it runs, so the command starts fast. Commands share the core with the
// console, the server and the MCP servers: they call core/ directly, or a
// running XenoNet over its HTTP API.
import { Fail, print, c, setColor } from "./lib/out.js";
import { COMMANDS, ALIASES } from "./lib/commands.js";
import { parse } from "./lib/args.js";

function help() {
  const shown = Object.entries(COMMANDS).filter(([, v]) => !v.hidden);
  const w = Math.max(...shown.map(([k, v]) => `${k} ${v.args ?? ""}`.trim().length));
  print(`${c.bold("xenonet")} ${c.dim("· the network for the SIs on this computer")}

${c.bold("Usage")}
  xenonet <command> [options]

${c.bold("Commands")}
${shown.map(([k, v]) => `  ${`${k} ${v.args ?? ""}`.trim().padEnd(w + 2)}${c.dim(v.about)}`).join("\n")}

${c.bold("Options")}
  --json         JSON, for scripts
  --dev, --demo  XenoNet Dev (port 47842) or its demo (47841), rather than XenoNet
  --port <n>     The XenoNet on this port
  --no-color     Plain text
  -h, --help     Help, for xenonet or a command
  -v, --version  The version

${c.bold("Examples")}
  xenonet doctor
  xenonet agents --repo
  xenonet who src/scene.js
  xenonet connect`);
}

// Output piped into something that stopped reading (xenonet agents | head)
// ends the command quietly.
process.stdout.on("error", (err) => (err.code === "EPIPE" ? process.exit(0) : (process.exitCode = 1)));

async function main() {
  const { flags, rest } = parse(process.argv.slice(2));
  if (flags.color === false || flags.json) setColor(false);
  let [name, ...args] = rest;
  name = ALIASES[name] ?? name;
  if (flags.version) {
    const { readFile } = await import("node:fs/promises");
    const pkg = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
    return print(flags.json ? JSON.stringify({ version: pkg.version }) : `xenonet ${pkg.version}`);
  }
  if (!name) return flags.help ? help() : run("status", [], flags);
  if (name === "help") return args[0] ? run(ALIASES[args[0]] ?? args[0], [], { ...flags, help: true }) : help();
  if (!COMMANDS[name]) throw new Fail(`no command "${name}". xenonet --help lists them.`, 2);
  return run(name, args, flags);
}

async function run(name, args, flags) {
  const mod = await import(`./commands/${name}.js`);
  if (flags.help) {
    const cmd = COMMANDS[name];
    print(`${c.bold(`xenonet ${name}`)}${cmd.args ? ` ${cmd.args}` : ""}\n\n${mod.help ?? cmd.about}`);
    return;
  }
  return mod.run(args, flags);
}

main().then(
  (code) => {
    if (typeof code === "number") process.exitCode = code;
  },
  // A message, and the stack only with XENONET_DEBUG=1. What XenoNet didn't
  // expect (not a Fail, which says how it was used wrong) goes in its
  // error log too, which xenonet logs reads.
  async (err) => {
    process.stderr.write(`xenonet: ${process.env.XENONET_DEBUG === "1" ? (err?.stack ?? err) : (err?.message ?? err)}\n`);
    process.exitCode = err instanceof Fail ? err.code : 1;
    if (err instanceof Fail) return;
    try {
      const [{ createLog, logDir }, { appDir }] = await Promise.all([import("../core/log.js"), import("../core/dirs.js")]);
      const [name] = process.argv.slice(2).filter((a) => !a.startsWith("-"));
      createLog({ dir: logDir(process.env.XENONET_STATE_DIR || appDir("state")), component: "cli" }).error(`cli.${/^[a-z]+$/.test(name ?? "") ? name : "command"}`, err?.message ?? String(err), { err });
    } catch {}
  },
);
