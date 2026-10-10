// XenoNet's version, from its package.json, for what it tells the programs
// and services it talks to.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const VERSION = (() => {
  try {
    return JSON.parse(readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "package.json"), "utf8")).version;
  } catch {
    return "dev";
  }
})();
