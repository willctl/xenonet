// The engine's HTTP API, on XenoNet's own port with its protections
// (127.0.0.1 only, the Host check, no cross-origin posts). Agents reach it
// through the MCP server (mcp/xenonet.js), which says who's calling in the
// x-xenonet-agent header; Codex's hooks post to /api/hook/codex (through
// xenonet hook), Claude Code's to /api/hook/claude (see server/index.js); the
// console reads /api/engine/state, the xenonet command /api/engine/agents and
// /api/engine/who; and /api/engine/connect previews, adds and removes XenoNet
// in Claude Code's and Codex's settings (connect.js). Every request in an
// agent's name is checked against the process it comes from, and connecting
// agents only goes through when no agent is asking (core/engine/verify.js).
//
// Registered actors call the same operations as actor:<slug>, with their
// token (Authorization: Bearer, never with x-xenonet-agent), checked as their
// own routes are (server/routes/actors.js, createActorCaller). They're held
// to what you allowed them: sending needs message-agents (60 messages an
// hour, 6 of them to a repo or to all), claiming needs edit-repos. An
// actor's focus is its status, which goes on the feed as an event, so each
// counts against its events, as a heartbeat's new status does. Nothing
// here lets an actor approve anything, and connecting agents is never an
// actor's.
import path from "node:path";
import { isAgentId } from "../../core/engine/engine.js";
import { bearerOf, tokenInUrl } from "../../core/actors/tokens.js";
import { RATES } from "./actors.js";
import { attest } from "../../core/engine/attest.js";
import { parseSelector } from "../../core/delivery/selector.js";

// The capability each operation needs from an actor.
const NEEDS = { send: "message-agents", claim: "edit-repos" };
const ACTOR_BODY = 16 << 10;

// A peers scope: repo, machine, or a selector (core/delivery/selector.js).
const scopeOf = (s) => (s === "machine" ? "machine" : typeof s === "string" && parseSelector(s) ? s : "repo");

export function engineRoutes({ engine, connect, verify, readJson, send, actorCaller = null, actors = null, channels = null }) {
  const ops = {
    whoami: (me) => engine.whoami(me),
    peers: (me, b) => engine.peers(me, { scope: scopeOf(b.scope) }),
    "who-has": (me, b) => engine.whoHas(me, String(b.path ?? "")),
    // With how XenoNet knows who sent it (core/engine/attest.js), from the
    // checks this route made, never from the body.
    send: (me, b, extra) => engine.send(me, { to: b.to, text: b.text, replyTo: b.replyTo }, extra),
    inbox: (me, b) => engine.inbox(me, { waitMs: Math.min(Number(b.waitSeconds) || 0, 120) * 1000 }),
    claim: (me, b) => engine.claim(me, { paths: b.paths, note: b.note, minutes: b.minutes }),
    release: (me, b) => engine.release(me, { paths: Array.isArray(b.paths) && b.paths.length ? b.paths : null }),
    focus: (me, b) => engine.setFocus(me, b.text),
    // A Claude Code session's channel waiting for messages (core/delivery/channel.js).
    channel: async (me, b) => {
      if (!channels || !me.startsWith("claude:")) return { items: [], off: true };
      return { items: await channels.poll(me, Math.min(Number(b.waitSeconds) || 25, 50) * 1000) };
    },
  };

  // An operation called by an actor, with its token: who it is, checked;
  // what it may do; then the operation as actor:<slug>. Errors carry a code.
  async function asActor(req, res, name, op) {
    const fail = (status, code, error, headers = {}, extra = {}) => send(res, status, { error, code, ...extra }, headers);
    if (!actorCaller) return fail(503, "off", "Actors are off here (XENONET_ACTORS=off)");
    const who = await actorCaller.authenticate(req);
    if (!who.ok) return fail(who.status, who.code, who.error, who.headers, who.extra);
    const checked = await actorCaller.check(req, who.auth);
    if (!checked.ok) return fail(checked.status, checked.code, checked.error);
    const { id, slug } = who.auth;
    let body;
    try {
      body = await readJson(req, ACTOR_BODY);
    } catch (err) {
      return err.status === 413 ? fail(413, "too-big", "that body is over the size this route takes") : fail(400, "bad-request", "that body isn't JSON");
    }
    const need = NEEDS[name];
    if (need && !actors?.can(id, need)) return fail(403, "not-allowed", `${name} needs the ${need} capability, which this actor wasn't allowed`);
    if (name === "send") {
      const wide = ["repo", "project", "all"].includes(body.to);
      const wait = (wide ? actorCaller.take(`a:${slug}|wide`, RATES.wide) : 0) || actorCaller.take(`a:${slug}|messages`, RATES.messages);
      if (wait) return fail(429, "too-many", wide ? "an actor sends at most 6 messages an hour to a repo or to all" : "an actor sends at most 60 messages an hour", { "retry-after": String(Math.max(1, Math.ceil(wait / 1000))) });
    }
    if (name === "focus") {
      const wait = actorCaller.take(`a:${slug}|events`, RATES.events);
      if (wait) return fail(429, "too-many", "an actor's focus is its status, an event on the feed, and 60 events a minute per actor, in bursts of up to 120, is the most XenoNet takes", { "retry-after": String(Math.max(1, Math.ceil(wait / 1000))) });
    }
    try {
      const info = actors?.info?.(id);
      const extra = name === "send" ? { envelope: attest({ sender: { id, kind: "actor", name: info?.name ?? slug, product: info?.product?.id ?? null }, method: "token", verdict: checked.verdict }) } : undefined;
      return send(res, 200, await op(id, body, extra));
    } catch (err) {
      return fail(err.status ?? 400, err.code ?? "bad-request", String(err?.message ?? err));
    }
  }

  return async function route(req, res, url) {
    const route = url.pathname;
    try {
      if (tokenInUrl(url)) return send(res, 400, { error: "a token goes in the Authorization header, never in a URL", code: "bad-request" });
      if (req.method === "POST" && route === "/api/hook/codex") {
        const body = await readJson(req).catch(() => ({}));
        if (body.session_id) {
          const ok = await verify.check(req.socket, `codex:${body.session_id}`, { hook: true });
          if (!ok.ok) return send(res, 403, { error: ok.reason });
        }
        const out = await engine.hook("codex", body.hook_event_name, body);
        return out ? send(res, 200, out) : (res.writeHead(204).end(), undefined);
      }
      if (req.method === "GET" && route === "/api/engine/state") return send(res, 200, await engine.snapshot());
      // For people (the xenonet command): every agent at work, and who has a file.
      if (req.method === "GET" && route === "/api/engine/agents") return send(res, 200, await engine.agents());
      if (req.method === "GET" && route === "/api/engine/who") {
        const file = url.searchParams.get("path") ?? "";
        if (!path.isAbsolute(file)) return send(res, 400, { error: "give the file's absolute path" });
        return send(res, 200, await engine.whoHas(null, file));
      }
      if (route === "/api/engine/connect") {
        if (req.method === "GET") return send(res, 200, await connect.preview());
        // Changing Claude Code's and Codex's settings is yours to do, not an
        // agent's, and never an actor's.
        if (bearerOf(req.headers) !== undefined) return send(res, 403, { error: "connecting Claude Code and Codex is yours to do, and an actor can't do it", code: "yours" });
        const you = await verify.person(req.socket);
        if (!you.ok) return send(res, 403, { error: you.reason });
        if (req.method === "POST") return send(res, 200, await connect.apply((await readJson(req)).apps));
        if (req.method === "DELETE") return send(res, 200, await connect.remove());
      }
      const name = route.slice("/api/engine/".length);
      const op = ops[name];
      if (req.method !== "POST" || !op) return send(res, 404, { error: "not found" });
      if (bearerOf(req.headers) !== undefined) return name === "channel" ? send(res, 404, { error: "not found" }) : await asActor(req, res, name, op);
      const me = req.headers["x-xenonet-agent"];
      if (!isAgentId(me)) return send(res, 400, { error: "say which session is calling, in x-xenonet-agent (claude:<session id> or codex:<thread id>)" });
      const ok = await verify.check(req.socket, me);
      if (!ok.ok) return send(res, 403, { error: ok.reason });
      const body = await readJson(req);
      // The agent's own process, where XenoNet could see it; else what the MCP server says.
      engine.touch(me, { app: me.split(":")[0], cwd: typeof body.cwd === "string" ? body.cwd : null, via: "mcp", pid: ok.root?.pid ?? Number(body.pid) });
      const extra = name === "send" ? { envelope: attest({ sender: { id: me, kind: "agent", name: typeof engine.view === "function" ? ((await engine.view(me).catch(() => null))?.title ?? null) : null }, method: "session", verdict: ok }) } : undefined;
      return send(res, 200, await op(me, body, extra));
    } catch (err) {
      return send(res, 400, { error: String(err?.message ?? err) });
    }
  };
}
