import fs from "node:fs";
import path from "node:path";
import { print } from "../lib/out.js";
import { installedRuntime, recoverServiceUpdate } from "../../core/components/install.js";
import { ensureService, connectService, stopFile } from "../../core/components/runtime.js";

export const help = "Manage the background service: xenonet service start, status or stop. Stop refuses while terminals are running unless you pass --force.";
export async function run(args, flags) {
  const action = args[0] || "status";
  const env = { ...process.env, ...(flags.port ? { XENONET_PORT: String(flags.port) } : {}) };
  if (action === "start") {
    // A stop file an earlier stop left is cleared first.
    fs.rmSync(stopFile(env), { force: true });
    await recoverServiceUpdate({ env });
    return print(JSON.stringify({ status: await ensureService(installedRuntime(env)) }));
  }
  if (!["status", "stop"].includes(action)) throw new Error(help);
  // The stop file goes down before the stop: a supervisor waiting for the
  // port (a service Desktop started had it first) sees it and stays
  // stopped too, rather than starting its copy the moment the port is
  // free. A stop that's refused takes it away again. One that can't be
  // written doesn't hold up the stop, as in the Rust command.
  const file = action === "stop" ? stopFile(env) : null;
  if (file) {
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, JSON.stringify({ at: Date.now(), pid: process.pid }));
    } catch {}
  }
  try {
    const client = await connectService({ env });
    try { print(JSON.stringify(await client.request(action === "stop" ? "stop" : "status", { force: !!flags.force }))); }
    finally { client.close(); }
  } catch (err) {
    if (file) fs.rmSync(file, { force: true });
    throw err;
  }
}
