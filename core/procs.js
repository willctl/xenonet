// Which process is on the other end of a connection to XenoNet, and the
// processes above it, so the engine can check that a request made in an
// agent's name comes from that agent, and an actor's from where its token
// may be used (see engine/verify.js).
//
// macOS only for now: netstat -anv names the process that owns each TCP
// socket, ps gives each process's parent, start time and program, codesign
// says who signed a running program, lsof where a process works and the
// executable it runs, plutil an app bundle's id, and pgrep a process's
// children. One ps of the whole table costs about a tenth of a second
// whatever it asks for, so the table is shared for a moment, and a
// connection's process, a program's signature, its executable, a bundle's
// id and what a Node or Python process runs are looked up once.
//
// The program ps names is argv[0], which a process sets itself (exec -a).
// Where a check rests on a path (Hermes's hosts, XenoNet's own bundle), the
// executable's real path decides instead (chain's `exe`).
import { execFile } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { APPS } from "./apps.js";
import { agentOf } from "./processes.js";

const TABLE_MS = 1500;
// New Hermes.app bundles a census reads the ids of, at most.
const BUNDLES_AT_ONCE = 16;

// The developer team that signs each agent app (codesign's TeamIdentifier),
// as the app registry has it: Anthropic's for Claude Code, OpenAI's for
// Codex.
const TEAMS = Object.fromEntries(APPS.filter((a) => a.team).map((a) => [a.id, a.team]));
// The teams that sign the actor apps whose makers sign them, by product:
// Grok Bot is Cursor's (Anysphere), Muse is Meta's. Their actors register
// and report only from under the app as its maker built it
// (engine/verify.js). Hermes's Python is signed ad hoc, so it has none.
const PRODUCT_TEAMS = { "grok-bot": "DCNK4UB866", muse: "V9WTTPBFK9" };

// What the commands run with. ps writes start times in the locale's words
// and the zone's local time, which is ambiguous in the hour that repeats
// when clocks go back and wrong once the zone changes under a running
// XenoNet, and in the C locale it escapes letters outside ASCII in paths. So
// times come in English and UTC, and text as UTF-8, whatever XenoNet's own
// environment says (the desktop app sets LANG from the system's language).
function envFor() {
  const env = { ...process.env, LC_TIME: "C", LC_CTYPE: "UTF-8", TZ: "UTC" };
  delete env.LC_ALL;
  return env;
}

// A command's output, or an error that carries what it wrote anyway: lsof
// says 1 when one of the processes it's asked about can't be read (another
// user's), and still lists the rest. procs' own calls on Linux and Windows
// (PROCESSES, TCP_PEER, COMMAND_LINE) are read from the system itself.
function run(file, args) {
  if (OWN.has(file)) return ownCall(file, args);
  return new Promise((resolve, reject) => execFile(file, args, { maxBuffer: 16 * 1024 * 1024, timeout: 5000, windowsHide: true, env: envFor() }, (err, stdout) => (err ? reject(Object.assign(err, { stdout })) : resolve(stdout))));
}

// The agent app a program is, from its path: Claude Code, Codex or
// OpenCode, each by its own rules (processes.js). Programs run by Node or
// Bun are told by their arguments, when given.
export { agentOf };

// The apps of actors whose own processes could be relaying for a cloud
// (docs/design/actors/interface.md, section 5), by their bundle's name:
// Hermes.app (the desktop app, and its setup app), Grok Bot, Muse, and the
// ChatGPT app, where OpenAI's dots run.
const ACTOR_APPS = { Hermes: "hermes-agent", "Grok Bot": "grok-bot", Muse: "muse", ChatGPT: "dots" };
const PYTHON = /^python(\d+(\.\d+)*)?$/i;
// Where Hermes keeps the Pythons that run it: its own folder (the managed
// Python, its venv) or the inside of Hermes.app.
const HERMES_PATH = /\/\.hermes\/|\/Hermes\.app\//;

// Whether a program needs its arguments read to say what it is: Node and
// Bun (which agent, or Hermes's terminal UI), and the Pythons Hermes runs.
export const wantsArgs = (program) => {
  const p = String(program ?? "").replace(/\\/g, "/");
  const base = path.posix.basename(p);
  return /^(node|bun)(\.exe)?$/.test(base) || (PYTHON.test(base) && HERMES_PATH.test(p));
};

// The actor product a program belongs to, from its path and, for Node and
// Python, its arguments: { product, role }, or null. Roles:
//
//   host  runs the agent, so what runs under it is the product's: Hermes's
//         managed Python (or its venv's, or one inside Hermes.app) running
//         its CLI, its terminal UI's backend or its agent
//   tui   Hermes's terminal UI, whose child is the host
//   app   an actor app's own process, its main one or a helper
//
// No code signature vouches for Hermes's processes (its Python is signed
// ad hoc), so this rests on paths and arguments, and on Hermes.app's
// bundle id where a path lies in one (createProcs checks it). An agent's
// own program (Claude Code or Codex inside the ChatGPT app) is an agent,
// not this.
export function productOf(program, args = "") {
  const p = String(program ?? "").replace(/\\/g, "/");
  const base = path.posix.basename(p);
  if (PYTHON.test(base) && HERMES_PATH.test(p) && /\b(hermes_cli|tui_gateway|run_agent)\b/.test(args)) return { product: "hermes-agent", role: "host" };
  if (/^(node|bun)(\.exe)?$/.test(base) && /\/(ui-tui\/dist|hermes_cli\/tui_dist)\/entry\.js\b/.test(args)) return { product: "hermes-agent", role: "tui" };
  const bundle = /\/([^/]+)\.app\//.exec(p)?.[1];
  return bundle && Object.hasOwn(ACTOR_APPS, bundle) ? { product: ACTOR_APPS[bundle], role: "app" } : null;
}

// A Hermes.app is Hermes's, a Grok Bot.app Grok Bot's and a Muse.app Meta's
// Muse, only when its Info.plist says so: other makers' apps can have the
// same names (there are other Mac apps called Muse), and anyone can name a
// bundle Grok Bot.app. For each, the outermost bundle of that name a path
// lies in, and the bundle ids that are the product's: for Hermes, the
// desktop app's and those under it (its setup app's, its helpers'); for
// Grok Bot, Cursor's com.anysphere.sand; for Muse, com.meta.endo.
const VOUCHED = {
  "hermes-agent": { bundle: /^(.*?\/Hermes\.app)\//, id: /^com\.nousresearch\.hermes(\.|$)/ },
  "grok-bot": { bundle: /^(.*?\/Grok Bot\.app)\//, id: /^com\.anysphere\.sand(\.|$)/ },
  muse: { bundle: /^(.*?\/Muse\.app)\//, id: /^com\.meta\.endo(\.|$)/ },
};
// The bundle of a vouched product's name a program lies in, or null.
const vouchedBundle = (program) => {
  for (const v of Object.values(VOUCHED)) {
    const bundle = v.bundle.exec(program)?.[1];
    if (bundle) return bundle;
  }
  return null;
};

// What lsof -a -p <pids> -d txt -Fn says of several processes: each one's
// executable, by pid, as its real path. lsof lists a process's program
// text in the order it's mapped, and the executable is mapped first, before
// dyld and the libraries.
export function parseTexts(text) {
  const out = new Map();
  let pid = null;
  let fd = null;
  for (const line of String(text ?? "").split("\n")) {
    if (line.startsWith("p")) [pid, fd] = [Number(line.slice(1)) || null, null];
    else if (line.startsWith("f")) fd = line.slice(1);
    else if (line.startsWith("n/") && pid && fd === "txt" && !out.has(pid)) out.set(pid, line.slice(1));
  }
  return out;
}

// What lsof -a -p <pid> -d cwd -Fn says of a process's folder: its "n"
// line, or null.
export function parseCwd(text) {
  const line = String(text ?? "").split("\n").find((l) => l.startsWith("n/"));
  return line ? line.slice(1) : null;
}

// What lsof -a -p <pids> -d cwd -Fpn says of several processes' folders:
// their "p" and "n" lines, as a map by pid.
export function parseCwds(text) {
  const out = new Map();
  let pid = null;
  for (const line of String(text ?? "").split("\n")) {
    if (line.startsWith("p")) pid = Number(line.slice(1)) || null;
    else if (line.startsWith("n/") && pid) out.set(pid, line.slice(1));
  }
  return out;
}

// What ps -o pid=,command= -p <pids> says: each process's command line, by
// pid.
export function parseCommands(text) {
  const out = new Map();
  for (const line of String(text ?? "").split("\n")) {
    const m = /^\s*(\d+)\s(.*)$/.exec(line);
    if (m) out.set(Number(m[1]), m[2]);
  }
  return out;
}

// ps -axo pid=,ppid=,lstart=,comm= as a table by pid. lstart reads like
// "Sat Oct  3 16:34:29 2026", in UTC (see envFor); the program's path runs
// to the end of the line.
const MONTHS = { Jan: 0, Feb: 1, Mar: 2, Apr: 3, May: 4, Jun: 5, Jul: 6, Aug: 7, Sep: 8, Oct: 9, Nov: 10, Dec: 11 };
export function parsePs(text) {
  const table = new Map();
  for (const line of text.split("\n")) {
    const m = /^\s*(\d+)\s+(\d+)\s+\w{3}\s+(\w{3})\s+(\d+)\s+(\d+):(\d+):(\d+)\s+(\d{4})\s+(.*)$/.exec(line);
    if (!m) continue;
    const start = Object.hasOwn(MONTHS, m[3]) ? Date.UTC(Number(m[8]), MONTHS[m[3]], Number(m[4]), Number(m[5]), Number(m[6]), Number(m[7])) : 0;
    table.set(Number(m[1]), { pid: Number(m[1]), ppid: Number(m[2]), start, program: m[9].trim() });
  }
  return table;
}

// The process holding the client's end of a connection to 127.0.0.1:
// netstat -anv -p tcp lines read "tcp4 … <local> <foreign> <state>
// <4 counters> <name>:<pid> <flags> …", and the name may have spaces, or be
// empty once the process has exited (the pid is still the last one that
// held the socket). While XenoNet is answering, its own end is open, so the
// client's end is ESTABLISHED, or FIN_WAIT_1 or FIN_WAIT_2 when the client
// has closed its side; TIME_WAIT on that pair is an earlier connection's.
const OPEN = new Set(["ESTABLISHED", "FIN_WAIT_1", "FIN_WAIT_2"]);
export function parseNetstat(text, { localPort, remotePort }) {
  const client = `127.0.0.1.${remotePort}`, server = `127.0.0.1.${localPort}`;
  let closing = null;
  for (const line of text.split("\n")) {
    const m = /^tcp4\s+\d+\s+\d+\s+(\S+)\s+(\S+)\s+([A-Z_0-9]+)\s+\d+\s+\d+\s+\d+\s+\d+\s+(.*):(\d+)\s+[0-9a-f]{5}\s/.exec(line);
    if (!m || m[1] !== client || m[2] !== server || !OPEN.has(m[3])) continue;
    if (m[3] === "ESTABLISHED") return Number(m[5]);
    closing ??= Number(m[5]);
  }
  return closing;
}

// ---------- Linux and Windows ----------
//
// There XenoNet reads the processes itself rather than through ps and lsof:
// on Linux from /proc, on Windows from the system's process list (the
// binary takes a ToolHelp32 snapshot; here PowerShell asks CIM). They're
// asked for as these calls, which run() answers and a test answers with a
// made-up table.
//
// PROCESSES: every process, one line each, "pid ppid start program exe",
// tab separated, with its start time in ms (UTC), the program it says it is
// (Linux: argv[0]; Windows: its image) and its executable's real path (empty
// where it can't be read). TCP_PEER [local port, remote port]: the process
// holding the client's end of a connection to 127.0.0.1, on Linux (Windows
// asks netstat -ano). COMMAND_LINE [pid]: a process's command line.
export const PROCESSES = "xenonet:processes";
export const TCP_PEER = "xenonet:tcp-peer";
export const COMMAND_LINE = "xenonet:command-line";
const OWN = new Set([PROCESSES, TCP_PEER, COMMAND_LINE]);

// What PROCESSES answers: the table, and each process's executable.
export function parseTable(text) {
  const table = new Map(), exes = new Map();
  for (const line of String(text ?? "").split("\n")) {
    const f = line.replace(/\r$/, "").split("\t");
    if (f.length < 4) continue;
    const [pid, ppid, start] = [Number(f[0]), Number(f[1]), Number(f[2])];
    if (!Number.isInteger(pid) || !f[3]) continue;
    table.set(pid, { pid, ppid: Number.isFinite(ppid) ? ppid : 0, start: Number.isFinite(start) ? start : 0, program: f[3] });
    if (f[4]) exes.set(pid, f[4]);
  }
  return { table, exes };
}

// The process holding the client's end of a connection to 127.0.0.1, as
// netstat -ano -p TCP lists it on Windows: "TCP <local> <foreign> <state>
// <pid>", the state in the system's language, so it isn't read; a pair whose
// pid is 0 has been closed.
export function parseNetstatWindows(text, { localPort, remotePort }) {
  const client = `127.0.0.1:${remotePort}`, server = `127.0.0.1:${localPort}`;
  for (const line of String(text ?? "").split("\n")) {
    const f = line.trim().split(/\s+/);
    const pid = Number(f[f.length - 1]);
    if (f.length >= 4 && f[0].toUpperCase() === "TCP" && f[1] === client && f[2] === server && pid > 0) return pid;
  }
  return null;
}

// The socket inode of the client's end of a connection to 127.0.0.1, as
// /proc/net/tcp lists it on Linux: "sl local rem st … uid timeout inode",
// addresses as hex ADDR:PORT (127.0.0.1 is 0100007F), and an inode of 0 for
// a socket that's closing.
export function parseProcNetTcp(text, { localPort, remotePort }) {
  const hex = (port) => `0100007F:${Number(port).toString(16).toUpperCase().padStart(4, "0")}`;
  const [client, server] = [hex(remotePort), hex(localPort)];
  for (const line of String(text ?? "").split("\n").slice(1)) {
    const f = line.trim().split(/\s+/);
    if (f.length > 9 && f[1].toUpperCase() === client && f[2].toUpperCase() === server && f[9] !== "0") return f[9];
  }
  return null;
}

// A field of a table line: no tabs or line breaks of its own.
const field = (s) => String(s ?? "").replace(/[\t\n\r]/g, " ");

// procs' own calls, from this system.
async function ownCall(file, args) {
  if (process.platform === "linux") {
    const pids = () => fs.readdirSync("/proc").filter((n) => /^\d+$/.test(n));
    const argv = (pid) => {
      try {
        return fs.readFileSync(`/proc/${pid}/cmdline`).toString("utf8").split("\0").filter(Boolean);
      } catch {
        return [];
      }
    };
    if (file === PROCESSES) {
      const boot = Number(/^btime (\d+)/m.exec(fs.readFileSync("/proc/stat", "utf8"))?.[1] ?? 0) * 1000;
      const lines = [];
      for (const pid of pids()) {
        let stat;
        try {
          stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
        } catch {
          continue;
        }
        // The fields come after the name's last ")": linux shows userspace
        // 100 clock ticks a second.
        const name = stat.slice(stat.indexOf("(") + 1, stat.lastIndexOf(")"));
        const rest = stat.slice(stat.lastIndexOf(")") + 1).trim().split(/\s+/);
        let exe = "";
        try {
          exe = fs.readlinkSync(`/proc/${pid}/exe`).replace(/ \(deleted\)$/, "");
        } catch {}
        lines.push(`${pid}\t${rest[1]}\t${boot + Number(rest[19]) * 10}\t${field(argv(pid)[0] ?? name)}\t${field(exe)}`);
      }
      return lines.join("\n");
    }
    if (file === TCP_PEER) {
      const inode = parseProcNetTcp(fs.readFileSync("/proc/net/tcp", "utf8"), { localPort: args[0], remotePort: args[1] });
      if (!inode) return "";
      const target = `socket:[${inode}]`;
      for (const pid of pids()) {
        let fds = [];
        try {
          fds = fs.readdirSync(`/proc/${pid}/fd`);
        } catch {
          continue;
        }
        for (const fd of fds) {
          try {
            if (fs.readlinkSync(`/proc/${pid}/fd/${fd}`) === target) return pid;
          } catch {}
        }
      }
      return "";
    }
    if (file === COMMAND_LINE) return argv(args[0]).join(" ");
  }
  if (process.platform === "win32" && file !== TCP_PEER) {
    // CIM's Win32_Process: what the binary reads through ToolHelp32 and the
    // process itself, here through PowerShell.
    const filter = file === COMMAND_LINE ? ` -Filter "ProcessId=${Number(args[0])}"` : "";
    const script = `Get-CimInstance Win32_Process${filter} | ForEach-Object { [pscustomobject]@{ pid = $_.ProcessId; ppid = $_.ParentProcessId; start = if ($_.CreationDate) { [DateTimeOffset]::new($_.CreationDate).ToUnixTimeMilliseconds() } else { 0 }; name = $_.Name; exe = $_.ExecutablePath; line = $_.CommandLine } } | ConvertTo-Json -Compress`;
    const out = await new Promise((resolve, reject) => execFile("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { maxBuffer: 64 * 1024 * 1024, timeout: 20000, windowsHide: true }, (err, stdout) => (err ? reject(err) : resolve(stdout))));
    const rows = [JSON.parse(out || "[]")].flat();
    if (file === COMMAND_LINE) return String(rows[0]?.line ?? "");
    return rows.map((r) => `${r.pid}\t${r.ppid}\t${r.start}\t${field(r.exe || r.name)}\t${field(r.exe ?? "")}`).join("\n");
  }
  throw new Error(`${file} couldn't be read on this system`);
}

export function createProcs({ platform = process.platform, exec = run, now = () => Date.now() } = {}) {
  // supported: everything procs does (macOS, where codesign says who signed
  // a running program). chains: the process on the other end of a
  // connection and the ones above it, on macOS, Linux and Windows, which
  // what's yours to do rests on alone. native: the table is read as
  // PROCESSES.
  const supported = platform === "darwin";
  const native = platform === "linux" || platform === "win32";
  const chains = supported || native;
  // Where the table is read natively, each process's executable as the last
  // read said, by pid, start time and program.
  const nativeExes = new Map();
  const readNative = async () => {
    const { table: t, exes } = parseTable(await exec(PROCESSES, []));
    if (nativeExes.size > 4096) nativeExes.clear();
    for (const [pid, exe] of exes) {
      const p = t.get(pid);
      if (p) nativeExes.set(`${p.pid}:${p.start}:${p.program}`, exe);
    }
    return t;
  };
  let table = null, tableAt = 0, pending = null;
  const signatures = new Map();
  // What each process of Node, Bun or Hermes's Pythons is, by pid, start
  // time and program, so its arguments are read once: only the verdict is
  // kept, never the arguments.
  const verdicts = new Map();
  // Each process's executable, by pid, start time and program, and the
  // bundle id of each Hermes.app, Grok Bot.app and Muse.app, read and
  // settled, by its folder.
  const executables = new Map();
  const bundleReads = new Map();
  const bundleIds = new Map();

  async function snapshot() {
    if (table && now() - tableAt < TABLE_MS) return table;
    pending ??= (native ? readNative() : exec("ps", ["-axo", "pid=,ppid=,lstart=,comm="]).then(parsePs))
      .then((t) => ((table = t), (tableAt = now()), table))
      .finally(() => (pending = null));
    return pending;
  }

  // The process on the other end of a socket, or null when it can't be
  // told. Only a process found is kept on the socket, so a lookup that
  // failed is tried again on its next request.
  async function peer(socket) {
    if (!chains || !socket?.remotePort) return null;
    if (socket.__xenonetPeer != null) return socket.__xenonetPeer;
    const ports = { localPort: socket.localPort, remotePort: socket.remotePort };
    let pid;
    if (platform === "win32") pid = parseNetstatWindows(await exec("netstat", ["-ano", "-p", "TCP"]).catch(() => ""), ports);
    else if (platform === "linux") {
      const n = Number(String(await exec(TCP_PEER, [String(ports.localPort), String(ports.remotePort)]).catch(() => "")).trim());
      pid = Number.isFinite(n) && n > 0 ? n : null;
    } else pid = parseNetstat(await exec("netstat", ["-anv", "-p", "tcp"]).catch(() => ""), ports);
    if (pid != null) socket.__xenonetPeer = pid;
    return pid;
  }

  // A process and the ones above it, nearest first, up to launchd, each
  // with the agent app it is (agent) or the actor product it belongs to
  // (product, see productOf). A process of Node, Bun or one of Hermes's
  // Pythons is asked for its arguments, to tell what it runs.
  //
  // A process started after the shared table was read isn't in it: every
  // xenonet command is a new process, and anything reading the table a
  // moment earlier would hide it. So a pid the table doesn't have is read
  // by itself, with its parents up to the first one the table has as it is,
  // before it's taken for gone (`again`: false for a process seen before,
  // such as a session's pin).
  //
  // `exe`: each process also gets its executable's real path (null where it
  // can't be read: another user's process, or one that's gone), and what it
  // is goes by that path rather than by argv[0], which ps calls its program
  // and which a process sets itself. An agent host is one only by its real
  // path. The checks that rest on a path ask for it (an actor's token, a
  // registration, the window's routes); the rest can do without the lsof.
  async function chain(pid, { again = true, exe = false } = {}) {
    const t = await snapshot();
    const fresh = again && !t.has(pid) ? await newcomers(pid, t) : null;
    const get = (p) => fresh?.get(p) ?? t.get(p);
    const rows = [];
    // Windows keeps a parent's pid after the parent is gone, and may give it
    // to a newer process: a "parent" that started after its child isn't the
    // one that started it.
    const parentOf = (p) => {
      const q = p.ppid > 1 ? get(p.ppid) : null;
      return q && native && q.start > 0 && p.start > 0 && q.start > p.start ? null : q;
    };
    for (let p = get(pid); p && rows.length < 64; p = parentOf(p)) rows.push(p);
    const real = exe ? await executablesOf(rows) : null;
    const out = [];
    for (const p of rows) {
      const file = real ? (real.get(p.pid) ?? null) : p.program;
      const of = (args) => agentOf(p.program, args) ?? (file ? agentOf(file, args) : null);
      let agent = of("");
      let args = "";
      if (!agent && (wantsArgs(p.program) || (file && wantsArgs(file)))) {
        args = await (native ? exec(COMMAND_LINE, [String(p.pid)]) : exec("ps", ["-o", "command=", "-p", String(p.pid)])).catch(() => "");
        agent = of(args);
      }
      let product = agent ? null : productOf(file ?? p.program, args);
      // argv[0] alone never makes an agent host.
      if (product?.role === "host" && !file) product = null;
      out.push({ ...p, ...(real ? { exe: file } : {}), agent, product: await vouched(product, file ?? p.program) });
    }
    return out;
  }

  // The executables of these processes (rows of the table), by pid: those
  // read before, and the rest in one lsof. Kept by pid, start time and
  // program, so a process that runs another program under a new name is
  // read again.
  async function executablesOf(rows) {
    const key = (p) => `${p.pid}:${p.start}:${p.program}`;
    if (native) return new Map(rows.map((p) => [p.pid, nativeExes.get(key(p)) ?? null]));
    const missing = rows.filter((p) => !executables.has(key(p)));
    if (missing.length) {
      const read = parseTexts(await exec("lsof", ["-a", "-p", missing.map((p) => p.pid).join(","), "-d", "txt", "-Fn"]).catch((err) => err?.stdout ?? ""));
      if (executables.size > 1024) executables.clear();
      // A process lsof said nothing about isn't kept, so it's asked again.
      for (const p of missing) if (read.has(p.pid)) executables.set(key(p), read.get(p.pid));
    }
    return new Map(rows.map((p) => [p.pid, executables.get(key(p)) ?? null]));
  }

  // A vouched app's bundle id: the CFBundleIdentifier of its Info.plist,
  // that key alone, read once (null when it can't be read).
  function bundleId(bundle) {
    if (!bundleReads.has(bundle)) {
      if (bundleReads.size > 256) {
        bundleReads.clear();
        bundleIds.clear();
      }
      bundleReads.set(
        bundle,
        exec("plutil", ["-extract", "CFBundleIdentifier", "raw", "-o", "-", path.posix.join(bundle.replace(/\\/g, "/"), "Contents", "Info.plist")]).then(
          (out) => String(out).trim() || null,
          () => null,
        ),
      );
      bundleReads.get(bundle).then((id) => bundleIds.set(bundle, id));
    }
    return bundleReads.get(bundle);
  }

  // A product a process was taken for, once the vouched app its path lies
  // in (if any) has been asked: kept when that bundle is the
  // product's, and dropped when it's another maker's. One whose id can't be
  // read stays the product's for an app's process, which only ever counts
  // against it, but not for an agent host, which Hermes's token may use.
  // `settled` answers from the ids read so far, for the census.
  const bundleFor = (product, program) => (Object.hasOwn(VOUCHED, product?.product ?? "") ? (VOUCHED[product.product].bundle.exec(program)?.[1] ?? null) : null);
  function vouch(product, program, id) {
    if (!bundleFor(product, program)) return product;
    if (id == null) return product.role === "host" ? null : product;
    return VOUCHED[product.product].id.test(id) ? product : null;
  }
  async function vouched(product, program) {
    const bundle = bundleFor(product, program);
    return bundle ? vouch(product, program, await bundleId(bundle)) : product;
  }
  const settled = (product, program) => vouch(product, program, bundleIds.get(bundleFor(product, program)) ?? null);

  // A process the shared table doesn't have, and its parents until one the
  // table has as it is (the same start and program, so not a reused pid),
  // by pid: each read with a ps of its own, a few milliseconds where the
  // whole table costs a tenth of a second. Empty for a process that's gone.
  async function newcomers(pid, t) {
    const out = new Map();
    // Natively, the whole table is read again: it's quick.
    const fresh = native ? await readNative().catch(() => new Map()) : null;
    for (let p = pid; p > 1 && out.size < 64; ) {
      const row = fresh ? fresh.get(p) : (await describe([p])).get(p);
      const known = t.get(p);
      if (!row || (known && known.start === row.start && known.program === row.program)) break;
      out.set(p, row);
      p = row.ppid;
    }
    return out;
  }

  // The folder a process works in, or null when it can't be told: the
  // asker's folder on a registration's card.
  async function cwd(pid) {
    if (!supported || !Number.isInteger(pid) || pid < 1) return null;
    return parseCwd(await exec("lsof", ["-a", "-p", String(pid), "-d", "cwd", "-Fn"]).catch(() => ""));
  }

  // Whether a process is still the one that was seen (pids are reused).
  async function alive({ pid, start }) {
    const p = (await snapshot()).get(pid);
    return !!p && p.start === start;
  }

  // Every process on this machine with what it is, for the observers of
  // actor products (core/actors): { table, of(p) }, where of(p) is { agent,
  // product } as chain() has them. The arguments of the new processes of
  // Node, Bun and Hermes's Pythons are read in one ps and matched there;
  // the rest are told by their program alone, and a Hermes.app, Grok
  // Bot.app or Muse.app by its bundle id. Empty where XenoNet can't look.
  async function census() {
    if (!supported) return { table: new Map(), of: () => ({ agent: null, product: null }) };
    const t = await snapshot();
    // The vouched apps' bundles programs name, read a few new ones at a
    // time: a program's name is its own word, and could name
    // thousands.
    const bundles = new Set();
    for (const p of t.values()) {
      const bundle = vouchedBundle(p.program);
      if (bundle) bundles.add(bundle);
    }
    await Promise.all([...bundles].filter((b) => !bundleReads.has(b)).slice(0, BUNDLES_AT_ONCE).map(bundleId));
    await Promise.all([...bundles].filter((b) => bundleReads.has(b)).map(bundleId));
    const key = (p) => `${p.pid}:${p.start}:${p.program}`;
    const fresh = [...t.values()].filter((p) => wantsArgs(p.program) && !verdicts.has(key(p)));
    if (fresh.length) {
      const lines = parseCommands(await exec("ps", ["-o", "pid=,command=", "-p", fresh.map((p) => p.pid).join(",")]).catch(() => ""));
      // A process that's gone before ps looked gets no verdict, so it's
      // asked about again if it turns out it wasn't.
      for (const p of fresh) {
        if (!lines.has(p.pid)) continue;
        const agent = agentOf(p.program, lines.get(p.pid));
        verdicts.set(key(p), { agent, product: agent ? null : productOf(p.program, lines.get(p.pid)) });
      }
    }
    // The verdicts of processes that are gone go once there are many.
    if (verdicts.size > Math.max(256, 2 * t.size)) {
      const live = new Set([...t.values()].map(key));
      for (const k of verdicts.keys()) if (!live.has(k)) verdicts.delete(k);
    }
    const of = (p) => {
      if (!p) return { agent: null, product: null };
      const v = verdicts.get(key(p));
      if (v) return { agent: v.agent, product: settled(v.product, p.program) };
      const agent = agentOf(p.program);
      return { agent, product: agent ? null : settled(productOf(p.program), p.program) };
    };
    return { table: t, of };
  }

  // The children of these processes, as pids. pgrep costs a few
  // milliseconds of CPU where ps costs tens, so an observer can look at
  // what an app runs about once a second (interface section 5).
  async function children(pids) {
    const list = [...new Set(pids)].filter((pid) => Number.isInteger(pid) && pid > 1);
    if (!supported || !list.length) return [];
    // pgrep says 1 when it finds none.
    const text = await exec("pgrep", ["-P", list.join(",")]).catch(() => "");
    return text.split("\n").map(Number).filter((pid) => Number.isInteger(pid) && pid > 0);
  }

  // These processes, as the table has them (parent, start time, program),
  // without reading the whole table; the ones that are gone are left out.
  async function describe(pids) {
    const list = [...new Set(pids)].filter((pid) => Number.isInteger(pid) && pid > 0);
    if (!supported || !list.length) return new Map();
    return parsePs(await exec("ps", ["-o", "pid=,ppid=,lstart=,comm=", "-p", list.join(",")]).catch(() => ""));
  }

  // The folders these processes work in, by pid, in one lsof.
  async function cwds(pids) {
    const list = [...new Set(pids)].filter((pid) => Number.isInteger(pid) && pid > 0);
    if (!supported || !list.length) return new Map();
    return parseCwds(await exec("lsof", ["-a", "-p", list.join(","), "-d", "cwd", "-Fpn"]).catch(() => ""));
  }

  // Whether a process from chain() runs its agent app's own code, or its
  // actor app's (an app process of Grok Bot or Muse), signed by the app's
  // maker. codesign asks about the running process, not a file, so another
  // program named claude fails, and so does an agent built from source or
  // run by Node, or a bundle that only calls itself Grok Bot.app.
  async function signed(p) {
    const team = TEAMS[p?.agent] ?? (p?.product?.role === "app" ? PRODUCT_TEAMS[p.product.product] : null);
    if (!supported || !team) return false;
    const key = `${p.pid}:${p.start}:${p.program}:${team}`;
    if (!signatures.has(key)) {
      if (signatures.size > 500) signatures.clear();
      const requirement = `anchor apple generic and certificate leaf[subject.OU] = "${team}"`;
      signatures.set(key, exec("codesign", ["--verify", `-R=${requirement}`, String(p.pid)]).then(() => true, () => false));
    }
    return signatures.get(key);
  }

  return { supported, chains, peer, chain, alive, signed, snapshot, cwd, census, children, describe, cwds };
}

// Whether the lookups work on this machine, for the doctor: a connection
// this process makes to itself should come back as this process, with the
// executable it runs. If netstat or ps ever answer in a way XenoNet doesn't
// read, actors' tokens and your own routes would be refused and agents
// taken at their word (see engine/verify.js), and if lsof doesn't say what
// a process runs, the window's own routes and Hermes's actor would be
// refused; this is how you'd know.
export async function selfTest(procs = createProcs()) {
  if (!procs.supported) return { supported: false };
  const server = net.createServer();
  await new Promise((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve));
  const accepted = new Promise((resolve) => server.once("connection", resolve));
  const client = net.connect(server.address().port, "127.0.0.1");
  try {
    await new Promise((resolve, reject) => client.once("connect", resolve).once("error", reject));
    const socket = await accepted;
    const pid = await procs.peer(socket).catch(() => null);
    const chain = pid ? await procs.chain(pid, { exe: true }).catch(() => []) : [];
    socket.destroy();
    return { supported: true, peer: pid === process.pid, chain: chain[0]?.pid === process.pid, exe: !!chain[0]?.exe };
  } finally {
    client.destroy();
    server.close();
  }
}
