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
  let answer;
  try {
    const client = await connectService({ env });
    try { answer = await client.request(action === "stop" ? "stop" : "status", { force: !!flags.force }); }
    finally { client.close(); }
  } catch (err) {
    if (file) {
      fs.rmSync(file, { force: true });
      // The service writes service-<port>.json beside the stop file while
      // it runs; without it, there's none to stop.
      if (!fs.existsSync(file.replace(/\.stop$/, ".json"))) throw new Error("XenoNet's service isn't running");
    }
    throw err;
  }
  // The service answers as it begins to stop, and this waits until it has:
  // a `service start` right after would otherwise find it still answering,
  // say it reused it, and leave nothing running once it ended.
  if (file && answer?.stopping === true) {
    const until = Date.now() + STOP_WAIT_MS;
    while (Date.now() < until && (await answers(env))) await new Promise((r) => setTimeout(r, 200));
  }
  print(JSON.stringify(answer));
}

// How long `service stop` waits for the service to end.
const STOP_WAIT_MS = 15000;

// Whether a service answers on env's port.
async function answers(env) {
  try {
    const client = await connectService({ env });
    try { await client.request("status", {}); return true; }
    finally { client.close(); }
  } catch {
    return false;
  }
}
