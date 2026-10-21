// core/xenolith.js in Rust (crates/xenonet-sources/src/xenolith.rs): where
// the shared context's wiki is (Xenolith's, else Agent Wiki's), and which
// MCP server writes to it.
import os from "node:os";
import { native } from "../index.js";

export const CONTEXT_SERVERS = new Set(native.contextServers());
export const isContextServer = (server) => CONTEXT_SERVERS.has(server);

export function findContextDir({ env = process.env, home = os.homedir(), platform = process.platform } = {}) {
  return native.findContextDir({ ...env }, String(home), String(platform));
}

export function contextServerOf(dir, { env = process.env, home = os.homedir(), platform = process.platform } = {}) {
  return dir ? native.contextServerOf(String(dir), { ...env }, String(home), String(platform)) : null;
}

export const contextName = (server) => native.contextName(typeof server === "string" ? server : null);
