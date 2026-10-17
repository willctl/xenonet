// core/components/rpc.js in Rust (crates/xenonet-components/src/rpc.rs): a
// private, bounded local connection, JSON a line. The framing and its rules
// are the Rust's; the socket, the requests waiting for answers and their
// timers are here. A request interrupted after it was sent is never sent
// again: an action may already have taken effect.
import { EventEmitter } from "node:events";
import { native } from "../../index.js";

const C = native.compConstants();

export function rpc(socket, handle = async () => { throw new Error("unsupported request"); }, { incomingEvents = [], maxLine = C.maxLine } = {}) {
  const core = new native.CompRpc(Number(maxLine), incomingEvents.filter((e) => typeof e === "string"));
  const events = new EventEmitter();
  const pending = new Map();
  let closed = false;
  const send = (value) => {
    const r = core.line(value, closed || socket.destroyed, Number(socket.writableLength ?? 0));
    if (r.error) throw new Error(r.error);
    socket.write(r.ok);
  };
  socket.setEncoding("utf8");
  socket.on("data", (chunk) => {
    for (const m of core.data(String(chunk))) {
      if (m.destroy) {
        socket.destroy();
        break;
      }
      if ("reply" in m) {
        const p = pending.get(m.reply);
        if (p) {
          clearTimeout(p.timer);
          pending.delete(m.reply);
          m.error !== undefined ? p.reject(new Error(m.error)) : p.resolve(m.result);
        }
      } else if ("method" in m) {
        Promise.resolve().then(() => handle(m.method, m.params)).then(
          (result) => send({ reply: m.id, result: result ?? null }),
          (error) => send({ reply: m.id, error: error.message }),
        ).catch(() => socket.destroy());
      // Wire events can't pass for the transport's own or EventEmitter's
      // special error event: the Rust lets only incomingEvents through.
      } else events.emit(m.event, m.value);
    }
  });
  const disconnect = () => {
    if (closed) return;
    closed = true;
    for (const p of pending.values()) {
      clearTimeout(p.timer);
      p.reject(new Error("XenoNet disconnected; the action was not retried."));
    }
    pending.clear();
    events.emit("disconnect");
  };
  socket.on("error", disconnect);
  socket.on("close", disconnect);
  return Object.assign(events, {
    request(method, params, timeout = 150000) {
      return new Promise((resolve, reject) => {
        const id = core.nextId();
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`${method} did not answer; the action was not retried.`));
        }, timeout);
        pending.set(id, { resolve, reject, timer });
        try {
          send({ id, method, params });
        } catch (err) {
          clearTimeout(timer);
          pending.delete(id);
          reject(err);
        }
      });
    },
    notify: (event, value) => send({ event, value }),
    close: () => socket.destroy(),
  });
}
