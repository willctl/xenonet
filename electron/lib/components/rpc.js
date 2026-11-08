// A private, bounded local connection. Requests interrupted after sending
// are never replayed: an action may already have taken effect.
import { EventEmitter } from "node:events";

// `maxLine`: the longest message either way, in bytes (16 MiB unless a
// connection says less, as the assistant's does).
export function rpc(socket, handle = async () => { throw new Error("unsupported request"); }, { incomingEvents = [], maxLine = 16 << 20 } = {}) {
  const events = new EventEmitter();
  const pending = new Map();
  let sequence = 0;
  let buffer = "";
  let closed = false;
  const send = (value) => {
    if (closed || socket.destroyed) throw new Error("XenoNet disconnected; the action was not retried.");
    const line = JSON.stringify(value) + "\n";
    if (Buffer.byteLength(line) > maxLine || socket.writableLength > 16 << 20) throw new Error("XenoNet's connection is too busy.");
    socket.write(line);
  };
  socket.setEncoding("utf8");
  socket.on("data", (chunk) => {
    buffer += chunk;
    if (Buffer.byteLength(buffer) > maxLine) return socket.destroy();
    for (;;) {
      const end = buffer.indexOf("\n");
      if (end < 0) break;
      const line = buffer.slice(0, end);
      buffer = buffer.slice(end + 1);
      let message;
      try { message = JSON.parse(line); } catch { socket.destroy(); break; }
      if (!message || typeof message !== "object") { socket.destroy(); break; }
      if (message.reply) {
        const p = pending.get(message.reply);
        if (p) { clearTimeout(p.timer); pending.delete(message.reply); message.error ? p.reject(new Error(message.error)) : p.resolve(message.result); }
      } else if (typeof message.method === "string" && Number.isSafeInteger(message.id)) {
        Promise.resolve().then(() => handle(message.method, message.params)).then(
          (result) => send({ reply: message.id, result: result ?? null }),
          (error) => send({ reply: message.id, error: error.message }),
        ).catch(() => socket.destroy());
      // Wire events cannot impersonate transport lifecycle events or emit
      // EventEmitter's special error event. Servers accept none by default.
      } else if (incomingEvents.includes(message.event)) events.emit(message.event, message.value);
    }
  });
  const disconnect = () => {
    if (closed) return;
    closed = true;
    for (const p of pending.values()) { clearTimeout(p.timer); p.reject(new Error("XenoNet disconnected; the action was not retried.")); }
    pending.clear();
    events.emit("disconnect");
  };
  socket.on("error", disconnect);
  socket.on("close", disconnect);
  return Object.assign(events, {
    request(method, params, timeout = 150000) {
      return new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} did not answer; the action was not retried.`)); }, timeout);
        pending.set(id, { resolve, reject, timer });
        try { send({ id, method, params }); } catch (err) { clearTimeout(timer); pending.delete(id); reject(err); }
      });
    },
    notify: (event, value) => send({ event, value }),
    close: () => socket.destroy(),
  });
}
