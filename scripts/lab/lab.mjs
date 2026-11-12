#!/usr/bin/env node
// The XenoNet lab: Linux VMs on this Mac, in Lima, to test XenoNet against
// several machines. scripts/lab/README.md says how to use it. Commands
// take the lab's VMs by host id (vm1, vm2), or all of them when none is
// named.
//
//   npm run lab -- status
//   npm run lab -- build ubuntu|fedora [--ref <git ref>]
//   npm run lab -- clone vm1|vm2
//   npm run lab -- up [vm…]
//   npm run lab -- down [vm…]
//   npm run lab -- push [vm…] [--ref <git ref>]
//   npm run lab -- check [vm…] [--only <check,…>]
//   npm run lab -- teardown [--yes]
//
// It never signs in to anything and never reads a sign-in: the owner signs
// in inside each VM (README.md, "Signing in"). It never joins a machine to
// a network either: that's the owner's too (README.md, "The transport
// checks").
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import dgram from "node:dgram";
import { randomUUID } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { BASES, VMS, LAB_INSTANCES, DEMO_PORT, REAL_PORT, LAN_DEV, labPaths, envFileText, guestScript, newPassword, sha512crypt, shq } from "./config.mjs";
import { reposScript } from "./fixtures.mjs";
import { CHECKS, HEAL, UDP_PORT, addrsOf, nonce, runCheck } from "./transport.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "../..");
const VERSIONS = JSON.parse(fs.readFileSync(path.join(HERE, "versions.json"), "utf8"));
const GUEST = fs.readFileSync(path.join(HERE, "guest.sh"), "utf8");
const paths = labPaths();
const home = os.homedir();

class LabError extends Error {}
const fail = (message) => {
  throw new LabError(message);
};
const say = (message) => console.log(`  ${message}`);
const step = (message) => console.log(`\n${message}`);

// ---------- running things ----------

function run(cmd, args, { input, capture = false, env, cwd, allowFail = false, timeout } = {}) {
  const r = spawnSync(cmd, args, {
    input,
    cwd,
    env: env ?? process.env,
    encoding: "utf8",
    timeout,
    maxBuffer: 64 * 1024 * 1024,
    stdio: [input == null ? "ignore" : "pipe", capture ? "pipe" : "inherit", capture ? "pipe" : "inherit"],
  });
  if (r.error && r.error.code !== "ETIMEDOUT") fail(`${cmd}: ${r.error.message}`);
  if (r.status !== 0 && !allowFail) {
    const why = (r.stderr ?? "").trim().split("\n").pop();
    fail(`${cmd} ${args.find((a) => !a.startsWith("-")) ?? ""} failed${why ? `: ${why}` : ""}`);
  }
  return r;
}

const limactl = (args, opts) => run("limactl", args, opts);

// The lab's instances, as Lima lists them, by name.
function instances() {
  const r = run("limactl", ["list", "--json"], { capture: true, allowFail: true });
  const out = {};
  for (const line of (r.stdout ?? "").split("\n")) {
    try {
      const inst = JSON.parse(line);
      if (LAB_INSTANCES.includes(inst.name)) out[inst.name] = inst;
    } catch {}
  }
  return out;
}

function checkLima() {
  const v = run("limactl", ["--version"], { capture: true }).stdout.match(/(\d+)\.(\d+)\.(\d+)/);
  if (!v || Number(v[1]) < 2 || (Number(v[1]) === 2 && Number(v[2]) < 1)) fail("the lab needs Lima 2.1 or later (brew install lima)");
}

// A command in a VM over Lima's own SSH channel: Lima's key, and its ssh
// config for 127.0.0.1:<port>. -T, so a script on stdin isn't typed into a
// terminal.
function limaSsh(instance, command, opts = {}) {
  const config = path.join(paths.instanceDir(instance), "ssh.config");
  return run("ssh", ["-F", config, "-T", "-o", "LogLevel=ERROR", `lima-${instance}`, "--", ...command], opts);
}

// One of guest.sh's steps, as the lab user.
const guest = (instance, name, vars = {}, opts = {}) => limaSsh(instance, ["bash", "-s"], { ...opts, input: guestScript(name, vars, GUEST) });

// A guest step left running while the lab does something else: `waitFor`
// resolves once its output matches, `done` when it exits.
function later(instance, name, vars = {}) {
  const config = path.join(paths.instanceDir(instance), "ssh.config");
  const child = spawn("ssh", ["-F", config, "-T", "-o", "LogLevel=ERROR", `lima-${instance}`, "--", "bash", "-s"], { stdio: ["pipe", "pipe", "ignore"] });
  child.stdin.end(guestScript(name, vars, GUEST));
  let stdout = "";
  const seen = [];
  child.stdout.on("data", (d) => {
    stdout += d;
    for (const w of seen) if (w.re.test(stdout)) w.resolve();
  });
  const done = new Promise((resolve) => child.once("close", (status) => resolve({ status, stdout })));
  const waitFor = (re, ms = 30000) =>
    new Promise((resolve, reject) => {
      if (re.test(stdout)) return resolve();
      const timer = setTimeout(() => reject(new LabError(`${name} in ${instance} didn't get ready`)), ms);
      seen.push({ re, resolve: () => (clearTimeout(timer), resolve()) });
      done.then(() => (clearTimeout(timer), re.test(stdout) ? resolve() : reject(new LabError(`${name} in ${instance} ended before it was ready`))));
    });
  return { waitFor, done };
}

// XenoNet's code reaches a VM by git push over Lima's channel, so a VM needs
// no GitHub access. The VM's checkout takes it on its own branch, and its
// files follow (receive.denyCurrentBranch updateInstead).
function pushXenoNet(instance, ref) {
  const sha = run("git", ["rev-parse", "--verify", `${ref}^{commit}`], { capture: true, cwd: REPO }).stdout.trim();
  const config = path.join(paths.instanceDir(instance), "ssh.config");
  run("git", ["push", "--quiet", "--force", "--no-verify", `lima-${instance}:xenonet`, `${sha}:refs/heads/lab`], {
    cwd: REPO,
    env: { ...process.env, GIT_SSH_COMMAND: `ssh -F ${shq(config)} -o LogLevel=ERROR` },
  });
  say(`pushed ${ref} (${sha.slice(0, 7)})`);
}

// ---------- the lab's files ----------

const read = (file) => {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return "";
  }
};

function write(file, text, mode = 0o600) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  fs.writeFileSync(`${file}.tmp`, text, { mode });
  fs.renameSync(`${file}.tmp`, file);
  fs.chmodSync(file, mode);
}

// A new random password for a VM's sudo. The VM gets only its hash.
function newSudo() {
  const password = newPassword();
  return { password, hash: sha512crypt(password) };
}

const savePassword = (instance, password) => write(paths.password(instance), `${password}\n`);

// A shell command in a VM, as the lab user, and one as root: sudo reads the
// VM's password from standard input, with no terminal to echo it.
const vmSh = (instance, cmd, input) => limaSsh(instance, ["bash", "-c", shq(cmd)], { capture: true, allowFail: true, input });
const asRoot = (instance, script) => vmSh(instance, `sudo -S -p '' bash -c ${shq(script)}`, `${read(paths.password(instance)).trim()}\n`);

// ---------- which VMs ----------

// The VMs named, which have to exist, or every VM that does.
function pick(ids) {
  for (const id of ids) if (!VMS[id]) fail(`"${id}" isn't one of the lab's VMs (${Object.keys(VMS).join(", ")})`);
  const all = instances();
  for (const id of ids) if (!all[VMS[id].instance]) fail(`${id} isn't cloned yet: npm run lab -- clone ${id}`);
  return ids.length ? ids : Object.keys(VMS).filter((id) => all[VMS[id].instance]);
}

function running(id) {
  const inst = instances()[VMS[id].instance];
  if (inst?.status !== "Running") fail(`${id} isn't running: npm run lab -- up ${id}`);
  return VMS[id].instance;
}

function facts(instance) {
  const r = guest(instance, "facts", {}, { capture: true, allowFail: true });
  return Object.fromEntries((r.stdout ?? "").split("\n").filter((l) => l.includes("=")).map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]));
}

function report(id) {
  const f = facts(VMS[id].instance);
  const ok = (h) => (/"ok":true/.test(h ?? "") ? "answers" : "doesn't answer");
  say(`XenoNet ${f.commit ?? "?"}`);
  say(`127.0.0.1:${REAL_PORT} (xenonet.service, ${f.xenonet || "?"}): ${ok(f["health-47840"])}`);
  say(`127.0.0.1:${DEMO_PORT} (xenonet-demo.service, ${f["xenonet-demo"] || "?"}): ${ok(f["health-47841"])}`);
  say(`${f.versions ?? ""}; time zone ${f.zone ?? "?"}`);
  say(`on the lab's LAN (${LAN_DEV}): ${f.lan || "no address"}`);
}

// ---------- commands ----------

function status() {
  const all = instances();
  const row = (name, role) => {
    const i = all[name];
    const state = i ? `${i.status}${i.protected ? ", protected" : ""}` : "not built";
    console.log(`  ${name.padEnd(18)} ${state.padEnd(20)} ${role}`);
  };
  console.log("The XenoNet lab");
  for (const b of Object.values(BASES)) row(b.instance, `golden copy, ${b.os}`);
  for (const [id, vm] of Object.entries(VMS)) row(vm.instance, `${id}: ${BASES[vm.base].os}, ssh on 127.0.0.1:${vm.sshPort}, ${vm.timezone}`);
  for (const [id, vm] of Object.entries(VMS)) {
    if (all[vm.instance]?.status !== "Running") continue;
    step(id);
    report(id);
  }
  step("On this Mac");
  const df = run("df", ["-k", home], { capture: true, allowFail: true }).stdout?.trim().split("\n").pop()?.split(/\s+/);
  if (df?.[3]) say(`free disk: ${(Number(df[3]) / 1024 / 1024).toFixed(1)} GiB`);
}

// A golden copy that stopped halfway (a download failed, say) picks up
// where it was: every step can run again.
function build([which], flags) {
  const b = BASES[which] ?? fail(`build takes ${Object.keys(BASES).join(" or ")}`);
  checkLima();
  const existing = instances()[b.instance];
  if (existing?.protected) fail(`${b.instance} is built. To build it again: limactl unprotect ${b.instance}, limactl delete ${b.instance}, then this.`);
  const ref = flags.ref ?? "HEAD";
  if (!existing) {
    const sudo = newSudo();
    step(`Creating ${b.instance} from scripts/lab/lima/${b.template}`);
    limactl(["create", "--tty=false", `--name=${b.instance}`, `--param=labSudoHash=${sudo.hash}`, path.join(HERE, "lima", b.template)]);
    savePassword(b.instance, sudo.password);
  }
  if (existing?.status !== "Running") {
    step(existing ? "Starting it again." : "Starting it. The first boot installs git and the firewall.");
    limactl(["start", "--tty=false", b.instance]);
  }
  step(`Installing Node ${VERSIONS.node.version}, Codex ${VERSIONS.codex} and Claude Code ${VERSIONS.claude}`);
  guest(b.instance, "tools", { NODE_VERSION: VERSIONS.node.version, NODE_SHA256: VERSIONS.node.sha256["linux-arm64"], CLAUDE_VERSION: VERSIONS.claude, CODEX_VERSION: VERSIONS.codex });
  step("XenoNet");
  pushXenoNet(b.instance, ref);
  guest(b.instance, "deploy");
  guest(b.instance, "smoke");
  guest(b.instance, "seal");
  step("Stopping and protecting it. It's never signed in to; working VMs are clones of it.");
  limactl(["stop", "--tty=false", b.instance]);
  limactl(["protect", b.instance]);
}

function clone([id]) {
  const vm = VMS[id] ?? fail(`clone takes ${Object.keys(VMS).join(" or ")}`);
  const b = BASES[vm.base];
  const all = instances();
  if (!all[b.instance]) fail(`build its golden copy first: npm run lab -- build ${vm.base}`);
  if (all[b.instance].status === "Running") fail(`${b.instance} is running; stop it first (limactl stop ${b.instance})`);
  if (all[vm.instance]) fail(`${vm.instance} already exists`);
  const sudo = newSudo();
  step(`Cloning ${vm.instance} from ${b.instance}`);
  limactl(["clone", "--tty=false", b.instance, vm.instance, `--ssh-port=${vm.sshPort}`, `--param=labSudoHash=${sudo.hash}`, `--param=labTimezone=${vm.timezone}`]);
  savePassword(vm.instance, sudo.password);
  step(`Starting it. Its first boot makes its own SSH host keys.`);
  limactl(["start", "--tty=false", vm.instance]);
  step(`${id}: made-up repos`);
  limaSsh(vm.instance, ["bash", "-s"], { input: reposScript(id) });
  step(`${id}: its settings and its two XenoNets`);
  guest(vm.instance, "vm", { XENONET_ENV: envFileText(vm.env) });
  report(id);
}

function up(ids) {
  const all = instances();
  for (const id of pick(ids)) {
    step(id);
    if (all[VMS[id].instance].status !== "Running") limactl(["start", "--tty=false", VMS[id].instance]);
    report(id);
  }
}

function down(ids) {
  const all = instances();
  for (const id of pick(ids)) {
    if (all[VMS[id].instance].status === "Running") limactl(["stop", "--tty=false", VMS[id].instance]);
    say(`${id}: stopped`);
  }
}

function push(ids, flags) {
  const ref = flags.ref ?? "HEAD";
  for (const id of pick(ids)) {
    const instance = running(id);
    step(`${id}: XenoNet ${ref}`);
    pushXenoNet(instance, ref);
    guest(instance, "deploy");
    report(id);
  }
}

// A server on the Mac's 127.0.0.1, in a process of its own, for a guest to
// fail to reach.
function canary() {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["-e", `require("http").createServer((q, s) => s.end("xenonet-lab-canary")).listen(0, "127.0.0.1", function () { console.log(this.address().port); })`], { stdio: ["ignore", "pipe", "inherit"] });
    child.stdout.once("data", (d) => resolve({ port: Number(String(d).trim()), stop: () => child.kill() }));
    child.once("error", reject);
  });
}

// The Mac's own names, as whole words, to look for in what a guest holds.
// They're read here, searched for here, and never printed or sent to a
// guest.
function macNames() {
  const names = [os.userInfo().username];
  const full = run("id", ["-F"], { capture: true, allowFail: true }).stdout?.trim();
  if (full) names.push(full, ...full.split(/\s+/).filter((w) => w.length > 2));
  return [...new Set(names)].filter(Boolean).map((n) => new RegExp(`(^|[^\\p{L}\\p{N}])${n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}($|[^\\p{L}\\p{N}])`, "iu"));
}

// Each VM's own checks: its XenoNets, the loopback block, mounts, names
// and root.
function vmChecks(id, instance, target, mine, record) {
  const sh = (cmd, input) => vmSh(instance, cmd, input);
  const f = facts(instance);
  record(/"ok":true/.test(f["health-47840"] ?? ""), `XenoNet answers on 127.0.0.1:${REAL_PORT}`);
  record(/"ok":true/.test(f["health-47841"] ?? ""), `the demo XenoNet answers on 127.0.0.1:${DEMO_PORT}`);
  const local = run("curl", ["-s", "-m", "3", `http://127.0.0.1:${target.port}/`], { capture: true, allowFail: true }).stdout;
  const probe = sh(`curl -s -m 3 http://192.168.5.2:${target.port}/; echo " exit=$?"`).stdout.trim();
  record(local === "xenonet-lab-canary" && !probe.includes("xenonet-lab-canary"), "can't reach this Mac's 127.0.0.1 through 192.168.5.2", `a server there answers here; from ${id}: curl ${probe.replace(/.*exit=/, "exit ")}`);
  const net = sh("curl -s -o /dev/null -m 10 -w '%{http_code}' https://api.anthropic.com").stdout.trim();
  record(/^[1-5]\d\d$/.test(net), "reaches the internet", `api.anthropic.com: HTTP ${net}`);
  record(Boolean(f.lan), `has an address on the lab's LAN (${LAN_DEV}, Lima's vzNAT)`, f.lan || "none: README.md, The VMs, says how to add it");
  const listening = sh("ss -ltnH").stdout.split("\n").map((l) => l.trim().split(/\s+/)[3]).filter((a) => /:(47840|47841)$/.test(a ?? ""));
  record(listening.length >= 2 && listening.every((a) => a.startsWith("127.0.0.1:")), "XenoNet's API listens on 127.0.0.1 only", listening.join(", "));
  // What Lima or a share would mount: virtiofs, 9p, sshfs (fuse.sshfs),
  // NFS or SMB. fusectl, the kernel's own, isn't one.
  const mounts = sh("findmnt -rn -o FSTYPE,TARGET").stdout.split("\n").filter((l) => /^(virtiofs|9p|fuse\.\S+|nfs4?|cifs|smb3) /.test(l));
  record(mounts.length === 0, "nothing is mounted from this Mac", mounts.join(", "));
  const held = sh("getent passwd will; cat /etc/passwd /etc/group ~/.gitconfig ~/.ssh/authorized_keys; hostname; git config --global user.name").stdout;
  const gecos = held.split("\n")[0]?.split(":")[4];
  record(gecos === "Will" && !mine.some((n) => n.test(held)), "only made-up names", `the lab user is "${gecos}", git's is "${held.trim().split("\n").pop()}"`);
  const nopass = sh("sudo -n true 2>&1; echo exit=$?").stdout;
  const withPassword = sh("sudo -S -p '' true; echo exit=$?", `${read(paths.password(instance)).trim()}\n`).stdout;
  record(!/exit=0/.test(nopass) && /exit=0/.test(withPassword), "root needs the VM's password, which only this Mac has");
}

// What the transport checks (transport.mjs) do in the VMs.
function transportCtx(both) {
  const inst = (id) => VMS[id].instance;
  const json = (s) => {
    try {
      return JSON.parse(s);
    } catch {
      return null;
    }
  };
  const lines = (s) => String(s ?? "").split("\n").map(json).filter(Boolean);
  const g = (id, name, vars = {}, opts = {}) => guest(inst(id), name, vars, { capture: true, allowFail: true, ...opts });
  const vars = ({ session, project, calls = [], register = true, end = false }) => ({ SESSION: session, PROJECT: project, CALLS: JSON.stringify(calls), REGISTER: register ? "1" : "0", END: end ? "1" : "0" });
  const lan = (id) => addrsOf(g(id, "lanaddrs").stdout ?? "");
  const answering = (id) => g(id, "answering", {}, { timeout: 90000 });
  const subjects = {};
  return {
    both,
    lanDev: LAN_DEV,
    sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
    session: () => randomUUID(),
    // The commits a VM's XenoNet was built from, or ("here") this checkout's.
    subjects: (id) => (subjects[id] ??= String((id === "here" ? run("git", ["log", "--format=%s", "-n", "5000"], { cwd: REPO, capture: true, allowFail: true }) : g(id, "subjects")).stdout ?? "").split("\n").filter(Boolean)),
    state: (id) => json(g(id, "netstate").stdout),
    standin: (id, o) => lines(g(id, "standin", vars(o), { timeout: 200000 }).stdout),
    standinLater: (id, o) => {
      const p = later(inst(id), "standin", vars(o));
      return { ready: p.waitFor(/"ready":true/), done: p.done.then((r) => lines(r.stdout)) };
    },
    lanAddrs: lan,
    nft: (id, text) => asRoot(inst(id), text ? `printf %s ${shq(Buffer.from(text).toString("base64"))} | base64 -d | nft -f -` : HEAL),
    // The Mac's clock plus the offset, with NTP off; null puts the Mac's
    // time back and NTP on.
    clock: (id, offset) => asRoot(inst(id), offset == null ? `date -s @${Math.round(Date.now() / 1000)} >/dev/null; timedatectl set-ntp true` : `timedatectl set-ntp false && date -s @${Math.round((Date.now() + offset) / 1000)} >/dev/null`),
    down: (id) => limactl(["stop", "--tty=false", inst(id)], { capture: true }),
    up: (id) => {
      if (instances()[inst(id)]?.status !== "Running") limactl(["start", "--tty=false", inst(id)], { capture: true });
      answering(id);
    },
    service: (id, verb) => {
      vmSh(inst(id), `systemctl --user ${verb === "start" ? "start" : "stop"} xenonet.service`);
      if (verb === "start") answering(id);
    },
    // XenoNet's state dir, copied aside, put back from the copy, or the
    // copy dropped.
    snapshot: (id, what) => {
      const dir = "$HOME/.local/state/xenonet";
      const cmd = { save: `rm -rf ${dir}.lab-copy && cp -a ${dir} ${dir}.lab-copy`, restore: `test -d ${dir}.lab-copy && rm -rf ${dir} && cp -a ${dir}.lab-copy ${dir}`, drop: `rm -rf ${dir}.lab-copy` }[what];
      if (vmSh(inst(id), cmd).status !== 0) fail(`${id}: couldn't ${what} XenoNet's state dir`);
    },
    // One of the lab's relays (guest.sh's relay step), started in a VM: its URL.
    relay: (id, name) => {
      const r = g(id, "relay", { RELAY_VERSION: VERSIONS["iroh-relay"], RELAY: name }, { timeout: 30 * 60000 });
      const url = (r.stdout ?? "").match(/^relay=(\S+)$/m)?.[1];
      if (!url) fail(`${id}: iroh-relay ${name} didn't start: ${(r.stderr ?? "").trim().split("\n").pop()}`);
      return url;
    },
    relayOff: (id, name) => g(id, "relayoff", { RELAY: name ?? "" }),
    relayMetrics: (id, name) => g(id, "relaymetrics", { RELAY: name }).stdout ?? "",
    // The lab's CA, from vm1 where the relays run, in each VM's relay-ca.pem;
    // a VM whose XenoNet started again for it answers before this returns.
    trustCa: (ids) => {
      const ca = (g("vm1", "relayca").stdout ?? "").match(/^ca=(\S+)$/m)?.[1];
      if (!ca) fail("vm1: the lab's relay CA isn't there");
      for (const id of ids) if (/^changed$/m.test(g(id, "trustca", { CA_PEM: ca }).stdout ?? "")) answering(id);
    },
    ownRelay: (id, url, add) => g(id, "ownrelay", { URL: url, ADD: add ? "1" : "0" }),
    // A packet capture of the lab's LAN in a VM, as root: "start" says
    // whether it runs (false without tcpdump); "stop" ends it and says how
    // many bytes it caught and whether `text` was among them, then deletes it.
    capture: (id, verb, text = "") => {
      const file = "/tmp/xenonet-lab-check.pcap";
      if (verb === "start") return asRoot(inst(id), `command -v tcpdump >/dev/null || exit 3; rm -f ${file}; setsid nohup tcpdump -i ${LAN_DEV} -U -s 0 -w ${file} >/dev/null 2>&1 < /dev/null & sleep 1; pgrep -f 'tcpdump -i ${LAN_DEV}' >/dev/null`).status === 0;
      const out = asRoot(inst(id), `pkill -f 'tcpdump -i ${LAN_DEV}' || true; sleep 1; echo "bytes=$(stat -c %s ${file} 2>/dev/null || echo 0)"; echo "found=$(grep -a -F -c -- ${shq(text || "-")} ${file} 2>/dev/null || echo 0)"; rm -f ${file}`).stdout ?? "";
      return { bytes: Number(out.match(/^bytes=(\d+)$/m)?.[1] ?? 0), found: text !== "" && Number(out.match(/^found=(\d+)$/m)?.[1] ?? 0) > 0 };
    },
    doctor: (id) => vmSh(inst(id), "~/.local/bin/xenonet doctor --json").stdout ?? "",
    logs: (id) => vmSh(inst(id), "~/.local/bin/xenonet logs --json").stdout ?? "",
    // A datagram from one machine ("mac", or a VM) to a VM on the lab's
    // LAN, or to a multicast group there, and whether it came.
    udp: async (from, to, { group } = {}) => {
      const addr = lan(to).find((a) => !a.includes(":"));
      if (!addr) return false;
      const n = nonce("udp");
      const listener = later(inst(to), "udplisten", { PORT: String(UDP_PORT), GROUP: group ?? "", NONCE: n });
      await listener.waitFor(/^ready$/m, 15000);
      if (from === "mac") {
        const s = dgram.createSocket("udp4");
        await new Promise((resolve) => s.send(n, UDP_PORT, addr, () => (s.close(), resolve())));
      } else g(from, "udpsend", { ADDR: group ? "" : addr, GROUP: group ?? "", PORT: String(UDP_PORT), NONCE: n });
      return /^got$/m.test((await listener.done).stdout);
    },
  };
}

const WORDS = { ok: "ok  ", FAIL: "FAIL", wait: "wait", owner: "owner", skip: "skip" };

async function check(ids, flags) {
  const only = flags.only ? String(flags.only).split(",").filter(Boolean) : null;
  for (const id of only ?? []) if (!CHECKS.some((c) => c.id === id)) fail(`no check "${id}": ${CHECKS.map((c) => c.id).join(", ")}`);
  const tally = { ok: 0, FAIL: 0, wait: 0, owner: 0, skip: 0 };
  const line = (status, what, detail = "") => {
    tally[status]++;
    console.log(`  ${WORDS[status].padEnd(5)} ${what}${detail ? ` (${detail})` : ""}`);
  };
  const vms = pick(ids);
  if (!only) {
    const target = await canary();
    try {
      const mine = macNames();
      for (const id of vms) {
        const instance = running(id);
        step(id);
        vmChecks(id, instance, target, mine, (ok, what, detail) => line(ok ? "ok" : "FAIL", what, detail));
      }
    } finally {
      target.stop();
    }
  }
  const all = instances();
  const both = ["vm1", "vm2"].every((id) => vms.includes(id) && all[VMS[id].instance]?.status === "Running");
  step("Across the VMs: the transport (phase 5's plan, 4b)");
  const ctx = transportCtx(both);
  if (both) for (const id of ["vm1", "vm2"]) ctx.nft(id, null);
  for (const c of CHECKS.filter((c) => !only || only.includes(c.id))) {
    const r = c.optIn && !only ? { status: "skip", detail: `only when named: ${c.optIn}` } : await runCheck(c, ctx);
    line(r.status, c.what, r.detail);
    if (r.status === "owner" && c.standIn) say(`      an agent looks at it in XenoNet Dev instead: ${c.standIn}`);
  }
  const parts = [`${tally.ok} ok`, tally.FAIL && `${tally.FAIL} failed`, tally.wait && `${tally.wait} not in this build yet`, tally.owner && `${tally.owner} need the owner`, tally.skip && `${tally.skip} skipped`].filter(Boolean);
  step(parts.join(", "));
  if (tally.FAIL) process.exitCode = 1;
}

function teardown(_ids, flags) {
  const all = instances();
  const present = LAB_INSTANCES.filter((n) => all[n]);
  if (!flags.yes) {
    console.log(present.length ? `This deletes ${present.join(", ")}.` : "There are no lab VMs.");
    console.log("Sign out inside each VM you signed in to first (README.md, Teardown). Then: npm run lab -- teardown --yes");
    return;
  }
  for (const name of present) {
    if (all[name].status !== "Running") continue;
    // Whether the files exist, never what's in them.
    const signedIn = limaSsh(name, ["test -e ~/.claude/.credentials.json -o -e ~/.codex/auth.json"], { capture: true, allowFail: true }).status === 0;
    if (signedIn && !flags.force) fail(`${name} still has a sign-in. Sign out inside it first (/logout in Claude Code, codex logout), or pass --force.`);
  }
  for (const name of present) {
    step(name);
    if (all[name].status === "Running") limactl(["stop", "--tty=false", name]);
    if (all[name].protected) limactl(["unprotect", name]);
    limactl(["delete", "--tty=false", name]);
  }
  step(`Deleted ${present.join(", ") || "no VMs"}. Lima's cached images stay.`);
}

function help() {
  console.log(`npm run lab -- <command>   The XenoNet lab's VMs (scripts/lab/README.md)

  status                         the lab's VMs and their XenoNets
  build ubuntu|fedora [--ref R]  a golden copy: create, provision, stop, protect
  clone vm1|vm2                  a working VM from its golden copy, set up and started
  up [vm…]                       start working VMs
  down [vm…]                     stop them
  push [vm…] [--ref R]           XenoNet at a git ref (HEAD) into running VMs, and restart it
  check [vm…] [--only C,…]       each VM's checks (health, the loopback block, names, root),
                                 then the transport checks across vm1 and vm2:
                                 ${CHECKS.map((c) => c.id).join(", ")}
                                 --only runs just the transport checks named
  teardown [--yes]               delete the lab's VMs, by name

A transport check that waits on phase 5 code says "not in this build yet",
and one that needs a network says what the owner runs; neither fails.`);
}

const COMMANDS = { status, build, clone, up, down, push, check, teardown, help };

function parse(argv) {
  const flags = {};
  const rest = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-h" || a === "--help") flags.help = true;
    else if (a.startsWith("--")) {
      const [k, v] = a.slice(2).split(/=(.*)/s);
      if (v !== undefined) flags[k] = v;
      else if (["ref", "only"].includes(k)) flags[k] = argv[++i];
      else flags[k] = true;
    } else rest.push(a);
  }
  return { name: rest[0] ?? "help", args: rest.slice(1), flags };
}

const { name, args, flags } = parse(process.argv.slice(2));
const command = flags.help ? help : COMMANDS[name];
try {
  if (!command) fail(`no command "${name}". npm run lab -- help lists them.`);
  await command(args, flags);
} catch (err) {
  if (!(err instanceof LabError)) throw err;
  console.error(`lab: ${err.message}`);
  process.exitCode = 1;
}
