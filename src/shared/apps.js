// The apps XenoNet knows sessions of, and what's fixed about each: one
// entry per app, as app/v1 (schemas/app/v1.json) describes it, in the order
// session ids and the schemas list them. Only what is data goes here; what
// an app does stays in code.
//
//   numbat     the names numbat records its runs under
//   products   its vendor products, each with the sources XenoNet reads
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
//   counted    the map's stats count its sessions, under its id
//   headless   the arguments that run one turn with the prompt after them,
//              for an agent an OpenShell sandbox runs
//   acp        { command, args }: the command that runs it as an ACP v1
//              agent over its standard input and output, for an app
//              XenoNet starts sessions of over ACP
//   routing   as a candidate for a routing suggestion: its place among
//              the candidates and what it's good for
//   mcpStatus  its own server lists the MCP servers it runs and their
//              catalogs, which XenoNet asks when one of its sessions uses
//              a server
//   reserved   what no actor may pass for: the names it goes by and its
//              makers, which no actor's name or product reads as, and the
//              actor slugs kept for it
//   reportsSpend  its sessions report what they spent, which the spend
//              view shows under its name
//   sorts      XenoNet runs turns of its own with it: the narration, and
//              the sort of the projects folder into projects, whose picks
//              name it as theirs
//   handoff    how a handoff carries a session of it or to it, where that
//              isn't 80 transcript entries read and 1500 characters kept
//              of where the work left off: entries read of one of its
//              sessions, characters a brief for it keeps (clip)
//
// The apps XenoNet only observes, through numbat's records or a run an
// actor reports, have their name and numbat's only, and Pi, which an
// OpenShell sandbox runs, its headless arguments too.
//
// No imports, so the page and src/shared/ids.js can use it. The Rust has a
// copy, crates/xenonet-adapter/src/apps.json: node scripts/apps.mjs writes it.
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
    counted: true,
    headless: ["-p"],
    routing: { order: 2, description: "Coding and repository work in Claude Code." },
    reserved: { names: ["Claude", "Claude Code"], makers: ["Anthropic"], slugs: ["claude", "anthropic"] },
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
    counted: true,
    headless: ["exec"],
    routing: { order: 1, description: "Coding, repository inspection and tool-assisted research in Codex." },
    reserved: { names: ["Codex", "ChatGPT"], makers: ["OpenAI"], slugs: ["codex", "chatgpt", "openai"] },
    mcpStatus: true,
    sorts: true,
    handoff: { entries: 60, clip: 900 },
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
    has: ["lines", "sessions", "connect", "delivery", "processes", "caller"],
    // A process of its own for each session XenoNet starts, never a leader
    // shared with your own sessions.
    acp: { command: "grok", args: ["agent", "--no-leader", "stdio"] },
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
    reportsSpend: true,
  },
  { id: "openclaw", name: "OpenClaw", numbat: ["openclaw"] },
  { id: "antigravity", name: "Antigravity", numbat: ["antigravity"] },
  { id: "factory", name: "Factory Droid", numbat: ["factory"] },
  { id: "devin-cli", name: "Devin CLI", numbat: ["devin-cli"] },
  { id: "kimi-code", name: "Kimi Code", numbat: ["kimi-code"] },
  { id: "pi", name: "Pi", numbat: ["pi"], headless: ["--print"] },
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

// Cursor and Grok Build run Claude Code's hooks too (Cursor's third-party
// configs, Grok's [compat.claude]), and Grok runs Cursor's, each with a
// payload of its own: Cursor's names its cursor_version, and Grok's keys
// are camelCase (hookEventName, and sessionId where Claude Code has
// session_id). Which of the two wrote a hook's input, or null for neither.
export const hookWrittenBy = (body) =>
  typeof body?.cursor_version === "string" ? "cursor" : body?.hookEventName !== undefined || (body?.sessionId !== undefined && body?.session_id === undefined) ? "grok" : null;

// What a reader of transcripts says of an app XenoNet doesn't tail.
export const untailed = (app) => `XenoNet doesn't read ${Object.hasOwn(APP, app) ? APP[app].name : JSON.stringify(String(app))}'s transcripts`;
