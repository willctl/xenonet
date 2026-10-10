// The adapter's way to the service (docs/design/cloud-assistants.md,
// section 4): `xenonet mcp assistant --grant <grant>` finds the private
// endpoint (server/assistant-endpoint.js) by the XenoNet it's for, reads its
// grant's credential from that XenoNet's state folder (credentials.js), and
// says hello with it, over 127.0.0.1 only. One connection is kept for its
// calls; when it fails, the next try waits two seconds, then four, and so
// on up to a minute, with some jitter, and calls meanwhile are answered at
// once as SERVICE_UNAVAILABLE, with when to try again.
//
// Every answer is an envelope: a result, or an error with a fixed code
// (service.js), which the adapter passes on as it is. A credential never
// leaves this process except in hello.
import fs from "node:fs";
import net from "node:net";
import { rpc } from "../components/rpc.js";
import { ASSISTANT_API, ERROR_CODES, GRANT_ID } from "../schema/assistant.js";
import { endpointFile, MAX_LINE } from "./protocol.js";
import { readCredential, isDisabled, isMarkedRevoked, revisionOf } from "./credentials.js";
import { errorEnvelope } from "./service.js";

export const CALL_MS = 12e3;
const BACKOFF = { first: 2000, max: 60e3 };

export class AssistantRefused extends Error {
  constructor(code, detail = code) {
    super(detail);
    this.code = code;
  }
}

// Whether the process that wrote the endpoint's file still runs, as this
// account's: a file left by a service that crashed names a port anyone
// could listen on since, and a credential never goes there.
const ours = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

export function assistantClient({ grant, env = process.env, now = Date.now, random = Math.random, connect = (port) => net.connect({ host: "127.0.0.1", port }), alive = ours }) {
  if (!GRANT_ID.test(String(grant))) throw new Error("--grant takes a grant's id (gr_…), as XenoNet's window shows it");
  let client = null;
  let hello = null;
  let opening = null;
  let failures = 0;
  let retryAt = 0;
  // The state folder the endpoint named last, for standing() when it's gone.
  let stateDir = null;

  async function open() {
    let descriptor;
    try {
      descriptor = JSON.parse(fs.readFileSync(endpointFile(env), "utf8"));
    } catch {
      throw new AssistantRefused("SERVICE_UNAVAILABLE", "XenoNet's endpoint for remote actors isn't listening: XenoNet isn't running, or no grant is active");
    }
    if (!Number.isInteger(descriptor?.port) || typeof descriptor.state !== "string") throw new AssistantRefused("SERVICE_UNAVAILABLE", "XenoNet's endpoint file is unreadable");
    stateDir = descriptor.state;
    if (!Number.isInteger(descriptor.pid) || !alive(descriptor.pid)) throw new AssistantRefused("SERVICE_UNAVAILABLE", "XenoNet's endpoint file is from a service that isn't running");
    const credential = readCredential(descriptor.state, grant);
    if (!credential) throw new AssistantRefused("NOT_AUTHORIZED", "this computer has no credential for that grant: it was revoked, or made by another XenoNet");
    const socket = connect(descriptor.port);
    const c = rpc(socket, undefined, { maxLine: MAX_LINE });
    try {
      const info = await c.request("hello", { role: "assistant", api: ASSISTANT_API, grant_id: grant, token: credential.token }, 5000);
      c.on("disconnect", () => {
        if (client === c) (client = null), (hello = null);
      });
      return { c, info };
    } catch (err) {
      c.close();
      const code = ERROR_CODES.includes(err.message) ? err.message : "SERVICE_UNAVAILABLE";
      throw new AssistantRefused(code, err.message);
    }
  }

  async function connection() {
    if (client) return client;
    const t = now();
    if (t < retryAt) throw new AssistantRefused("SERVICE_UNAVAILABLE", "waiting before trying XenoNet again");
    opening ??= open()
      .then(({ c, info }) => {
        [client, hello, failures, retryAt] = [c, info, 0, 0];
        return c;
      })
      .catch((err) => {
        // A refusal for good (revoked, not allowed) isn't retried sooner
        // than an outage would be.
        failures++;
        const base = Math.min(BACKOFF.max, BACKOFF.first * 2 ** (failures - 1));
        retryAt = now() + Math.round(base * (0.8 + 0.4 * random()));
        throw err;
      })
      .finally(() => (opening = null));
    return opening;
  }

  return {
    // One read: { ok, result } or { ok: false, error }, envelopes both.
    async read(operation, args, requestId) {
      let c;
      try {
        c = await connection();
      } catch (err) {
        const code = err instanceof AssistantRefused ? err.code : "SERVICE_UNAVAILABLE";
        return { ok: false, error: errorEnvelope(requestId, code, code === "SERVICE_UNAVAILABLE" ? { retryAfterMs: Math.max(1000, retryAt - now()) } : {}), detail: err.message };
      }
      try {
        const r = await c.request("assistant.read", { request_id: requestId, operation, arguments: args }, CALL_MS);
        if (r && typeof r === "object" && (r.ok === true || r.ok === false)) return r;
        return { ok: false, error: errorEnvelope(requestId, "INTERNAL_ERROR") };
      } catch (err) {
        // Interrupted after it was sent: not sent again (rpc.js); a read
        // can be asked for again by the caller.
        return { ok: false, error: errorEnvelope(requestId, /did not answer/.test(err.message) ? "TIMEOUT" : "SERVICE_UNAVAILABLE"), detail: err.message };
      }
    },
    // Whether the grant still stands as far as this computer's files say,
    // with no service to ask: not revoked (its credential and tombstone),
    // not expired, cloud access not turned off, and at which revision. The
    // adapter asks before it serves anything from its cache. { ok,
    // revision } (null when there's no record of one) or { ok: false, code }.
    standing() {
      if (!stateDir) return { ok: false, code: "SERVICE_UNAVAILABLE" };
      if (isDisabled(stateDir)) return { ok: false, code: "DISABLED" };
      if (isMarkedRevoked(stateDir, grant)) return { ok: false, code: "GRANT_REVOKED" };
      const credential = readCredential(stateDir, grant);
      if (!credential) return { ok: false, code: "GRANT_REVOKED" };
      if (credential.expiresAt != null && credential.expiresAt <= now()) return { ok: false, code: "GRANT_EXPIRED" };
      return { ok: true, revision: revisionOf(stateDir, grant) };
    },
    // What hello said (device, actor, connection, revision), once connected.
    info: () => hello,
    close() {
      client?.close();
      client = null;
    },
  };
}
