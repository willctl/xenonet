// Whether the actor gateway's address reaches it from the internet, as a
// cloud agent would call it: the name looked up in the internet's DNS
// (public resolvers, not this computer's own, which Tailscale's MagicDNS
// or a hosts file may answer for), then a request to https://<name>/mcp at
// the address it gave, carrying a one-time word the gateway answers only
// while XenoNet waits for it (server/actor-gateway.js). So a yes means this
// XenoNet's gateway answered, through whatever tunnel is in between, and a
// no says where it stopped.
import dns from "node:dns";
import http from "node:http";
import https from "node:https";

export const PROBE_HEADER = "x-xenonet-probe";
const RESOLVERS = ["1.1.1.1", "8.8.8.8", "9.9.9.9"];

// The first address the internet's DNS has for `host`: { address, family },
// or { error } when there's none ("missing") or DNS didn't answer.
export async function publicAddress(host, { servers = RESOLVERS, timeoutMs = 3000 } = {}) {
  const r = new dns.promises.Resolver({ timeout: timeoutMs, tries: 2 });
  r.setServers(servers);
  let failed = null;
  for (const [family, look] of [
    [4, () => r.resolve4(host)],
    [6, () => r.resolve6(host)],
  ]) {
    try {
      const got = await look();
      if (got.length) return { address: got[0], family };
    } catch (err) {
      if (!["ENODATA", "ENOTFOUND"].includes(err.code)) failed = err.code ?? "error";
    }
  }
  return { error: failed ?? "missing" };
}

// One GET of /mcp with the word, at `address` for `host` (TLS checked for
// that name), or at the gateway's own port when `local`: { status, echoed }
// or { error } (a code such as ECONNREFUSED, or a TLS one).
export function probeRequest({ host, address = null, family = 4, nonce, local = null, timeoutMs = 8000 }) {
  return new Promise((resolve) => {
    const headers = { host, [PROBE_HEADER]: nonce, "user-agent": "xenonet-gateway-check", accept: "application/json" };
    const lookup = (_name, opts, cb) => (opts?.all ? cb(null, [{ address, family }]) : cb(null, address, family));
    const req = local
      ? http.request({ host: "127.0.0.1", port: local, path: "/mcp", method: "GET", headers, timeout: timeoutMs })
      : https.request({ hostname: host, port: 443, path: "/mcp", method: "GET", headers, lookup, timeout: timeoutMs });
    req.on("response", (res) => {
      res.resume();
      resolve({ status: res.statusCode, echoed: String(res.headers[PROBE_HEADER] ?? "") });
    });
    req.on("timeout", () => req.destroy(Object.assign(new Error("timed out"), { code: "ETIMEDOUT" })));
    req.on("error", (err) => resolve({ error: err.code ?? err.message }));
    req.end();
  });
}

const TLS = /CERT|SSL|TLS|SELF_SIGNED|ALTNAME/;

// The check, in words: { ok, stage, why }. `expect(nonce)` tells the
// gateway which word to answer. `local`: the gateway's port, to check the
// loop on this computer alone (XENONET_GATEWAY_PROBE=local, for tests).
export async function checkReach({ host, expect, nonce, local = null, lookup = publicAddress, request = probeRequest }) {
  if (!host) return { ok: false, stage: "address", why: "The gateway has no address yet." };
  let where = { address: null, family: 4 };
  if (!local) {
    where = await lookup(host);
    if (where.error === "missing") return { ok: false, stage: "dns", why: `${host} isn't in public DNS yet. A new tunnel's name can take a few minutes.` };
    if (where.error) return { ok: false, stage: "dns", why: `Public DNS didn't answer (${where.error}). Is this computer online?` };
  }
  expect(nonce);
  const got = await request({ host, address: where.address, family: where.family, nonce, local });
  if (got.error) {
    if (TLS.test(got.error)) return { ok: false, stage: "tls", why: `${host} answered with a certificate for another name (${got.error}).` };
    return { ok: false, stage: "connect", why: `Nothing answered at ${host} (${got.error}). Is the tunnel running?` };
  }
  if (got.status === 204 && got.echoed === nonce) return { ok: true, stage: "ok", why: null };
  if (got.status === 421) return { ok: false, stage: "host", why: "The tunnel changes the Host header, which the gateway refuses. Have it pass the header on unchanged." };
  if (got.status === 502 || got.status === 503 || got.status === 504) return { ok: false, stage: "answer", why: `The tunnel answers, but can't reach the gateway on this computer (HTTP ${got.status}).` };
  return { ok: false, stage: "answer", why: `Something else answers at ${host} (HTTP ${got.status}).` };
}
