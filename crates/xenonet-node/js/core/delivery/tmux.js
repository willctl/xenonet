// core/delivery/tmux.js in Rust (crates/xenonet-engine/src/delivery/tmux.rs):
// typing a message into the tmux pane a session runs in, checked twice
// that the pane's terminal runs the session in its foreground. The checks
// and what's typed are Rust's; the commands run here.
import { execFile } from "node:child_process";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";

const run = (cmd, args) => new Promise((resolve) => execFile(cmd, args, { timeout: 3000, windowsHide: true }, (err, out) => resolve(err ? { ok: false, out: "" } : { ok: true, out: String(out) })));

export const MAX_TYPED = 2000;

export function createTmux({ exec = run, platform = process.platform } = {}) {
  const core = new native.TmuxCore(platform === "win32");
  const go = (machine) => drive(machine, calls({ exec: (cmd, args) => exec(cmd, args) }));
  return {
    name: "tmux",
    locate: (pid) => go(core.locate(pid)),
    deliver: async ({ pid, text }) => go(core.deliver(pid, text)),
  };
}
