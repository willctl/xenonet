// The service Desktop runs: the Rust binary, `xenonet serve`, and nothing
// else (docs/design/rust-core.md, "The switch"). A binary can't be a
// utility process, so Desktop runs it standalone and connects to it over
// the component service's socket, as it does to an installed service.
import fs from "node:fs";
import path from "node:path";

// The xenonet binary to run: the one XENONET_SERVICE_BIN names, the
// installed Service's bin/xenonet, or a checkout's own build, the newer of
// release and debug, so a stale build of the other never wins. Null when
// there's none.
export function rustBinary({ env = process.env, root, serviceRoot = root, platform = process.platform, stat = (f) => fs.statSync(f) } = {}) {
  const name = platform === "win32" ? "xenonet.exe" : "xenonet";
  const found = (f) => {
    try {
      const s = f && stat(f);
      return s?.isFile() ? s : null;
    } catch {
      return null;
    }
  };
  for (const f of [env.XENONET_SERVICE_BIN, serviceRoot && path.join(serviceRoot, "bin", name)]) if (found(f)) return f;
  const builds = root ? ["release", "debug"].map((profile) => path.join(root, "target", profile, name)).map((f) => [f, found(f)]).filter(([, s]) => s) : [];
  builds.sort((a, b) => b[1].mtimeMs - a[1].mtimeMs);
  return builds[0]?.[0] ?? null;
}

// What to say when there's no binary to run.
export function missingBinary({ fromSource }) {
  return fromSource
    ? "There's no xenonet binary in this checkout. Build it with npm run rust:bin, then start XenoNet Dev again."
    : "XenoNet Service's xenonet binary is missing. Reinstall XenoNet with the bundle installer.";
}
