// The XenoNet lab's settings and the pure parts of lab.mjs: which VMs there
// are, where the lab keeps its files, and the text of the files it writes
// (the ssh config, known_hosts, the console key's authorized_keys line).
// Nothing here runs a command, so the tests can check all of it.
import os from "node:os";
import path from "node:path";
import { createHash, randomBytes } from "node:crypto";
import { appDir } from "../../core/dirs.js";

// The golden copies: built once from a template, stopped, protected, and
// never signed in to. Working VMs are clones of them.
export const BASES = {
  ubuntu: { instance: "xenonet-base-ubuntu", template: "ubuntu.yaml", os: "Ubuntu 24.04" },
  fedora: { instance: "xenonet-base-fedora", template: "fedora.yaml", os: "Fedora 44" },
};

// The working VMs. A host id follows contracts.md and is never a machine's
// name. The SSH port is where Lima forwards the guest's port 22 on the
// Mac's 127.0.0.1, fixed so each VM's alias stays put. vm1 runs in UTC and
// vm2 in a distant zone, so the hosts disagree on local time. Only vm1 asks
// Anthropic for plan usage, so the lab doesn't multiply those requests.
export const VMS = {
  vm1: { instance: "xenonet-vm1", base: "ubuntu", sshPort: 60851, timezone: "UTC", env: {} },
  vm2: { instance: "xenonet-vm2", base: "fedora", sshPort: 60852, timezone: "Pacific/Auckland", env: { XENONET_CLAUDE_USAGE: "off" } },
};

// Every instance name the lab may own, for teardown, which deletes by name
// and never by pattern: other Lima VMs on the Mac aren't the lab's. The
// checkpoints are the ones README.md makes by hand; the third Linux VM and
// the macOS ones are the plan's later phases.
export const LAB_INSTANCES = ["xenonet-vm1", "xenonet-vm2", "xenonet-vm1-ckpt", "xenonet-vm2-ckpt", "xenonet-vm3", "xenonet-mac1", "xenonet-base-ubuntu", "xenonet-base-fedora", "xenonet-base-macos"];

export const HOST_ID = /^[a-z][a-z0-9-]{1,31}$/;
export const DEMO_PORT = 47841;
export const REAL_PORT = 47840;
export const CONSOLE_KEY_COMMENT = "xenonet-lab-console";
// Where the xenonet command goes in a guest (`xenonet install`), written out in
// full because a forced command runs without the login shell's PATH.
export const GUEST_XENONET = "/home/will/.local/bin/xenonet";

// The lab's own files: its ssh config, the console key and known_hosts, in a
// lab folder in XenoNet Dev's state folder. XENONET_LAB_DIR moves it. Each VM's
// sudo password sits in its Lima folder, which limactl delete removes.
export function labPaths({ env = process.env, home = os.homedir(), platform = process.platform } = {}) {
  const dir = env.XENONET_LAB_DIR ? path.resolve(env.XENONET_LAB_DIR) : path.join(appDir("state", { env, platform, homeDir: home, name: "XenoNet Dev" }), "lab");
  const limaHome = env.LIMA_HOME ? path.resolve(env.LIMA_HOME) : path.join(home, ".lima");
  return {
    dir,
    sshConfig: path.join(dir, "ssh_config"),
    consoleKey: path.join(dir, "console_ed25519"),
    knownHosts: path.join(dir, "known_hosts"),
    limaHome,
    instanceDir: (instance) => path.join(limaHome, instance),
    password: (instance) => path.join(limaHome, instance, "xenonet-lab-password"),
  };
}

// A path for a file the lab writes, with the home folder as ~ (ssh expands
// it), so the files carry no user name. Quoted, since XenoNet Dev's state
// folder has a space in it.
export function sshPath(file, home = os.homedir()) {
  const rel = path.relative(home, file);
  const shown = rel && !rel.startsWith("..") && !path.isAbsolute(rel) ? `~/${rel.split(path.sep).join("/")}` : file;
  if (/["\\]/.test(shown)) throw new Error(`can't write ${shown} in an ssh config`);
  return `"${shown}"`;
}

export const hostKeyAlias = (id) => `xenonet-lab-${id}`;

// The lab's ssh config: an alias per VM, with the console key only (no
// agent, no other identities) and host keys checked against the lab's own
// known_hosts. Lima's own config turns checking off; this one doesn't.
export function sshConfigText({ vms = VMS, paths, home = os.homedir() }) {
  const lines = [
    "# The XenoNet lab's VMs (scripts/lab/README.md), written by `npm run lab`.",
    "# XenoNet's hosts use it with ssh -F; nothing here is in ~/.ssh/config.",
    "# The console key can run `xenonet relay` in each VM and nothing else.",
  ];
  for (const [id, vm] of Object.entries(vms)) {
    if (!HOST_ID.test(id)) throw new Error(`"${id}" isn't a host id`);
    lines.push(
      "",
      `Host ${id}`,
      "  HostName 127.0.0.1",
      `  Port ${vm.sshPort}`,
      `  HostKeyAlias ${hostKeyAlias(id)}`,
      "  User will",
      `  IdentityFile ${sshPath(paths.consoleKey, home)}`,
      "  IdentitiesOnly yes",
      "  IdentityAgent none",
      `  UserKnownHostsFile ${sshPath(paths.knownHosts, home)}`,
      "  StrictHostKeyChecking yes",
      "  HostKeyAlgorithms ssh-ed25519",
      "  PasswordAuthentication no",
      "  ForwardAgent no",
      "  ForwardX11 no",
    );
  }
  return lines.join("\n") + "\n";
}

// known_hosts with this VM's line replaced: the key the lab read from the
// VM's /etc/ssh through Lima's own channel, under the VM's alias.
export function withHostKey(knownHosts, id, publicKey) {
  const [type, key] = String(publicKey).trim().split(/\s+/);
  if (type !== "ssh-ed25519" || !/^[A-Za-z0-9+/]+={0,2}$/.test(key ?? "")) throw new Error(`not an ed25519 host key: ${String(publicKey).slice(0, 40)}`);
  const alias = hostKeyAlias(id);
  const kept = String(knownHosts ?? "").split("\n").filter((l) => l.trim() && l.split(/\s+/)[0] !== alias);
  return [...kept, `${alias} ${type} ${key}`].join("\n") + "\n";
}

export function hostKeyOf(knownHosts, id) {
  const alias = hostKeyAlias(id);
  const line = String(knownHosts ?? "").split("\n").find((l) => l.split(/\s+/)[0] === alias);
  return line ? line.split(/\s+/).slice(1, 3).join(" ") : null;
}

// The console key's line in a guest's authorized_keys: `restrict` turns off
// forwarding, the terminal and ~/.ssh/rc, and `command` runs the relay
// whatever the client asks for (hosts.md). The port is the XenoNet under
// test: the demo's for the first runs, the real one's after.
export function authorizedKeyLine(publicKey, port = DEMO_PORT) {
  const [type, key] = String(publicKey).trim().split(/\s+/);
  if (type !== "ssh-ed25519" || !/^[A-Za-z0-9+/]+={0,2}$/.test(key ?? "")) throw new Error("the console key isn't an ed25519 public key");
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error(`bad relay port: ${port}`);
  return `restrict,command="${GUEST_XENONET} relay --port ${port}" ${type} ${key} ${CONSOLE_KEY_COMMENT}`;
}

// What a working VM's services read from ~/.config/xenonet-lab/xenonet.env.
export function envFileText(env = {}) {
  return Object.entries(env)
    .map(([k, v]) => {
      if (!/^[A-Z][A-Z0-9_]*$/.test(k) || /[\n"\\$`]/.test(String(v))) throw new Error(`bad setting ${k}`);
      return `${k}=${v}\n`;
    })
    .join("");
}

// Values for guest.sh, set as shell variables above its text.
export const shq = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;
export function guestScript(step, vars, body) {
  if (!/^[a-z]+$/.test(step)) throw new Error(`bad step ${step}`);
  const header = Object.entries({ STEP: step, ...vars }).map(([k, v]) => {
    if (!/^[A-Z][A-Z0-9_]*$/.test(k)) throw new Error(`bad variable ${k}`);
    return `${k}=${shq(v)}`;
  });
  return `${header.join("\n")}\n${body}`;
}

// ---------- the VMs' sudo passwords ----------

// A password for one VM's sudo: random, kept in its Lima folder on the Mac.
export const newPassword = () => randomBytes(24).toString("base64url");

// SHA-512 crypt ($6$), the hash /etc/shadow takes, from Ulrich Drepper's
// specification (https://www.akkadia.org/drepper/SHA-crypt.txt). Only the
// hash goes into a VM, as a template parameter. macOS has no tool for it,
// and Node has SHA-512, so it's done here.
const ITOA64 = "./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ORDER = [
  [0, 21, 42], [22, 43, 1], [44, 2, 23], [3, 24, 45], [25, 46, 4], [47, 5, 26], [6, 27, 48],
  [28, 49, 7], [50, 8, 29], [9, 30, 51], [31, 52, 10], [53, 11, 32], [12, 33, 54], [34, 55, 13],
  [56, 14, 35], [15, 36, 57], [37, 58, 16], [59, 17, 38], [18, 39, 60], [40, 61, 19], [62, 20, 41],
];

export function sha512crypt(password, salt = randomSalt(), { rounds } = {}) {
  const sha = () => createHash("sha512");
  const P = Buffer.from(String(password), "utf8");
  const S = Buffer.from(String(salt).slice(0, 16), "utf8");
  const n = rounds == null ? 5000 : Math.min(999999999, Math.max(1000, rounds));
  const B = sha().update(P).update(S).update(P).digest();
  const a = sha().update(P).update(S);
  let left = P.length;
  for (; left > 64; left -= 64) a.update(B);
  a.update(B.subarray(0, left));
  for (let len = P.length; len > 0; len >>= 1) a.update(len & 1 ? B : P);
  const A = a.digest();
  const dp = sha();
  for (let i = 0; i < P.length; i++) dp.update(P);
  const DP = dp.digest();
  const Pseq = Buffer.alloc(P.length, 0).map((_, i) => DP[i % 64]);
  const ds = sha();
  for (let i = 0; i < 16 + A[0]; i++) ds.update(S);
  const DS = ds.digest();
  const Sseq = Buffer.alloc(S.length, 0).map((_, i) => DS[i % 64]);
  let C = A;
  for (let i = 0; i < n; i++) {
    const c = sha();
    c.update(i & 1 ? Pseq : C);
    if (i % 3) c.update(Sseq);
    if (i % 7) c.update(Pseq);
    c.update(i & 1 ? C : Pseq);
    C = c.digest();
  }
  let out = "";
  const b64 = (b2, b1, b0, chars) => {
    let w = (b2 << 16) | (b1 << 8) | b0;
    for (let i = 0; i < chars; i++, w >>= 6) out += ITOA64[w & 0x3f];
  };
  for (const [x, y, z] of ORDER) b64(C[x], C[y], C[z], 4);
  b64(0, 0, C[63], 2);
  return `$6$${rounds == null ? "" : `rounds=${n}$`}${S.toString("utf8")}$${out}`;
}

export function randomSalt() {
  return [...randomBytes(16)].map((b) => ITOA64[b & 0x3f]).join("");
}
