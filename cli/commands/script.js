// xenonet script: what you did in XenoNet Dev, saved as a script (the console
// logs it as steps; src/oplog.js), and scripts run again, in any instance,
// shown or hidden (core/script.js).
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { c, print, json, Fail } from "../lib/out.js";
import { xenonet } from "../../core/control.js";
import { runScript, runStep, brief } from "../../core/script.js";

export const help = `Scripts drive XenoNet Dev one step after another, as xenonet console does,
and the console keeps a log of what you do there as the same steps.

  xenonet script record --instance dev > tour.json   what you did, as a script
  xenonet script run tour.json                       play it in the demo
  xenonet script run tour.json --instance dev --hidden
  xenonet script run film.mjs                        a JS module, for anything else

A script is a JSON list of steps: strings as xenonet console takes them
("select:atlas-api", "shot:atlas", "film:30") or { "op": "select",
"args": ["project:atlas-api"] }. A module's default export gets the
instance (core/control.js) and a step function:

  export default async function ({ xenonet, step }) {
    await step("theme:helios");
    await step({ op: "stats", args: ["claude:…"] });
    await xenonet.screenshot({ name: "stats" });
  }

  --instance <id>   demo (the default), dev, test or film
  --hidden          start it hidden if it isn't running
  --last <n>        record only the last n steps
  --clear           record, then empty the log`;

export async function run([verb, file], flags) {
  const s = xenonet(flags.instance || process.env.XENONET_INSTANCE || "demo");
  const say = flags.json ? () => {} : (v) => print(c.dim(typeof v === "string" ? v : JSON.stringify(v)));
  if (verb === "record") {
    const ops = await s.driver("ops");
    const steps = ops.slice(-(Number(flags.last) || ops.length)).map(({ op, args }) => ({ op, args }));
    if (flags.clear) await s.driver("clearOps");
    return json(steps);
  }
  if (verb !== "run" || !file) throw new Fail("xenonet script record, or xenonet script run <file>; xenonet script --help says more", 2);
  const full = path.resolve(file);
  if (!fs.existsSync(full)) throw new Fail(`there's no ${file}`);
  await s.launch({ show: flags.hidden ? "hidden" : "inactive" });
  if (/\.[cm]?js$/.test(full)) {
    const mod = await import(pathToFileURL(full).href);
    if (typeof mod.default !== "function") throw new Fail(`${file} needs a default export: async function ({ xenonet, step }) { … }`);
    await mod.default({ xenonet: s, step: (st) => runStep(s, st, say) });
  } else {
    let steps;
    try {
      steps = JSON.parse(fs.readFileSync(full, "utf8"));
    } catch (err) {
      throw new Fail(`${file} isn't JSON: ${err.message}`);
    }
    if (!Array.isArray(steps)) throw new Fail(`${file} should be a list of steps`);
    await runScript(s, steps, say);
  }
  const st = brief(await s.driver("state"));
  if (flags.json) return json(st);
  print(`${c.green("✓")} ${path.basename(file)} ran ${c.dim(JSON.stringify(st))}`);
}
