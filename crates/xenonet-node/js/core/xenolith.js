// core/xenolith.js in Rust (crates/xenonet-sources/src/xenolith.rs): where
// the shared context's folder is (Xenolith's, else Agent Wiki's), and
// which MCP server writes to it.
import os from "node:os";
import { native } from "../index.js";

export const CONTEXT_SERVERS = new Set(native.contextServers());
export const isContextServer = (server) => CONTEXT_SERVERS.has(server);

export const CONTEXT_TOOLS = new Set(native.contextTools());
export const isContextCall = (server, tool, named = null) => native.contextIsCall(server, tool, named);

export const CONTEXT_FORMAT = native.contextFormat();
export const MARKER_FILE = native.contextMarkerFile();

export const readMarker = (dir) => (dir ? native.contextReadMarker(String(dir)) : null);

export const formatProblem = (marker) => (marker ? native.contextFormatProblem(Number(marker.format)) : null);

export function findContextDir({ env = process.env, home = os.homedir(), platform = process.platform } = {}) {
  return native.findContextDir({ ...env }, String(home), String(platform));
}

export function contextServerOf(dir, { env = process.env, home = os.homedir(), platform = process.platform } = {}) {
  return dir ? native.contextServerOf(String(dir), { ...env }, String(home), String(platform)) : null;
}

export const contextName = (server) => native.contextName(typeof server === "string" ? server : null);

export const SPACE_KINDS = native.contextSpaceKinds();

export function spaceOf(v) {
  const { space, refused } = native.contextSpaceOf(v);
  return refused ? undefined : space;
}
