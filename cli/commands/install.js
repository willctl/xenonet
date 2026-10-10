// xenonet install: puts the xenonet command on your PATH (core/command.js), run
// by the Node that runs this.
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { c, print, json, Fail } from "../lib/out.js";
import { commandDir, currentRuntime, installCommand } from "../../core/command.js";
import { tilde } from "../../core/paths.js";

export const help = `Installs a small script that runs this checkout's xenonet with this Node.
It goes in ~/.local/bin, or %LOCALAPPDATA%/XenoNet/bin on Windows.
Add that folder to PATH if it isn't there yet.

  --dir <folder>  somewhere else on your PATH
  --force         replace a xenonet there that isn't XenoNet's
  --json          the result as JSON`;

export async function run(_args, flags) {
  const entry = fileURLToPath(new URL("../xenonet.js", import.meta.url));
  const dir = typeof flags.dir === "string" ? path.resolve(flags.dir) : commandDir();
  let st;
  try {
    st = installCommand({ dir, entry, force: !!flags.force, ...currentRuntime({ env: { ...process.env, XENONET_EXEC: "" } }) });
  } catch (err) {
    throw new Fail(`${err.message}${/isn't XenoNet's/.test(err.message) ? ". --force replaces it." : ""}`);
  }
  if (flags.json) return json(st);
  const home = os.homedir();
  print(`${c.green("✓")} ${tilde(st.file, home)} runs ${tilde(st.runs.join(" "), home)}`);
  if (!st.onPath) print(c.yellow(process.platform === "win32" ? `  ${tilde(dir, home)} isn't on your PATH yet. Add it to your user Path in Windows' Environment Variables, then open a new terminal.` : `  ${tilde(dir, home)} isn't on your PATH yet. Add it in your shell's profile, for example:\n  export PATH="${dir.startsWith(home + path.sep) ? `$HOME${dir.slice(home.length)}` : dir}:$PATH"`));
}
