// xenonet disconnect: takes XenoNet back out of Claude Code's and Codex's
// settings (core/engine/connect.js), leaving everything else as it was.
// With no XenoNet running on the port it names, it takes that XenoNet's
// entries out by itself, so uninstalling XenoNet leaves no hook behind that
// runs files about to be deleted.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { c, print } from "../lib/out.js";
import { find, get, del, nameOf, candidates } from "../../core/client.js";
import { createConnect } from "../../core/engine/connect.js";
import { render, confirm } from "./connect.js";

export const help = `Takes XenoNet's MCP server and hooks out of Claude Code's and Codex's
settings, and leaves your own entries alone. It asks first. With no XenoNet
running on that port, it takes them out itself.

  --yes        don't ask
  --port <n>   which XenoNet's entries (47840, the installed XenoNet's, unless
               XENONET_PORT or --dev says otherwise)`;

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

export async function run(_args, flags) {
  const s = await find(flags);
  // Nothing answers: this XenoNet's entries, by its port, taken out here.
  const local = s ? null : createConnect({ port: candidates(flags)[0], root: ROOT });
  const preview = s ? await get(s, "/api/engine/connect") : await local.preview();
  // What XenoNet's plugin for Claude Code carries is the plugin's, not
  // connect's, so it stays.
  const ours = (ch) => ch.state !== "missing" && ch.via !== "plugin";
  const plugin = preview.apps.some((a) => a.changes.some((ch) => ch.via === "plugin"));
  const there = preview.apps.filter((a) => a.changes.some(ours));
  if (!there.length) {
    print(c.dim("XenoNet isn't in Claude Code's or Codex's settings; nothing to take out."));
    if (plugin) print(c.dim(PLUGIN_STAYS));
    return;
  }
  render({ ...preview, apps: there.map((a) => ({ ...a, notes: [], broken: [], changes: a.changes.filter(ours).map((ch) => ({ ...ch, state: "remove" })) })) });
  if (!(await confirm(`Take XenoNet out of ${there.map((a) => a.name).join(" and ")}?`, flags))) return print(c.dim("Nothing changed."));
  if (s) await del(s, "/api/engine/connect");
  else await local.remove();
  print(`${c.green("✓")} Took XenoNet out of ${there.map((a) => a.name).join(" and ")}${s ? ` (${nameOf(s)} keeps running)` : ""}. Sessions already open keep their tools until restarted.`);
  if (plugin) print(c.dim(PLUGIN_STAYS));
}

const PLUGIN_STAYS = "XenoNet's plugin for Claude Code stays. /plugin in Claude Code uninstalls it.";
