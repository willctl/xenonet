// Starting XenoNet's service at login (docs/guide/always-on.md): the
// Background service setting, and the offer Desktop makes once during
// setup. Desktop asks the xenonet command (xenonet service status, install,
// uninstall and start), which registers the service with launchd or
// systemd; Desktop registers nothing itself. XenoNet Dev starts and stops
// its own service, so there the setting only says so (`live` false), and
// Windows has no supervisor for it yet (`supported` false).
import { execFile } from "node:child_process";
import os from "node:os";
import { tilde } from "../core/paths.js";

// Runs the command: { code, out, err }, whatever it exits with.
export function runCommand(file, args, { env, timeout = 90000 } = {}) {
  return new Promise((resolve) => {
    execFile(file, args, { env, timeout, windowsHide: true, encoding: "utf8", maxBuffer: 4 << 20 }, (err, out, stderr) => {
      resolve({ code: err ? (typeof err.code === "number" ? err.code : 1) : 0, out: out ?? "", err: stderr || (err && typeof err.code !== "number" ? String(err.message) : "") });
    });
  });
}

// A xenonet command from before Background service answers status without
// `login`.
export const OLD_TOOLS = "The xenonet command installed is older than Desktop. Update the service and tools in Updates, then try again.";

// What the command said went wrong, without its "xenonet: " prefix.
export function said(r) {
  const line = String(r.err || r.out || "").trim().split(/\r?\n/).pop() ?? "";
  return line.replace(/^xenonet:\s*/, "") || `xenonet exited with code ${r.code}`;
}

// `bin` and `env`: the xenonet binary to ask and the environment to ask it
// in, as functions, since both are known only once Desktop has found its
// service. `afterStop`: what brings a service back once turning this off
// stopped the one the supervisor ran (Desktop starts its own again).
export function createBackground({ bin, env, live, prefs, port, platform = process.platform, home = os.homedir(), run = runCommand, afterStop = async () => {} }) {
  const ask = (args, timeout) => run(bin(), args, { env: env(), timeout });

  // The command's word on the service: { running, supervised, login }.
  async function raw() {
    const r = await ask(["service", "status", "--json"], 30000);
    try {
      return JSON.parse(r.out);
    } catch {
      return { running: false, error: said(r), login: null };
    }
  }

  function shape(st) {
    const login = st?.login ?? {};
    const supported = !!login.supported;
    return {
      platform,
      live,
      supported,
      reason: supported ? null : (login.reason ?? (st?.login ? null : st?.running ? OLD_TOOLS : (st?.error ?? null))),
      registered: !!login.registered,
      loaded: !!login.loaded,
      supervisor: login.supervisor ?? null,
      file: login.file ? tilde(login.file, home) : null,
      state: login.state ?? null,
      running: !!st?.running,
      supervised: !!st?.supervised,
      linger: login.linger ?? null,
      error: login.error ?? null,
      // Asked once, during setup, of a XenoNet that can do it and doesn't yet.
      offer: live && supported && !login.registered && !prefs.get("askedBackground"),
    };
  }

  // XenoNet Dev doesn't ask: it starts and stops its own service.
  const status = async () => (live ? shape(await raw()) : shape({ login: { supported: ["darwin", "linux"].includes(platform), registered: false } }));

  async function set(on, { force = false } = {}) {
    if (!live) throw new Error("XenoNet Dev starts and stops its own service. This setting is for installed XenoNet.");
    prefs.set("askedBackground", true);
    const r = await ask(["service", on ? "install" : "uninstall", "--json", ...(force ? ["--force"] : [])]);
    if (r.code !== 0) throw new Error(said(r));
    let result = null;
    try { result = JSON.parse(r.out); } catch {}
    if (!on && result?.stopped) await afterStop();
    return { ...(await status()), result };
  }

  async function dismiss() {
    prefs.set("askedBackground", true);
    return status();
  }

  // At startup, with no service answering: one registered to start at login
  // for this port is started by its supervisor rather than beside it.
  // "started", or null when Desktop should start its own.
  async function startRegistered() {
    const st = await raw();
    if (!st?.login?.registered || String(st.login.port) !== String(port)) return null;
    const r = await ask(["service", "start"], 90000);
    if (r.code !== 0) throw new Error(said(r));
    return "started";
  }

  async function act(action, value = {}) {
    if (action === "status") return status();
    if (action === "set") return set(!!value?.on, { force: !!value?.force });
    if (action === "dismiss") return dismiss();
    throw new Error(`Background service has no action ${action}.`);
  }

  // Desktop's uninstaller takes the registration out too, so nothing is
  // left starting at login: false when there was none to take out.
  async function remove() {
    const r = await ask(["service", "uninstall", "--json", "--force"]);
    if (r.code !== 0) throw new Error(said(r));
    try { return JSON.parse(r.out).status === "removed"; } catch { return false; }
  }

  return { act, status, set, dismiss, startRegistered, remove };
}
