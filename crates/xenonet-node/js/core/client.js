// core/client.js in Rust (crates/xenonet-base/src/client.rs): the client
// the xenonet command and the MCP servers reach a running XenoNet with.
// The Rust makes its own requests, to 127.0.0.1; a server whose base isn't
// that is left to the JS.
import { native } from "../index.js";
import * as js from "../../../../core/client.js";

const C = native.clientConstants();
export const PORTS = { xenonet: C.xenonet, demo: C.demo, dev: C.dev };
export const HEALTH_MS = C.healthMs;
export const NAMED_HEALTH_MS = C.namedHealthMs;

// The options as JS reads them: a port that's there (truthy) as Number()
// makes it, and XENONET_PORT when it's set and not empty.
const pick = ({ port, demo, dev, env = process.env } = {}) => ({ port: port ? Number(port) : undefined, demo: !!demo, dev: !!dev, envPort: env.XENONET_PORT ? String(env.XENONET_PORT) : undefined });

export const candidates = (opts) => native.clientCandidates(pick(opts));

export const healthMs = (opts) => native.clientHealthMs(pick(opts));

export async function find(opts = {}, { answerMs = healthMs(opts) } = {}) {
  const found = await native.clientFind(candidates(opts), Number(answerMs));
  return found && { base: `http://127.0.0.1:${found.port}`, port: found.port, health: found.health };
}

export async function need(opts = {}) {
  const s = await find(opts);
  if (!s) throw new Error(native.clientNotRunning(candidates(opts)));
  return s;
}

export const nameOf = (s) => s.health?.name ?? native.clientDefaultName(typeof s.port === "number" ? s.port : NaN);

export async function request(s, method, route, body, headers = {}) {
  if (s.base !== `http://127.0.0.1:${s.port}` || typeof route !== "string") return js.request(s, method, route, body, headers);
  const entries = Object.entries(headers ?? {}).map(([k, v]) => [k, String(v)]);
  const answer = await native.clientRequest(Number(s.port), String(s.health?.name ?? "XenoNet"), String(method), route, body, entries);
  if (answer.error) throw Object.assign(new Error(answer.error.message), answer.error.status === undefined ? {} : { status: answer.error.status });
  return answer.ok;
}

export const get = (s, route) => request(s, "GET", route);
export const post = (s, route, body = {}, headers) => request(s, "POST", route, body, headers);
export const del = (s, route) => request(s, "DELETE", route);
