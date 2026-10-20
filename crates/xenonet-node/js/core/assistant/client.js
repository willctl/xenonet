// core/assistant/client.js in Rust (crates/xenonet-assistant/src/
// client.rs): the adapter's way to the service. When to try, the endpoint
// file's checks, the credential's reading and the grant's standing are
// the Rust's; the connection (components/rpc.js) and its requests are
// here. A credential never leaves this process except in hello.
import fs from "node:fs";
import net from "node:net";
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { endpointFile, MAX_LINE } from "./protocol.js";
import { errorEnvelope } from "./service.js";

const { rpc } = await import(peer("components/rpc.js"));
const { ASSISTANT_API } = await import(peer("schema/assistant.js"));

export const CALL_MS = native.asstConstants().callMs;

export class AssistantRefused extends Error {
  constructor(code, detail = code) {
    super(detail);
    this.code = code;
  }
}

// Whether the process that wrote the endpoint's file still runs, as this
// account's.
const ours = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

export function assistantClient({ grant, env = process.env, now = Date.now, random = Math.random, connect = (port) => net.connect({ host: "127.0.0.1", port }), alive = ours }) {
  const core = native.AsstAdapter.of(grant);
  let client = null;
  let hello = null;
  let opening = null;

  async function open() {
    let text = null;
    try {
      text = fs.readFileSync(endpointFile(env), "utf8");
    } catch {}
    const e = core.endpoint(text, (pid) => !!alive(pid));
    if (e.error) throw new AssistantRefused(e.error.code, e.error.detail);
    const t = core.token();
    if (t.error) throw new AssistantRefused(t.error.code, t.error.detail);
    const socket = connect(e.ok.port);
    const c = rpc(socket, undefined, { maxLine: MAX_LINE });
    try {
      const info = await c.request("hello", { role: "assistant", api: ASSISTANT_API, grant_id: grant, token: t.ok }, 5000);
      c.on("disconnect", () => {
        if (client === c) (client = null), (hello = null);
      });
      return { c, info };
    } catch (err) {
      c.close();
      throw new AssistantRefused(native.asstHelloCode(String(err.message)), err.message);
    }
  }

  async function connection() {
    if (client) return client;
    if (!core.mayTry(now())) throw new AssistantRefused("SERVICE_UNAVAILABLE", "waiting before trying XenoNet again");
    opening ??= open()
      .then(({ c, info }) => {
        [client, hello] = [c, info];
        core.connected();
        return c;
      })
      .catch((err) => {
        core.failed(now(), random());
        throw err;
      })
      .finally(() => (opening = null));
    return opening;
  }

  return {
    async read(operation, args, requestId) {
      let c;
      try {
        c = await connection();
      } catch (err) {
        const code = err instanceof AssistantRefused ? err.code : "SERVICE_UNAVAILABLE";
        return { ok: false, error: errorEnvelope(requestId, code, code === "SERVICE_UNAVAILABLE" ? { retryAfterMs: Math.max(1000, core.retryAt - now()) } : {}), detail: err.message };
      }
      try {
        const r = await c.request("assistant.read", { request_id: requestId, operation, arguments: args }, CALL_MS);
        if (native.asstIsAnswer(r)) return r;
        return { ok: false, error: errorEnvelope(requestId, "INTERNAL_ERROR") };
      } catch (err) {
        // Interrupted after it was sent: not sent again (rpc.js).
        return { ok: false, error: errorEnvelope(requestId, native.asstReadCode(String(err.message))), detail: err.message };
      }
    },
    standing: () => core.standing(now()),
    info: () => hello,
    close() {
      client?.close();
      client = null;
    },
  };
}
