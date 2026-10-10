// A small client for `codex app-server` over stdio JSON-RPC, for the drift
// check: requests and their answers, the notifications it sent, and
// nothing kept between runs. core/codex.js is XenoNet's own client; this one
// starts a given binary in a made-up home (sandbox.mjs) and answers the
// server's own requests with an error, since the check never approves
// anything.
import { spawn } from "node:child_process";

export function openAppServer(cmd, args, { env, cwd, timeoutMs = 30_000 } = {}) {
  const child = spawn(cmd, [...args, "app-server"], { env, cwd, stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  const pending = new Map();
  const notifications = [];
  let stderr = "";
  let buf = "";
  let nextId = 1;
  let exited = null;
  const send = (msg) => child.stdin.write(JSON.stringify(msg) + "\n");
  child.stdout.setEncoding("utf8").on("data", (chunk) => {
    buf += chunk;
    let nl;
    while ((nl = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }
      if (msg.id !== undefined && msg.method) send({ jsonrpc: "2.0", id: msg.id, error: { code: -32601, message: "the drift check answers no requests" } });
      else if (msg.id !== undefined && pending.has(msg.id)) {
        const { resolve, reject, timer } = pending.get(msg.id);
        pending.delete(msg.id);
        clearTimeout(timer);
        if (msg.error) reject(Object.assign(new Error(msg.error.message ?? "error"), { rpc: msg.error }));
        else resolve(msg.result);
      } else if (msg.method) notifications.push({ method: msg.method, params: msg.params });
    }
  });
  child.stderr.setEncoding("utf8").on("data", (c) => (stderr = (stderr + c).slice(-4000)));
  const fail = (err) => {
    for (const { reject, timer } of pending.values()) {
      clearTimeout(timer);
      reject(err);
    }
    pending.clear();
  };
  child.on("error", (err) => {
    exited = err;
    fail(err);
  });
  child.on("exit", (code) => {
    exited = new Error(`codex app-server exited (${code}): ${stderr.trim().split(/\r?\n/).slice(-3).join(" | ")}`);
    fail(exited);
  });
  const request = (method, params) =>
    new Promise((resolve, reject) => {
      if (exited) return reject(exited);
      const id = nextId++;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`${method} timed out`));
      }, timeoutMs);
      pending.set(id, { resolve, reject, timer });
      send({ jsonrpc: "2.0", id, method, params });
    });
  return {
    request,
    notify: (method, params) => send({ jsonrpc: "2.0", method, ...(params ? { params } : {}) }),
    notifications,
    get stderr() {
      return stderr;
    },
    close: () =>
      new Promise((resolve) => {
        if (exited) return resolve();
        child.once("exit", () => resolve());
        child.stdin.end();
        setTimeout(() => child.kill(), 3000).unref();
      }),
  };
}
