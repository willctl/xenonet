// XenoNet's plugin for Hermes Agent (plugins/hermes/xenonet): putting it
// in $HERMES_HOME/plugins/xenonet, taking it out, and saying whether it's
// installed, turned on and reporting. `xenonet connect hermes` and
// `xenonet doctor` use this.
//
// XenoNet only copies the plugin's two files. It never edits Hermes's
// config.yaml (plugins are off until their name is in plugins.enabled, which
// is the owner's step) and never makes a token: the owner's token goes in
// the plugin's `token` file (docs/design/actors/hermes.md, section 6).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { hermesRoot } from "./hermes-files.js";

export const PLUGIN_FILES = ["plugin.yaml", "__init__.py"];
const SOURCE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../plugins/hermes/xenonet");
// A report older than this means the plugin isn't running (it beats every 15 s).
export const STALE_MS = 2 * 60e3;

export const pluginDir = (home, env = process.env) => path.join(hermesRoot(home, env), "plugins", "xenonet");

// Whether config.yaml's plugins.enabled lists xenonet, in either YAML form:
// `enabled: [a, xenonet]` or a block of `- xenonet` lines. A plain scan, since
// it only has to find one name under one key.
export function enabledIn(text) {
  const lines = String(text ?? "").split("\n");
  const at = lines.findIndex((l) => /^plugins:\s*(#.*)?$/.test(l));
  if (at < 0) return false;
  const bare = (s) => s.trim().replace(/^["']|["']$/g, "");
  for (let i = at + 1; i < lines.length; i++) {
    const l = lines[i];
    // A comment at column 0 doesn't end the block: Hermes's own config has
    // them inside `plugins:`.
    if (/^#/.test(l)) continue;
    if (/^\S/.test(l)) return false;
    const m = /^(\s+)enabled:\s*(.*?)\s*(#.*)?$/.exec(l);
    if (!m) continue;
    if (m[2].startsWith("[")) return m[2].slice(1, -1).split(",").some((n) => bare(n) === "xenonet");
    for (let j = i + 1; j < lines.length; j++) {
      if (/^\s*(#.*)?$/.test(lines[j])) continue;
      const item = /^(\s*)-\s*(\S.*?)\s*(#.*)?$/.exec(lines[j]);
      if (!item || item[1].length < m[1].length) return false;
      if (bare(item[2]) === "xenonet") return true;
    }
    return false;
  }
  return false;
}

const read = (file) => {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return null;
  }
};

// What's there: { dir, installed, current, enabled, token, reported }.
//   current   the files are the ones this XenoNet ships
//   token     "none", "loose" (others can read it: the plugin refuses it) or "ok"
//   reported  null, or { ok, ageMs }: the plugin's last heartbeat
export function inspect({ home, env = process.env, now = Date.now() } = {}) {
  const dir = pluginDir(home, env);
  const installed = PLUGIN_FILES.every((f) => fs.existsSync(path.join(dir, f)));
  const current = installed && PLUGIN_FILES.every((f) => read(path.join(dir, f)) === read(path.join(SOURCE, f)));
  let token = "none";
  try {
    const st = fs.statSync(path.join(dir, "token"));
    token = !st.isFile() ? "none" : process.platform !== "win32" && st.mode & 0o077 ? "loose" : "ok";
  } catch {}
  let reported = null;
  try {
    const s = JSON.parse(fs.readFileSync(path.join(dir, "state.json"), "utf8"));
    if (typeof s.at === "number") reported = { ok: s.ok === true, ageMs: Math.max(0, now - s.at) };
  } catch {}
  const enabled = enabledIn(read(path.join(hermesRoot(home, env), "config.yaml")));
  return { dir, installed, current, enabled, token, reported };
}

export function install({ home, env = process.env } = {}) {
  const dir = pluginDir(home, env);
  fs.mkdirSync(dir, { recursive: true });
  for (const f of PLUGIN_FILES) fs.writeFileSync(path.join(dir, f), fs.readFileSync(path.join(SOURCE, f)));
  return dir;
}

// Takes the plugin's folder out, token and state included; config.yaml stays
// as it is, and a name left in plugins.enabled does nothing without a folder.
export function uninstall({ home, env = process.env } = {}) {
  const dir = pluginDir(home, env);
  const was = fs.existsSync(dir);
  fs.rmSync(dir, { recursive: true, force: true });
  return was;
}

// What the owner still does, as lines to print, given inspect()'s answer.
export function ownerSteps(st, tilde = (p) => p) {
  const steps = [];
  if (st.token !== "ok") steps.push(`Give the plugin a token: in XenoNet's Actors panel, add an actor for Hermes Agent and save its token to ${tilde(path.join(st.dir, "token"))} (only you can read it).${st.token === "loose" ? " The file there is readable by others, so the plugin ignores it: chmod 600 it." : ""}`);
  if (!st.enabled) steps.push("Turn the plugin on, which Hermes leaves to you: hermes plugins enable xenonet, or add xenonet to plugins.enabled in config.yaml. It takes effect with the next Hermes session.");
  return steps;
}
