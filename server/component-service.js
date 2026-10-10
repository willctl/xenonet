import net from "node:net";
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { rpc } from "../core/components/rpc.js";
import { SERVICE_API, CONSOLE_API, CAPABILITIES, negotiate } from "../core/components/protocol.js";
import { descriptorFile } from "../core/components/runtime.js";
import { protectFile } from "../core/private-file.js";
import { createTerminals } from "./terminals.js";

// `onDesktop(socket)` is told of the connected Desktop's connection, and
// says whether it's XenoNet's app (core/engine/verify.js, desktop); null when
// the Desktop goes.
export async function componentService({ version, windowKey, windowState, shutdown, onDesktop = async () => ({ ok: true }) }) {
  const terminals = createTerminals();
  const token = crypto.randomBytes(32).toString("hex");
  const clients = new Set();
  let desktop = null;
  const info = () => ({ version, pid: process.pid, api: SERVICE_API, consoleApi: CONSOLE_API, capabilities: CAPABILITIES, desktop: !!desktop });
  const server = net.createServer((socket) => {
    let authenticated = false;
    let role;
    const deadline = setTimeout(() => socket.destroy(), 5000);
    const client = rpc(socket, async (method, params) => {
      if (!authenticated) {
        if (method !== "hello" || typeof params?.token !== "string" || params.token.length !== token.length || !crypto.timingSafeEqual(Buffer.from(params.token), Buffer.from(token))) throw new Error("XenoNet refused this connection.");
        negotiate(SERVICE_API, params.api);
        if (!["desktop", "tools"].includes(params.role)) throw new Error("Unknown XenoNet component.");
        if (params.role === "desktop" && desktop) throw new Error("XenoNet already has a connected Desktop.");
        authenticated = true; role = params.role; clearTimeout(deadline);
        if (role !== "desktop") return info();
        desktop = client;
        // The window key goes only to a Desktop the service can tell is an
        // app's own process, which the page's requests then come from.
        const seen = await onDesktop(socket).catch(() => ({ ok: false }));
        return { ...info(), ...(seen.ok ? { windowKey: windowKey.reveal() } : {}) };
      }
      if (method === "status") return { ...info(), terminals: terminals.list() };
      if (method === "console") {
        if (!desktop) throw new Error("XenoNet Desktop is closed or restarting. The service is still running.");
        return desktop.request("console", params);
      }
      if (method === "stop") {
        if (terminals.list().some((t) => !t.exited) && !params?.force) throw new Error("XenoNet has running terminals. Close them first, or use --force.");
        setTimeout(shutdown, 100); return { stopping: true };
      }
      if (role !== "desktop") throw new Error("This request belongs to XenoNet Desktop.");
      if (method === "window") { windowState(params); return null; }
      if (method === "terminal.profiles") return terminals.profiles();
      if (method === "terminal.list") return terminals.list();
      if (method === "terminal.create") return terminals.create(params);
      if (method === "terminal.read") return terminals.read(params);
      if (method === "terminal.write") return terminals.write(params);
      if (method === "terminal.resize") return terminals.resize(params);
      if (method === "terminal.kill") return terminals.kill(params.id);
      throw new Error(`Unsupported service request: ${method}`);
    });
    clients.add(client);
    client.on("disconnect", () => { clearTimeout(deadline); clients.delete(client); if (desktop === client) { desktop = null; windowState("closed"); onDesktop(null).catch(() => {}); } });
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const file = descriptorFile();
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  const tmp = `${file}.${process.pid}.tmp`;
  try {
    fs.writeFileSync(tmp, "", { flag: "wx", mode: 0o600 });
    protectFile(tmp);
    fs.writeFileSync(tmp, JSON.stringify({ port: server.address().port, token, pid: process.pid, version }));
    fs.renameSync(tmp, file);
  } catch (err) { clients.forEach((c) => c.close()); server.close(); throw err; }
  return {
    info,
    notify: (value) => { try { desktop?.notify("notification", value); } catch {} },
    // Has the connected Desktop open an app's link, a folder or a page; false
    // when there's none, or it's too old to (server/index.js opens it then).
    open: async (target) => {
      if (!desktop) return false;
      try { await desktop.request("open", { target }); return true; } catch { return false; }
    },
    close() {
      clients.forEach((c) => c.close()); server.close(); terminals.killAll();
      try { if (JSON.parse(fs.readFileSync(file, "utf8")).pid === process.pid) fs.unlinkSync(file); } catch {}
    },
  };
}
