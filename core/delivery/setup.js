// What live delivery needs on this machine, for xenonet doctor --live: Claude
// Code new enough for channels and the command that starts a session
// listening, the settings that keep a session listening after its turns,
// tmux for typing, and the OpenCode servers XenoNet can find.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
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

// Listening after each turn: a Claude Code session started with these
// settings (claude --settings <file>) has a Stop hook that waits for a
// message for up to `seconds` once a turn ends, and hands it over. Claude
// Code adds the hook to the ones in its other settings. The hook gets a
// little longer than the window, and only these sessions get it, so a
// slow answer from XenoNet never holds up any other session's turns.
export const LISTEN_SECONDS = 300;
export function listenSettings({ port = 47840, seconds = LISTEN_SECONDS } = {}) {
  return { hooks: { Stop: [{ hooks: [{ type: "http", url: `http://127.0.0.1:${port}/api/hook/claude/listen?seconds=${seconds}`, timeout: seconds + 30 }] }] } };
}

// The settings file for this XenoNet, written into `dir` when it's missing
// or out of date. Returns its path.
export function writeListenSettings({ dir, port = 47840, seconds = LISTEN_SECONDS }) {
  const file = path.join(dir, `listen-${port}.json`);
  const text = `${JSON.stringify(listenSettings({ port, seconds }), null, 2)}\n`;
  let old = null;
  try {
    old = fs.readFileSync(file, "utf8");
  } catch {}
  if (old !== text) {
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(file, text);
  }
  return file;
}

// A path as a shell takes it: in double quotes, with your home folder as
// $HOME (not on Windows, whose shells spell it otherwise), so the line
// works as printed and shows no home folder.
const shellPath = (file, home) => `"${home && process.platform !== "win32" && file.startsWith(home + path.sep) ? `$HOME${file.slice(home.length)}` : file}"`;

export async function liveSetup({ port = 47840, exec = run, opencode = createOpenCode(), listenFile = null, home = os.homedir() } = {}) {
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
    listen: listenFile ? `claude --settings ${shellPath(listenFile, home)}` : null,
    // OpenCode's TUI runs its HTTP server only when told where to listen;
    // with a host and no port it takes 4096, or a free port when that's taken.
    opencodeAlias: "alias opencode='opencode --hostname 127.0.0.1'",
    tmux: tmux?.replace(/^tmux\s+/, "") ?? null,
    opencode: servers.map((s) => s.base),
  };
}
