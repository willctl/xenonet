// The XenoNet lab's settings and the pure parts of lab.mjs: which VMs there
// are, where Lima keeps their files, and the text of what the lab sends a
// guest. Nothing here runs a command, so the tests can check all of it.
import os from "node:os";
import path from "node:path";
import { createHash, randomBytes } from "node:crypto";

// The golden copies: built once from a template, stopped, protected, and
// never signed in to. Working VMs are clones of them.
export const BASES = {
  ubuntu: { instance: "xenonet-base-ubuntu", template: "ubuntu.yaml", os: "Ubuntu 24.04" },
  fedora: { instance: "xenonet-base-fedora", template: "fedora.yaml", os: "Fedora 44" },
};

// The working VMs. A host id follows contracts.md and is never a machine's
// name: it's the id the owner gives each VM when it joins a network. The
// SSH port is where Lima forwards the guest's port 22 on the Mac's
// 127.0.0.1, fixed so a VM keeps it across clones and restores. vm1 runs in
// UTC and vm2 in a distant zone, so the hosts disagree on local time. Only
// vm1 asks Anthropic for plan usage, so the lab doesn't multiply those
// requests.
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
// The interface Lima's vzNAT network gets in a guest (lab.yaml's
// `networks`): the VMs and the Mac reach each other on it, UDP and
// multicast included.
export const LAN_DEV = "lima0";

// Where Lima keeps each VM's files. Each VM's sudo password sits in its
// Lima folder, which limactl delete removes.
export function labPaths({ env = process.env, home = os.homedir() } = {}) {
  const limaHome = env.LIMA_HOME ? path.resolve(env.LIMA_HOME) : path.join(home, ".lima");
  return {
    limaHome,
    instanceDir: (instance) => path.join(limaHome, instance),
    password: (instance) => path.join(limaHome, instance, "xenonet-lab-password"),
  };
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
