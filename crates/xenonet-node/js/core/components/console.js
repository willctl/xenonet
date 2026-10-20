// core/components/console.js in Rust (crates/xenonet-components/src/console.rs):
// the console an installed XenoNet offers the console tools, through the
// service. Each action finds the service again, and one that was
// interrupted is never sent twice. The connections and Desktop's process
// are here.
import { spawn } from "node:child_process";
import { native } from "../../index.js";
import { drive } from "../../task.js";
import { envOf, machine } from "../../machine.js";
import { connectService } from "./runtime.js";

function consoleOps(env) {
  const clients = new Map();
  const responses = new Map();
  let next = 0;
  return async (q) => {
    switch (q.op) {
      case "connect": {
        const client = await connectService({ env });
        const id = ++next;
        clients.set(id, client);
        return { client: id, info: client.info };
      }
      case "request":
        return clients.get(q.client).request(q.method, q.params);
      case "close": {
        const client = clients.get(q.client);
        clients.delete(q.client);
        client?.close();
        return null;
      }
      case "spawn": {
        const child = spawn(q.command, q.args, { cwd: q.cwd, env: q.env, detached: true, windowsHide: true, stdio: "ignore" });
        const started = new Promise((resolve, reject) => {
          child.once("spawn", resolve);
          child.once("error", reject);
        });
        child.unref();
        await started;
        return null;
      }
      case "fetch": {
        const id = ++next;
        responses.set(id, await fetch(q.url));
        return { res: id };
      }
      case "json":
        return responses.get(q.res).json();
      default:
        throw new Error(`the console asked for ${q.op}`);
    }
  };
}

export function installedConsole({ env = process.env } = {}) {
  const c = new native.CompConsole(envOf(env), machine());
  const run = (kind, args = {}) => drive(c.task(kind, args), consoleOps(env));
  const onlyDev = () => {
    throw new Error(native.compOnlyDev());
  };
  return {
    id: "installed",
    name: "XenoNet",
    port: c.port,
    call: (body) => run("call", { body }),
    ping: () => run("ping"),
    ready: ({ timeout = 60000 } = {}) => run("ready", { timeout }),
    launch: ({ show = "inactive", restart = false, timeout } = {}) => run("launch", { show, restart, timeout }),
    quit: () => run("quit"),
    health: async () => !!(await run("ping")),
    driver: (method, ...args) => run("driver", { method, args }),
    screenshot: (opts) => run("screenshot", { opts }),
    input: (opts) => run("input", { opts }),
    window: (opts) => run("window", { opts }),
    logs: (opts) => run("logs", { opts }),
    record: (opts) => run("record", { opts }),
    eval: onlyDev,
    simulate: onlyDev,
    api: (route) => run("api", { route: String(route) }),
  };
}
