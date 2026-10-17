// What the components' Rust tasks ask of the machine: the install lock's
// look at a process, and the lifecycle of a service update (connecting to
// the service, starting a runtime, a staged component's startup check, the
// clock), done with the operations given or the twins' own.

// Whether a process is there: process.kill(pid, 0), which throws ESRCH
// when it isn't.
export const lockOps = async (q) => {
  if (q.op !== "alive") throw new Error(`the install lock asked for ${q.op}`);
  process.kill(q.pid, 0);
  return true;
};

// `defaults`: { connect, start, check }; `overrides`: what a test gave.
export function lifecycleOps(defaults, overrides, env) {
  const io = { now: Date.now, pause: (ms) => new Promise((r) => setTimeout(r, ms)), ...defaults, ...overrides };
  const clients = new Map();
  let next = 0;
  return async (q) => {
    switch (q.op) {
      case "connect": {
        const client = await io.connect({ env });
        if (!client) return { client: null };
        const id = ++next;
        clients.set(id, client);
        return { client: id, info: client.info ?? null };
      }
      case "request":
        return clients.get(q.client).request(q.method, q.params);
      case "close": {
        const client = clients.get(q.client);
        clients.delete(q.client);
        client?.close();
        return null;
      }
      case "start":
        return io.start(q.runtime);
      case "check":
        return io.check(q.staged);
      case "now":
        return io.now();
      case "pause":
        return io.pause(q.ms);
      case "alive":
        return lockOps(q);
      default:
        throw new Error(`a component update asked for ${q.op}`);
    }
  };
}

// How long a service gets to stop: 15 s unless the operations say.
export const stopTimeout = (overrides) => Number({ stopTimeout: 15000, ...overrides }.stopTimeout);
