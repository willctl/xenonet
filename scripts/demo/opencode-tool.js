// XenoNet's tools for an OpenCode session, as an OpenCode custom tool
// (https://opencode.ai/docs/custom-tools/). Copied into a project as
// .opencode/tools/xenonet.js, each tool export below is a tool named
// xenonet_<export>: xenonet_peers, xenonet_send, xenonet_reply and
// xenonet_inbox.
//
// OpenCode gives every call the session's id, so XenoNet hears which session
// is calling, as opencode:ses-…, the id it keeps OpenCode sessions under.
// The calls go from OpenCode's own process to XenoNet's API on this machine
// (XENONET_PORT, else the installed XenoNet's 47840), as the MCP server's do
// for Claude Code and Codex. Messages to this session come in the other way,
// through OpenCode's own server, once you allow it: xenonet live allow.
//
// No imports: the arguments are plain JSON Schema, which OpenCode takes
// beside Zod, so the file works without installing anything.

const base = () => `http://127.0.0.1:${Number(process.env.XENONET_PORT) || 47840}`;

// XenoNet's id for an OpenCode session: its ses_… with "-" for "_".
export const agentId = (sessionID) => `opencode:${String(sessionID).replace(/^ses_/, "ses-")}`;

// One call to XenoNet's engine, as this session. An error comes back as text
// the model can read, rather than a failed tool.
export async function call(op, body, ctx, { fetch = globalThis.fetch } = {}) {
  let res;
  try {
    res = await fetch(`${base()}/api/engine/${op}`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-xenonet-agent": agentId(ctx.sessionID) },
      body: JSON.stringify({ ...body, cwd: ctx.directory, pid: process.pid }),
      signal: AbortSignal.timeout(10e3),
    });
  } catch {
    return { error: `XenoNet isn't answering on ${base()}. Tell your user; they can start it.` };
  }
  const data = await res.json().catch(() => ({}));
  return res.ok ? data : { error: data.error ?? `XenoNet answered ${res.status}` };
}

// How a session reads in what the tools say: its title, and its id to reply by.
const who = (a) => `${a.title} (${a.id}${a.project ? `, ${a.project}` : ""})`;

export function sentText(out) {
  if (out.error) return `Not sent: ${out.error}`;
  const lines = out.to.map((r) => `- ${who(r)}: ${r.delivery ?? "when it next starts a turn"}`);
  return [`Sent as message #${out.id} to:`, ...lines, ...(out.alsoTo ? [`Also to ${out.alsoTo}.`] : [])].join("\n");
}

export function peersText(out) {
  if (out.error) return `XenoNet: ${out.error}`;
  if (!out.length) return "No other session XenoNet can reach is at work.";
  return out.map((a) => `- ${who(a)}, ${a.app}, ${a.status}${a.focus ? `: ${a.focus}` : ""}`).join("\n");
}

export function inboxText(out) {
  if (out.error) return `XenoNet: ${out.error}`;
  if (!out.length) return "No new messages.";
  return out.map((m) => `#${m.id} from ${who(m.from)}${m.replyTo ? `, in reply to #${m.replyTo}` : ""}:\n${m.text}`).join("\n\n");
}

const TO = "Who it's for: words from the session's title (such as \"docs site\"), its id, or a selector of key:value terms that must all hold, such as \"app:opencode\" or \"project:atlas app:opencode\".";

export const peers = {
  description: "Lists the other sessions on this machine that XenoNet can reach (Claude Code, Codex, OpenCode), with each one's title, id, project and whether it's working.",
  args: { scope: { type: "string", description: "\"machine\" for every session, \"repo\" for this repo's, or a selector such as \"app:opencode\"." } },
  execute: async (args, ctx) => peersText(await call("peers", { scope: args.scope || "machine" }, ctx)),
};

export const send = {
  description: "Sends a message to another session on this machine through XenoNet. It reaches the session right away when your user allowed XenoNet to wake it, and otherwise when it next starts a turn. Keep it short and self-contained: the other session can't see this conversation.",
  args: { to: { type: "string", description: TO }, text: { type: "string", description: "The message." } },
  execute: async (args, ctx) => sentText(await call("send", { to: args.to, text: args.text }, ctx)),
};

export const reply = {
  description: "Answers a message another session sent you through XenoNet, by its number (the #… in \"[XenoNet from …] (message #…\"). The answer goes to the session that sent it.",
  args: { message: { type: "integer", description: "The message's number." }, text: { type: "string", description: "Your answer." } },
  execute: async (args, ctx) => sentText(await call("send", { replyTo: Number(args.message), text: args.text }, ctx)),
};

export const inbox = {
  description: "Reads the messages other sessions sent you through XenoNet that haven't reached you yet.",
  args: {},
  execute: async (_args, ctx) => inboxText(await call("inbox", {}, ctx)),
};
