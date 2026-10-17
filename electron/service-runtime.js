// Which service Desktop starts when it runs one of its own: the JS service
// (node server/index.js) or the Rust binary (xenonet serve; see
// docs/design/rust-core.md, "At the end"). XENONET_SERVICE_RUNTIME (rust or
// node) says, else the serviceRuntime preference in desktop.json; Node
// until every part of the service is in Rust. A Rust binary can't be a
// utility process, so Desktop runs it as the standalone service and
// connects to it over the component service's socket, as it does to an
// installed service.
import fs from "node:fs";
import path from "node:path";

export const RUNTIMES = ["node", "rust"];

export function serviceRuntime({ env = process.env, pref = null } = {}) {
  const asked = String(env.XENONET_SERVICE_RUNTIME ?? "").trim().toLowerCase() || String(pref ?? "").trim().toLowerCase();
  return RUNTIMES.includes(asked) ? asked : "node";
}

// The xenonet binary to run: the one XENONET_SERVICE_BIN names, the
// installed Service's bin/xenonet, or a checkout's own build, release
// first. Null when there's none.
export function rustBinary({ env = process.env, root, serviceRoot = root, platform = process.platform, exists = (f) => fs.statSync(f).isFile() } = {}) {
  const name = platform === "win32" ? "xenonet.exe" : "xenonet";
  const candidates = [env.XENONET_SERVICE_BIN, serviceRoot && path.join(serviceRoot, "bin", name), root && path.join(root, "target", "release", name), root && path.join(root, "target", "debug", name)];
  for (const file of candidates) {
    try {
      if (file && exists(file)) return file;
    } catch {}
  }
  return null;
}
