// A made-up machine's processes, for tests: a table of processes (pid,
// parent, start time, program and arguments), and spawn and execFile that
// add a process to it and run a script of the test's own in place of a real
// program, with the output, the reading and the exit the script says.
//
//   const machine = fakeProcesses({ clock });
//   machine.program("git", ({ args }) => ({ stdout: "## main\n" }));
//   machine.program("codex", (io) => io.on("line", (l) => io.json({ id: JSON.parse(l).id, result: {} })));
//   const client = new AppServerClient({ spawn: machine.spawn });
//
// A script gets an `io`: its args, env and cwd, "line" events for what's
// written to its stdin (and input(), all of it once stdin ends), write(),
// json() and error() for its stdout and stderr, and exit(code). A script
// that returns { stdout, stderr, code, delayMs } has that written and exits
// with it; one that returns nothing keeps running until it exits or is
// killed. A script can also be that object alone.
//
// exec(file, args) answers the way core/procs.js asks the system (ps,
// netstat, codesign, lsof, plutil and pgrep), from the table, so
// createProcs({ exec }) sees this machine instead of the real one; `asked`
// lists what it was asked. A process's executable is its program unless
// the test says otherwise (`exe`, null for one lsof can't read), and an
// app bundle has the id the test gives it (bundle()), or none XenoNet can
// read.
import { EventEmitter } from "node:events";
import path from "node:path";
import { PassThrough } from "node:stream";

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n, w = 2, c = "0") => String(n).padStart(w, c);
// ps's lstart, "Sat Oct  3 16:34:29 2026", in UTC: core/procs.js runs ps
// with TZ=UTC.
const lstart = (ms) => {
  const d = new Date(ms);
  return `${DAYS[d.getUTCDay()]} ${MONTHS[d.getUTCMonth()]} ${pad(d.getUTCDate(), 2, " ")} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())} ${d.getUTCFullYear()}`;
};

export function fakeProcesses({ clock = null, self = {} } = {}) {
  const now = () => clock?.now() ?? Date.now();
  const timers = clock ?? globalThis;
  // ps reports start times to the second.
  const second = (ms) => Math.floor(ms / 1000) * 1000;
  const table = new Map();
  const programs = new Map();
  const connections = []; // { pid, name, clientPort, serverPort }
  const signed = new Map(); // pid → team
  const bundles = new Map(); // an app bundle's folder → its id
  const spawned = [];
  // What exec() was asked, as [file, ...args], for tests that count it.
  const asked = [];
  let nextPid = 5000;

  const me = { pid: 4000, ppid: 1, program: "/usr/local/bin/node", args: ["server/index.js"], ...self };
  me.start = second(me.start ?? now());
  table.set(1, { pid: 1, ppid: 0, start: second(now() - 86400e3), program: "/sbin/launchd", args: [] });
  table.set(me.pid, me);

  // A process of the test's making, for the table only, with the folder it
  // works in if the test says.
  function add({ pid = nextPid++, ppid = me.pid, start = now(), program, args = [], cwd = null, exe }) {
    table.set(pid, { pid, ppid, start: second(start), program, args, cwd, ...(exe !== undefined ? { exe } : {}) });
    return pid;
  }

  // What running `name` (a path, or a program's base name) does.
  const program = (name, script) => programs.set(name, script);
  const scriptOf = (file) => programs.get(file) ?? programs.get(path.basename(file));

  function spawn(file, args = [], options = {}) {
    const child = new EventEmitter();
    child.stdin = new PassThrough();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.exitCode = null;
    child.signalCode = null;
    child.killed = false;
    child.spawnfile = file;
    child.spawnargs = [file, ...args];
    const script = scriptOf(file);
    if (!script) {
      // As Node does for a program that isn't there: no pid, and an error a
      // moment later.
      child.pid = undefined;
      child.kill = () => false;
      spawned.push({ file, args, pid: null, options });
      setImmediate(() => child.emit("error", Object.assign(new Error(`spawn ${file} ENOENT`), { code: "ENOENT", errno: -2, syscall: `spawn ${file}`, path: file, spawnargs: args })));
      return child;
    }
    const pid = add({ program: file, args });
    child.pid = pid;
    spawned.push({ file, args, pid, options });

    const io = new EventEmitter();
    let ended = false;
    let input = "";
    let inputEnded = false;
    const exit = (code = 0, signal = null) => {
      if (ended) return;
      ended = true;
      table.delete(pid);
      child.exitCode = signal ? null : code;
      child.signalCode = signal;
      child.stdout.end();
      child.stderr.end();
      setImmediate(() => {
        child.emit("exit", child.exitCode, signal);
        child.emit("close", child.exitCode, signal);
      });
    };
    Object.assign(io, {
      pid,
      args,
      env: options.env ?? process.env,
      cwd: options.cwd ?? null,
      write: (text) => void (!ended && child.stdout.write(text)),
      json: (obj) => void (!ended && child.stdout.write(JSON.stringify(obj) + "\n")),
      error: (text) => void (!ended && child.stderr.write(text)),
      exit,
      // All of stdin, once it ends.
      input: () => (inputEnded ? Promise.resolve(input) : new Promise((resolve) => io.once("end", () => resolve(input)))),
      get ended() {
        return ended;
      },
    });
    child.kill = (signal = "SIGTERM") => {
      if (ended) return false;
      child.killed = true;
      io.emit("kill", signal);
      exit(null, signal);
      return true;
    };
    // A detached child the caller lets go of (core/control.js's launch).
    child.unref = child.ref = () => child;

    // What's written to its stdin, a line at a time.
    let buf = "";
    child.stdin.setEncoding("utf8");
    child.stdin.on("data", (chunk) => {
      input += chunk;
      buf += chunk;
      for (let nl = buf.indexOf("\n"); nl >= 0; nl = buf.indexOf("\n")) {
        const line = buf.slice(0, nl);
        buf = buf.slice(nl + 1);
        io.emit("line", line);
      }
    });
    child.stdin.on("end", () => {
      inputEnded = true;
      io.emit("end");
    });

    // The script starts now, so it hears every line written to it; what it
    // writes waits in the pipes until the caller reads, and its exit comes a
    // moment later, once the caller has its listeners on.
    (async () => {
      try {
        const answer = typeof script === "function" ? await script(io) : script;
        if (!answer || ended) return;
        if (answer.delayMs) await new Promise((r) => timers.setTimeout(r, answer.delayMs));
        if (answer.stdout) io.write(answer.stdout);
        if (answer.stderr) io.error(answer.stderr);
        exit(answer.code ?? 0, answer.signal ?? null);
      } catch (err) {
        io.error(String(err?.stack ?? err));
        exit(70);
      }
    })();
    return child;
  }

  // child_process.execFile(file, args, options, callback), on spawn: the
  // callback gets (err, stdout, stderr), with err.code the exit code, and
  // err.killed when the timeout ended it.
  function execFile(file, args = [], options = {}, callback = () => {}) {
    if (typeof options === "function") [options, callback] = [{}, options];
    const child = spawn(file, args, options);
    let stdout = "",
      stderr = "",
      done = false,
      timer = null;
    child.stdout.setEncoding("utf8").on("data", (d) => (stdout += d));
    child.stderr.setEncoding("utf8").on("data", (d) => (stderr += d));
    child.stdin.end();
    if (options.timeout) timer = timers.setTimeout(() => child.kill("SIGTERM"), options.timeout);
    const finish = (err) => {
      if (done) return;
      done = true;
      timers.clearTimeout(timer);
      callback(err, stdout, stderr);
    };
    child.on("error", finish);
    child.on("close", (code, signal) =>
      finish(code === 0 && !signal ? null : Object.assign(new Error(`Command failed: ${[file, ...args].join(" ")}\n${stderr}`), { code, signal, killed: child.killed, cmd: [file, ...args].join(" ") })),
    );
    return child;
  }

  // The listed pids of a -p argument that are in the table; ps, pgrep and
  // lsof say 1 when none are.
  const listed = (arg) => {
    const found = String(arg).split(",").map(Number).filter((pid) => table.has(pid));
    if (!found.length) throw Object.assign(new Error("no such process"), { code: 1 });
    return found;
  };

  // The system as core/procs.js asks it: ps, netstat, codesign, lsof and
  // pgrep from the table; anything else runs as a program.
  async function exec(file, args = []) {
    asked.push([file, ...args]);
    if (file === "ps" && args[0] === "-axo") return ps();
    // ps -o pid=,command= -p <pids>: each one's command line.
    if (file === "ps" && args[0] === "-o" && args[1] === "pid=,command=") return listed(args.at(-1)).map((pid) => `${pad(pid, 5, " ")} ${[table.get(pid).program, ...table.get(pid).args].join(" ")}`).join("\n") + "\n";
    // ps -o pid=,ppid=,lstart=,comm= -p <pids>: those rows of the table.
    if (file === "ps" && args[0] === "-o" && args[1] === "pid=,ppid=,lstart=,comm=") return listed(args.at(-1)).map((pid) => row(table.get(pid))).join("\n") + "\n";
    if (file === "ps" && args[0] === "-o") {
      const p = table.get(Number(args.at(-1)));
      if (!p) throw Object.assign(new Error("no such process"), { code: 1 });
      return [p.program, ...p.args].join(" ");
    }
    // pgrep -P <pids>: the children of any of them.
    if (file === "pgrep" && args[0] === "-P") {
      const parents = new Set(String(args[1]).split(",").map(Number));
      const kids = [...table.values()].filter((p) => parents.has(p.ppid)).map((p) => p.pid);
      if (!kids.length) throw Object.assign(new Error("none"), { code: 1 });
      return kids.join("\n") + "\n";
    }
    if (file === "netstat") return connections.map((c) => `tcp4       0      0  127.0.0.1.${c.clientPort}        127.0.0.1.${c.serverPort}        ESTABLISHED    111   2970  406912  146988 ${c.name}:${c.pid}            00102 00000104 0000000003bca9cb 00000081 01000800      2      0 000000`).join("\n");
    if (file === "codesign") {
      const pid = Number(args.at(-1));
      const team = /subject\.OU\] = "([^"]+)"/.exec(args.find((a) => a.startsWith("-R=")) ?? "")?.[1];
      if (table.has(pid) && signed.get(pid) === team) return "";
      throw Object.assign(new Error("code object is not signed at all"), { code: 1 });
    }
    // lsof -a -p <pids> -d cwd -Fn (or -Fpn): the folders they work in.
    // lsof -a -p <pids> -d txt -Fn: the executables they run, then a library.
    if (file === "lsof" && args.includes("txt")) {
      return listed(args[args.indexOf("-p") + 1])
        .map((pid) => {
          const exe = table.get(pid).exe === undefined ? table.get(pid).program : table.get(pid).exe;
          return exe ? `p${pid}\nftxt\nn${exe}\nftxt\nn/usr/lib/dyld\n` : "";
        })
        .join("");
    }
    if (file === "lsof") return listed(args[args.indexOf("-p") + 1]).map((pid) => (table.get(pid).cwd ? `p${pid}\nfcwd\nn${table.get(pid).cwd}\n` : `p${pid}\n`)).join("");
    // plutil -extract CFBundleIdentifier raw -o - <bundle>/Contents/Info.plist
    if (file === "plutil") {
      const bundle = String(args.at(-1)).replace(/\/Contents\/Info\.plist$/, "");
      if (args[1] === "CFBundleIdentifier" && bundles.has(bundle)) return `${bundles.get(bundle)}\n`;
      throw Object.assign(new Error(`${args.at(-1)}: file does not exist or is not readable`), { code: 1 });
    }
    return new Promise((resolve, reject) => execFile(file, args, {}, (err, stdout) => (err ? reject(err) : resolve(stdout))));
  }

  // `ps -axo pid=,ppid=,lstart=,comm=` for the table, a row per process.
  const row = (p) => `${pad(p.pid, 5, " ")} ${pad(p.ppid, 5, " ")} ${lstart(p.start)} ${p.program}`;
  const ps = () => [...table.values()].map(row).join("\n") + "\n";

  return {
    self: me,
    table,
    add,
    remove: (pid) => table.delete(pid),
    program,
    spawn,
    execFile,
    exec,
    ps,
    asked,
    spawned,
    // A loopback connection from `pid` to the server's port, as netstat
    // lists it.
    connect: ({ pid, name = path.basename(table.get(pid)?.program ?? "node"), clientPort, serverPort }) => connections.push({ pid, name, clientPort, serverPort }),
    // What codesign says of a running process: signed by `team`.
    sign: (pid, team) => signed.set(pid, team),
    // An app bundle's id, as its Info.plist has it.
    bundle: (folder, id) => bundles.set(folder, id),
    // The processes spawned and still running.
    running: () => spawned.filter((s) => s.pid && table.has(s.pid)).map((s) => table.get(s.pid)),
  };
}
