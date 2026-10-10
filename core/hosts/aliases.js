// A home folder's SSH aliases (Host gh-work with HostName github.com), read
// again when ~/.ssh/config changes. A repo key resolves them on the
// project's own machine (repoKey in core/schema/ids.js): the relay with its
// host's, the hub with this machine's. Only read, never written.
import fs from "node:fs";
import path from "node:path";
import { parseSshAliases } from "../identities.js";

export function aliasReader(home) {
  if (!home) return () => new Map();
  const file = path.join(home, ".ssh", "config");
  let stamp;
  let aliases = new Map();
  return () => {
    const now = fs.statSync(file, { throwIfNoEntry: false })?.mtimeMs ?? null;
    if (now !== stamp) {
      stamp = now;
      try {
        aliases = now == null ? new Map() : parseSshAliases(fs.readFileSync(file, "utf8"));
      } catch {
        aliases = new Map();
      }
    }
    return aliases;
  };
}
