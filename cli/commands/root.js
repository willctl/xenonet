// xenonet root: the projects folder XenoNet reads, where that comes from, and
// reading another one (core/workspace.js, through a running XenoNet's API).
import path from "node:path";
import { c, print, json, plural, ago, Fail } from "../lib/out.js";
import { need, get, post, del } from "../../core/client.js";
import { tilde } from "../../core/paths.js";

export const help = `The projects folder XenoNet reads, and where that comes from: XENONET_PROJECTS_DIR
when it's set, a folder you chose, or ~/Projects.

  xenonet root                  the folder, and what XenoNet found in it
  xenonet root ~/code           read ~/code instead: XenoNet checks it, reads it
                              and redraws the map
  xenonet root --check ~/code   what XenoNet would find there, without reading it
  xenonet root --reset          back to ~/Projects
  --json                      the answer as JSON

Your layout, your rules and Codex's picks stay with each folder, so coming
back to one brings them back. The console's palette has the same, as
Change the projects folder…`;

const FROM = { env: "set by XENONET_PROJECTS_DIR", chosen: "chosen in XenoNet", default: "the default" };
const found = (sc) => `${plural(sc.projects, "project")} in ${plural(sc.groups, "group")}`;

// A folder as given here. ~ is left for XenoNet, which reads it as its own
// home folder (the demo has one of its own); anything else is taken from
// where the command runs.
const folder = (arg) => (/^~([\\/]|$)/.test(arg) ? arg : path.resolve(arg));

export async function run([dir], flags) {
  if (flags.reset && dir) throw new Fail("give a folder, or --reset for the default, not both", 2);
  if (flags.check && !dir) throw new Fail("say which folder to check: xenonet root --check <folder>", 2);
  const s = await need(flags);
  const info = await get(s, "/api/workspace/root");
  const shown = (p) => tilde(p, info.home);

  if (flags.check) {
    const r = await post(s, "/api/workspace/check", { path: folder(dir) });
    if (flags.json) json(r);
    else if (!r.ok) print(`${c.red("✗")} ${r.error}`);
    else {
      const what = r.scan ? `${found(r.scan)} ${c.dim(`(${plural(r.scan.dirsRead, "folder")} read in ${r.scan.ms} ms)`)}` : "still scanning";
      print(`${c.green("✓")} ${shown(r.path)}: ${what}${r.current ? c.dim(" · the folder XenoNet reads now") : ""}`);
      for (const w of r.warnings) print(c.yellow(`! ${w}`));
    }
    return r.ok ? 0 : 1;
  }

  if (dir || flags.reset) {
    const r = flags.reset ? await del(s, "/api/workspace/root") : await post(s, "/api/workspace/root", { path: folder(dir) });
    if (flags.json) return json(r);
    const scan = r.scan?.ok ? `: ${found(r.scan)}` : "";
    print(r.changed ? `${c.green("✓")} XenoNet reads ${shown(r.path)} now${scan}` : `${c.dim("·")} XenoNet reads ${shown(r.path)} already${scan}`);
    if (r.changed) print(c.dim(`  The layout, your rules and Codex's picks for ${shown(info.path)} stay, for when you come back.`));
    return;
  }

  if (flags.json) return json(info);
  print(`${c.green("●")} ${c.bold(shown(info.path))} ${c.dim(FROM[info.source] ?? info.source)}`);
  const sc = info.scan;
  print(`  ${sc?.ok ? `${found(sc)}${sc.truncated ? c.yellow(`, stopped at the ${sc.maxDirs}-folder limit`) : ""} ${c.dim(`(scanned ${ago(sc.at)})`)}` : sc ? c.red("XenoNet can't read it") : c.dim("not scanned yet")}`);
  if (info.source === "env") print(c.dim("  To choose a folder with xenonet root or in the console, unset XENONET_PROJECTS_DIR and restart XenoNet."));
  else if (info.source === "chosen") print(c.dim(`  xenonet root --reset goes back to ${shown(info.default)}.`));
  else print(c.dim("  xenonet root <folder> reads another."));
}
