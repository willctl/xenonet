// Made-up components for the tests of installing and updating them
// (component-install.test.js, service-runtime.test.js): packages whose
// xenonet binary is a shell script that answers the startup check, bundles
// of them, a service that answers status and stop, and an install as
// XenoNet 0.3.0 left it.
import fs from "node:fs";
import path from "node:path";
import * as tar from "tar";
import { currentComponent, digest, selectComponent } from "../../core/components/packages.js";

export const EXE = process.platform === "win32" ? "xenonet.exe" : "xenonet";
export const BIN = path.join("bin", EXE);
export const NODE = process.platform === "win32" ? "node.exe" : "node";
export const posixOnly = { skip: process.platform === "win32" ? "a shell script stands in for the binary" : false };

// A xenonet binary that passes the startup check as `version`, or fails it:
// `fail` "runtime" doesn't run, "command" reports another version.
export const fakeBin = (version, fail = null) => `#!/bin/sh
${fail === "runtime" ? "echo 'dyld: Library not loaded' >&2; exit 1" : `printf '{"version":"${fail === "command" ? "0.0.1" : version}"}'`}
`;

// A component package in `out`: { file, item } as a release manifest
// lists it.
export async function pack(out, component, version, { fail = null, platform = process.platform, arch = process.arch, bin = null } = {}) {
  const app = fs.mkdtempSync(path.join(out, `${component}-${version}-`));
  const meta = { format: 1, component, version, platform, arch, dataSchema: 1, api: { min: 1, max: 1 } };
  fs.writeFileSync(path.join(app, "component.json"), JSON.stringify(meta));
  // The xenonet binary: one that answers the startup check, or `bin`, as
  // made-up text.
  fs.mkdirSync(path.join(app, "bin"));
  fs.writeFileSync(path.join(app, BIN), bin ?? fakeBin(version, fail), { mode: 0o755 });
  const name = `XenoNet-${component}-${version}-${platform}-${arch}.tar.gz`;
  const file = path.join(out, name);
  await tar.c({ file, cwd: app, gzip: true }, fs.readdirSync(app));
  const bytes = fs.readFileSync(file);
  return { file, item: { ...meta, file: name, size: bytes.length, sha256: digest(bytes) } };
}

// A bundle folder as the installer ships it: the packages and their
// components.json.
export async function bundle(dir, name, packages) {
  const out = path.join(dir, name);
  fs.mkdirSync(out);
  const items = [];
  for (const [component, version, options] of packages) items.push((await pack(out, component, version, options)).item);
  fs.writeFileSync(path.join(out, "components.json"), JSON.stringify({ format: 1, packages: items }));
  return out;
}

// A service that answers status and stop, as the operations
// activatePackage, installBundle and recoverServiceUpdate take: `state` is
// what status says, null when it's down; `failing`, a version that doesn't
// start. Each start's runtime is kept.
export function fakeService(env, state, { failing = null } = {}) {
  const s = { state, stops: 0, starts: [], runtimes: [] };
  s.operations = {
    check() {},
    now: () => 0,
    pause: async () => {},
    stopTimeout: 0,
    async connect() {
      if (!s.state) throw new Error("not running");
      return {
        close() {},
        async request(method) {
          if (method === "status") return s.state;
          s.stops++;
          s.state = null;
          return { stopping: true };
        },
      };
    },
    async start(runtime) {
      const version = currentComponent("service", env).version;
      s.starts.push(version);
      s.runtimes.push(runtime);
      if (version === failing) throw new Error("XenoNet Service exited (1). See its service log.");
      s.state = { version, terminals: [] };
    },
  };
  return s;
}

// An install as XenoNet 0.3.0 left it (git show v0.3.0:core/components/
// packages.js and install.js): each component a node and XenoNet's JS, with
// no xenonet binary, in <version>-<sha16> beside its current.json, and the
// launcher's own node and cli/xenonet.js, which the hooks and MCP servers
// 0.3.0 wrote run. Says what's selected.
export function earlierInstall(env, { components = ["service", "tools"], version = "0.3.0" } = {}) {
  const selected = {};
  for (const component of components) {
    const sha256 = digest(`XenoNet ${component} ${version}`);
    const root = path.join(env.XENONET_RUNTIME_DIR, "components", component, `${version}-${sha256.slice(0, 16)}`);
    for (const folder of ["server", "cli", "core"]) fs.mkdirSync(path.join(root, folder), { recursive: true });
    fs.writeFileSync(path.join(root, "component.json"), JSON.stringify({ format: 1, component, version, platform: process.platform, arch: process.arch, node: "24.19.0", api: { min: 1, max: 1 }, consoleApi: { min: 1, max: 1 }, dataSchema: 1 }));
    fs.writeFileSync(path.join(root, NODE), `${version}'s node`, { mode: 0o755 });
    fs.writeFileSync(path.join(root, "server", "index.js"), `// ${version}'s service\n`);
    fs.writeFileSync(path.join(root, "cli", "xenonet.js"), `// ${version}'s command\n`);
    selectComponent(component, { version, sha256 }, env);
    selected[component] = currentComponent(component, env);
  }
  const launcher = path.join(env.XENONET_RUNTIME_DIR, "launcher");
  fs.mkdirSync(path.join(launcher, "cli"), { recursive: true });
  fs.writeFileSync(path.join(launcher, NODE), `${version}'s node`, { mode: 0o755 });
  fs.writeFileSync(path.join(launcher, "cli", "xenonet.js"), "// xenonet-launcher 2 (protocol 1)\n");
  return selected;
}
