// Claude desktop's files, as XenoNet reads them (core/claude.js and
// core/claude-usage.js): its record of each Claude Code session it runs,
// and its plan usage samples. Neither is documented, so every field XenoNet
// reads from them is one it can do without.
import { vendor, open, closed, maybe, num, str, z } from "../common.js";

export const DesktopSession = vendor(
  "anthropic/claude-desktop/session",
  1,
  // A record may leave out cliSessionId, createdAt and lastActivityAt:
  // XenoNet merges only the records that name a Claude Code session, and
  // takes the times from the Agent SDK's listing when a record has none.
  open({
    cliSessionId: str.optional().describe("Claude Code's session id, which the Agent SDK lists the session by; a record without one isn't merged"),
    sessionId: str.describe("Claude desktop's own id for it (local_…)"),
    title: maybe(str),
    originCwd: maybe(str).describe("The folder it started in; a worktree session's repo"),
    worktreePath: maybe(str),
    branch: maybe(str),
    createdAt: num.optional().describe("Epoch milliseconds"),
    lastActivityAt: num.optional().describe("Epoch milliseconds"),
    isArchived: z.boolean().optional(),
    isStarred: z.boolean().optional(),
    model: maybe(str),
    effort: maybe(str),
    permissionMode: maybe(str),
    spawnedFrom: open({ sessionId: str.describe("The local_… id of the session that started it") }).optional(),
  }),
  {
    title: "A Claude desktop session file",
    description:
      "Claude desktop's claude-code-sessions/<account>/<organization>/local_*.json, which XenoNet reads for what the Agent SDK doesn't say (title, starred, archived, worktree, lineage) and for the account a session ran as. The files carry no version, so the newest schema reads them.",
    versions: "*",
  },
);

export const PlanUsage = vendor(
  "anthropic/claude-desktop/plan-usage",
  1,
  open({
    version: num.optional().describe("The file's own format version, which picks the schema"),
    samples: z.array(
      open({
        t: num.describe("Epoch milliseconds"),
        u: closed({ fh: maybe(num).describe("The five-hour window, in percent"), sd: maybe(num).describe("The seven-day window, in percent") }).optional(),
      }),
    ),
  }),
  {
    title: "Claude desktop's plan usage samples",
    description: "Claude desktop's plan-usage-history.json: samples of plan usage it takes while its usage view has been open lately. The version is the file's own `version`.",
    versions: "2",
  },
);
