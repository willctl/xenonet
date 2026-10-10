// xenonet uninstall: takes the xenonet command off your PATH (core/command.js).
import os from "node:os";
import path from "node:path";
import { c, print, json, Fail } from "../lib/out.js";
import { commandDir, commandState, uninstallCommand } from "../../core/command.js";
import { tilde } from "../../core/paths.js";

export const help = `Takes the xenonet command out of ~/.local/bin, if XenoNet put it there.

  --dir <folder>  where you installed it, if somewhere else`;

export async function run(_args, flags) {
  const dir = typeof flags.dir === "string" ? path.resolve(flags.dir) : commandDir();
  const before = commandState({ dir });
  let st;
  try {
    st = uninstallCommand({ dir });
  } catch (err) {
    throw new Fail(err.message);
  }
  if (flags.json) return json(st);
  print(before.installed ? `${c.green("✓")} Took out ${tilde(before.file, os.homedir())}` : c.dim(`There's no xenonet command in ${tilde(dir, os.homedir())}.`));
}
