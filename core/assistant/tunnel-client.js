// OpenAI's tunnel-client, as XenoNet finds and drives it for a remote
// actor's connection (docs/guide/cloud-assistants.md): where it is, which
// version, the profiles you made for it already, and the profile XenoNet
// writes for a grant. XenoNet never downloads it; you install it from
// OpenAI, and XenoNet looks where its installs put it.
//
// A profile names the tunnel (its id isn't a secret), where the runtime
// key comes from (env:CONTROL_PLANE_API_KEY, which XenoNet sets only in the
// client's own environment), and the MCP command the tunnel forwards to:
// xenonet mcp assistant for one grant, as XenoNet's launcher runs it.
import fs from "node:fs";
import path from "node:path";
import { execFile } from "node:child_process";
import { onPath } from "../paths.js";
import { GRANT_ID } from "../schema/assistant.js";

export const TUNNEL_ID = /^tunnel_[0-9a-f]{16,64}$/;
// A runtime API key for the tunnel: an OpenAI key, restricted to Tunnels.
export const RUNTIME_KEY = /^sk-[A-Za-z0-9_-]{20,400}$/;
export const RELEASES = "https://github.com/openai/tunnel-client/releases";
export const TUNNELS_PAGE = "https://platform.openai.com/settings/organization/tunnels";
const EXE = (platform) => (platform === "win32" ? "tunnel-client.exe" : "tunnel-client");

// Where an install of tunnel-client puts it, after XENONET_TUNNEL_CLIENT and
// PATH: a per-user folder on Windows, Homebrew's and ~/.local/bin elsewhere.
// Paths in the system's own style, whichever system the tests run on.
const style = (platform) => (platform === "win32" ? path.win32 : path.posix);

export function clientPlaces({ env = process.env, platform = process.platform, home = env.HOME ?? env.USERPROFILE ?? "" } = {}) {
  const exe = EXE(platform);
  const path = style(platform);
  if (platform === "win32") {
    const local = env.LOCALAPPDATA ?? path.join(home, "AppData", "Local");
    return [path.join(local, "Programs", "tunnel-client", exe), path.join(local, "tunnel-client", exe)];
  }
  return ["/opt/homebrew/bin", "/usr/local/bin", path.join(home, ".local", "bin"), path.join(home, "bin")].map((d) => path.join(d, exe));
}

// The tunnel-client to run, or null when there's none.
export function findClient({ env = process.env, platform = process.platform, home, exists = (f) => fs.statSync(f).isFile() } = {}) {
  const ok = (f) => {
    try {
      return !!f && exists(f);
    } catch {
      return false;
    }
  };
  if (env.XENONET_TUNNEL_CLIENT && ok(env.XENONET_TUNNEL_CLIENT)) return env.XENONET_TUNNEL_CLIENT;
  const onpath = onPath("tunnel-client", { env, platform, canRun: (f) => {
    if (!exists(f)) throw new Error("no");
  } });
  if (onpath) return onpath;
  return clientPlaces({ env, platform, home }).find(ok) ?? null;
}

// Its version, such as 0.0.16, or null.
export function clientVersion(file, { run = execFile } = {}) {
  return new Promise((resolve) => {
    try {
      run(file, ["--version"], { timeout: 8000, windowsHide: true }, (err, stdout) => resolve(err ? null : (/(\d+\.\d+\.\d+)/.exec(String(stdout))?.[1] ?? null)));
    } catch {
      resolve(null);
    }
  });
}

// Where tunnel-client keeps the profiles you made with it: the user
// config folder Go picks on each system, and ~/.config, which its help
// names.
export function profileDirs({ env = process.env, platform = process.platform, home = env.HOME ?? env.USERPROFILE ?? "" } = {}) {
  if (env.TUNNEL_CLIENT_PROFILE_DIR) return [env.TUNNEL_CLIENT_PROFILE_DIR];
  const path = style(platform);
  const dirs = [];
  if (platform === "win32") dirs.push(path.join(env.APPDATA ?? path.join(home, "AppData", "Roaming"), "tunnel-client"));
  if (platform === "darwin") dirs.push(path.join(home, "Library", "Application Support", "tunnel-client"));
  dirs.push(path.join(env.XDG_CONFIG_HOME ?? path.join(home, ".config"), "tunnel-client"));
  return [...new Set(dirs)];
}

// What a profile says, read line by line: its tunnel, its health address,
// and which grant its MCP command serves, if it runs xenonet mcp assistant.
// Enough to recognize a profile, not a YAML parser.
export function readProfile(text) {
  const value = (key) => {
    const m = new RegExp(`^\\s*-?\\s*${key}:\\s*(?:"((?:[^"\\\\]|\\\\.)*)"|'((?:[^']|'')*)'|([^#\\r\\n]*?))\\s*(?:#.*)?$`, "m").exec(text);
    if (!m) return null;
    return m[1] != null ? m[1].replace(/\\(.)/g, "$1") : m[2] != null ? m[2].replace(/''/g, "'") : m[3] || null;
  };
  const command = value("command");
  const grant = /\bmcp\s+assistant\b.*--grant[ =](gr_[0-9a-f]{24})\b/.exec(command ?? "")?.[1] ?? null;
  const tunnelId = value("tunnel_id");
  return { tunnelId: TUNNEL_ID.test(tunnelId ?? "") ? tunnelId : null, health: value("listen_addr"), grant, xenonet: !!grant };
}

// The profiles in `dirs` that run xenonet mcp assistant, newest first:
// { name, file, tunnelId, health, grant }.
export function findProfiles(dirs, { read = (f) => fs.readFileSync(f, "utf8"), list = (d) => fs.readdirSync(d), stat = (f) => fs.statSync(f) } = {}) {
  const out = [];
  for (const dir of dirs) {
    let names = [];
    try {
      names = list(dir).filter((n) => /\.ya?ml$/i.test(n));
    } catch {
      continue;
    }
    for (const n of names) {
      const file = path.join(dir, n);
      try {
        const p = readProfile(read(file));
        if (p.xenonet) out.push({ name: n.replace(/\.ya?ml$/i, ""), file, tunnelId: p.tunnelId, health: p.health, grant: p.grant, at: stat(file).mtimeMs });
      } catch {}
    }
  }
  return out.sort((a, b) => b.at - a.at).map(({ at, ...p }) => p);
}

// A command line as tunnel-client splits it: words apart by spaces, a
// word with spaces in double quotes, and forward slashes, since it reads
// a backslash as an escape.
export function commandLine(argv) {
  return argv
    .map((a) => String(a).replace(/\\/g, "/"))
    .map((a) => (/[\s"']/.test(a) ? `"${a.replace(/"/g, '\\"')}"` : a))
    .join(" ");
}

// What tunnel-client runs for a grant: XenoNet's launcher (its node and
// cli/xenonet.js, which stay put across updates) with mcp assistant.
export function adapterArgv({ exec, entry, grant, port = 47840 }) {
  if (!GRANT_ID.test(String(grant))) throw new Error("not a grant id");
  return [exec, entry, "mcp", "assistant", "--grant", grant, ...(port !== 47840 ? ["--port", String(port)] : [])];
}

// The profile XenoNet writes for a grant: no secret in it, its health on a
// port the system picks, and logs as JSON lines XenoNet can read.
export function profileYaml({ tunnelId, command, label = null }) {
  if (!TUNNEL_ID.test(String(tunnelId))) throw new Error("not a tunnel id");
  const q = (s) => `'${String(s).replace(/'/g, "''")}'`;
  return [
    `# Written by XenoNet${label ? ` for “${String(label).replace(/[\r\n]/g, " ")}”` : ""}. XenoNet writes it again when the connection`,
    "# changes, so change the connection in XenoNet rather than here.",
    "config_version: 1",
    "control_plane:",
    '  base_url: "https://api.openai.com"',
    `  tunnel_id: ${q(tunnelId)}`,
    '  api_key: "env:CONTROL_PLANE_API_KEY"',
    "health:",
    '  listen_addr: "127.0.0.1:0"',
    "admin_ui:",
    "  open_browser: false",
    "log:",
    "  level: info",
    "  format: json",
    "mcp:",
    "  commands:",
    "    - channel: main",
    `      command: ${q(command)}`,
    "",
  ].join("\n");
}

// One line of tunnel-client's log, as XenoNet shows it: its level and
// message, with anything that looks like a key taken out. Null for a line
// that isn't JSON.
export function logLine(line) {
  let e;
  try {
    e = JSON.parse(line);
  } catch {
    return null;
  }
  if (!e || typeof e !== "object") return null;
  const msg = String(e.msg ?? e.message ?? "").replace(/sk-[A-Za-z0-9_-]{8,}/g, "sk-…").replace(/[\u0000-\u001f]/g, " ").slice(0, 300);
  const level = String(e.level ?? "info").toLowerCase();
  const err = e.error ?? e.err;
  return { level, msg: err ? `${msg}: ${String(err).replace(/sk-[A-Za-z0-9_-]{8,}/g, "sk-…").slice(0, 200)}` : msg };
}

// How a running client stands, from its /health?details=true: ready, or
// degraded with why. `h` is that answer, or null when it didn't answer.
export function healthOf(h) {
  if (!h) return { state: "starting", why: "its health check hasn't answered yet" };
  const cp = h.components?.["control-plane"];
  const mcp = h.components?.mcp;
  if (!h.live) return { state: "degraded", why: "it says it isn't live" };
  if (cp && cp.status !== "ok") return { state: "degraded", why: `OpenAI's control plane: ${cp.state ?? cp.status}${cp.details?.consecutive_failures ? `, ${cp.details.consecutive_failures} failures in a row` : ""}` };
  if (mcp?.details?.child_state && mcp.details.child_state !== "running") return { state: "degraded", why: `xenonet mcp assistant is ${mcp.details.child_state}` };
  if (!h.ready) return { state: "starting", why: "it isn't ready yet" };
  return { state: "ready", why: null, version: h.runtime?.version?.split("+")[0] ?? null, polledAt: cp?.details?.last_success ?? null };
}
