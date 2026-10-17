// core/procs.js in Rust (crates/xenonet-engine/src/procs.rs): which process
// is on the other end of a connection to XenoNet, and the processes above
// it. Rust reads the commands' answers, keeps what's kept (the shared
// table, signatures, executables, bundle ids, the census's verdicts) and
// decides what each process is; the commands run here, through `exec`, and
// the clock is `now`.
import { execFile } from "node:child_process";
import net from "node:net";
import { native } from "../index.js";
import { drive } from "../machine.js";

// What the commands run with: start times in English and UTC, and text as
// UTF-8, whatever XenoNet's own environment says.
function envFor() {
  const env = { ...process.env, LC_TIME: "C", LC_CTYPE: "UTF-8", TZ: "UTC" };
  delete env.LC_ALL;
  return env;
}

// A command's output, or an error that carries what it wrote anyway.
function run(file, args) {
  return new Promise((resolve, reject) => execFile(file, args, { maxBuffer: 16 * 1024 * 1024, timeout: 5000, windowsHide: true, env: envFor() }, (err, stdout) => (err ? reject(Object.assign(err, { stdout })) : resolve(stdout))));
}

// text.split, as the JS called it: a TypeError for what isn't text.
const lines = (text) => (typeof text === "string" ? text : text.split("\n").join("\n"));

export const agentOf = (program, args = "") => native.procsAgentOf(String(program ?? ""), String(args)) ?? null;
export const wantsArgs = (program) => native.procsWantsArgs(String(program ?? ""));
export const productOf = (program, args = "") => native.procsProductOf(String(program ?? ""), String(args));

export const parseTexts = (text) => new Map(native.procsParseTexts(String(text ?? "")));
export const parseCwd = (text) => native.procsParseCwd(String(text ?? "")) ?? null;
export const parseCwds = (text) => new Map(native.procsParseCwds(String(text ?? "")));
export const parseCommands = (text) => new Map(native.procsParseCommands(String(text ?? "")));
export const parsePs = (text) => new Map(native.procsParsePs(lines(text)));
export const parseNetstat = (text, { localPort, remotePort }) => native.procsParseNetstat(lines(text), localPort, remotePort) ?? null;

export function createProcs({ platform = process.platform, exec = run, now = () => Date.now() } = {}) {
  const core = new native.ProcsCore(String(platform));
  const shared = new Map();
  const go = (machine) => drive(machine, (q) => (q.op === "now" ? now() : exec(q.file, q.args)), shared);
  const supported = core.supported;

  async function peer(socket) {
    const { pid, keep } = await go(core.peer(socket?.localPort, socket?.remotePort, socket?.__xenonetPeer));
    if (keep) socket.__xenonetPeer = pid;
    return pid;
  }

  const census = async () => {
    const table = await go(core.census());
    if (table == null) return { table: new Map(), of: () => ({ agent: null, product: null }) };
    return { table: new Map(table), of: (p) => core.of(p) };
  };

  return {
    supported,
    peer,
    chain: (pid, { again = true, exe = false } = {}) => go(core.chain(pid, !!again, !!exe)),
    alive: async ({ pid, start }) => go(core.alive(pid, start)),
    signed: async (p) => go(core.signed(p)),
    snapshot: async () => new Map(await go(core.snapshot())),
    cwd: async (pid) => go(core.cwd(pid)),
    census,
    children: async (pids) => go(core.children([...pids])),
    describe: async (pids) => new Map(await go(core.describe([...pids]))),
    cwds: async (pids) => new Map(await go(core.cwds([...pids]))),
  };
}

// Whether the lookups work on this machine, for the doctor: a connection
// this process makes to itself should come back as this process, with the
// executable it runs. The connection is made here; Rust says what the
// answers mean.
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
    return native.procsSelfTest(process.pid, typeof pid === "number" ? pid : null, chain[0]);
  } finally {
    client.destroy();
    server.close();
  }
}
