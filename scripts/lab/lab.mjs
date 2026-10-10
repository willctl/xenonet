#!/usr/bin/env node
// The XenoNet lab: Linux VMs on this Mac, in Lima, to test XenoNet against
// several hosts (docs/design/vm-lab.md). scripts/lab/README.md says how to
// use it. Commands take the lab's VMs by host id (vm1, vm2), or all of them
// when none is named.
//
//   npm run lab -- status
//   npm run lab -- build ubuntu|fedora [--ref <git ref>]
//   npm run lab -- clone vm1|vm2
//   npm run lab -- up [vm…]
//   npm run lab -- down [vm…]
//   npm run lab -- push [vm…] [--ref <git ref>]
//   npm run lab -- keys [vm…] [--relay-port <port>]
//   npm run lab -- check [vm…]
//   npm run lab -- teardown [--yes]
//
// It never signs in to anything and never reads a sign-in: the owner signs
// in inside each VM (README.md, "Signing in").
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { tilde as tildeIn } from "../../core/paths.js";
import { trustCommand } from "../../core/hosts/link.js";
import { BASES, VMS, LAB_INSTANCES, DEMO_PORT, REAL_PORT, CONSOLE_KEY_COMMENT, labPaths, sshConfigText, withHostKey, hostKeyOf, authorizedKeyLine, envFileText, guestScript, newPassword, sha512crypt, shq } from "./config.mjs";
import { reposScript } from "./fixtures.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "../..");
const VERSIONS = JSON.parse(fs.readFileSync(path.join(HERE, "versions.json"), "utf8"));
const GUEST = fs.readFileSync(path.join(HERE, "guest.sh"), "utf8");
const paths = labPaths();
const home = os.homedir();
const tilde = (p) => tildeIn(p, home);

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

// The console's key, made for the lab the first time it's needed. It's
// never a copy of another key, and its comment carries no user name.
function consoleKey() {
  if (!fs.existsSync(paths.consoleKey)) {
    fs.mkdirSync(paths.dir, { recursive: true, mode: 0o700 });
    run("ssh-keygen", ["-q", "-t", "ed25519", "-N", "", "-C", CONSOLE_KEY_COMMENT, "-f", paths.consoleKey]);
  }
  return read(`${paths.consoleKey}.pub`).trim();
}

const writeSshConfig = () => write(paths.sshConfig, sshConfigText({ paths, home }));

// The VM's host key, read from its /etc/ssh over Lima's channel, so nothing
// is trusted blindly the first time the console connects.
function recordHostKey(id) {
  const key = limaSsh(VMS[id].instance, ["cat", "/etc/ssh/ssh_host_ed25519_key.pub"], { capture: true }).stdout.trim();
  const before = read(paths.knownHosts);
  const after = withHostKey(before, id, key);
  if (hostKeyOf(before, id) && hostKeyOf(before, id) !== hostKeyOf(after, id)) say(`${id}: its host key changed; the lab's known_hosts has the new one`);
  write(paths.knownHosts, after);
}

// ssh through the lab's own config with the console key, as XenoNet's hosts
// connect. The key runs the relay whatever is asked, so this returns what
// the relay said: its hello once the branch has it, an error before.
function viaConsole(id, command = []) {
  const r = run("ssh", ["-F", paths.sshConfig, "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", id, ...command], { capture: true, allowFail: true, timeout: 8000 });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`.trim();
  return { out, hello: /"t"\s*:\s*"hello"/.test(out), relayMissing: /no command "relay"/.test(out), refused: /Permission denied|Host key verification failed|REMOTE HOST IDENTIFICATION/i.test(out) };
}

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
  const c = viaConsole(id);
  const how = c.hello ? "the relay answers" : c.relayMissing ? "the relay isn't in this branch yet" : c.refused ? `refused: ${c.out.split("\n").pop()}` : c.out.split("\n").pop() || "no answer";
  // Written so it runs as it's printed: the lab's config is in XenoNet Dev's
  // state folder, whose name has a space in it.
  say(`${trustCommand(id, paths.sshConfig, tilde)}: runs ${f.relay || "?"}; ${how}`);
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
  say(`${tilde(paths.dir)}: ${["ssh_config", "console_ed25519", "known_hosts"].filter((f) => fs.existsSync(path.join(paths.dir, f))).join(", ") || "nothing yet"}`);
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

function clone([id], flags) {
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
  recordHostKey(id);
  const pub = consoleKey();
  writeSshConfig();
  step(`${id}: made-up repos`);
  limaSsh(vm.instance, ["bash", "-s"], { input: reposScript(id) });
  step(`${id}: its settings, the console's key and its two XenoNets`);
  guest(vm.instance, "vm", { HOST_ID: id, XENONET_ENV: envFileText(vm.env), CONSOLE_KEY_LINE: authorizedKeyLine(pub, relayPort(flags)) });
  report(id);
}

function up(ids) {
  const all = instances();
  for (const id of pick(ids)) {
    step(id);
    if (all[VMS[id].instance].status !== "Running") limactl(["start", "--tty=false", VMS[id].instance]);
    recordHostKey(id);
    writeSshConfig();
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
    guest(instance, "name", { HOST_ID: id });
    report(id);
  }
}

function relayPort(flags) {
  const port = Number(flags["relay-port"] ?? DEMO_PORT);
  if (!Number.isInteger(port) || port < 1 || port > 65535) fail(`--relay-port takes a port, such as ${DEMO_PORT} (the demo XenoNet) or ${REAL_PORT}`);
  return port;
}

function keys(ids, flags) {
  const port = relayPort(flags);
  const pub = consoleKey();
  for (const id of pick(ids)) {
    const instance = running(id);
    recordHostKey(id);
    guest(instance, "key", { CONSOLE_KEY_LINE: authorizedKeyLine(pub, port) });
    say(`${id}: the console key runs xenonet relay --port ${port}`);
  }
  writeSshConfig();
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

async function check(ids) {
  const target = await canary();
  const results = [];
  const record = (ok, what, detail = "") => {
    results.push(ok);
    console.log(`  ${ok ? "ok  " : "FAIL"}  ${what}${detail ? ` (${detail})` : ""}`);
  };
  try {
    const mine = macNames();
    for (const id of pick(ids)) {
      const instance = running(id);
      step(id);
      const sh = (cmd, input) => limaSsh(instance, ["bash", "-c", shq(cmd)], { capture: true, allowFail: true, input });
      const f = facts(instance);
      record(/"ok":true/.test(f["health-47840"] ?? ""), `XenoNet answers on 127.0.0.1:${REAL_PORT}`);
      record(/"ok":true/.test(f["health-47841"] ?? ""), `the demo XenoNet answers on 127.0.0.1:${DEMO_PORT}`);
      const local = run("curl", ["-s", "-m", "3", `http://127.0.0.1:${target.port}/`], { capture: true, allowFail: true }).stdout;
      const probe = sh(`curl -s -m 3 http://192.168.5.2:${target.port}/; echo " exit=$?"`).stdout.trim();
      record(local === "xenonet-lab-canary" && !probe.includes("xenonet-lab-canary"), "can't reach this Mac's 127.0.0.1 through 192.168.5.2", `a server there answers here; from ${id}: curl ${probe.replace(/.*exit=/, "exit ")}`);
      const net = sh("curl -s -o /dev/null -m 10 -w '%{http_code}' https://api.anthropic.com").stdout.trim();
      record(/^[1-5]\d\d$/.test(net), "reaches the internet", `api.anthropic.com: HTTP ${net}`);
      const listening = sh("ss -ltnH").stdout.split("\n").map((l) => l.trim().split(/\s+/)[3]).filter((a) => /:(47840|47841)$/.test(a ?? ""));
      record(listening.length >= 2 && listening.every((a) => a.startsWith("127.0.0.1:")), "XenoNet listens on 127.0.0.1 only", listening.join(", "));
      // What Lima or a share would mount: virtiofs, 9p, sshfs (fuse.sshfs),
      // NFS or SMB. fusectl, the kernel's own, isn't one.
      const mounts = sh("findmnt -rn -o FSTYPE,TARGET").stdout.split("\n").filter((l) => /^(virtiofs|9p|fuse\.\S+|nfs4?|cifs|smb3) /.test(l));
      record(mounts.length === 0, "nothing is mounted from this Mac", mounts.join(", "));
      const c = viaConsole(id, ["echo", "shell-reached"]);
      record(!c.out.includes("shell-reached") && (c.hello || c.relayMissing), "the console key gets the relay, not a shell", c.hello ? "relay hello" : c.relayMissing ? "relay not in this branch yet" : c.out.split("\n").pop());
      const held = sh("getent passwd will; cat /etc/passwd /etc/group ~/.gitconfig ~/.ssh/authorized_keys; hostname; git config --global user.name").stdout;
      const gecos = held.split("\n")[0]?.split(":")[4];
      record(gecos === "Will" && !mine.some((n) => n.test(held)), "only made-up names", `the lab user is "${gecos}", git's is "${held.trim().split("\n").pop()}"`);
      const nopass = sh("sudo -n true 2>&1; echo exit=$?").stdout;
      const withPassword = sh("sudo -S -p '' true; echo exit=$?", `${read(paths.password(instance)).trim()}\n`).stdout;
      record(!/exit=0/.test(nopass) && /exit=0/.test(withPassword), "root needs the VM's password, which only this Mac has");
    }
  } finally {
    target.stop();
  }
  const failed = results.filter((ok) => !ok).length;
  step(failed ? `${failed} of ${results.length} checks failed` : `All ${results.length} checks passed`);
  if (failed) process.exitCode = 1;
}

function teardown(_ids, flags) {
  const all = instances();
  const present = LAB_INSTANCES.filter((n) => all[n]);
  if (!flags.yes) {
    console.log(present.length ? `This deletes ${present.join(", ")} and ${tilde(paths.dir)}.` : `No lab VMs. This deletes ${tilde(paths.dir)}.`);
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
  fs.rmSync(paths.dir, { recursive: true, force: true });
  step(`Deleted ${present.join(", ") || "no VMs"} and ${tilde(paths.dir)}. Lima's cached images stay.`);
}

function help() {
  console.log(`npm run lab -- <command>   The XenoNet lab's VMs (scripts/lab/README.md)

  status                         the lab's VMs, their XenoNets, and the lab's files
  build ubuntu|fedora [--ref R]  a golden copy: create, provision, stop, protect
  clone vm1|vm2                  a working VM from its golden copy, set up and started
  up [vm…]                       start working VMs
  down [vm…]                     stop them
  push [vm…] [--ref R]           XenoNet at a git ref (HEAD) into running VMs, and restart it
  keys [vm…] [--relay-port N]    the console key's relay port (${DEMO_PORT}, the demo, or ${REAL_PORT})
  check [vm…]                    the plan's checks: health, the loopback block, names, keys
  teardown [--yes]               delete the lab's VMs, by name, and its files`);
}

const COMMANDS = { status, build, clone, up, down, push, keys, check, teardown, help };

function parse(argv) {
  const flags = {};
  const rest = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-h" || a === "--help") flags.help = true;
    else if (a.startsWith("--")) {
      const [k, v] = a.slice(2).split(/=(.*)/s);
      if (v !== undefined) flags[k] = v;
      else if (["ref", "relay-port"].includes(k)) flags[k] = argv[++i];
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
