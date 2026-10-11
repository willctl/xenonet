// xenonet relay: this XenoNet's map for a console on another machine, over the
// SSH connection the console opened (docs/design/hosts.md). The relay is a
// local client of the XenoNet on this machine, like xenonet agents: it reads
// the map, the live state and the engine's from 127.0.0.1, follows the
// event stream, and writes it all as frames (frames.js) on its standard
// output. On its standard input it reads only the console's requests,
// and answers them from an allowlist of GET routes. It never opens a port,
// never writes to XenoNet, and never sends the home folder: every path under
// it goes as ~, so no user name crosses machines. Nor does the name of
// another folder named for a person that the map's places are in, such as
// a projects folder on Windows's side of WSL (/mnt/c/Users/<name>).
//
// What crosses by default is titles, states and counts (the owner's call
// at merge point A): prompts, messages, a waiting question's text, tool
// details, the tasks subagents were given and the files sessions touch
// stay here unless the console asks for --detail. Transcripts, stats and
// collision diffs cross only when the console asks for one, which is when
// you open one there.
//
// Only this XenoNet's own view leaves: a XenoNet that shows other hosts itself
// is asked for its local view (?hosts=0), and anything qualified with
// another host is dropped besides, so data never passes from one machine
// to the next and two machines can watch each other without a loop.
import http from "node:http";
import path from "node:path";
import { spawn as childSpawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { pathKey } from "../paths.js";
import { repoKey } from "../schema/ids.js";
import { aliasReader } from "./aliases.js";
import { systemClock } from "../blocks/clock.js";
import { VERSION } from "../version.js";
import { PROTOCOL, MAX_FRAME, MAX_REQUEST, FROM_CONSOLE, lineSplitter, parseFrame, encode } from "./frames.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
export const PING_MS = 15e3;
export const DEFAULT_SINCE_MS = 7 * 86400e3;
const GET_TIMEOUT_MS = 20e3;
const MAX_GETS = 4;
// What waits while the console reads slowly: the newest map and live state
// always (each replaces the last), and other events up to this many bytes.
// Past that, the relay sends a fresh snapshot once the console catches up.
const QUEUE_BYTES = 4 << 20;
// The host's XenoNet restarting (an update, XenoNet Dev reloading) ends the
// relay: the console sees the host offline and comes back by itself.
export const EXIT = { done: 0, failed: 1, noXenoNet: 3, xenonetGone: 4, oldXenoNet: 5 };

// "7d" → 604800000; null for anything else.
export function parseSince(text) {
  const m = /^(\d{1,4})([mhd])$/.exec(String(text ?? ""));
  return m ? Number(m[1]) * { m: 60e3, h: 3600e3, d: 86400e3 }[m[2]] : null;
}

// A key restricted to the relay in the host's authorized_keys
// (command="xenonet relay --port 47840") runs that command whatever the
// console asked for, and ssh puts what it asked for in
// SSH_ORIGINAL_COMMAND. Only --since is taken from there: the forced
// command decides the port, and whether detail leaves this machine.
export function fromOriginalCommand(text) {
  const words = String(text ?? "").trim().split(/\s+/);
  const at = words.indexOf("--since");
  const since = at >= 0 ? parseSince(words[at + 1]) : null;
  return since ? { sinceMs: since } : {};
}

// ---------- what leaves this machine ----------

const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// A function that copies a value with the home folder written as ~ in
// every string and every key, in each form it takes here: as XenoNet was
// given it, as a path key (lowercase, without macOS's /private), and as
// Claude Code names its project folders (-Users-nova-Projects-atlas).
//
// `also` lists other folders named for a person (userFolders), which go
// with ~ in place of the name alone: /mnt/c/Users/nova/Projects becomes
// /mnt/c/Users/~/Projects. The rest still says where it is, and it can't
// be taken for the home folder, whose projects may share its names.
//
// It copies even when there's nothing to write as ~: the shaping below
// changes the copy it gets back, never what XenoNet gave it.
export function homeScrubber(home, { also = [] } = {}) {
  const h = String(home ?? "").replace(/[\\/]+$/, "");
  const swaps = [];
  const lengths = [];
  if (h.length >= 2) {
    const forms = [...new Set([h, h.replace(/\\/g, "/"), pathKey(h)])];
    swaps.push([new RegExp(`(?:${forms.map(esc).join("|")})(?![\\w.-])`, "gi"), "~"], [new RegExp(`${esc(h.replace(/[\\/.:]/g, "-"))}(?![\\w.])`, "gi"), "~"]);
    lengths.push(...forms.map((f) => f.length));
  }
  for (const folder of also) {
    const f = String(folder ?? "").replace(/[\\/]+$/, "");
    const cut = Math.max(f.lastIndexOf("/"), f.lastIndexOf("\\"));
    if (cut < 1 || cut === f.length - 1) continue;
    const [parent, name] = [f.slice(0, cut), f.slice(cut + 1)];
    const parents = [...new Set([parent, parent.replace(/\\/g, "/")])];
    swaps.push([new RegExp(`((?:${parents.map(esc).join("|")})[\\\\/])${esc(name)}(?![\\w.-])`, "gi"), "$1~"], [new RegExp(`(${esc(parent.replace(/[\\/.:]/g, "-"))}-)${esc(name)}(?![\\w.])`, "gi"), "$1~"]);
    lengths.push(f.length);
  }
  const shortest = swaps.length ? Math.min(...lengths) : Infinity;
  const str = (s) => (s.length < shortest ? s : swaps.reduce((t, [re, to]) => t.replace(re, to), s));
  const walk = (v) => {
    if (typeof v === "string") return str(v);
    if (Array.isArray(v)) return v.map(walk);
    if (v && typeof v === "object") {
      const out = {};
      for (const [k, x] of Object.entries(v)) out[str(k)] = walk(x);
      return out;
    }
    return v;
  };
  return walk;
}

// A plain copy, for the shaping below when no scrubber is given.
const copyOf = homeScrubber(null);

// The folder named for a person that a path is in, when it isn't the home
// folder: /Users/<name> or /home/<name>, after a drive if there is one
// (C:\Users\nova, /c/Users/nova, or WSL's /mnt/c/Users/nova for Windows's
// side). A projects folder there would carry the name across machines as
// the home folder would. Null for a path in none, or in the home folder.
// A folder there that no one is named for (/Users/Shared) can't be told
// apart, and goes the same way.
const USER_FOLDER = /^((?:[A-Za-z]:|\/mnt\/[A-Za-z]|\/[A-Za-z](?=[\\/]))?[\\/](?:Users|home)[\\/][^\\/]+)(?=[\\/]|$)/i;
export function userFolderOf(p, home) {
  const m = USER_FOLDER.exec(typeof p === "string" ? p : "");
  if (!m) return null;
  const key = pathKey(m[1]);
  const homeKey = pathKey(home);
  return homeKey && (key === homeKey || key.startsWith(`${homeKey}/`)) ? null : m[1];
}

// The folders named for a person, besides the home folder, that the map's
// places are in: the projects folder, the folders outside it where
// sessions ran, and Codex's projects. One folder once, however it's
// written. It's read from the map as XenoNet gives it, before shaping, so
// another host's places, which never leave, don't count.
export function userFolders(graph, home) {
  const found = new Map();
  const add = (p) => {
    const f = userFolderOf(p, home);
    if (f && !found.has(pathKey(f))) found.set(pathKey(f), f);
  };
  for (const n of graph?.nodes ?? []) {
    if (qualified(n?.id)) continue;
    if (n?.kind === "core" || n?.kind === "zone") add(n.path);
    else if (n?.kind === "native" && Array.isArray(n.roots)) n.roots.forEach(add);
  }
  return [...found.values()].sort();
}

const qualified = (id) => typeof id === "string" && id.startsWith("@");

// A stuck reason without the command in it.
const plainReason = (r) => String(r).replace(/^ran .+ (\d+) times in a row$/, "ran the same command $1 times in a row");
// A subagent's label is the start of what it was asked to do (a Task's
// description, or its first prompt); without detail it goes as the name
// core/live.js gives one it has no label for.
const SUBAGENT = "subagent";

// Titles, states and counts only: what's said in sessions (prompts,
// messages, questions, previews, subagents' tasks), what tools were run
// on, and the files sessions touched, taken out in place.
export const metadataOnly = {
  graph(g) {
    for (const n of g.nodes) {
      if (n.kind === "session" && "preview" in n) n.preview = null;
      if (n.kind === "page") n.summary = "";
    }
    return g;
  },
  live(l) {
    for (const s of l.sessions ?? []) {
      s.lastMessage = null;
      if (s.needsYou) s.needsYou = { ...s.needsYou, text: "" };
      if (s.lastTool) s.lastTool = { ...s.lastTool, detail: "" };
      s.files = [];
      if (s.stuck?.reasons) s.stuck = { ...s.stuck, reasons: s.stuck.reasons.map(plainReason) };
      if (Array.isArray(s.subagents)) s.subagents = s.subagents.map((a) => ({ ...a, label: SUBAGENT }));
    }
    for (const c of l.collisions ?? []) Object.assign(c, { files: [], paths: [] });
    for (const c of l.collab ?? []) c.files = [];
    return l;
  },
  activity(a) {
    if (["tool", "prompt", "message", "needs-you"].includes(a.kind)) a.detail = "";
    if (a.kind === "subagent-start" || a.kind === "subagent-stop") a.label = SUBAGENT;
    if (Array.isArray(a.files)) a.files = [];
    return a;
  },
  engine(e) {
    for (const m of e.messages ?? []) m.text = "";
    for (const a of e.agents ?? []) a.claims = (a.claims ?? []).map((c) => ({ ...c, pattern: "", note: null }));
    return e;
  },
  message(m) {
    m.text = "";
    return m;
  },
};

// The map as it leaves: without another host's nodes, without sessions
// older than `sinceMs` or archived (a session kept under one that went
// moves up to the nearest place kept), each project with its repo key, and
// titles, states and counts only unless `detail`. Says which sessions it
// left out, so the live state can leave them out too.
//
// What's kept is copied once, by `copy`: the relay's scrubber, which
// writes the home folder as ~ as it copies. The map it's given stays as it
// was, and what's left out is never copied: on a large machine that's
// most of its sessions.
export function shapeGraph(input, { sinceMs = DEFAULT_SINCE_MS, now = Date.now(), aliases = new Map(), detail = false, copy = copyOf } = {}) {
  const nodes = input.nodes ?? [];
  const cutoff = sinceMs == null ? -Infinity : now - sinceMs;
  const dropped = new Set();
  const gone = new Set();
  for (const n of nodes) {
    if (qualified(n.id) || n.kind === "host") gone.add(n.id);
    else if (n.kind === "session" && (n.archived || (n.updatedAt ?? 0) < cutoff)) dropped.add(n.id);
  }
  const out = (id) => gone.has(id) || dropped.has(id) || qualified(id);
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const parent = new Map((input.links ?? []).filter((l) => l.kind !== "lineage-cross").map((l) => [l.target, l.source]));
  const links = [];
  for (const l of input.links ?? []) {
    if (out(l.target)) continue;
    if (!out(l.source)) {
      links.push(l);
      continue;
    }
    if (l.kind === "lineage-cross" || gone.has(l.source) || qualified(l.source)) continue;
    let up = l.source;
    for (let i = 0; i < 64 && up && out(up); i++) up = parent.get(up);
    if (up && !out(up)) links.push({ source: up, target: l.target, kind: byId.get(up)?.kind === "session" ? "lineage" : "session" });
  }
  const kept = [];
  for (const n of nodes) {
    if (out(n.id)) continue;
    // A project's repo key comes from its remote as written here, before
    // the copy.
    const key = n.kind === "project" ? repoKey(n.git?.origin, aliases) : null;
    kept.push(key ? { ...n, repoKey: key } : n);
  }
  const top = { ...input, nodes: kept, links, relations: (input.relations ?? []).filter((r) => !out(r.a) && !out(r.b)), identities: (input.identities ?? []).filter((i) => !qualified(i.id)) };
  delete top.hosts;
  delete top.self;
  const g = copy(top);
  if (g.stats) {
    const sessions = g.nodes.filter((n) => n.kind === "session");
    Object.assign(g.stats, {
      sessions: sessions.length,
      claude: sessions.filter((n) => n.app === "claude").length,
      codex: sessions.filter((n) => n.app === "codex").length,
      grok: sessions.filter((n) => n.app === "grok").length,
      active: sessions.filter((n) => n.activity === "active").length,
      archived: 0,
    });
  }
  return { graph: detail ? g : metadataOnly.graph(g), dropped };
}

// The live state as it leaves, without another host's sessions or the
// sessions the map left out. Like the map, only what's kept is copied.
export function shapeLive(input, { dropped = new Set(), detail = false, copy = copyOf } = {}) {
  const out = (id) => qualified(id) || dropped.has(id);
  const l = copy({
    ...input,
    sessions: (input.sessions ?? []).filter((s) => !out(s.id)),
    collisions: (input.collisions ?? []).filter((c) => !out(c.a) && !out(c.b)),
    collab: (input.collab ?? []).filter((c) => !out(c.a) && !out(c.b)),
  });
  if (l.limits) delete l.limits.others;
  return detail ? l : metadataOnly.live(l);
}

export function shapeEngine(input, { detail = false, copy = copyOf } = {}) {
  const e = input ?? {};
  const c = copy({ ...e, agents: (e.agents ?? []).filter((a) => !qualified(a.id)), messages: (e.messages ?? []).filter((m) => !qualified(m.from) && !(m.to ?? []).some((d) => qualified(d?.id ?? d))) });
  return detail ? c : metadataOnly.engine(c);
}

// One of the host's events as it leaves, or null for one that stays: what
// the page gets that the console needs, about this host's own sessions.
// Copied once it's known to go.
export function shapeEvent(name, data, { dropped = new Set(), detail = false, copy = copyOf } = {}) {
  const out = (id) => qualified(id) || dropped.has(id);
  if (!data || typeof data !== "object") return null;
  switch (name) {
    case "activity":
      if (!data.sessionId || out(data.sessionId) || qualified(data.projectId)) return null;
      return detail ? copy(data) : metadataOnly.activity(copy(data));
    case "tokens":
      return data.sessionId && !out(data.sessionId) ? copy(data) : null;
    case "pulse":
      return data.nodeId && !out(data.nodeId) ? copy(data) : null;
    case "agent-message":
      if (qualified(data.from) || (Array.isArray(data.to) && data.to.some(qualified))) return null;
      return detail ? copy(data) : metadataOnly.message(copy(data));
    case "doctor":
      // How the host's own checks went, for its state on the console.
      return copy({ at: data.at, summary: data.summary, problems: (data.results ?? []).filter((r) => r.status === "warn" || r.status === "fail").map(({ id, title, status, detail: said }) => ({ id, title, status, detail: said })) });
    default:
      return null;
  }
}

// ---------- the console's reads ----------

const SESSION = "(?:claude|codex|grok):[0-9A-Za-z][0-9A-Za-z._-]{0,127}";
const SESSION_ID = new RegExp(`^${SESSION}$`);
// The GET routes a console may read through the relay, and the query each
// takes. Anything else is refused here, before it reaches XenoNet.
const ROUTES = [
  { path: /^\/api\/mcp$/, query: { server: /^[0-9A-Za-z._:-]{1,160}$/ }, shape: "none" },
  { path: new RegExp(`^/api/session/${SESSION}/transcript$`), query: { cursor: /^[0-9A-Za-z._:-]{1,200}$/, limit: /^\d{1,4}$/ }, shape: "none" },
  { path: new RegExp(`^/api/session/${SESSION}/stats$`), query: {}, shape: "none" },
  { path: /^\/api\/collision$/, query: { a: SESSION_ID, b: SESSION_ID }, shape: "none" },
  { path: /^\/api\/graph$/, query: {}, shape: "graph" },
  { path: /^\/api\/live$/, query: {}, shape: "live" },
  { path: /^\/api\/engine\/state$/, query: {}, shape: "engine" },
];

// A get's path against the allowlist: the route it is, or why not. Only
// plain paths pass: no dots, nothing encoded, no backslashes or doubled
// slashes, nothing a URL parser would read differently.
export function allowed(raw) {
  if (typeof raw !== "string" || !raw || raw.length > 2048) return { error: "not a path" };
  if (/[^\x21-\x7e]|%|\\|\.\.|\/\/|#/.test(raw)) return { error: "only plain paths can be read through the relay" };
  const [pathname, search = ""] = raw.split(/\?(.*)/s);
  let url;
  try {
    url = new URL(raw, "http://relay.invalid");
  } catch {
    return { error: "not a path" };
  }
  if (url.origin !== "http://relay.invalid" || url.pathname !== pathname) return { error: "only plain paths can be read through the relay" };
  const route = ROUTES.find((r) => r.path.test(pathname));
  if (!route) return { error: `${pathname} can't be read through the relay` };
  const params = new URLSearchParams(search);
  // A key is one the route lists itself: constructor or __proto__ is one
  // every object has, and no route's.
  for (const [k, v] of params) if (!Object.hasOwn(route.query, k) || !route.query[k].test(v)) return { error: `${pathname} doesn't take ${k}=${v.slice(0, 40)}` };
  return { route, path: `${pathname}${search ? `?${params}` : ""}` };
}

// ---------- talking to the XenoNet on this machine ----------

// GET a route of the XenoNet on 127.0.0.1:<port>: { status, body }. The Host
// header is 127.0.0.1:<port>, as XenoNet's own check wants.
export function getJson(port, route, { timeoutMs = GET_TIMEOUT_MS, max = MAX_FRAME } = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: "127.0.0.1", port, path: route, method: "GET", headers: { accept: "application/json" }, timeout: timeoutMs }, (res) => {
      const chunks = [];
      let size = 0;
      res.on("data", (c) => {
        size += c.length;
        if (size > max) req.destroy(Object.assign(new Error("too big to relay"), { status: 413 }));
        else chunks.push(c);
      });
      res.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8");
        try {
          resolve({ status: res.statusCode, body: text ? JSON.parse(text) : null });
        } catch {
          resolve({ status: 502, body: { error: "XenoNet answered with something that isn't JSON" } });
        }
      });
      res.on("error", reject);
    });
    req.on("timeout", () => req.destroy(Object.assign(new Error(`no answer in ${timeoutMs / 1000} s`), { status: 504 })));
    req.on("error", reject);
    req.end();
  });
}

// Follows XenoNet's event stream: onEvent(name, data) for each event, and
// onEnd(err) once, when it stops. Resolves once the stream has started, so
// nothing sent after that is missed. { close } stops it.
export function followEvents(port, route, { onEvent, onEnd }) {
  return new Promise((resolve, reject) => {
    let started = false;
    let ended = false;
    const end = (err) => {
      if (ended) return;
      ended = true;
      onEnd?.(err);
    };
    const req = http.request({ host: "127.0.0.1", port, path: route, method: "GET", headers: { accept: "text/event-stream" } }, (res) => {
      if (res.statusCode !== 200) {
        res.resume();
        return reject(Object.assign(new Error(`the event stream answered ${res.statusCode}`), { status: res.statusCode }));
      }
      started = true;
      res.setEncoding("utf8");
      let buf = "";
      let from = 0;
      res.on("data", (text) => {
        buf += text;
        for (let at = buf.indexOf("\n\n", from); at >= 0; at = buf.indexOf("\n\n")) {
          const block = buf.slice(0, at);
          buf = buf.slice(at + 2);
          let name = "message";
          const data = [];
          for (const line of block.split("\n")) {
            if (line.startsWith("event:")) name = line.slice(6).trim();
            else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
          }
          if (!data.length) continue;
          let value;
          try {
            value = JSON.parse(data.join("\n"));
          } catch {
            continue;
          }
          onEvent(name, value);
        }
        // Where the next search for a block's end starts, so a big event
        // arriving in many chunks isn't searched from the top each time.
        from = Math.max(0, buf.length - 1);
        if (buf.length > MAX_FRAME * 2) req.destroy(new Error("an event too big to relay"));
      });
      res.on("end", () => end(null));
      res.on("close", () => end(null));
      res.on("error", end);
      resolve({ close: () => req.destroy() });
    });
    // Before the stream starts, an error is the start failing; after, it's
    // the stream ending.
    req.on("error", (err) => (started ? end(err) : reject(err)));
    req.end();
  });
}

// --start: a headless XenoNet for the length of the relay, for a machine
// with no service set up.
function startXenoNet({ port, demo, spawn }) {
  const child = spawn(process.execPath, [path.join(root, "server", "index.js"), ...(demo ? ["--demo"] : [])], { env: { ...process.env, XENONET_PORT: String(port) }, stdio: "ignore", windowsHide: true });
  child.on("error", () => {});
  return child;
}

// ---------- the relay ----------

// Runs until the console goes away (its frames end) or the XenoNet here
// does; resolves with the exit code (EXIT). `get` and `follow` stand in
// for the HTTP calls in tests.
export async function runRelay({
  port,
  sinceMs = DEFAULT_SINCE_MS,
  detail = false,
  start = false,
  demo = false,
  input = process.stdin,
  output = process.stdout,
  clock = systemClock,
  get = getJson,
  follow = followEvents,
  spawn = childSpawn,
  signals = process,
  log = (line) => process.stderr.write(`xenonet relay: ${line}\n`),
}) {
  let lastWrite = 0;
  let busy = false;
  let queue = [];
  let queued = 0;
  let gap = false;
  const latest = new Map(); // graph and live, newest only, while busy
  let stopped = false;
  let child = null;
  let stream = null;
  let pinger = null;
  let finish;
  const done = new Promise((resolve) => (finish = resolve));

  const send = (frame, coalesce = null) => {
    if (stopped) return;
    const text = encode(frame);
    if (text.length > MAX_FRAME) return log(`left out a ${frame.t} too big to send (${Math.round(text.length / 1e6)} MB)`);
    if (busy) {
      if (coalesce) latest.set(coalesce, text);
      else if (queued + text.length <= QUEUE_BYTES) (queue.push(text), (queued += text.length));
      else gap = true;
      return;
    }
    lastWrite = clock.now();
    if (!output.write(text)) busy = true;
  };
  const drain = () => {
    busy = false;
    const waiting = [...queue, ...latest.values()];
    queue = [];
    queued = 0;
    latest.clear();
    for (const text of waiting) {
      if (busy) {
        queue.push(text);
        queued += text.length;
        continue;
      }
      lastWrite = clock.now();
      if (!output.write(text)) busy = true;
    }
    if (!busy && gap) {
      gap = false;
      snapshot().catch(() => {});
    }
  };
  output.on?.("drain", drain);

  // Asked to stop (the SSH session hanging up, say): a XenoNet this relay
  // started goes with it.
  const onSignal = () => stop(EXIT.done);
  for (const sig of ["SIGTERM", "SIGHUP", "SIGINT"]) signals?.on?.(sig, onSignal);
  const stop = (code, bye = null) => {
    if (stopped) return;
    if (bye) send({ t: "bye", ...bye });
    stopped = true;
    for (const sig of ["SIGTERM", "SIGHUP", "SIGINT"]) signals?.off?.(sig, onSignal);
    clock.clearInterval(pinger);
    stream?.close();
    if (child && child.exitCode === null) child.kill("SIGTERM");
    input.off?.("data", onData);
    input.off?.("end", onEnd);
    output.off?.("drain", drain);
    finish(code);
  };

  // The XenoNet on this machine, started if asked and none answers.
  const health = async () => (await get(port, "/api/health", { timeoutMs: 2000 }).catch(() => null))?.body ?? null;
  let up = await health();
  if (stopped) return done;
  if (!up && start) {
    child = startXenoNet({ port, demo, spawn });
    for (const until = clock.now() + 60e3; !up && clock.now() < until && child.exitCode === null; ) {
      await new Promise((r) => clock.setTimeout(r, 300));
      up = await health();
    }
  }
  if (!up) {
    stop(EXIT.noXenoNet, { code: "no-xenonet", message: `no XenoNet answers on 127.0.0.1:${port} on this machine` });
    return done;
  }
  const self = await get(port, "/api/hosts/self").catch((err) => ({ status: 0, body: { error: err.message } }));
  if (self.status !== 200 || !self.body?.key) {
    const old = self.status === 404;
    stop(old ? EXIT.oldXenoNet : EXIT.failed, { code: old ? "old-xenonet" : "no-self", message: old ? `the XenoNet here (${up.version ?? "an older version"}) can't be a host yet; update it` : `the XenoNet here couldn't say which host it is (${self.body?.error ?? self.status})` });
    return done;
  }
  const home = self.body.home ?? null;
  let scrub = homeScrubber(home);
  let folders = "";
  const aliases = aliasReader(home);
  let dropped = new Set();
  // Whether the first snapshot has gone. Until then nothing else goes:
  // the console takes no event before it has a snapshot (link.js), and
  // until the relay has read a map it doesn't know which sessions that
  // leaves out, or which folders to scrub.
  let ready = false;

  send({ t: "hello", schema: "host/hello/v1", protocol: PROTOCOL, version: VERSION, host: { id: self.body.id ?? null, key: self.body.key }, platform: process.platform, demo: !!up.demo, now: clock.now(), sinceMs, detail });

  // Each map says which folders named for a person its places are in
  // besides the home folder, so the scrubbing follows the projects folder
  // when it changes, and which sessions are left out. Shaping copies what
  // goes once, scrubbed as it's copied.
  const graphOf = (g) => {
    const found = userFolders(g, home);
    if (found.join("\n") !== folders) {
      folders = found.join("\n");
      scrub = homeScrubber(home, { also: found });
    }
    const shaped = shapeGraph(g, { sinceMs, now: clock.now(), aliases: aliases(), detail, copy: scrub });
    dropped = shaped.dropped;
    return shaped.graph;
  };
  const liveOf = (l) => shapeLive(l, { dropped, detail, copy: scrub });
  const engineOf = (e) => shapeEngine(e, { detail, copy: scrub });
  async function snapshot() {
    const [g, l, e] = await Promise.all([get(port, "/api/graph?hosts=0"), get(port, "/api/live?hosts=0"), get(port, "/api/engine/state")]);
    if (g.status !== 200 || l.status !== 200) throw new Error(`XenoNet answered ${g.status} for the map and ${l.status} for the live state`);
    const graph = graphOf(g.body);
    send({ t: "snapshot", at: clock.now(), graph, live: liveOf(l.body), engine: e.status === 200 ? engineOf(e.body) : null });
    ready = true;
  }

  // The event stream first, then the snapshot, so nothing between them is
  // lost: what an event before the snapshot changed comes in the snapshot.
  try {
    stream = await follow(port, "/api/events?hosts=0", {
      onEvent(name, data) {
        if (stopped || !ready) return;
        if (name === "graph") return send({ t: "event", name, data: graphOf(data) }, "graph");
        if (name === "live") return send({ t: "event", name, data: liveOf(data) }, "live");
        const shaped = shapeEvent(name, data, { dropped, detail, copy: scrub });
        if (shaped) send({ t: "event", name, data: shaped });
      },
      onEnd: () => stop(EXIT.xenonetGone, { code: "xenonet-gone", message: `the XenoNet on 127.0.0.1:${port} stopped` }),
    });
    // Stopped meanwhile (a signal while it waited): the stream goes too.
    if (stopped) return (stream.close(), done);
    await snapshot();
  } catch (err) {
    stop(EXIT.xenonetGone, { code: "xenonet-gone", message: `couldn't read the XenoNet on 127.0.0.1:${port}: ${err.message}` });
    return done;
  }
  if (stopped) return done;

  // The console's requests: gets from the allowlist, and the time.
  let inflight = 0;
  const answer = async (f) => {
    const id = Number.isSafeInteger(f.id) ? f.id : null;
    if (id === null) return;
    const ok = allowed(f.path);
    if (ok.error) return send({ t: "res", id, status: 403, body: { error: ok.error } });
    if (inflight >= MAX_GETS) return send({ t: "res", id, status: 429, body: { error: "too many reads at once" } });
    inflight++;
    try {
      const sep = ok.path.includes("?") ? "&" : "?";
      const res = await get(port, ok.route.shape === "graph" || ok.route.shape === "live" ? `${ok.path}${sep}hosts=0` : ok.path);
      const shape = res.status === 200 ? ok.route.shape : "none";
      const body = shape === "graph" ? graphOf(res.body) : shape === "live" ? liveOf(res.body) : shape === "engine" ? engineOf(res.body) : scrub(res.body);
      send({ t: "res", id, status: res.status, body });
    } catch (err) {
      send({ t: "res", id, status: err.status ?? 502, body: { error: err.message } });
    } finally {
      inflight--;
    }
  };
  const lines = lineSplitter({
    max: MAX_REQUEST,
    onLine(line) {
      const { frame } = parseFrame(line, FROM_CONSOLE);
      if (!frame || stopped) return;
      if (frame.t === "time" && Number.isSafeInteger(frame.id)) send({ t: "time", id: frame.id, now: clock.now() });
      // A read that fails some way nothing in answer() expects still gets
      // an answer, and never ends the relay: a console only ever asks.
      else if (frame.t === "get")
        answer(frame).catch((err) => {
          log(`couldn't answer a read: ${err.message}`);
          if (Number.isSafeInteger(frame.id)) send({ t: "res", id: frame.id, status: 500, body: { error: "the relay couldn't answer that" } });
        });
    },
  });
  function onData(chunk) {
    lines(chunk);
  }
  function onEnd() {
    stop(EXIT.done);
  }
  input.on("data", onData);
  input.on("end", onEnd);
  pinger = clock.setInterval(() => clock.now() - lastWrite >= PING_MS && send({ t: "ping", now: clock.now() }), 5e3);
  pinger?.unref?.();
  return done;
}
