// Starting a session. You, a session or an actor asks a machine for a new
// session of Claude Code, Codex or Grok Build, in a folder there, with a
// first prompt. Whether the machine takes it is reach.js's (its start
// setting); what a request may say, how its first prompt is framed, the
// command that starts it, the limits and what the answers say are here.
// Running it is the shared runner's (runs.js), and carrying it to another
// machine the network's: a message frame to TARGET, answered by receipts
// keyed by its id.
//
//   claude -p --session-id <id> [--settings <listen file>] [--permission-mode <m>]
//          --permission-prompt-tool mcp__<server>__permission
//   codex exec --json -C <folder> -
//   grok --prompt-file <file> -s <id> --output-format json [--permission-mode <m>]
//
// The prompt goes on standard input (Grok Build reads it from a file of
// XenoNet's), never as an argument. A session never starts bypassing its
// permission checks.
import { sanitize } from "./policy.js";

export const TARGET = "xenonet:start";
export const APPS = ["claude", "codex", "grok"];
export const MODES = ["manual", "plan", "acceptEdits", "auto", "dontAsk"];
export const PROMPT_MAX = 16000;
export const PER_HOUR = 20;
export const PER_HOST_HOUR = 10;
const HOUR_MS = 3600e3;
export const FRESH_MS = 120e3;

const str = (v) => (typeof v === "string" ? v : null);
const chars = (s) => [...s].length;

// A request as it came ({ folder?, app?, mode?, prompt }): { ok: true,
// request } or { ok: false, why }.
export function request(v = {}) {
  const app = str(v?.app) || "claude";
  if (!APPS.includes(app)) return { ok: false, why: `XenoNet starts Claude Code (claude), Codex (codex) or Grok Build (grok), not ${app}` };
  const m = str(v?.mode);
  const mode = m && m !== "manual" && m !== "default" ? m : null;
  if (mode) {
    if (mode === "bypassPermissions") return { ok: false, why: "XenoNet never starts a session that bypasses its permission checks" };
    if (!MODES.includes(mode)) return { ok: false, why: `the mode is one of ${MODES.join(", ")}, not ${mode}` };
    if (app === "codex") return { ok: false, why: "Codex runs with its own config.toml's sandbox and approvals: leave the mode out" };
  }
  const prompt = (str(v?.prompt) ?? "").trim();
  if (!prompt) return { ok: false, why: "say what the new session should do: a prompt" };
  if (chars(prompt) > PROMPT_MAX) return { ok: false, why: `a first prompt is at most ${PROMPT_MAX} characters` };
  const folder = (str(v?.folder) ?? "").trim() || "~";
  if (chars(folder) > 1000) return { ok: false, why: "that folder is too long to be one" };
  return { ok: true, request: { folder, app, mode, prompt } };
}

const titleOf = (t) => sanitize(t, { oneLine: true, max: 120 }).replace(/[«»]/g, '"');

// The first prompt: who asked, then the prompt. `by` is { kind, id, title };
// `from` is the machine it came from, null for this one.
export function framed(by, from, prompt) {
  const kind = by?.kind ?? "session";
  let line;
  if (kind === "you") line = from ? `[XenoNet] Started by you, at ${from}'s console.` : "[XenoNet] Started by you, at this machine's console.";
  else {
    const id = sanitize(by?.id ?? "", { oneLine: true, max: 200 });
    const title = titleOf(by?.title ?? id);
    const what = kind === "actor" ? "the actor" : "the session";
    line = from
      ? `[XenoNet] ${from} says this was asked by ${what} «${title}» (${id}). It came from an agent, not from your user.`
      : `[XenoNet] This was asked by ${what} «${title}» (${id}) on this machine. It came from an agent, not from your user.`;
  }
  return `${line}\n\n${prompt}`;
}

const grokMode = (m) => (m === "manual" ? "default" : m);

// The app and arguments that start a new session `id` in `cwd`: [app,
// args]. `settings` is the listen file, `server` the xenonet MCP server's
// name, `promptFile` where Grok Build reads its prompt.
export function command(r, id, cwd, { settings = null, server = "xenonet", promptFile = null } = {}) {
  if (r.app === "codex") return ["codex", ["exec", "--json", "-C", cwd, "-"]];
  if (r.app === "grok") return ["grok", ["--prompt-file", promptFile ?? "", "-s", id, "--output-format", "json", ...(r.mode ? ["--permission-mode", grokMode(r.mode)] : [])]];
  return ["claude", ["-p", "--session-id", id, ...(settings ? ["--settings", settings] : []), ...(r.mode ? ["--permission-mode", r.mode] : []), "--permission-prompt-tool", `mcp__${server}__permission`]];
}

// Who asked a start here, in the starts' history: one asker on this
// machine (a session, an actor, you) by its id, apart from the others.
export const asker = (id) => `here:${id}`;

// Whether a start may go now, by the starts in the last hour ([at, from]:
// another machine by its name, or one asker here, asker(id), each with its
// own ten): null, or why not.
export function admit(history, from, now = Date.now()) {
  const recent = history.filter(([at]) => now - at < HOUR_MS);
  if (recent.length >= PER_HOUR) return `this machine starts at most ${PER_HOUR} sessions an hour`;
  if (recent.filter(([, h]) => h === from).length < PER_HOST_HOUR) return null;
  const here = String(from).startsWith("here:") ? String(from).slice("here:".length) : null;
  const who = here === "you" ? "you ask" : here !== null ? `${here} asks` : `${from || "this machine"} asks`;
  return `${who} for at most ${PER_HOST_HOUR} sessions an hour here`;
}

export const kept = (history, now = Date.now()) => history.filter(([at]) => now - at < HOUR_MS);

const trimEnd = (p) => p.replace(/[/\\]+$/, "") || p;

// The folder a request names, as a path on this machine: ~ (home), a
// project or folder by its id on the map (`known`: [id, path]), or a path
// the machine reports (`reported`), with ~ for home. On Windows a path
// takes its separator, and paths compare as its file system does, either
// slash and any case alike. { ok, path } or { ok: false, why }.
export function folder(asked, home, known = [], reported = [], { windows = process.platform === "win32" } = {}) {
  const a = String(asked ?? "").trim();
  if (!a || a === "~") return { ok: true, path: home };
  const byId = known.find(([id]) => id === a);
  if (byId) return { ok: true, path: byId[1] };
  const expand = (p) => {
    if (!(p.startsWith("~/") || p.startsWith("~\\"))) return p;
    return windows ? `${trimEnd(home)}\\${p.slice(2).replaceAll("/", "\\")}` : `${trimEnd(home)}/${p.slice(2)}`;
  };
  const key = (p) => {
    const t = trimEnd(expand(p));
    return windows ? t.replaceAll("/", "\\").toLowerCase() : t;
  };
  const want = key(a);
  const same = (p) => key(p) === want;
  const k = known.find(([, p]) => same(p));
  if (k) return { ok: true, path: k[1] };
  const r = reported.find(same);
  if (r) return { ok: true, path: expand(r) };
  return { ok: false, why: `${a} isn't a project or folder on this machine's map: name one by its id or path, or ~` };
}

// Whether a run's output says Claude Code isn't signed in.
export const signedOut = (log) => /not logged in|oauth (session|token) (has )?expired|please run \/login|invalid api key|authentication_error|token has expired|login required/i.test(String(log ?? ""));

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// The session a run's output names: Claude Code's session_id, Codex's
// thread_id.
export function sessionOf(app, log) {
  const key = app === "codex" ? "thread_id" : "session_id";
  for (const line of String(log ?? "").split("\n")) {
    let v;
    try {
      v = JSON.parse(line.trim());
    } catch {
      continue;
    }
    const id = v?.[key];
    if (typeof id === "string" && UUID.test(id)) return `${app}:${id}`;
  }
  return null;
}

// What the requester is told of an answer, on `host` ("this machine" for
// this one). `a` is the receipt's body.
export function answerWords(state, host, a = {}) {
  const session = str(a.session);
  const reason = str(a.reason) ?? "";
  const on = host === "this machine" ? "" : ` on ${host}`;
  if (state === "started") {
    const where = str(a.folder) ? `, in ${a.folder}` : "";
    return session ? `[XenoNet] Started ${session}${on}${where}. Message it as you would any session.` : `[XenoNet] Started a session${on}${where}.`;
  }
  if (state === "ended") {
    const exit = Number.isFinite(a.exit) ? `exit ${a.exit}` : "stopped";
    return `[XenoNet] ${session ?? "The session"}${on} ended (${exit}) after ${Number.isFinite(a.minutes) ? a.minutes : 0} min${reason ? `: ${reason}` : ""}.`;
  }
  return `[XenoNet] No session was started${on}: ${reason}.`;
}

// Where your computer's credential store keeps the long-lived Claude Code
// token XenoNet's runs sign in with (claude setup-token's), which goes to
// them alone as CLAUDE_CODE_OAUTH_TOKEN.
export const SIGN_IN_TARGET = "XenoNet/Claude Code token for XenoNet's runs";

// A token as you pasted it, trimmed: { ok, token } or { ok: false, why }.
export function signInToken(pasted) {
  const t = String(pasted ?? "").trim();
  if (!t) return { ok: false, why: "paste the token claude setup-token printed" };
  if (!t.startsWith("sk-ant-") || t.length < 20 || t.length > 400 || /[\s\x00-\x1f\x7f]/.test(t)) return { ok: false, why: "that isn't a Claude Code token: run claude setup-token, and paste the line it prints" };
  return { ok: true, token: t };
}

export const signInWords = (host) => `Claude Code on ${host} needs a sign-in for XenoNet's runs: run xenonet runs sign-in there`;
