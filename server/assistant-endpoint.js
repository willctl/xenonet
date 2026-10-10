// The private endpoint remote actors' adapters reach XenoNet through
// (docs/design/cloud-assistants.md, section 4): `xenonet mcp assistant`,
// which a tunnel client runs, connects here on 127.0.0.1, says which
// grant it's for with that grant's credential, and then may ask one thing,
// assistant.read (core/assistant/service.js). It's apart from the
// component service (component-service.js), whose credential the adapter
// never has, and from the HTTP API, whose routes it can't reach.
//
//   hello          {role: "assistant", api, grant_id, token}, and nothing
//                  else, within five seconds of connecting
//   assistant.read {request_id, operation, arguments}, once let in
//
// Anything else is refused, before hello and after, with nothing more
// said than that. A grant revoked, expired, given a new credential or
// turned off lets go of its adapters at once; the tombstones written
// without the service (xenonet assistant disable, revoke) are looked at
// every two seconds too. It listens only while a grant could be used, and
// says where in runtime/assistant-<port>.json, which holds no secret.
import net from "node:net";
import fs from "node:fs";
import path from "node:path";
import { rpc } from "../core/components/rpc.js";
import { negotiate } from "../core/components/protocol.js";
import { protectFile } from "../core/private-file.js";
import { createBucket } from "../core/actors/limits.js";
import { ASSISTANT_API, ERROR_CODES, GRANT_ID } from "../core/schema/assistant.js";
import { endpointFile, MAX_LINE, HELLO_KEYS } from "../core/assistant/protocol.js";

const HELLO_MS = 5000;
const SWEEP_MS = 2000;

// `connections`: grants (core/actors/connections.js); `service`: the reads.
export function assistantEndpoint({ connections, service, audit = null, env = process.env, protect = protectFile, log = () => {} }) {
  let server = null;
  let starting = null;
  const sessions = new Set();
  // Refused hellos, from anyone: 20 a minute, then none for a minute.
  const refusals = createBucket({ rate: 20 / 60, burst: 20 });
  let quietUntil = 0;

  function drop(match) {
    for (const s of [...sessions]) if (match(s)) s.client.close();
  }

  function accept(socket) {
    let session = null;
    const deadline = setTimeout(() => socket.destroy(), HELLO_MS);
    const client = rpc(
      socket,
      async (method, params) => {
        if (session) {
          if (method !== "assistant.read") throw new Error("Unsupported request.");
          return service.read(session, params);
        }
        if (method !== "hello") throw new Error("XenoNet refused this connection.");
        if (Date.now() < quietUntil) throw new Error("RATE_LIMITED");
        const refused = (code, reason) => {
          audit?.access({ grant: GRANT_ID.test(String(params?.grant_id)) ? params.grant_id : null, operation: "hello", result: reason });
          if (refusals.take()) quietUntil = Date.now() + 60e3;
          throw new Error(code);
        };
        if (!params || typeof params !== "object" || Array.isArray(params) || Object.keys(params).sort().join() !== HELLO_KEYS.join() || params.role !== "assistant") refused("NOT_AUTHORIZED", "bad-hello");
        try {
          negotiate(ASSISTANT_API, params.api);
        } catch {
          refused("INCOMPATIBLE_VERSION", "version");
        }
        const s = connections.authenticate({ grant: params.grant_id, token: params.token });
        if (!s.ok) refused(ERROR_CODES.includes(s.code) ? s.code : "NOT_AUTHORIZED", s.reason);
        clearTimeout(deadline);
        session = { client, grantId: s.grant.grant_id, actorId: s.actorId, connectionId: s.connection.connection_id };
        sessions.add(session);
        connections.bridge(session.connectionId, +1);
        audit?.access({ grant: session.grantId, revision: s.revision, actor: session.actorId, connection: session.connectionId, operation: "hello", result: "ok" });
        const p = connections.policy(s);
        return { assistant_api: 1, device_id: p.deviceId, actor_id: s.actorId, connection_id: session.connectionId, grant_revision: s.revision, permissions: p.permissions };
      },
      { maxLine: MAX_LINE },
    );
    client.on("disconnect", () => {
      clearTimeout(deadline);
      if (session && sessions.delete(session)) connections.bridge(session.connectionId, -1);
    });
  }

  function writeDescriptor(port) {
    const file = endpointFile(env);
    fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
    const tmp = `${file}.${process.pid}.tmp`;
    try {
      fs.writeFileSync(tmp, "", { flag: "wx", mode: 0o600 });
      protect(tmp);
      fs.writeFileSync(tmp, JSON.stringify({ port, pid: process.pid, api: ASSISTANT_API, state: connections.dir }));
      fs.renameSync(tmp, file);
    } catch (err) {
      fs.rmSync(tmp, { force: true });
      throw err;
    }
  }
  function removeDescriptor() {
    try {
      const file = endpointFile(env);
      if (JSON.parse(fs.readFileSync(file, "utf8")).pid === process.pid) fs.unlinkSync(file);
    } catch {}
  }

  async function start() {
    if (server) return;
    starting ??= (async () => {
      const s = net.createServer(accept);
      await new Promise((resolve, reject) => {
        s.once("error", reject);
        s.listen(0, "127.0.0.1", resolve);
      });
      try {
        writeDescriptor(s.address().port);
      } catch (err) {
        s.close();
        throw err;
      }
      server = s;
      log(`remote actors' endpoint on 127.0.0.1:${s.address().port}`);
    })().finally(() => (starting = null));
    return starting;
  }

  function stop() {
    drop(() => true);
    if (!server) return;
    server.close();
    server = null;
    removeDescriptor();
  }

  // Listening while a grant could be used, and not otherwise.
  async function sync() {
    if (connections.anyActive()) await start().catch((err) => log(`remote actors' endpoint didn't start: ${err.message}`));
    else stop();
  }

  // Sessions whose grant no longer stands, let go; the tombstones written
  // without the service count.
  function sweep() {
    for (const s of [...sessions]) if (!connections.recheck(s.grantId, -1).ok) s.client.close();
    sync();
  }

  const off = (grant) => {
    drop((s) => s.grantId === grant);
    service.forget?.(grant);
  };
  connections.on("revoked", ({ grant }) => (off(grant), sync()));
  connections.on("rotated", ({ grant }) => off(grant));
  connections.on("expired", ({ grant }) => (off(grant), sync()));
  connections.on("disabled", () => stop());
  connections.on("enabled", () => sync());
  connections.on("changed", () => sync());

  const timer = setInterval(sweep, SWEEP_MS);
  timer.unref?.();

  return {
    sync,
    stop() {
      clearInterval(timer);
      stop();
    },
    // How many adapters are connected, and where it listens (for tests and
    // the doctor).
    info: () => ({ listening: !!server, port: server?.address()?.port ?? null, sessions: sessions.size }),
  };
}
