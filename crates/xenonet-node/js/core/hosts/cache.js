// core/hosts/cache.js in Rust (crates/xenonet-hosts/src/cache.rs): each
// host's last snapshot, in the cache folder, by the host's key.
import { native } from "../../index.js";
import { errorOf } from "../../settle.js";

export const CACHE_FOLDER = "hosts";

export function readCache(dir, key) {
  if (!dir) return null;
  return native.hostsReadCache(String(dir), String(key ?? ""));
}

export function writeCache(dir, key, { id, savedAt, hello = null, graph, live, engine = null }) {
  if (!dir || !graph || !live) return false;
  return native.hostsWriteCache(String(dir), String(key ?? ""), { id, savedAt, hello, graph, live, engine });
}

export function removeCache(dir, key) {
  if (!dir) return;
  const r = native.hostsRemoveCache(String(dir), String(key ?? ""));
  if (r.error) throw errorOf(r.error);
}
