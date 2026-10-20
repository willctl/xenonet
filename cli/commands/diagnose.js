// xenonet diagnose: what XenoNet can tell someone helping you, safe to share
// (core/diagnostics.js), and with --ask, an agent on this computer to look
// at it with you. It works with XenoNet running or not.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { c, print } from "../lib/out.js";
import { find, get } from "../../core/client.js";
import { checkup } from "../../core/doctor/checkup.js";
import { machine } from "../../core/doctor/index.js";
import { appDir } from "../../core/dirs.js";
import { onPath } from "../../core/paths.js";
import { logDir, readLogs } from "../../core/log.js";
import { makeBundle, bundleMarkdown, writeLook, askCommand } from "../../core/diagnostics.js";

export const help = `A summary of XenoNet's state that's safe to share: its versions, the doctor's
findings that aren't ok, its errors from the last day by code, where its
hooks point and which XENONET_ settings are set. No transcripts, titles,
project names, identities or keys, and home folders show as ~.

  --json            as JSON
  --out <file>      write it to a file
  --ask             have a session on this computer look at it with you.
                    It's a Codex session (or Claude Code, with --app claude)
                    in a folder of its own, read-only, that can read XenoNet's
                    code and never files anything. If it finds a bug in
                    XenoNet, it writes issue-draft.md there, for you to send
                    if you choose
  --app <app>       codex (default) or claude; XENONET_DIAGNOSE_APP sets it
  --model <model>   the model it uses; XENONET_DIAGNOSE_MODEL sets it
  --dev             about XenoNet Dev, not the installed XenoNet`;

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

export async function gather(flags = {}) {
  const s = await find(flags);
  const stateDir = process.env.XENONET_STATE_DIR || appDir("state", flags.dev ? { name: "XenoNet Dev" } : {});
  const version = JSON.parse(fs.readFileSync(path.join(ROOT, "package.json"), "utf8")).version;
  const [doctor, connect, launcher] = await Promise.all([
    checkup(flags).catch(() => null),
    s ? get(s, "/api/engine/connect").catch(() => null) : null,
    machine().launcher().catch(() => null),
  ]);
  return {
    stateDir,
    bundle: makeBundle({
      xenonet: { version: s?.health?.version ?? version, instance: s?.health?.instance ?? (flags.dev ? "dev" : null), packaged: !!(process.env.XENONET_LAUNCHER_BIN || process.env.XENONET_LAUNCHER_EXEC), running: !!s },
      doctor,
      errors: readLogs(logDir(stateDir), { since: Date.now() - 24 * 3600e3, level: "warn" }),
      connect,
      launcher,
    }),
  };
}

export async function run(_args, flags) {
  const { stateDir, bundle } = await gather(flags);
  if (!flags.ask) {
    const text = flags.json ? `${JSON.stringify(bundle, null, 2)}\n` : bundleMarkdown(bundle);
    if (flags.out) {
      fs.writeFileSync(flags.out, text);
      return print(`${c.green("✓")} Wrote ${flags.out}. It's safe to share: no transcripts, titles, names or keys.`);
    }
    return process.stdout.write(text);
  }
  const app = (flags.app || process.env.XENONET_DIAGNOSE_APP || "codex") === "claude" ? "claude" : "codex";
  const model = flags.model || process.env.XENONET_DIAGNOSE_MODEL || null;
  const bin = onPath(app);
  if (!bin) throw new Error(`${app === "claude" ? "Claude Code" : "Codex"} isn't on your PATH; install it, or use --app ${app === "claude" ? "codex" : "claude"}.`);
  const dir = writeLook(stateDir, bundle, { sources: [ROOT] });
  const cmd = askCommand({ app, model, dir, sources: [ROOT] });
  print(`${c.dim(`XenoNet wrote what it could tell to ${dir}.`)}\n${c.dim(`Starting ${app === "claude" ? "Claude Code" : "Codex"}${model ? ` with ${model}` : ""}: it reads, explains and asks before changing anything, and never files anything.`)}\n`);
  // A .cmd (an npm install) runs only through the shell.
  const viaShell = /\.(cmd|bat)$/i.test(bin);
  const child = spawn(viaShell ? `"${bin}"` : bin, viaShell ? cmd.args.map((a) => `"${a}"`) : cmd.args, { cwd: cmd.cwd, stdio: "inherit", shell: viaShell });
  return await new Promise((resolve) => {
    child.on("error", (err) => {
      process.stderr.write(`xenonet: couldn't start ${app}: ${err.message}\n`);
      resolve(1);
    });
    child.on("exit", (code) => {
      const draft = path.join(dir, "issue-draft.md");
      if (fs.existsSync(draft)) print(`\n${c.yellow("!")} It wrote an issue draft: ${draft}\n  Read it, and send it yourself if you choose: https://github.com/willctl/xenonet/issues/new`);
      resolve(code ?? 0);
    });
  });
}
