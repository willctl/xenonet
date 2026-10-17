// What live delivery needs on this machine, for xenonet doctor --live: Claude
// Code new enough for channels and the command that starts a session
// listening, tmux for typing, and the OpenCode servers XenoNet can find.
import { execFile } from "node:child_process";
import { createOpenCode } from "./opencode.js";

const run = (cmd, args) => new Promise((resolve) => execFile(cmd, args, { timeout: 5000, windowsHide: true, shell: process.platform === "win32" }, (err, out) => resolve(err ? null : String(out).trim())));

// The first Claude Code with channels: its changelog adds --channels in
// 2.1.80. Checked by hand against 2.1.302.
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
    // `claude --version` says "2.1.302 (Claude Code)", `tmux -V` "tmux 3.6a".
    claude: { version: claude?.match(/\d+\.\d+\.\d+/)?.[0] ?? claude, channels: newer(claude, CHANNELS_FROM) },
    command,
    alias: `alias claude='${command}'`,
    // OpenCode's TUI runs its HTTP server only when told where to listen;
    // with a host and no port it takes 4096, or a free port when that's taken.
    opencodeAlias: "alias opencode='opencode --hostname 127.0.0.1'",
    tmux: tmux?.replace(/^tmux\s+/, "") ?? null,
    opencode: servers.map((s) => s.base),
  };
}
