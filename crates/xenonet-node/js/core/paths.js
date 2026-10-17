// core/paths.js in Rust (crates/xenonet-base/src/paths.rs): keys that
// match however an app wrote a folder, home as "~", the throwaway folders'
// zones, and the walks that list files.
import path from "node:path";
import { native } from "../index.js";

const STYLES = { win32: path.win32, posix: path.posix, native: path };

export const pathStyle = (p) => STYLES[native.pathStyle(String(p ?? ""))];

export const pathKey = (p) => (p ? native.pathKey(String(p)) : null);

export const isUnder = (childKey, parentKey) => !!childKey && !!parentKey && native.isUnder(String(childKey), String(parentKey));

export const tilde = (text, home) => (!text || !home ? text : native.tilde(String(text), String(home)));

export const baseName = (p) => native.baseName(String(p ?? ""));

const ZONES = native.scratchZones().map((z, i) => ({ ...z, test: (k) => native.inScratchZone(i, String(k)) }));

export const scratchZone = (key) => (key ? ZONES[native.scratchZone(String(key))] ?? null : null);

export function onPath(cmd, { env = process.env, platform = process.platform, canRun = null } = {}) {
  return native.onPath(String(cmd), String(env.PATH ?? ""), env.PATHEXT, platform === "win32", canRun) ?? null;
}

export async function listFiles(dir, match, depth = 6) {
  return native.listFiles(String(dir), (name) => !!match(name), depth);
}
