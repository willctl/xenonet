// session/v1: a session as the adapters list it (core/claude.js,
// core/codex.js, core/grok.js, and the demo's in core/demo.js), before the map adds where
// it lives and what it's doing (graph.js).
//
// Loose where the adapters are: Claude Code's, Codex's and Grok Build's sessions share the
// first fields and each adds its own, and the demo's sessions carry some of
// both apps' fields whatever their app, so the app-specific fields are all
// optional here rather than split by app.
import { define } from "./registry.js";
import { z, obj, ms, nstr } from "./common.js";

export const SESSION_ID = /^(@[a-z][a-z0-9-]{1,31}:)?(claude|codex|grok):./;

export const Session = define(
  "session",
  1,
  obj({
    id: z.string().regex(SESSION_ID).describe("<app>:<the app's own id>, the id the rest of XenoNet uses; another host's carry @<host>:"),
    app: z.enum(["claude", "codex", "grok"]),
    rawId: z.string().describe("The app's own id: the Agent SDK's session id, Codex's thread id or Grok Build's session id"),
    title: z.string(),
    preview: nstr.describe("The first prompt, or Codex's preview"),
    cwd: nstr.describe("Where it ran; a Claude desktop worktree session gives the folder it branched from"),
    createdAt: ms.nullable(),
    updatedAt: ms.nullable(),
    archived: z.boolean(),
    model: nstr,
    branch: nstr,
    parentId: nstr.describe("The session this one was spawned or forked from"),
    parentKind: z.enum(["spawn", "fork", "subagent"]).nullable(),
    effort: nstr.optional(),

    // Claude Code's, from Claude desktop's files where it has them.
    desktopId: nstr.optional().describe("Claude desktop's own id for it, local_…"),
    inDesktop: z.boolean().optional(),
    worktree: nstr.optional(),
    starred: z.boolean().optional(),
    permissionMode: nstr.optional(),
    tag: nstr.optional(),
    sizeBytes: z.number().nullable().optional(),
    claudeAccount: obj({ account: z.string(), org: z.string() })
      .nullable()
      .optional()
      .describe("The Claude account and organization Claude desktop filed it under"),

    // Codex's.
    status: nstr.optional(),
    provider: nstr.optional().describe("The model provider, when it isn't your ChatGPT account's"),
    origin: nstr.optional().describe("The repo's origin remote"),
    source: nstr.optional().describe("What started it: cli, vscode, exec, subAgent…"),
    originator: nstr.optional(),
    nativeProjectId: nstr.optional().describe("The Codex desktop project it belongs to"),
    agentNickname: nstr.optional(),
    agentKind: nstr.optional().describe("For a sub-agent thread: spawned, or the kind of background work"),
    background: z.boolean().optional().describe("Background work (codex exec, reviews, compaction) the map leaves out unless asked"),

    // Grok Build's.
    titleManual: z.boolean().optional().describe("Whether the title is one you gave it (/rename)"),
    running: z.boolean().optional().describe("Whether a grok process works in its folder now, as the process table says"),
  }),
  {
    title: "Session",
    description: "A Claude Code session, a Codex thread or a Grok Build session, as XenoNet lists it. The map's session nodes (graph/v1) are these with where they live and what they're doing added.",
  },
);
