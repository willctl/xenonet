// core/hosts/relay.js in Rust (crates/xenonet-hosts/src/relay.rs): xenonet
// relay, this XenoNet's map for a console on another machine, over the SSH
// connection the console opened. What leaves this machine is the Rust's:
// the home folder and other folders named for a person written as ~, the
// map, live state, engine and events shaped (titles, states and counts
// only unless --detail, never another host's), the allowlist of reads,
// and what waits while the console reads slowly. The HTTP calls to the
// XenoNet here, the pipes, the timers and the signals are here.
import http from "node:http";
import path from "node:path";
import { spawn as childSpawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { aliasReader } from "./aliases.js";
import { PROTOCOL, MAX_FRAME, MAX_REQUEST, FROM_CONSOLE, lineSplitter, parseFrame, encode } from "./frames.js";

const { systemClock } = await import(peer("blocks/clock.js"));
const { VERSION } = await import(peer("version.js"));

// The checkout this twin is in: crates/xenonet-node/js/core/hosts/.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..", "..");
const C = native.hostsRelayConstants();
export const PING_MS = C.pingMs;
export const DEFAULT_SINCE_MS = C.defaultSinceMs;
const GET_TIMEOUT_MS = C.getTimeoutMs;
const MAX_GETS = C.maxGets;
export const EXIT = { ...C.exit };

export const parseSince = (text) => native.hostsParseSince(String(text ?? "")) ?? null;

export function fromOriginalCommand(text) {
  const since = native.hostsFromOriginalCommand(String(text ?? ""));
  return since ? { sinceMs: since } : {};
}

// ---------- what leaves this machine ----------

const SCRUB = Symbol("scrubber");

export function homeScrubber(home, { also = [] } = {}) {
  const scrubber = new native.HostsScrubber(home == null ? null : String(home), (also ?? []).map((f) => String(f ?? "")));
  const walk = (v) => scrubber.walk(v);
  walk[SCRUB] = scrubber;
  return walk;
}

const copyOf = homeScrubber(null);

// The Rust copies with the relay's scrubber itself; any other copy is
// asked for by name.
const copying = (copy) => (copy?.[SCRUB] ? [copy[SCRUB], null] : [null, copy]);

export const userFolderOf = (p, home) => native.hostsUserFolderOf(p, home == null ? null : String(home)) ?? null;

export const userFolders = (graph, home) => native.hostsUserFolders(graph, home == null ? null : String(home));

export const metadataOnly = {
  graph: (g) => native.hostsMetadataOnly("graph", g),
  live: (l) => native.hostsMetadataOnly("live", l),
  activity: (a) => native.hostsMetadataOnly("activity", a),
  engine: (e) => native.hostsMetadataOnly("engine", e),
  message: (m) => native.hostsMetadataOnly("message", m),
};

export function shapeGraph(input, { sinceMs = DEFAULT_SINCE_MS, now = Date.now(), aliases = new Map(), detail = false, copy = copyOf } = {}) {
  const r = native.hostsShapeGraph(input, sinceMs == null ? null : Number(sinceMs), Number(now), aliases instanceof Map ? [...aliases] : aliases, Boolean(detail), ...copying(copy));
  return { graph: r.graph, dropped: new Set(r.dropped) };
}

export const shapeLive = (input, { dropped = new Set(), detail = false, copy = copyOf } = {}) => native.hostsShapeLive(input, [...dropped].map(String), Boolean(detail), ...copying(copy));

export const shapeEngine = (input, { detail = false, copy = copyOf } = {}) => native.hostsShapeEngine(input, Boolean(detail), ...copying(copy));

export const shapeEvent = (name, data, { dropped = new Set(), detail = false, copy = copyOf } = {}) => native.hostsShapeEvent(String(name), data, [...dropped].map(String), Boolean(detail), ...copying(copy));

// ---------- the console's reads ----------

export function allowed(raw) {
  const r = native.hostsAllowed(raw);
  return r.error ? { error: r.error } : { route: { shape: r.shape }, path: r.path, read: r.read };
}

// ---------- talking to the XenoNet on this machine ----------

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
        from = Math.max(0, buf.length - 1);
        if (buf.length > MAX_FRAME * 2) req.destroy(new Error("an event too big to relay"));
      });
      res.on("end", () => end(null));
      res.on("close", () => end(null));
      res.on("error", end);
      resolve({ close: () => req.destroy() });
    });
    req.on("error", (err) => (started ? end(err) : reject(err)));
    req.end();
  });
}

function startXenoNet({ port, demo, spawn }) {
  const child = spawn(process.execPath, [path.join(root, "server", "index.js"), ...(demo ? ["--demo"] : [])], { env: { ...process.env, XENONET_PORT: String(port) }, stdio: "ignore", windowsHide: true });
  child.on("error", () => {});
  return child;
}

// ---------- the relay ----------

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
  const box = new native.HostsOutbox();
  let stopped = false;
  let child = null;
  let stream = null;
  let pinger = null;
  let finish;
  const done = new Promise((resolve) => (finish = resolve));

  const write = (text) => {
    if (!output.write(text)) box.full();
  };
  const send = (frame, coalesce = null) => {
    if (stopped) return;
    const text = encode(frame);
    if (text.length > MAX_FRAME) return log(`left out a ${frame.t} too big to send (${Math.round(text.length / 1e6)} MB)`);
    const out = box.send(text, coalesce, clock.now());
    if (out !== null) write(out);
  };
  const drain = () => {
    const waiting = box.drain();
    for (let i = 0; i < waiting.length; i++) {
      if (box.busy) {
        box.left(waiting.slice(i));
        break;
      }
      box.wrote(clock.now());
      write(waiting[i]);
    }
    if (box.takeGap()) snapshot().catch(() => {});
  };
  output.on?.("drain", drain);

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
    const bye = native.hostsSelfBye(Number(self.status), self.body ?? null, up.version);
    stop(bye.exit, bye.bye);
    return done;
  }
  const home = self.body.home ?? null;
  let scrub = homeScrubber(home);
  let folders = "";
  const aliases = aliasReader(home);
  let dropped = new Set();
  let ready = false;

  send({ t: "hello", schema: "host/hello/v1", protocol: PROTOCOL, version: VERSION, host: { id: self.body.id ?? null, key: self.body.key }, platform: process.platform, demo: !!up.demo, now: clock.now(), sinceMs, detail });

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
    if (stopped) return (stream.close(), done);
    await snapshot();
  } catch (err) {
    stop(EXIT.xenonetGone, { code: "xenonet-gone", message: `couldn't read the XenoNet on 127.0.0.1:${port}: ${err.message}` });
    return done;
  }
  if (stopped) return done;

  let inflight = 0;
  const answer = async (f) => {
    const id = Number.isSafeInteger(f.id) ? f.id : null;
    if (id === null) return;
    const ok = allowed(f.path);
    if (ok.error) return send({ t: "res", id, status: 403, body: { error: ok.error } });
    if (inflight >= MAX_GETS) return send({ t: "res", id, status: 429, body: { error: "too many reads at once" } });
    inflight++;
    try {
      const res = await get(port, ok.read);
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
  pinger = clock.setInterval(() => box.pingDue(clock.now()) && send({ t: "ping", now: clock.now() }), 5e3);
  pinger?.unref?.();
  return done;
}
