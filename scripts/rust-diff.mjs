// The JS service and the Rust one side by side (docs/design/rust-core.md):
// both started on the same made-up machine, each on a port and in a home
// folder of its own, then the same reads asked of both and their answers
// compared, after what's bound to differ (times, process ids, the ports and
// folders, the service's name) is set aside. Says, route by route, whether
// they answer alike, and where they don't.
//
//   node scripts/rust-diff.mjs                  the demo (--demo)
//   node scripts/rust-diff.mjs --home <dir>     a home folder to copy for
//                                               both (test/fixtures style),
//                                               not the demo
//   node scripts/rust-diff.mjs --routes a,b     only these routes
//   node scripts/rust-diff.mjs --bin <file>     the Rust binary
//                                               (target/debug/xenonet)
//   node scripts/rust-diff.mjs --js             the JS service against
//                                               itself, for what differs
//                                               between any two runs
//
// The demo's live sessions keep writing, and its subagents' ids and the
// stars' names are new each run, so some answers differ between two JS
// services too: --js shows which.
//
// Never point --home at your own: each service gets a copy, but reading a
// real home folder reads your sessions.
import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : null;
};
const bin = flag("bin") ?? path.join(root, "target", "debug", process.platform === "win32" ? "xenonet.exe" : "xenonet");
const homeFrom = flag("home");
const demo = !homeFrom;

const ROUTES = [
  "/api/health",
  "/api/graph",
  "/api/live",
  "/api/summary",
  "/api/history?since=0",
  "/api/digest?since=0",
  "/api/daylog",
  "/api/constellation",
  "/api/constellation/labels",
  "/api/sessions?limit=200",
  "/api/engine/state",
  "/api/engine/agents",
  "/api/actors",
  "/api/actors/events?since=0",
  "/api/actors/observers",
  "/api/spend/actors",
  "/api/themes",
  "/api/theme",
  "/api/environment",
  "/api/layout",
  "/api/workspace/root",
  "/api/projects",
  "/api/identities",
  "/api/drift",
  "/api/hosts/self",
  "/api/doctor",
  "/api/delivery",
  "/api/refusals",
  "/api/logs?since=0",
  "/api/system",
  "/api/spotify",
  "/api/decide",
  "/api/command",
];
const only = flag("routes")?.split(",") ?? null;
const routes = only ? ROUTES.filter((r) => only.some((o) => r.startsWith(o))) : ROUTES;

const freePort = () =>
  new Promise((resolve) => {
    const s = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });

function home(name) {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-diff-${name}-`));
  if (homeFrom) fs.cpSync(homeFrom, dir, { recursive: true });
  return dir;
}

function start(name, file, argv, port, dir) {
  const env = { ...process.env, HOME: dir, USERPROFILE: dir, XENONET_PORT: String(port), XENONET_HOT: "0", XENONET_CONTROL: "", XENONET_INSTANCE: "", XENONET_EXEC: "", XENONET_STATE_DIR: path.join(dir, ".xenonet-state"), XENONET_CACHE_DIR: path.join(dir, ".xenonet-cache"), XENONET_RUNTIME_DIR: path.join(dir, ".xenonet-runtime"), XDG_STATE_HOME: "", XDG_CACHE_HOME: "" };
  const proc = spawn(file, argv, { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] });
  let out = "";
  proc.stdout.on("data", (d) => (out += d));
  proc.stderr.on("data", (d) => (out += d));
  return { name, proc, port, dir, log: () => out };
}

async function get(port, route) {
  try {
    const res = await fetch(`http://127.0.0.1:${port}${route}`, { signal: AbortSignal.timeout(30000) });
    const text = await res.text();
    let body = text;
    try {
      body = JSON.parse(text);
    } catch {}
    return { status: res.status, body };
  } catch (err) {
    return { status: 0, body: String(err.message) };
  }
}

async function ready(s) {
  for (const until = Date.now() + 90000; Date.now() < until; await new Promise((r) => setTimeout(r, 200))) {
    const g = await get(s.port, "/api/graph");
    if (g.status === 200 && g.body?.nodes?.some((n) => n.kind === "session" || n.kind === "project")) return true;
    if (s.proc.exitCode !== null) return false;
  }
  return false;
}

// What's bound to differ between two runs, set aside: times near now,
// process ids, the ports and folders, and the services' own versions.
function normal(v, s) {
  const now = Date.now();
  const swap = (text) =>
    String(text)
      .replaceAll(s.dir, "<home>")
      .replaceAll(fs.realpathSync.native(os.tmpdir()), "<tmp>")
      .replaceAll(`:${s.port}`, ":<port>")
      .replace(/xenonet-demo-\d+/g, "xenonet-demo-<port>")
      .replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z/g, "<iso>");
  const walk = (x, key) => {
    if (typeof x === "number") {
      if (["pid", "port", "ms", "took", "uptime", "elapsed"].includes(key)) return `<${key}>`;
      if (x > 1e12 && Math.abs(x - now) < 400 * 86400e3) return "<time>";
      return x;
    }
    if (typeof x === "string") return swap(x);
    if (Array.isArray(x)) return x.map((y) => walk(y, key));
    if (x && typeof x === "object") return Object.fromEntries(Object.entries(x).map(([k, y]) => [swap(k), walk(y, k)]));
    return x;
  };
  return walk(v, null);
}

// The first differences between two values, as paths.
function differences(a, b, at = "", out = []) {
  if (out.length >= 12) return out;
  if (JSON.stringify(a) === JSON.stringify(b)) return out;
  if (a && b && typeof a === "object" && typeof b === "object" && Array.isArray(a) === Array.isArray(b)) {
    if (Array.isArray(a) && a.length !== b.length) out.push(`${at || "."}: ${a.length} items in JS, ${b.length} in Rust`);
    const keys = Array.isArray(a) ? [...Array(Math.min(a.length, b.length)).keys()] : [...new Set([...Object.keys(a), ...Object.keys(b)])];
    for (const k of keys) {
      if (!(k in a)) out.push(`${at}.${k}: only in Rust`);
      else if (!(k in b)) out.push(`${at}.${k}: only in JS`);
      else differences(a[k], b[k], `${at}.${k}`, out);
      if (out.length >= 12) break;
    }
    if (!Array.isArray(a) && JSON.stringify(Object.keys(a)) !== JSON.stringify(Object.keys(b)) && keys.every((k) => k in a && k in b)) out.push(`${at || "."}: keys in another order`);
    return out;
  }
  out.push(`${at || "."}: ${JSON.stringify(a)?.slice(0, 120)} in JS, ${JSON.stringify(b)?.slice(0, 120)} in Rust`);
  return out;
}

const [pj, pr] = [await freePort(), await freePort()];
const js = start("js", process.execPath, ["server/index.js", ...(demo ? ["--demo"] : [])], pj, home("js"));
const rust = args.includes("--js") ? start("rust", process.execPath, ["server/index.js", ...(demo ? ["--demo"] : [])], pr, home("rust")) : start("rust", bin, ["serve", ...(demo ? ["--demo"] : [])], pr, home("rust"));
let same = 0;
const differ = [];
try {
  const [okJs, okRust] = await Promise.all([ready(js), ready(rust)]);
  if (!okJs) throw new Error(`the JS service didn't come up:\n${js.log().slice(-2000)}`);
  if (!okRust) console.log(`the Rust service has no map yet:\n${rust.log().slice(-2000)}\n`);
  // A moment for the readers' first pass on both.
  await new Promise((r) => setTimeout(r, 3000));
  for (const route of routes) {
    const [a, b] = await Promise.all([get(pj, route), get(pr, route)]);
    const na = normal(a, js);
    const nb = normal(b, rust);
    const d = differences(na, nb);
    if (!d.length) {
      same++;
      console.log(`✓ ${route}`);
    } else {
      differ.push(route);
      console.log(`✗ ${route}${b.status !== a.status ? ` (${a.status} in JS, ${b.status} in Rust)` : ""}`);
      for (const line of d) console.log(`    ${line}`);
    }
  }
} finally {
  for (const s of [js, rust]) {
    s.proc.kill("SIGTERM");
    await new Promise((r) => (s.proc.exitCode !== null ? r() : s.proc.once("exit", r)));
    fs.rmSync(s.dir, { recursive: true, force: true, maxRetries: 5 });
    if (demo) fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${s.port}`), { recursive: true, force: true, maxRetries: 5 });
  }
}
console.log(`\n${same} of ${routes.length} routes answer alike${differ.length ? `; ${differ.length} differ` : ""}`);
process.exitCode = differ.length ? 1 : 0;
