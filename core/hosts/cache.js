// Each host's last snapshot, in the cache folder (docs/design/hosts.md), so
// an offline host is still on the map after a restart. Kept by the host's
// key, so a rename here or a new SSH alias finds it again; written at most
// every 30 seconds (the hub decides when); and only what came with the
// snapshot and the events after it, never a transcript or stats.
import fs from "node:fs";
import path from "node:path";
import { HOST_KEY } from "../schema/host.js";

export const CACHE_FOLDER = "hosts";

const fileOf = (dir, key) => path.join(dir, CACHE_FOLDER, `${key}.json`);

// { key, id, savedAt, hello, graph, live, engine }, or null when there's
// none, or none that can be read.
export function readCache(dir, key) {
  if (!dir || !HOST_KEY.test(String(key ?? ""))) return null;
  try {
    const data = JSON.parse(fs.readFileSync(fileOf(dir, key), "utf8"));
    if (data?.key !== key || !Number.isFinite(data.savedAt) || !Array.isArray(data.graph?.nodes) || !data.live || typeof data.live !== "object") return null;
    return data;
  } catch {
    return null;
  }
}

export function writeCache(dir, key, { id, savedAt, hello = null, graph, live, engine = null }) {
  if (!dir || !HOST_KEY.test(String(key ?? "")) || !graph || !live) return false;
  const file = fileOf(dir, key);
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
    fs.writeFileSync(`${file}.tmp`, JSON.stringify({ key, id, savedAt, hello, graph, live, engine }), { mode: 0o600 });
    fs.renameSync(`${file}.tmp`, file);
    return true;
  } catch {
    return false;
  }
}

export function removeCache(dir, key) {
  if (!dir || !HOST_KEY.test(String(key ?? ""))) return;
  fs.rmSync(fileOf(dir, key), { force: true });
}
