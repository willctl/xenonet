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
//   editTools  for a native reader, the tools whose calls are edits
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

// The tailed app `app` names, or `fallback` for any other. The places that
// read a transcript by its app read an app that isn't tailed as one of
// them, and `fallback` says which, as each always has.
export const tailedApp = (app, fallback) => (TAILED.includes(app) ? app : fallback);

// The apps XenoNet asks a question of or hands a session to (xenonet
// diagnose --ask, a handoff): Claude Code and Codex. Any other app reads as
// `fallback`, as it always has.
export const ASKED = ["claude", "codex"];
export const askedApp = (app, fallback) => (ASKED.includes(app) ? app : fallback);
