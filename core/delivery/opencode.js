// OpenCode: every TUI runs its own HTTP server (https://opencode.ai/docs/server/),
// on a random port unless started with --port. XenoNet finds the running
// opencode processes and the ports they listen on, asks each which sessions
// it has, and posts the message to the session with prompt_async, which
// starts a turn in the live TUI.
import { execFile } from "node:child_process";
import fs from "node:fs";

const run = (cmd, args) => new Promise((resolve) => execFile(cmd, args, { timeout: 3000, windowsHide: true, maxBuffer: 4 << 20 }, (err, out) => resolve(err ? "" : String(out))));

// XenoNet's ids take OpenCode's ses_… with "-" for "_" (see core/native/opencode.js).
export const openCodeId = (id) => String(id).replace(/^opencode:/, "").replace(/^ses-/, "ses_");
// And back: XenoNet's id for a session named the way OpenCode shows it
// (ses_…, or opencode:ses_…). Any other id comes back as it was.
export const fromOpenCodeId = (id) => {
  const s = String(id ?? "").trim();
  const m = /^(?:opencode:)?ses_([0-9A-Za-z]+)$/.exec(s);
  return m ? `opencode:ses-${m[1]}` : s;
};

// opencode processes: [{ pid, args }].
export async function openCodeProcesses({ platform = process.platform, ps = run } = {}) {
  if (platform === "win32") {
    const out = await ps("powershell.exe", ["-NoProfile", "-Command", "Get-CimInstance Win32_Process -Filter \"Name like 'opencode%'\" | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }"]);
    return parsePs(out);
  }
  // The command itself, not a shell or tmux that names it in its arguments.
  return parsePs(await ps("ps", ["-axo", "pid=,args="])).filter((p) => /^(\S*[\/\\])?opencode(\.exe)?(\s|$)/.test(p.args) || /^\S*node\S*\s+\S*opencode-ai/.test(p.args));
}

export function parsePs(out) {
  return String(out).split("\n").map((l) => /^\s*(\d+)\s+(.*)$/.exec(l)).filter(Boolean).map((m) => ({ pid: Number(m[1]), args: m[2] }));
}

// The TCP ports a process listens on, on loopback or any address.
export async function listeningPorts(pid, { platform = process.platform, exec = run, readFile = fs.readFileSync, readdir = fs.readdirSync, readlink = fs.readlinkSync } = {}) {
  const fromArgs = [];
  if (platform === "win32") {
    const out = await exec("netstat", ["-ano", "-p", "tcp"]);
    return [...new Set(out.split("\n").map((l) => l.trim().split(/\s+/)).filter((c) => c[3] === "LISTENING" && Number(c[4]) === pid).map((c) => Number(c[1].split(":").pop())))];
  }
  if (platform === "linux") {
    try {
      const inodes = new Set();
      for (const fd of readdir(`/proc/${pid}/fd`)) {
        try {
          const m = /^socket:\[(\d+)\]$/.exec(readlink(`/proc/${pid}/fd/${fd}`));
          if (m) inodes.add(m[1]);
        } catch {}
      }
      const ports = [];
      for (const f of ["/proc/net/tcp", "/proc/net/tcp6"]) {
        let text = "";
        try {
          text = readFile(f, "utf8");
        } catch {}
        for (const line of text.split("\n").slice(1)) {
          const c = line.trim().split(/\s+/);
          if (c[3] === "0A" && inodes.has(c[9])) ports.push(parseInt(c[1].split(":")[1], 16));
        }
      }
      if (ports.length) return [...new Set(ports)];
    } catch {}
  }
  const out = await exec("lsof", ["-nP", "-a", "-p", String(pid), "-iTCP", "-sTCP:LISTEN", "-Fn"]);
  for (const m of out.matchAll(/^n.*:(\d+)$/gm)) fromArgs.push(Number(m[1]));
  return [...new Set(fromArgs)];
}

// One folder, as OpenCode spells it in two places: the same once a trailing
// slash goes, and on Windows whatever the case or the slashes.
export function samePath(a, b, platform = process.platform) {
  const norm = (p) => {
    const s = String(p).replace(/[\\/]+$/, "");
    return platform === "win32" ? s.replace(/\\/g, "/").toLowerCase() : s;
  };
  return norm(a) === norm(b);
}

export function createOpenCode({ fetch = globalThis.fetch, processes = openCodeProcesses, ports = listeningPorts, cacheMs = 10e3, now = () => Date.now() } = {}) {
  let cache = { at: -Infinity, servers: [] };
  const get = async (base, route, ms = 1500) => {
    const r = await fetch(`${base}${route}`, { signal: AbortSignal.timeout(ms) });
    if (!r.ok) throw Object.assign(new Error(`${route}: ${r.status}`), { status: r.status });
    return r.json();
  };

  // The OpenCode servers running now: [{ pid, base, dir }], dir being the
  // folder the server was started in, as it says (GET /path).
  async function servers() {
    if (now() - cache.at < cacheMs) return cache.servers;
    const found = [];
    for (const p of await processes()) {
      const explicit = /--port[= ](\d+)/.exec(p.args);
      const list = explicit ? [Number(explicit[1])] : await ports(p.pid);
      for (const port of list) {
        const base = `http://127.0.0.1:${port}`;
        if (found.some((f) => f.base === base)) continue;
        try {
          const h = await get(base, "/global/health", 800);
          if (!h?.healthy) continue;
          const dir = await get(base, "/path", 800).then((x) => x?.directory ?? null, () => null);
          found.push({ pid: p.pid, base, dir });
        } catch {}
      }
    }
    cache = { at: now(), servers: found };
    return found;
  }

  // The server whose TUI has the session in hand. Every OpenCode on the
  // machine reads the same database, so each of them knows every session,
  // and a turn started through another's server runs in that process, out
  // of sight of the TUI that shows the session. So: the one that reports a
  // status for it (OpenCode keeps that per running instance), else one
  // started in the session's folder, else any that knows it.
  async function serverFor(id) {
    const ses = openCodeId(id);
    let here = null, known = null;
    for (const s of await servers()) {
      try {
        const status = await get(s.base, "/session/status");
        if (status && Object.hasOwn(status, ses)) return s;
        if (!here) {
          const info = await get(s.base, `/session/${encodeURIComponent(ses)}`);
          known ??= s;
          if (info?.directory && s.dir && samePath(info.directory, s.dir)) here = s;
        }
      } catch {}
    }
    return here ?? known;
  }

  // What OpenCode says of a session, for the engine: its title, its folder,
  // when it last changed, and whether a turn is under way. null when no
  // running OpenCode knows it. Each answer is kept for cacheMs, since the
  // engine asks every time it lists the agents.
  const infos = new Map(); // ses_… → { at, value }
  async function info(id) {
    const ses = openCodeId(id);
    const hit = infos.get(ses);
    if (hit && now() - hit.at < cacheMs) return hit.value;
    let value = null;
    const s = await serverFor(id).catch(() => null);
    if (s) {
      try {
        const [session, status] = await Promise.all([get(s.base, `/session/${encodeURIComponent(ses)}`), get(s.base, "/session/status")]);
        const st = status?.[ses]?.type;
        value = { title: typeof session?.title === "string" ? session.title : null, directory: session?.directory ?? null, updated: Number(session?.time?.updated) || null, busy: !!st && st !== "idle" };
      } catch {}
    }
    infos.delete(ses);
    infos.set(ses, { at: now(), value });
    if (infos.size > 256) infos.delete(infos.keys().next().value);
    return value;
  }

  return {
    name: "opencode",
    servers,
    serverFor,
    info,
    forget: () => {
      cache = { at: -Infinity, servers: [] };
      infos.clear();
    },
    async deliver(id, text) {
      const s = await serverFor(id);
      if (!s) return { ok: false, why: "no running OpenCode has this session" };
      const r = await fetch(`${s.base}/session/${encodeURIComponent(openCodeId(id))}/prompt_async`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ parts: [{ type: "text", text }] }),
        signal: AbortSignal.timeout(3000),
      });
      return r.status === 204 || r.ok ? { ok: true, where: s.base } : { ok: false, why: `OpenCode answered ${r.status}` };
    },
  };
}
