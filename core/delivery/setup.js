// What live delivery needs on this machine, for xenonet doctor --live: Claude
// Code new enough for channels and the command that starts a session
// listening, tmux for typing, and the OpenCode servers XenoNet can find.
import { execFile } from "node:child_process";
import { createOpenCode } from "./opencode.js";

const run = (cmd, args) => new Promise((resolve) => execFile(cmd, args, { timeout: 5000, windowsHide: true, shell: process.platform === "win32" }, (err, out) => resolve(err ? null : String(out).trim())));

// The oldest Claude Code XenoNet expects channels in. Approximate: the docs
// date the research preview, not the version; 2.1.302 is the one checked.
export const CHANNELS_FROM = [2, 1, 80];
export const newer = (v, min) => {
  const p = String(v ?? "").match(/(\d+)\.(\d+)\.(\d+)/)?.slice(1).map(Number);
  if (!p) return false;
  for (let i = 0; i < 3; i++) if (p[i] !== min[i]) return p[i] > min[i];
  return true;
};

// The MCP server's name in Claude Code's settings, as connect.js gives it.
export const serverName = (port) => (!port || Number(port) === 47840 ? "xenonet" : Number(port) === 47842 ? "xenonet-dev" : `xenonet-${port}`);

export async function liveSetup({ port = 47840, exec = run, opencode = createOpenCode() } = {}) {
  const name = serverName(port);
  const claude = await exec("claude", ["--version"]);
  const tmux = await exec("tmux", ["-V"]);
  const servers = await opencode.servers().catch(() => []);
  const command = `claude --dangerously-load-development-channels server:${name}`;
  return {
    claude: { version: claude, channels: newer(claude, CHANNELS_FROM) },
    command,
    alias: `alias claude='${command}'`,
    // OpenCode's TUI runs its HTTP server only when told where to listen;
    // with a host and no port it takes 4096, or a free port when that's taken.
    opencodeAlias: "alias opencode='opencode --hostname 127.0.0.1'",
    tmux: tmux ?? null,
    opencode: servers.map((s) => s.base),
  };
}
