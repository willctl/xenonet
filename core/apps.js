// The apps XenoNet knows sessions of, and what's fixed about each: one
// entry per app, as app/v1 (core/schema/app.js) describes it, in the order
// session ids and the schemas list them. Only what is data goes here; what
// an app does stays in code.
//
//   numbat     the names numbat records its runs under
//   products   its vendor products: the sources of each are kept in
//              scripts/provider-drift/registry.mjs
//   roots      the variable that moves its folder, and the folder otherwise
//   context    its context window, until a line says otherwise
//   identity   the kind of account it runs under (identity/v1)
//   limits     its plan limits in live/snapshot/v1
//   team       the developer team that signs it (codesign's TeamIdentifier)
//   connect    how XenoNet connects it: its config format, the hook
//              events, each with a matcher when it takes one, and the
//              events whose answers it passes over (unheard)
//   editTools  the tools whose calls are edits, where they aren't Claude
//              Code's: a native reader's, and Grok Build's own
//   has        what XenoNet can do with it at most: lines (it tails its
//              transcripts), sessions, native (an experimental reader),
//              connect, delivery, processes, caller and open. What it can
//              do with one session is the session's own (observed, reader)
//
// The apps XenoNet only observes, through numbat's records or a run an
// actor reports, have their name and numbat's only.
//
// No imports, so the page and core/schema/ids.js can use it. The Rust has a
// copy, crates/xenonet-base/src/apps.json: node scripts/apps.mjs writes it.
export const APPS = [
  {
    id: "claude",
    name: "Claude Code",
    numbat: ["claude-code"],
    products: ["anthropic/claude-code", "anthropic/claude-desktop", "anthropic/agent-sdk", "anthropic/oauth"],
    // CLAUDE_CONFIG_DIR moves ~/.claude, and ~/.claude.json into it.
    roots: { env: "CLAUDE_CONFIG_DIR", default: "~/.claude", elsewhere: { ".claude.json": "~" } },
    context: 200000,
    identity: "claude",
    limits: "claude",
    team: "Q6L2SF6YDW",
    connect: {
      format: "claude-settings",
      events: [["SessionStart"], ["UserPromptSubmit"], ["PreToolUse", "Edit|Write|MultiEdit|NotebookEdit"], ["PostToolUse"], ["Stop"], ["SessionEnd"], ["Notification"]],
    },
    has: ["lines", "sessions", "connect", "delivery", "processes", "caller", "open"],
  },
  {
    id: "codex",
    name: "Codex",
    numbat: ["codex"],
    products: ["openai/codex"],
    roots: { env: "CODEX_HOME", default: "~/.codex" },
    context: 258400,
    identity: "chatgpt",
    limits: "codex",
    team: "2DC432GLL2",
    connect: {
      format: "codex-hooks",
      events: [["SessionStart"], ["UserPromptSubmit"], ["PreToolUse", "apply_patch|Edit|Write"], ["PostToolUse"], ["Stop"], ["SessionEnd"]],
    },
    has: ["lines", "sessions", "connect", "delivery", "processes", "caller", "open"],
  },
  {
    id: "grok",
    name: "Grok Build",
    numbat: ["grok"],
    products: ["spacexai/grok-build"],
    roots: { env: "GROK_HOME", default: "~/.grok" },
    context: 256000,
    team: "5Y6N3AJ54S",
    // Its matchers name tools exactly: Claude Code's edit tools stand for
    // its own too, which it names as well. It passes over what
    // SessionStart and UserPromptSubmit answer.
    connect: {
      format: "grok-hooks",
      events: [
        ["SessionStart"],
        ["UserPromptSubmit"],
        ["PreToolUse", "Edit|Write|MultiEdit|search_replace|write|edit|hashline_edit|apply_patch"],
        ["PostToolUse"],
        ["PostToolUseFailure"],
        ["Stop"],
        ["StopCancelled"],
        ["StopFailure"],
        ["Notification", "permission_prompt"],
        ["SessionEnd"],
      ],
      unheard: ["SessionStart", "UserPromptSubmit"],
    },
    editTools: ["search_replace", "write", "edit", "hashline_edit", "apply_patch"],
    has: ["lines", "sessions", "connect", "processes", "caller"],
  },
  {
    id: "gemini-cli",
    name: "Gemini CLI",
    numbat: ["gemini-cli"],
    products: ["google/gemini-cli"],
    editTools: ["write_file", "replace"],
    has: ["native"],
  },
  {
    id: "cursor",
    name: "Cursor",
    numbat: ["cursor"],
    products: ["anysphere/cursor"],
    has: ["native"],
  },
  {
    id: "copilot",
    name: "GitHub Copilot CLI",
    numbat: ["copilot"],
    products: ["github/copilot-cli"],
    // str_replace_editor's view isn't an edit; the reader passes over it.
    editTools: ["str_replace_editor", "edit", "create", "write", "apply_patch"],
    has: ["native"],
  },
  { id: "vscode", name: "VS Code agent mode", numbat: ["vscode"] },
  {
    id: "opencode",
    name: "OpenCode",
    numbat: ["opencode"],
    products: ["anomaly/opencode"],
    // Its apply_patch and patch calls are edits too, read from their patch
    // text by the reader.
    editTools: ["edit", "write", "multiedit"],
    has: ["native", "delivery", "processes"],
  },
  { id: "openclaw", name: "OpenClaw", numbat: ["openclaw"] },
  { id: "antigravity", name: "Antigravity", numbat: ["antigravity"] },
  { id: "factory", name: "Factory Droid", numbat: ["factory"] },
  { id: "devin-cli", name: "Devin CLI", numbat: ["devin-cli"] },
  { id: "kimi-code", name: "Kimi Code", numbat: ["kimi-code"] },
  { id: "pi", name: "Pi", numbat: ["pi"] },
  { id: "qwen-code", name: "Qwen Code", numbat: ["qwen-code"] },
  { id: "cline", name: "Cline", numbat: ["cline"] },
  { id: "amp", name: "Amp", numbat: ["amp"] },
  { id: "auggie", name: "Auggie", numbat: ["auggie"] },
  { id: "kiro", name: "Kiro", numbat: ["kiro"] },
  { id: "goose", name: "Goose", numbat: ["goose"] },
  { id: "kilo", name: "Kilo Code", numbat: ["kilo"] },
  { id: "openhands", name: "OpenHands", numbat: ["openhands"] },
  { id: "crush", name: "Crush", numbat: ["crush"] },
  { id: "junie", name: "Junie", numbat: ["junie"] },
  { id: "windsurf", name: "Windsurf", numbat: ["windsurf"] },
  { id: "cowork", name: "Claude Cowork", numbat: ["cowork"] },
];

// Each app's entry, by its id.
export const APP = Object.fromEntries(APPS.map((a) => [a.id, a]));

// The apps that have a capability, in order.
export const appsWith = (capability) => APPS.filter((a) => a.has?.includes(capability));

// The apps whose transcripts XenoNet tails: Claude Code, Codex and Grok
// Build.
export const TAILED = appsWith("lines").map((a) => a.id);

// The tailed app `app` names, or `fallback` for any other. The live tailer
// and the normalizers still read an app they don't tail as one they do,
// and `fallback` says which.
export const tailedApp = (app, fallback) => (TAILED.includes(app) ? app : fallback);

// The tailed app `app` names; `fallback` when it names none (nothing, or
// ""); or null for any other app. What a command, a route, another host or
// a reader of transcripts is given is refused when it's an app XenoNet
// doesn't tail, not read as one it does. Where there's no default,
// `fallback` is null.
export const tailedOr = (app, fallback) => (app == null || app === "" ? fallback : TAILED.includes(app) ? app : null);

// The apps XenoNet asks a question of or hands a session to (xenonet
// diagnose --ask, a handoff): Claude Code and Codex, whose commands and
// links it knows.
export const ASKED = ["claude", "codex"];

// The asked app `app` names; `fallback` when it names none; or null for any
// other app, which is refused, as tailedOr does for the tailed apps.
export const askedOr = (app, fallback) => (app == null || app === "" ? fallback : ASKED.includes(app) ? app : null);

// Apps as a person reads a list of them: "claude, codex or grok".
const orList = (ids) => (ids.length < 2 ? ids.join("") : `${ids.slice(0, -1).join(", ")} or ${ids.at(-1)}`);

// Why a command or a route refused `app` where `what` takes one of `apps`
// (the tailed apps, unless it says): --app takes claude, codex or grok,
// not "gemini-cli".
export const appRefusal = (what, app, apps = TAILED) => `${what} takes ${orList(apps)}, not ${JSON.stringify(app)}`;

// What a reader of transcripts says of an app XenoNet doesn't tail.
export const untailed = (app) => `XenoNet doesn't read ${Object.hasOwn(APP, app) ? APP[app].name : JSON.stringify(String(app))}'s transcripts`;
