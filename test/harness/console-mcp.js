import { spawn } from "node:child_process";
import { command as xenonet } from "./xenonet-bin.js";

// xenonet mcp console: through the given binary (an installed launcher's),
// or as the tests run xenonet.
export async function consoleMcp({ bin, env }) {
  const [file, args] = bin ? [bin, ["mcp", "console"]] : xenonet(["mcp", "console"]);
  const proc = spawn(file, args, { env: { ...env, XENONET_INSTANCE: "installed" }, stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
  const pending = new Map();
  let buffer = "", seq = 0;
  proc.stdout.on("data", (data) => {
    buffer += data;
    for (let end; (end = buffer.indexOf("\n")) >= 0;) {
      const message = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1);
      const p = pending.get(message.id);
      if (p) { clearTimeout(p.timer); pending.delete(message.id); p.resolve(message); }
    }
  });
  const call = (method, params) => new Promise((resolve, reject) => {
    const id = ++seq;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP ${method} did not answer.`)); }, 30000);
    pending.set(id, { resolve, timer });
    proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  });
  await call("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "component-test", version: "1" } });
  proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
  return { proc, list: () => call("tools/list", {}), call: (name, args = {}) => call("tools/call", { name, arguments: args }), close: () => { proc.stdin.end(); proc.kill(); for (const p of pending.values()) clearTimeout(p.timer); } };
}
