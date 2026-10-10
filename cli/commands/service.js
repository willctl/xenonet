import { print } from "../lib/out.js";
import { installedRuntime, recoverServiceUpdate } from "../../core/components/install.js";
import { ensureService, connectService } from "../../core/components/runtime.js";

export const help = "Manage the background service: xenonet service start, status or stop. Stop refuses while terminals are running unless you pass --force.";
export async function run(args, flags) {
  const action = args[0] || "status";
  const env = { ...process.env, ...(flags.port ? { XENONET_PORT: String(flags.port) } : {}) };
  if (action === "start") {
    await recoverServiceUpdate({ env });
    return print(JSON.stringify({ status: await ensureService(installedRuntime(env)) }));
  }
  if (!["status", "stop"].includes(action)) throw new Error(help);
  const client = await connectService({ env });
  try { print(JSON.stringify(await client.request(action === "stop" ? "stop" : "status", { force: !!flags.force }))); }
  finally { client.close(); }
}
