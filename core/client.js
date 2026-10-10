// A client for a running XenoNet's HTTP API, for the xenonet command and the
// MCP servers. XenoNet listens on 127.0.0.1: 47840 for XenoNet, 47842 for XenoNet
// Dev and 47841 for its demo. A port, --dev or --demo picks one; otherwise
// XENONET_PORT, or else the first of XenoNet and XenoNet Dev that answers.
export const PORTS = { xenonet: 47840, demo: 47841, dev: 47842 };

export function candidates({ port, demo, dev, env = process.env } = {}) {
  if (port) return [Number(port)];
  if (demo) return [PORTS.demo];
  if (dev) return [PORTS.dev];
  if (env.XENONET_PORT) return [Number(env.XENONET_PORT)];
  return [PORTS.xenonet, PORTS.dev];
}

// How long a XenoNet has to say it's there. A port nobody listens on answers
// at once either way, so these are for a busy machine, where 600 ms made
// the xenonet command (and its tests) say XenoNet wasn't running when it was.
// Looking for whichever XenoNet runs, each port gets a quick probe. A port
// that's named (--port, --dev, --demo, or XENONET_PORT, as the tests and the
// MCP servers name it) is the XenoNet meant, so it gets longer.
export const HEALTH_MS = 2000;
export const NAMED_HEALTH_MS = 10000;

export const healthMs = ({ port, demo, dev, env = process.env } = {}) => (port || demo || dev || env.XENONET_PORT ? NAMED_HEALTH_MS : HEALTH_MS);

// The first XenoNet that answers, as { base, port, health }, or null.
// `answerMs` stands in for the wait in tests.
export async function find(opts = {}, { answerMs = healthMs(opts) } = {}) {
  const tries = await Promise.all(
    candidates(opts).map(async (port) => {
      try {
        const res = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(answerMs) });
        return res.ok ? { base: `http://127.0.0.1:${port}`, port, health: await res.json() } : null;
      } catch {
        return null;
      }
    }),
  );
  return tries.find(Boolean) ?? null;
}

export async function need(opts = {}) {
  const s = await find(opts);
  if (!s) throw new Error(`XenoNet isn't running (nothing answers on ${candidates(opts).map((p) => `127.0.0.1:${p}`).join(" or ")}). Open XenoNet, or start XenoNet Dev from its checkout with npm run app.`);
  return s;
}

export const nameOf = (s) => s.health?.name ?? (s.port === PORTS.dev ? "XenoNet Dev" : s.port === PORTS.demo ? "XenoNet (demo)" : "XenoNet");

// A call to the API; an error answer becomes an Error with XenoNet's message.
export async function request(s, method, route, body, headers = {}) {
  headers = { "x-xenonet-api-version": "1", ...headers };
  const res = await fetch(s.base + route, body === undefined ? { method, headers } : { method, headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) }).catch(() => {
    throw new Error(`${s.health?.name ?? "XenoNet"} stopped answering on 127.0.0.1:${s.port} (restarting, or quit); try again in a moment`);
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw Object.assign(new Error(data.error ?? `${route}: ${res.status} ${res.statusText}`), { status: res.status });
  return data;
}

export const get = (s, route) => request(s, "GET", route);
export const post = (s, route, body = {}, headers) => request(s, "POST", route, body, headers);
export const del = (s, route) => request(s, "DELETE", route);
