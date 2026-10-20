// core/secret-store.js in Rust (crates/xenonet-actors/src/secret_store.rs):
// secrets XenoNet keeps for you in your computer's own credential store.
// Rust checks a target and a secret, says what the store's program is
// given (a secret only ever on its standard input) and reads its answer;
// the program runs here, through `run(file, args, { input })`, which
// resolves to { code, stdout } and never throws for a non-zero exit.
import { spawn } from "node:child_process";
import { native } from "../index.js";
import { drive } from "../drive.js";
import { onPath } from "./paths.js";

const [target, secret] = native.secretPatterns();
export const TARGET = new RegExp(target);
export const SECRET = new RegExp(secret);

function runFile(file, args, { input = "", timeout = 20000, env = process.env } = {}) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(file, args, { windowsHide: true, env, stdio: ["pipe", "pipe", "pipe"] });
    } catch {
      return resolve({ code: -1, stdout: "" });
    }
    let stdout = "";
    const timer = setTimeout(() => child.kill(), timeout);
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", () => {});
    child.on("error", () => (clearTimeout(timer), resolve({ code: -1, stdout: "" })));
    child.on("close", (code) => (clearTimeout(timer), resolve({ code: code ?? -1, stdout })));
    child.stdin.on("error", () => {});
    child.stdin.end(input);
  });
}

const text = (v) => String(v ?? "");

export function createSecretStore({ platform = process.platform, env = process.env, run = runFile } = {}) {
  const core = new native.SecretStoreCore(String(platform), () => onPath("secret-tool", { env, platform }));
  const go = (machine) => drive(machine, (q) => (q.input === undefined ? run(q.file, q.args) : run(q.file, q.args, { input: q.input })));
  return {
    kind: core.kind,
    available: () => core.available(),
    has: async (target) => go(core.has(text(target))),
    get: async (target) => go(core.get(text(target))),
    set: async (target, secret, { comment = "Kept by XenoNet" } = {}) => go(core.set(text(target), text(secret), String(comment))),
    remove: async (target) => go(core.remove(text(target))),
  };
}

// A store that keeps secrets in memory only, for the demo and test
// instances, whose connections are made up: nothing reaches your real
// credential store from them.
export function createMemorySecrets() {
  const core = new native.MemorySecretsCore();
  return {
    kind: core.kind,
    available: () => true,
    has: async (target) => core.has(text(target)),
    get: async (target) => core.get(text(target)) ?? null,
    set: async (target, secret) => core.set(text(target), text(secret)),
    remove: async (target) => core.remove(text(target)),
  };
}
