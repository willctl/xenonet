// xenonet console: drives XenoNet Dev one step after another, through its
// control channel (core/control.js), the way the xenonet-console MCP tools do.
// It's how an agent without those tools, or a person at a terminal, checks
// a change to XenoNet. npm run drive is the same command.
import fs from "node:fs";
const installed = JSON.parse(fs.readFileSync(new URL("../../package.json", import.meta.url), "utf8")).xenonetComponent === "tools";
import { print, Fail } from "../lib/out.js";
import { xenonet } from "../../core/control.js";
import { STEPS, runScript } from "../../core/script.js";

export const help = `Drives XenoNet Dev from a terminal, one step after another: the demo
instance (made-up data) unless --instance says dev (your sessions) or test.

  xenonet console launch "select:atlas-api" shot:atlas
  xenonet console --instance dev state
  xenonet console key:Mod+K type:wallpaper key:Enter shot:wallpaper

Steps:
${STEPS}`;

export async function run(args, flags) {
  if (!args.length) throw new Fail("say what to do: xenonet console <steps…>; xenonet console --help lists the steps", 2);
  const s = xenonet(flags.instance || process.env.XENONET_INSTANCE || (installed ? "installed" : "demo"));
  await runScript(s, args, (v) => print(typeof v === "string" ? v : JSON.stringify(v)));
}
