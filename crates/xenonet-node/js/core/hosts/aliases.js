// core/hosts/aliases.js in Rust (crates/xenonet-hosts/src/aliases.rs): a
// home folder's SSH aliases, read again when ~/.ssh/config changes. When to
// read is the Rust's; the reading of the file's words is identities.js's.
import fs from "node:fs";
import { native } from "../../index.js";
import { peer } from "../../peer.js";

const { parseSshAliases } = await import(peer("identities.js"));

export function aliasReader(home) {
  if (!home) return () => new Map();
  const reader = new native.HostsAliases(String(home));
  let aliases = new Map();
  return () => {
    const look = reader.look();
    if (look === "none") aliases = new Map();
    else if (look !== "same") {
      try {
        aliases = parseSshAliases(fs.readFileSync(look, "utf8"));
      } catch {
        aliases = new Map();
      }
    }
    return aliases;
  };
}
