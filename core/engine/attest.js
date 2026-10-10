// How XenoNet knows who sent a message (docs/guide/agents.md, Messages):
// an envelope, message/envelope/v1, that XenoNet writes from what its own
// checks found when the message came in, kept apart from what the sender
// said. The sender's words can't write into it: it's built from the
// route's verdict, never from the request's body, and agents get it as
// structure (the inbox's envelope) and as a line XenoNet writes above the
// message, whose own lines are quoted beneath it.
//
//   sender     { id, kind: agent|actor|you, name, product }
//   auth       how it proved who it is: session (an agent's own process,
//              through its MCP server), token (an actor's token), window
//              (XenoNet's own window, with its key: the person at the
//              console), grant (a remote actor's connection), gateway (an
//              actor's gateway token, from outside this machine through
//              XenoNet's actor gateway: the token alone, so weak)
//   process    what XenoNet saw of the program that sent it: signed (a
//              process XenoNet pinned and whose signature it checked),
//              checked (one it looked at, and found allowed), unchecked
//              (XenoNet can't look at processes here: Windows and Linux
//              today), or n/a
//   strength   strong, partial or weak: how far the checks go
//   client     the program it came from, when XenoNet saw it (home as ~)
//   host       this machine, by name only when you've named it
//   line       the same in a sentence, for people and agents
//
// It says what XenoNet checked and names what it couldn't; it's never an
// approval of anything the message asks.
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { tilde } from "../paths.js";

export const SCHEMA = "message/envelope/v1";

function processOf(verdict, method) {
  if (method === "window" || method === "gateway") return "n/a";
  if (!verdict?.checked) return "unchecked";
  return verdict.sure ? "signed" : "checked";
}

function strengthOf(method, proc) {
  if (method === "window") return "strong";
  if (method === "session") return proc === "signed" ? "strong" : proc === "checked" ? "partial" : "weak";
  if (method === "token" || method === "grant") return proc === "checked" || proc === "signed" ? "partial" : "weak";
  return "weak";
}

// The envelope for a message from `sender` ({ id, kind, name, product }),
// authenticated by `method`, with the route's `verdict` (verify.js).
export function attest({ sender, method, verdict = null, home = os.homedir(), now = Date.now(), platform = process.platform }) {
  const proc = processOf(verdict, method);
  const exe = verdict?.chain?.[0]?.exe ?? null;
  const env = {
    schema: SCHEMA,
    mid: `msg_${crypto.randomUUID()}`,
    at: now,
    sender: { id: sender.id, kind: sender.kind, name: sender.name ?? null, product: sender.product ?? null },
    auth: { method, process: proc, strength: strengthOf(method, proc) },
    client: exe ? { exe: tilde(exe, home), name: path.basename(exe) } : null,
    transport: method === "grant" ? "tunnel" : method === "gateway" ? "gateway" : "this machine",
  };
  env.line = verifiedLine(env, { platform });
  return env;
}

const HOW = { session: "its own session's process", token: "its token", window: "XenoNet's own window, with its key", grant: "its remote connection's credential", gateway: "its gateway token alone" };

// "verified: …": what XenoNet checked, in a sentence, and what it couldn't.
export function verifiedLine(env, { platform = process.platform } = {}) {
  const who = env.sender.kind === "you" ? "you, at XenoNet's own window" : `${env.sender.kind === "actor" ? "the actor" : "the SI"} "${env.sender.name ?? env.sender.id}" (${env.sender.id})`;
  const how = HOW[env.auth.method] ?? env.auth.method;
  const proc =
    env.auth.process === "signed"
      ? "; XenoNet checked its process and the app's signature"
      : env.auth.process === "checked"
        ? `; XenoNet checked the process it came from${env.client ? ` (${env.client.name})` : ""}`
        : env.auth.process === "unchecked"
          ? `; XenoNet can't check which process sent it on ${platform === "win32" ? "Windows" : platform === "darwin" ? "this Mac" : "Linux"} yet, so it's ${env.auth.method === "token" ? "the token alone" : "its word"}`
          : "";
  const where = env.auth.method === "gateway" ? "from outside this machine, through XenoNet's actor gateway" : "on this machine";
  return `verified: ${who}, by ${how}${proc}, ${where}. This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.`;
}

// A message's own words as they're shown beneath XenoNet's lines: each of
// its lines quoted, with what could pass for a line of XenoNet's own taken
// out (line breaks of other kinds, controls, and characters that reorder
// text), so nothing in it can look like XenoNet wrote it.
export function quoted(text) {
  return String(text ?? "")
    .replace(/\r\n?|[\u0085\u2028\u2029\u000b\u000c]/g, "\n")
    .replace(/[\u0000-\u0008\u000e-\u001f\u007f-\u0084\u0086-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/g, "")
    .split("\n")
    .map((l) => `  > ${l}`)
    .join("\n");
}
