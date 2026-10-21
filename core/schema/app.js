// app/v1: an app XenoNet knows sessions of, as the app registry
// (core/apps.js) has it, and as the Rust's copy of it
// (crates/xenonet-base/src/apps.json) has it too.
import { define } from "./registry.js";
import { z, obj } from "./common.js";

// What XenoNet can do with an app (docs/design/vendor-layer.md, section 4).
export const CAPABILITIES = ["lines", "sessions", "native", "connect", "delivery", "processes", "caller", "open"];

const Id = z.string().regex(/^[a-z0-9][a-z0-9-]*$/);
// A folder, ~ for your home folder.
const Folder = z.string().regex(/^~(\/.*)?$/);

export const AppEntry = define(
  "app",
  1,
  obj({
    id: Id.describe("What its session ids begin with: <id>:<the run's id>"),
    name: z.string().describe("What it's called"),
    numbat: z.array(z.string()).describe("The names numbat records its runs under"),
    products: z
      .array(z.string().regex(/^[a-z0-9-]+\/[a-z0-9-]+$/))
      .optional()
      .describe("Its vendor products (<vendor>/<product>), each one of the provider drift check's, which lists their sources"),
    roots: obj({
      env: z.string().describe("The environment variable that moves its folder"),
      default: Folder.describe("Its folder when the variable isn't set"),
      elsewhere: z.record(z.string(), Folder).optional().describe("Files the variable moves too, which are elsewhere when it isn't set: each file's name, and the folder it's in then"),
    }).optional(),
    context: z.int().min(1).optional().describe("Its context window, in tokens, until a transcript says otherwise"),
    identity: z.enum(["claude", "chatgpt"]).optional().describe("The kind of account it runs under, as identity/v1 has it"),
    limits: z.enum(["claude", "codex"]).optional().describe("Its plan limits, as live/snapshot/v1's limits name them"),
    team: z.string().regex(/^[A-Z0-9]{10}$/).optional().describe("The developer team that signs it, codesign's TeamIdentifier"),
    connect: obj({
      format: z.enum(["claude-settings", "codex-hooks"]).describe("Where and how XenoNet adds its hooks: Claude Code's settings.json, or Codex's hooks.json"),
      events: z
        .array(z.union([z.tuple([z.string()]), z.tuple([z.string(), z.string()])]))
        .describe("The hook events XenoNet adds, in order, each with the matcher it takes, if any"),
    }).optional(),
    editTools: z.array(z.string()).optional().describe("For a native reader, the tools whose calls are edits"),
    has: z.array(z.enum(CAPABILITIES)).optional().describe("What XenoNet can do with it at most; what it can do with a session is the session's own"),
  }),
  {
    title: "An app",
    description: "An app XenoNet knows sessions of, and what's fixed about it, as the app registry (core/apps.js) lists it. The apps XenoNet only observes have their id, name and numbat's names only.",
  },
);
