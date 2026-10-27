// The tunnel that puts XenoNet's actor gateway (server/actor-gateway.js) on
// the internet, of the kind its owner picks in XenoNet's window: XenoNet
// assumes none of them is there, finds the ones that are, and keeps the
// one picked running across restarts, sleep and reboots.
//
//   tailscale         Tailscale Funnel: XenoNet runs `tailscale funnel --bg`
//                     with the command that matches the Tailscale running,
//                     and Tailscale keeps it across restarts. The address
//                     is this computer's name on the owner's tailnet. It
//                     publishes the gateway's /mcp alone.
//   cloudflare        a named Cloudflare Tunnel the owner made with
//                     cloudflared, routed to a name on a domain of theirs:
//                     XenoNet runs `cloudflared tunnel run` as its child.
//   cloudflare-quick  a Cloudflare quick tunnel: no account, but a new
//                     address each time it starts.
//   ngrok             ngrok, signed in, on the owner's ngrok domain if they
//                     name one: XenoNet runs it as its child.
//   own               anything the owner runs that forwards an address of
//                     theirs to the gateway's port: XenoNet only checks it.
//
// XenoNet never holds a tunnel's secrets: each tool reads its own sign-in
// from its own config. A tunnel XenoNet runs as its child stops with XenoNet
// and starts again with it; one left behind by a XenoNet that didn't stop
// cleanly is stopped first (its pid is kept in the state folder), since
// ngrok refuses a second agent on the same domain.
import { execFile, spawn as spawnChild } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { onPath, tilde } from "../core/paths.js";
import { normalHost } from "./actor-gateway.js";

export const KINDS = {
  tailscale: {
    label: "Tailscale Funnel",
    through: "Tailscale Funnel",
    short: "Your computer's tailnet name",
    tool: "tailscale",
    stable: true,
    asks: {},
    needs: "Needs Tailscale signed in, with Funnel allowed for this computer in your tailnet's policy.",
    note: "Funnel puts the gateway's /mcp alone on the internet, and makes this computer's tailnet name public. Rename it in Tailscale first if the name says too much.",
    docs: "https://tailscale.com/kb/1223/funnel",
  },
  cloudflare: {
    label: "Cloudflare Tunnel",
    through: "a Cloudflare Tunnel",
    short: "A name on your own domain",
    tool: "cloudflared",
    stable: true,
    asks: { name: "required", host: "required" },
    needs: "Needs a tunnel routed to your domain: cloudflared tunnel create xenonet, then cloudflared tunnel route dns xenonet <address>.",
    note: null,
    docs: "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/",
  },
  "cloudflare-quick": {
    label: "Cloudflare quick tunnel",
    through: "a Cloudflare quick tunnel",
    short: "No account; a new address each start",
    tool: "cloudflared",
    stable: false,
    asks: {},
    needs: "Needs only cloudflared, no account.",
    note: "Its address changes every start, and each app then needs the new one. Good for a try.",
    docs: "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/",
  },
  ngrok: {
    label: "ngrok",
    through: "ngrok",
    short: "Your ngrok domain",
    tool: "ngrok",
    stable: true,
    asks: { host: "optional" },
    needs: "Needs your authtoken added: ngrok config add-authtoken <token>.",
    note: "Add your ngrok domain so the address stays the same.",
    docs: "https://ngrok.com/docs/agent/",
  },
  own: {
    label: "Your own",
    through: "a tunnel of your own",
    short: "A proxy or tunnel you already run",
    tool: null,
    stable: true,
    asks: { host: "required" },
    needs: "Forward https://<address>/mcp to the gateway's port on this computer, keeping the Host header.",
    note: "XenoNet checks it, but doesn't run it.",
    docs: null,
  },
};
export const KIND_IDS = Object.keys(KINDS);
// A Cloudflare tunnel's name or id, as cloudflared tunnel list shows it.
export const TUNNEL_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

// Where each tool may be, besides PATH: a service started at login has a
// short PATH, and Homebrew's and the apps' folders aren't on it.
const PLACES = {
  tailscale: { darwin: ["/Applications/Tailscale.app/Contents/MacOS/Tailscale", "/opt/homebrew/bin/tailscale", "/usr/local/bin/tailscale"], linux: ["/usr/bin/tailscale", "/usr/local/bin/tailscale"], win32: ["C:\\Program Files\\Tailscale\\tailscale.exe"] },
  cloudflared: { darwin: ["/opt/homebrew/bin/cloudflared", "/usr/local/bin/cloudflared"], linux: ["/usr/bin/cloudflared", "/usr/local/bin/cloudflared"], win32: ["C:\\Program Files (x86)\\cloudflared\\cloudflared.exe", "C:\\Program Files\\cloudflared\\cloudflared.exe"] },
  ngrok: { darwin: ["/opt/homebrew/bin/ngrok", "/usr/local/bin/ngrok"], linux: ["/usr/local/bin/ngrok", "/usr/bin/ngrok", "/snap/bin/ngrok"], win32: [] },
};
const TOOL_MS = 60e3;
// A child that exits by itself: wait, then start it again, longer each
// time; past as many exits in ten minutes as there are waits, it stays
// failed until the next check (every ten minutes) or a Repair.
const BACKOFF_MS = [5e3, 20e3, 60e3];
const WINDOW_MS = 10 * 60e3;
const LINES = 20;

// Every copy of a tool on this computer: on PATH, then the usual places.
export function findTools(tool, { env = process.env, platform = process.platform, exists = (f) => fs.statSync(f).isFile() } = {}) {
  const ok = (f) => {
    try {
      return !!f && exists(f);
    } catch {
      return false;
    }
  };
  const onpath = onPath(tool, { env, platform, canRun: (f) => {
    if (!ok(f)) throw new Error("no");
  } });
  return [...new Set([onpath, ...(PLACES[tool]?.[platform] ?? [])].filter(ok))];
}

export const versionIn = (text) => /(\d+\.\d+\.\d+)/.exec(String(text ?? ""))?.[1] ?? null;

// `tailscale status --json`, as what the gateway needs from it: whether
// it's up, this computer's name, whether the tailnet lets it use Funnel,
// and the running Tailscale's version.
export function tailscaleSelf(json) {
  let s;
  try {
    s = typeof json === "string" ? JSON.parse(json) : json;
  } catch {
    return { up: false, why: "Tailscale's status isn't JSON" };
  }
  const state = s?.BackendState;
  const version = versionIn(s?.Version);
  if (state !== "Running") return { up: false, version, why: state === "NeedsLogin" ? "Tailscale isn't signed in on this computer" : state === "Stopped" ? "Tailscale is turned off on this computer" : `Tailscale is ${state ? state.toLowerCase() : "not running"}` };
  const caps = [...Object.keys(s.Self?.CapMap ?? {}), ...(s.Self?.Capabilities ?? [])];
  return { up: true, version, host: normalHost(s.Self?.DNSName), funnel: caps.some((c) => c === "funnel" || String(c).startsWith("https://tailscale.com/cap/funnel")) };
}

// The one path Funnel publishes: the actor gateway's MCP endpoint. The
// network's routes and A2A stay off the internet.
export const FUNNEL_PATH = "/mcp";

// The command that has Funnel publish the gateway's /mcp alone.
export const funnelArgs = (port) => ["funnel", "--bg", "--yes", "--set-path", FUNNEL_PATH, `http://127.0.0.1:${port}${FUNNEL_PATH}`];

// What `tailscale funnel status --json` serves on port 443 of this
// computer's name: nothing, the gateway's /mcp alone (on the internet, or
// the tailnet only), the gateway's whole port as an older XenoNet set it
// up ("wide", which XenoNet narrows), something else at one path (which
// Repair may replace), or other paths beside the gateway's, which Funnel
// would put on the internet with it. `public` lists the paths on the
// internet.
export function funnelOf(conf, host, port) {
  const hp = `${host}:443`;
  const handlers = conf?.Web?.[hp]?.Handlers ?? {};
  const mounts = Object.keys(handlers);
  const funnel = conf?.AllowFunnel?.[hp] === true;
  const publicPaths = funnel ? mounts : [];
  if (!mounts.length) return { state: "none", public: publicPaths };
  const toUs = (m, path) => handlers[m]?.Proxy != null && String(handlers[m].Proxy).replace("localhost", "127.0.0.1").replace(/\/+$/, "") === `http://127.0.0.1:${port}${path}`;
  const ours = (m) => (m === FUNNEL_PATH && toUs(m, FUNNEL_PATH)) || (m === "/" && toUs(m, ""));
  if (mounts.some((m) => !ours(m))) {
    if (mounts.length > 1) return { state: "shared", target: mounts.join(", "), public: publicPaths };
    return { state: "elsewhere", target: handlers["/"]?.Proxy ?? mounts[0], public: publicPaths };
  }
  return { state: "/" in handlers ? "wide" : funnel ? "ours" : "tailnet", public: publicPaths };
}

// A line a tunnel tool wrote, as what XenoNet learns from it: the address it
// got, that it's up, or what went wrong. cloudflared writes plain lines,
// ngrok JSON ones (with --log-format json).
export function readLine(kind, raw) {
  const line = String(raw ?? "").trim();
  if (!line) return null;
  if (kind === "ngrok") {
    let o;
    try {
      o = JSON.parse(line);
    } catch {
      return { msg: line };
    }
    const err = o.err && o.err !== "<nil>" ? String(o.err) : null;
    if (o.msg === "started tunnel" && o.url) return { msg: `started ${o.url}`, host: normalHost(o.url), ready: true };
    if (err || ["eror", "crit"].includes(o.lvl)) return { msg: err ?? String(o.msg ?? line), error: true };
    return { msg: String(o.msg ?? line) };
  }
  const quick = /https:\/\/([a-z0-9-]+\.trycloudflare\.com)/.exec(line);
  if (quick) return { msg: line, host: quick[1] };
  if (/Registered tunnel connection/.test(line)) return { msg: line, ready: true };
  if (/\b(ERR|FTL)\b|error=/.test(line)) return { msg: line, error: true };
  return { msg: line };
}

// The command line XenoNet runs for a kind it keeps as its child.
export function childArgs(kind, { port, name = null, host = null }) {
  const origin = `http://127.0.0.1:${port}`;
  if (kind === "cloudflare") return ["tunnel", "--no-autoupdate", "run", "--url", origin, name];
  if (kind === "cloudflare-quick") return ["tunnel", "--no-autoupdate", "--url", origin];
  if (kind === "ngrok") return ["http", `127.0.0.1:${port}`, "--log", "stdout", "--log-format", "json", ...(host ? ["--url", `https://${host}`] : [])];
  return null;
}

const runOf = (file, args, ms) =>
  new Promise((resolve) => {
    try {
      execFile(file, args, { timeout: ms, maxBuffer: 4 << 20, windowsHide: true }, (err, stdout, stderr) => resolve({ ok: !err, stdout: String(stdout ?? ""), stderr: String(stderr ?? ""), timedOut: !!err?.killed }));
    } catch (err) {
      resolve({ ok: false, stdout: "", stderr: String(err?.message ?? err), timedOut: false });
    }
  });

// `onHost(host)`: the address a tunnel got, when it's one XenoNet learns
// (Tailscale's name, a quick tunnel's or ngrok's). `onReady()`: a child
// tunnel says it's connected. The rest stand in for tests.
export function createTunnel({ port, dir = null, onHost = () => {}, onReady = () => {}, onChange = () => {}, log = () => {}, home = process.env.HOME ?? "", env = process.env, platform = process.platform, exists, run = runOf, spawn = spawnChild, now = Date.now, setTimer = setTimeout, clearTimer = clearTimeout, kill = (pid, sig) => process.kill(pid, sig) }) {
  const pidFile = dir ? path.join(dir, "gateway-tunnel.pid") : null;
  const toolCache = new Map(); // tool → { at, found }
  let want = null; // { kind, name, host } while it should be up
  let state = { state: "off", why: null, since: now() };
  let child = null;
  let exits = [];
  let lines = [];
  let timer = null;
  let stopping = false;
  let failedAt = 0;
  // The Tailscale command XenoNet uses, as it last found it, for kinds().
  let tailscaleBin = null;
  let publicPaths = null; // the paths Funnel put on the internet when last read

  const setState = (s, why = null) => {
    if (state.state === s && state.why === why) return;
    state = { state: s, why, since: now() };
    onChange();
  };

  // The copies of a tool here, each with its version, for a minute.
  async function tool(name) {
    const had = toolCache.get(name);
    if (had && now() - had.at < TOOL_MS) return had.found;
    const found = [];
    for (const file of findTools(name, { env, platform, ...(exists ? { exists } : {}) })) {
      const out = await run(file, name === "ngrok" || name === "cloudflared" ? ["--version"] : ["version"], 5000);
      found.push({ path: file, version: versionIn(out.stdout || out.stderr) });
    }
    toolCache.set(name, { at: now(), found });
    return found;
  }

  // ---------- Tailscale Funnel ----------

  // The Tailscale command to use: the one whose version is the running
  // Tailscale's. An older command beside a newer Tailscale (Homebrew's,
  // beside the Mac app) can set Funnel up so that its name never reaches
  // the internet's DNS.
  async function tailscale() {
    const found = await tool("tailscale");
    if (!found.length) return { why: "XenoNet can't find Tailscale on this computer", fix: KINDS.tailscale.docs };
    let self = null;
    for (const t of found) {
      const out = await run(t.path, ["status", "--json"], 5000);
      if (!out.stdout.trim()) continue;
      self = tailscaleSelf(out.stdout);
      break;
    }
    if (!self) return { why: "Tailscale didn't answer: is it running?" };
    const bin = found.find((t) => t.version && t.version === self.version) ?? found[0];
    tailscaleBin = { at: now(), bin };
    if (!self.up) return { bin, self, why: self.why };
    if (!self.host) return { bin, self, why: "Tailscale didn't say this computer's name: turn on MagicDNS and HTTPS certificates for your tailnet" };
    return { bin, self };
  }

  async function funnelNow(bin, host) {
    const out = await run(bin.path, ["funnel", "status", "--json"], 5000);
    let conf = {};
    try {
      conf = JSON.parse(out.stdout || "{}");
    } catch {}
    const f = funnelOf(conf, host, port);
    publicPaths = f.public;
    return f;
  }

  async function funnelOff(bin) {
    const out = await run(bin.path, ["funnel", "--yes", "--https=443", "off"], 10000);
    if (!out.ok) log("gateway.tunnel", `tailscale funnel off failed: ${out.stderr.trim() || out.stdout.trim()}`);
  }

  async function tailscaleUp({ takeOver = false } = {}) {
    if (state.state !== "up") setState("starting", "setting up Tailscale Funnel");
    const ts = await tailscale();
    if (ts.why) return setState("failed", ts.why);
    const { bin, self } = ts;
    onHost(self.host);
    if (!self.funnel) return setState("failed", "your tailnet doesn't allow Funnel for this computer yet: allow it in the tailnet's access controls (Tailscale's Funnel guide says how), then Repair");
    let f = await funnelNow(bin, self.host);
    if (f.state === "shared") return setState("failed", `Tailscale already serves other paths on this computer's port 443 (${f.target}), and Funnel would put them on the internet with the gateway: pick another tunnel, or move them`);
    if (f.state === "elsewhere" && !takeOver) return setState("failed", `Tailscale Funnel on this computer already forwards to ${f.target}, not to this XenoNet's gateway: Repair points it here instead`);
    if (f.state !== "ours") {
      // The whole port, as an older XenoNet published it, or what Repair
      // replaces: cleared first, so /mcp is all that's left.
      if (f.state === "wide" || f.state === "elsewhere") await funnelOff(bin);
      const out = await run(bin.path, funnelArgs(port), 20000);
      const said = `${out.stdout}\n${out.stderr}`;
      const link = /https:\/\/login\.tailscale\.com\/\S+/.exec(said)?.[0];
      if (link) return setState("failed", `Tailscale asks you to allow Funnel for this computer at ${link}; then Repair`);
      f = await funnelNow(bin, self.host);
      if (f.state !== "ours") return setState("failed", `tailscale funnel didn't take: ${said.trim().split("\n").at(-1) || "it said nothing"}`);
      log("gateway.tunnel", `Tailscale Funnel forwards ${self.host}${FUNNEL_PATH} alone to port ${port}`);
    }
    setState("up", null);
  }

  // Turns Funnel off, if what it serves is this gateway alone.
  async function tailscaleDown() {
    const ts = await tailscale();
    if (!ts.bin || !ts.self?.host) return;
    const f = await funnelNow(ts.bin, ts.self.host);
    if (f.state === "ours" || f.state === "tailnet" || f.state === "wide") await funnelOff(ts.bin);
    publicPaths = null;
  }

  // ---------- a child XenoNet runs ----------

  // A tunnel left running by a XenoNet that didn't stop cleanly, stopped if
  // it's still that tool.
  async function reap(toolName) {
    if (!pidFile || platform === "win32") return;
    let pid = null;
    try {
      pid = Number(fs.readFileSync(pidFile, "utf8").trim());
    } catch {
      return;
    }
    fs.rmSync(pidFile, { force: true });
    if (!Number.isInteger(pid) || pid <= 1) return;
    const out = await run("ps", ["-o", "comm=", "-p", String(pid)], 3000);
    if (!out.ok || !path.basename(out.stdout.trim()).toLowerCase().startsWith(toolName)) return;
    try {
      kill(pid, "SIGTERM");
      log("gateway.tunnel", `stopped a ${toolName} left running (pid ${pid})`);
    } catch {}
  }

  async function childUp(w) {
    const k = KINDS[w.kind];
    const found = await tool(k.tool);
    if (!found.length) return setState("failed", `XenoNet can't find ${k.tool} on this computer: install it, then Repair`);
    await reap(k.tool);
    if (want !== w) return;
    stopping = false;
    lines = [];
    setState("starting", `${k.tool} is starting`);
    const args = childArgs(w.kind, { port, name: w.name, host: w.host });
    let c;
    try {
      c = spawn(found[0].path, args, { env, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    } catch (err) {
      return exited(null, `it couldn't start: ${err.message}`);
    }
    child = c;
    if (pidFile && c.pid) {
      try {
        fs.mkdirSync(path.dirname(pidFile), { recursive: true });
        fs.writeFileSync(pidFile, String(c.pid), { mode: 0o600 });
      } catch {}
    }
    const read = (stream) => {
      let buf = "";
      stream?.on("data", (d) => {
        buf += d;
        for (let i; (i = buf.indexOf("\n")) >= 0; ) {
          const got = readLine(w.kind, buf.slice(0, i));
          buf = buf.slice(i + 1);
          if (!got) continue;
          lines.push({ at: now(), error: !!got.error, msg: tilde(got.msg, home).slice(0, 300) });
          if (lines.length > LINES) lines.splice(0, lines.length - LINES);
          if (got.host && child === c) onHost(got.host);
          if (got.ready && child === c) {
            setState("up", null);
            onReady();
          }
        }
        if (buf.length > 1 << 16) buf = "";
      });
    };
    read(c.stdout);
    read(c.stderr);
    c.on("error", (err) => exited(c, `it couldn't start: ${err.code ?? err.message}`));
    c.on("exit", (code) => exited(c, `it exited${code != null ? ` with code ${code}` : ""}`));
  }

  function exited(c, why) {
    if (c && child !== c) return;
    child = null;
    if (pidFile) fs.rmSync(pidFile, { force: true });
    if (stopping || !want) return setState("off", null);
    const t = now();
    exits = exits.filter((at) => t - at < WINDOW_MS);
    exits.push(t);
    const last = lines.filter((l) => l.error).at(-1)?.msg;
    const said = last ? `${why}: ${last}` : why;
    log("gateway.tunnel", said);
    if (exits.length > BACKOFF_MS.length) {
      failedAt = t;
      return setState("failed", said);
    }
    setState("failed", `${said}; trying again in ${Math.round(BACKOFF_MS[exits.length - 1] / 1000)} s`);
    const w = want;
    timer = setTimer(() => {
      timer = null;
      if (want === w && !child) childUp(w).catch((err) => setState("failed", err.message));
    }, BACKOFF_MS[exits.length - 1]);
  }

  function childDown() {
    stopping = true;
    clearTimer(timer);
    timer = null;
    exits = [];
    const c = child;
    child = null;
    if (pidFile) fs.rmSync(pidFile, { force: true });
    if (c) {
      try {
        c.kill("SIGTERM");
      } catch {}
    }
  }

  // ---------- the tunnel ----------

  async function down() {
    const was = want;
    want = null;
    childDown();
    if (was?.kind === "tailscale") await tailscaleDown();
    setState("off", null);
  }

  return {
    // Brings the tunnel picked up, stopping another kind first. `takeOver`
    // (Repair) lets Tailscale Funnel replace what it forwards to.
    async up({ kind, name = null, host = null }, { takeOver = false } = {}) {
      if (!KINDS[kind]) throw new Error(`no such tunnel: ${kind}`);
      const same = want && want.kind === kind && want.name === name && want.host === host;
      if (want && !same) await down();
      if (same && (child || state.state === "up") && !takeOver) return this.view();
      const w = same ? want : { kind, name, host };
      want = w;
      exits = [];
      if (kind === "own") setState("yours", null);
      else if (kind === "tailscale") await tailscaleUp({ takeOver });
      else if (!child) await childUp(w);
      return this.view();
    },
    down,
    // From the clock and after a failed check: Funnel set up again if it
    // went, and a child that gave up started again after ten minutes.
    async ensure() {
      if (!want) return;
      if (want.kind === "tailscale") return tailscaleUp();
      if (want.kind !== "own" && !child && !timer && now() - failedAt >= WINDOW_MS) {
        exits = [];
        return childUp(want);
      }
    },
    // On XenoNet's way out: its child stops; Funnel stays set up, so the
    // gateway is back as soon as XenoNet is.
    stop() {
      want = null;
      childDown();
      setState("off", null);
    },
    view() {
      const k = want ? KINDS[want.kind] : null;
      // What's on the internet: Funnel's paths as last read, or null for a
      // tunnel that forwards the gateway's whole port.
      return { kind: want?.kind ?? null, label: k?.label ?? null, through: k?.through ?? null, state: state.state, why: state.why, since: state.since, pid: child?.pid ?? null, log: lines.slice(-6), public: want?.kind === "tailscale" ? (publicPaths ?? null) : null };
    },
    // Each kind, and whether its tool is on this computer.
    async kinds() {
      const out = [];
      for (const [id, k] of Object.entries(KINDS)) {
        const found = k.tool ? await tool(k.tool) : [];
        // Tailscale's: the command that matches the Tailscale running,
        // which is the one XenoNet uses.
        if (id === "tailscale" && found.length > 1 && !(tailscaleBin && now() - tailscaleBin.at < TOOL_MS)) await tailscale();
        const used = (id === "tailscale" && tailscaleBin?.bin && found.find((t) => t.path === tailscaleBin.bin.path)) || found[0];
        out.push({ kind: id, label: k.label, through: k.through, short: k.short, tool: k.tool, found: !k.tool || found.length > 0, path: used ? tilde(used.path, home) : null, version: used?.version ?? null, stable: k.stable, asks: k.asks, needs: k.needs, note: k.note, docs: k.docs });
      }
      return out;
    },
  };
}
