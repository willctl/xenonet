// The service Desktop runs: the Rust binary, `xenonet serve` (docs/design/
// rust-core.md, "The switch"), or for a while after an update from 0.3.0
// the node service that XenoNet had. A binary can't be a utility process,
// so Desktop runs it standalone and connects to it over the component
// service's socket, as it does to an installed service.
import fs from "node:fs";
import path from "node:path";
import { installBundle, hasBinary } from "../core/components/install.js";
import { currentComponent } from "../core/components/packages.js";
import { componentUpdates, updateComponent } from "../core/components/updates.js";

const found = (f, stat) => {
  try {
    const s = f && stat(f);
    return s?.isFile() ? s : null;
  } catch {
    return null;
  }
};

// The xenonet binary to run: the one XENONET_SERVICE_BIN names, the
// installed Service's bin/xenonet, or a checkout's own build, the newer of
// release and debug, so a stale build of the other never wins. Null when
// there's none.
export function rustBinary({ env = process.env, root, serviceRoot = root, platform = process.platform, stat = (f) => fs.statSync(f) } = {}) {
  const name = platform === "win32" ? "xenonet.exe" : "xenonet";
  for (const f of [env.XENONET_SERVICE_BIN, serviceRoot && path.join(serviceRoot, "bin", name)]) if (found(f, stat)) return f;
  const builds = root ? ["release", "debug"].map((profile) => path.join(root, "target", profile, name)).map((f) => [f, found(f, stat)]).filter(([, s]) => s) : [];
  builds.sort((a, b) => b[1].mtimeMs - a[1].mtimeMs);
  return builds[0]?.[0] ?? null;
}

// How Desktop starts the service: { root, exec, args }, or null when
// there's nothing to run. `installed` is what installedRuntime says of a
// packaged XenoNet's Service; a Service from before 0.4.0 that couldn't be
// replaced yet runs as it says, with its node. Otherwise it's the binary
// (rustBinary) with `serve`, and `--dev` for XenoNet Dev, which gives the
// dev instances' routes (/api/dev/simulate and capture).
export function serviceCommand({ env = process.env, root = null, serviceRoot = root, installed = null, dev = false, platform = process.platform, stat = (f) => fs.statSync(f) } = {}) {
  if (installed && !env.XENONET_SERVICE_BIN && installed.args?.[0] !== "serve") return found(installed.exec, stat) ? { root: installed.root, exec: installed.exec, args: installed.args } : null;
  const exec = rustBinary({ env, root, serviceRoot: installed?.root ?? serviceRoot, platform, stat });
  return exec ? { root: installed?.root ?? serviceRoot, exec, args: ["serve", ...(dev ? ["--dev"] : [])] } : null;
}

// What to say when there's no binary to run.
export function missingBinary({ fromSource }) {
  return fromSource
    ? "There's no xenonet binary in this checkout. Build it with npm run rust:bin, then start XenoNet Dev again."
    : "XenoNet Service's xenonet binary is missing. Reinstall XenoNet with the bundle installer.";
}

// The Service and Tools a packaged Desktop starts with. A bundle (the Mac
// disk image, Windows' XenoNet Setup) brings its own for any that's missing
// or from before 0.4.0, a node and XenoNet's JS with no xenonet binary
// (core/components/install.js). Any it doesn't bring comes from the latest
// release, as it does for a Desktop-only update, Tools first so the service
// names their launcher.
//
// One from before 0.4.0 that can't be replaced now runs on as it was,
// until Desktop starts again: a service with a terminal open, whose update
// waits for it to close as any service update does, or one whose update
// failed and was rolled back. Only a missing component, or a failure with
// nothing from before 0.4.0 to explain it, stops Desktop.
//
// Says which components are still from before 0.4.0, whether the service's
// update waits on its terminals, and what went wrong.
export async function readyComponents({ env, bundle = null, test = false, operations, fetcher } = {}) {
  const state = (component) => {
    const selected = currentComponent(component, env);
    return !selected ? "missing" : hasBinary(selected) ? "current" : "earlier";
  };
  const problems = [];
  let carried = [];
  let waiting = false;
  if (bundle) {
    carried = JSON.parse(fs.readFileSync(path.join(bundle, "components.json"), "utf8")).packages.filter((p) => p.platform === process.platform && p.arch === process.arch).map((p) => p.component);
    try {
      await installBundle(bundle, { env, command: !test, operations });
      waiting = carried.includes("service") && state("service") === "earlier";
    } catch (err) {
      problems.push(err.message);
    }
  }
  const wanted = ["tools", "service"].filter((c) => state(c) === "missing" || (state(c) === "earlier" && !carried.includes(c)));
  if (wanted.length && test) throw new Error("The test bundle is missing its components.");
  let available = null;
  for (const component of wanted) {
    try {
      available ??= await componentUpdates({ env, fetcher });
      const item = available.find((p) => p.component === component);
      if (!item) throw new Error("Install the XenoNet bundle to add its background service and tools.");
      const result = await updateComponent(item, { env, fetcher, operations });
      if (result.status === "staged") waiting = true;
    } catch (err) {
      problems.push(err.message);
    }
  }
  const earlier = ["service", "tools"].filter((c) => state(c) === "earlier");
  if (["service", "tools"].some((c) => state(c) === "missing") || (problems.length && !earlier.length)) throw new Error(problems[0] ?? "Install the XenoNet bundle to add its background service and tools.");
  return { earlier, waiting, problems };
}

// What Desktop tells you when a Service or Tools from before 0.4.0 is
// still in use (readyComponents): null when nothing is.
export function earlierNotice({ earlier, waiting, problems }) {
  if (!earlier.length) return null;
  if (waiting && !problems.length) return { title: "Close your terminals to finish updating XenoNet", body: "Open terminals keep XenoNet Service from updating. Close them, then quit and reopen XenoNet. Until then, the service you had keeps running." };
  const why = (problems[0] ?? "The update didn't finish").replace(/([^.!?])$/, "$1.");
  const kept = earlier.includes("tools") ? (earlier.includes("service") ? "The Service and Tools you had keep" : "The Tools you had keep") : "The Service you had keeps";
  return { title: "XenoNet couldn't finish updating", body: `${why} ${kept} running. Quit and reopen XenoNet to try again.` };
}
