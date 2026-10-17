// Typing a message into the tmux pane a session runs in: the last resort
// for a harness with no way in, and only with your consent for that session
// (policy.js). Before every keystroke XenoNet checks again that the pane's
// terminal still runs the session's own process in the foreground, so it
// never types into the wrong place; the text is one line, with no control
// characters or escape sequences, sent literally (send-keys -l), then Enter.
import { execFile } from "node:child_process";
import { sanitize } from "./policy.js";

const run = (cmd, args) => new Promise((resolve) => execFile(cmd, args, { timeout: 3000, windowsHide: true }, (err, out) => resolve(err ? { ok: false, out: "" } : { ok: true, out: String(out) })));

export const MAX_TYPED = 2000;

export function createTmux({ exec = run, platform = process.platform } = {}) {
  const off = platform === "win32";

  // tmux panes: [{ pane, pid, tty, command }].
  async function panes() {
    const r = await exec("tmux", ["list-panes", "-a", "-F", "#{pane_id}\t#{pane_pid}\t#{pane_tty}\t#{pane_current_command}"]);
    if (!r.ok) return [];
    return r.out.trim().split("\n").filter(Boolean).map((l) => {
      const [pane, pid, tty, command] = l.split("\t");
      return { pane, pid: Number(pid), tty, command };
    });
  }

  // A process's terminal and whether it's in the terminal's foreground
  // process group (a + in its state).
  async function proc(pid) {
    const r = await exec("ps", ["-o", "tty=,stat=", "-p", String(pid)]);
    if (!r.ok || !r.out.trim()) return null;
    const [tty, stat] = r.out.trim().split(/\s+/);
    return { tty: tty.startsWith("/dev/") ? tty : `/dev/${tty}`, foreground: stat.includes("+") };
  }

  // The pane a session's process runs in, checked: same terminal, still in
  // its foreground. null when it can't be sure.
  async function locate(pid) {
    if (off || !Number.isInteger(pid) || pid <= 1) return null;
    const p = await proc(pid);
    if (!p || !p.foreground) return null;
    const pane = (await panes()).find((x) => x.tty === p.tty);
    return pane ? { ...pane, sessionPid: pid } : null;
  }

  return {
    name: "tmux",
    locate,
    async deliver({ pid, text }) {
      const first = await locate(pid);
      if (!first) return { ok: false, why: "the session isn't in the foreground of a tmux pane XenoNet can see" };
      const line = sanitize(text, { oneLine: true, max: MAX_TYPED });
      // Check again right before typing: the pane could have changed hands.
      const again = await locate(pid);
      if (!again || again.pane !== first.pane || again.tty !== first.tty) return { ok: false, why: "the pane changed before XenoNet could type" };
      const typed = await exec("tmux", ["send-keys", "-t", first.pane, "-l", "--", line]);
      if (!typed.ok) return { ok: false, why: "tmux didn't take the text" };
      await exec("tmux", ["send-keys", "-t", first.pane, "Enter"]);
      return { ok: true, where: first.pane };
    },
  };
}
