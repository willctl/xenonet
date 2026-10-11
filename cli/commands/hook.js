// xenonet hook <app>: what Claude Code's, Codex's and Grok Build's hooks run
// (XenoNet adds them when you connect your agents; see
// core/engine/connect.js). It passes the event on to the running XenoNet and
// prints what XenoNet wants the agent to hear, in the hook output format
// they read. If XenoNet isn't running, it prints nothing and the agent
// carries on. XENONET_PORT says which XenoNet.
//
// Grok Build runs Claude Code's hooks too (it reads ~/.claude/settings.json),
// and says so in GROK_HOOK_EVENT: XenoNet's Claude Code hook stays quiet
// there, since Grok's own speaks for the session. Grok's own sends its
// session and event in the hook's environment as well, which go in the
// event when it doesn't say them.
export const help = `Passes a Claude Code, Codex or Grok Build hook event (JSON on stdin) to
XenoNet and prints XenoNet's answer for the session. Their settings run it,
so you don't need to.`;

const APPS = new Set(["claude", "codex", "grok"]);

// A Grok Build event with what its hook's environment says added where it
// says nothing: its session, and its event.
export function withGrokEnv(input, env = process.env) {
  let body;
  try {
    body = JSON.parse(input || "{}");
  } catch {
    return input;
  }
  if (!body || typeof body !== "object" || Array.isArray(body)) return input;
  if (!body.sessionId && env.GROK_SESSION_ID) body.sessionId = env.GROK_SESSION_ID;
  if (!body.hookEventName && env.GROK_HOOK_EVENT) body.hookEventName = env.GROK_HOOK_EVENT;
  return JSON.stringify(body);
}

export async function run([app], flags) {
  app = APPS.has(app) ? app : "codex";
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) input += chunk;
  if (app === "claude" && process.env.GROK_HOOK_EVENT) return process.exit(0);
  if (app === "grok") input = withGrokEnv(input);
  const port = Number(flags.port) || Number(process.env.XENONET_PORT) || 47840;
  let out = "";
  try {
    const res = await fetch(`http://127.0.0.1:${port}/api/hook/${app}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: input || "{}",
      signal: AbortSignal.timeout(4000),
    });
    if (res.status === 200) out = await res.text();
  } catch {}
  // Out and gone: the agent waits on this process.
  process.stdout.write(out, () => process.exit(0));
}
