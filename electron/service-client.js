import { EventEmitter } from "node:events";
import { connectService } from "../core/components/runtime.js";

// Reconnect the transport after a service update. Never replay an action.
export async function desktopClient(options) {
  const events = new EventEmitter();
  let client;
  let closed = false;
  let timer;
  let delay = 500;
  async function connect() {
    const next = await connectService({ ...options, role: "desktop" });
    if (closed) { next.close(); return; }
    client = next;
    events.info = next.info;
    delay = 500;
    next.on("notification", (note) => events.emit("notification", note));
    next.on("disconnect", () => {
      if (client !== next) return;
      client = null;
      events.emit("disconnect");
      schedule();
    });
    events.emit("connected", next.info);
  }
  function schedule() {
    if (closed) return;
    timer = setTimeout(() => connect().catch(() => { delay = Math.min(delay * 2, 30000); schedule(); }), delay);
    timer.unref();
  }
  events.request = (...args) => client ? client.request(...args) : Promise.reject(new Error("XenoNet Service is restarting. Try again when it reconnects."));
  events.close = () => { closed = true; clearTimeout(timer); client?.close(); };
  await connect();
  return events;
}
