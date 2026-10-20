// core/delivery/opencode.js in Rust (crates/xenonet-engine/src/delivery/opencode.rs):
// the OpenCode servers running here, the one that has a session in hand,
// what it says of the session, and a message posted to it. What's found,
// kept and decided is Rust's; commands, files and HTTP are the twin's.
import { execFile } from "node:child_process";
import fs from "node:fs";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";

const run = (cmd, args) => new Promise((resolve) => execFile(cmd, args, { timeout: 3000, windowsHide: true, maxBuffer: 4 << 20 }, (err, out) => resolve(err ? "" : String(out))));

export const openCodeId = (id) => native.openCodeId(id);
export const fromOpenCodeId = (id) => native.fromOpenCodeId(id);

export async function openCodeProcesses({ platform = process.platform, ps = run } = {}) {
  return drive(native.openCodeProcesses(platform === "win32"), calls({ ps: (cmd, args) => ps(cmd, args) }));
}

export const parsePs = (out) => native.openCodeParsePs(String(out));

export async function listeningPorts(pid, { platform = process.platform, exec = run, readFile = fs.readFileSync, readdir = fs.readdirSync, readlink = fs.readlinkSync } = {}) {
  return drive(native.openCodePorts(pid, String(platform)), calls({ exec: (cmd, args) => exec(cmd, args), readFile: (f, e) => readFile(f, e), readdir: (d) => readdir(d), readlink: (p) => readlink(p) }));
}

export const samePath = (a, b, platform = process.platform) => native.openCodeSamePath(a, b, platform === "win32");

export function createOpenCode({ fetch = globalThis.fetch, processes = openCodeProcesses, ports = listeningPorts, cacheMs = 10e3, now = () => Date.now() } = {}) {
  const core = new native.DeliveryOpenCodeCore(cacheMs, process.platform === "win32");
  const deps = {
    now: () => now(),
    processes: () => processes(),
    ports: (pid) => ports(pid),
    // A GET, as { ok, status, json }; JSON read only from an answer that's ok.
    async get(url, ms) {
      const r = await fetch(url, { signal: AbortSignal.timeout(ms) });
      return r.ok ? { ok: true, status: r.status, json: await r.json() } : { ok: false, status: r.status };
    },
    async post(url, body) {
      const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body, signal: AbortSignal.timeout(3000) });
      return { ok: r.ok, status: r.status };
    },
  };
  const go = (machine) => drive(machine, calls(deps));
  return {
    name: "opencode",
    servers: () => go(core.servers()),
    serverFor: (id) => go(core.serverFor(id)),
    info: (id) => go(core.info(id)),
    forget: () => core.forget(),
    deliver: (id, text) => go(core.deliver(id, text)),
  };
}
