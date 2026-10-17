// core/components/runtime.js in Rust (crates/xenonet-components/src/runtime.rs):
// where the service's runtime files are, the descriptor that says how to
// reach it, and starting it when it isn't running. The socket and the
// service's process are here.
import fs from "node:fs";
import net from "node:net";
import { spawn } from "node:child_process";
import { native } from "../../index.js";
import { settle } from "../../settle.js";
import { drive } from "../../task.js";
import { envOf, machine } from "../../machine.js";
import { rpc } from "./rpc.js";
import { SERVICE_API, negotiate } from "./protocol.js";

export const runtimeDir = (env = process.env) => native.compRuntimeDir(envOf(env), machine());
export const descriptorFile = (env = process.env) => native.compDescriptorFile(envOf(env), machine());

export async function connectService({ env = process.env, role = "tools", handle } = {}) {
  const descriptor = settle(native.compReadDescriptor(envOf(env), machine()));
  const socket = net.connect({ host: "127.0.0.1", port: descriptor.port });
  const client = rpc(socket, handle, { incomingEvents: role === "desktop" ? ["notification"] : [] });
  try {
    client.info = await client.request("hello", native.compHello(descriptor.token, String(role)), 5000);
    negotiate(SERVICE_API, client.info.api);
    return client;
  } catch (err) {
    client.close();
    throw err;
  }
}

export function ensureService({ root, exec, env = process.env, timeout = 60000 }) {
  let failure;
  return drive(native.compEnsureService(String(root), String(exec), envOf(env), Number(timeout), machine()), async (q) => {
    if (q.op === "connect") {
      const c = await connectService({ env });
      c.close();
      return null;
    }
    if (q.op === "spawn") {
      const out = fs.openSync(q.log, "a", 0o600);
      const child = spawn(q.command, q.args, { cwd: q.cwd, env: q.env, detached: true, windowsHide: true, stdio: ["ignore", out, out] });
      fs.closeSync(out);
      child.unref();
      child.on("error", (err) => { failure = err; });
      child.on("exit", (code) => { failure = new Error(`XenoNet Service exited (${code}). See its service log.`); });
      return null;
    }
    if (q.op === "failure") {
      if (failure) throw failure;
      return null;
    }
    throw new Error(`ensureService asked for ${q.op}`);
  });
}
