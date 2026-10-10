// live/*: what the tailer makes of the transcripts as they're written
// (core/live.js), as GET /api/live, GET /api/history and the `live` and
// `activity` events send it, after state.js's filtering (state.liveSnapshot).
import { define } from "./registry.js";
import { z, obj, ms, count, nstr } from "./common.js";

const App = z.enum(["claude", "codex"]);
export const CATEGORIES = ["mcp", "shell", "edit", "read", "web", "agent", "code", "other"];
const Category = z.enum(CATEGORIES);

export const LiveSession = define(
  "live/session",
  1,
  obj({
    id: z.string(),
    app: App,
    status: z.enum(["tool", "thinking", "idle"]).describe("A tool call in flight, a turn under way, or neither"),
    tokens: obj({ processed: z.number(), output: z.number() }).describe("Over the last 24 hours of its transcript"),
    tokensToday: z.number(),
    tpm: z.number().describe("Tokens in the last minute"),
    context: obj({ used: z.number(), window: z.number() }),
    forecast: obj({
      pct: z.number().describe("How full the context is, 0 to 1"),
      growthPerTurn: z.number().nullable(),
      turnsLeft: z.int().nullable().describe("Turns until 90% full, at its usual growth"),
    }),
    model: nstr,
    turnStartedAt: ms.nullable(),
    lastTool: obj({ name: z.string(), category: Category, detail: z.string(), ts: ms }).nullable(),
    toolCounts: z.record(z.string(), count).describe("Tool calls by category"),
    subagents: z
      .array(obj({ id: z.string(), label: z.string(), startedAt: ms, lastAt: ms, tools: count, done: z.boolean(), active: z.boolean() }))
      .max(12),
    files: z.array(obj({ path: z.string(), ts: ms })).max(10).describe("Files it touched in the last 20 minutes, newest first"),
    mcp: z.array(obj({ server: z.string(), ts: ms })),
    needsYou: obj({ kind: z.enum(["permission", "question", "review"]), text: z.string(), since: ms, asOf: ms.nullable().optional().describe("From a host that isn't live: when it last was") })
      .nullable()
      .describe("Waiting on you; left out once you've seen it, and for archived and background sessions"),
    stuck: obj({ reasons: z.array(z.string()).min(1) }).nullable(),
    lastMessage: nstr,
    lastTurnEndAt: ms.nullable(),
    interruptedAt: ms.nullable().optional().describe("When you stopped its last turn (a Claude interruption, or a Codex turn you aborted), until its next prompt: it waits on you, but isn't ready for review"),
    lastEventAt: ms,
    host: z.string().optional().describe("The host it's from, for a session on another machine"),
    stale: z.boolean().optional().describe("Its host isn't live: it's shown idle, as of when the host last was"),
  }),
  {
    title: "A live session",
    description: "One session as the tailer sees it: what it's doing, its tokens and context, its tools, files and subagents, and whether it needs you or seems stuck.",
  },
);

// A Codex rate limit window, in XenoNet's shape (vendor/openai/codex/
// rollout-line has Codex's own).
const CodexWindow = obj({
  pct: z.number().nullable().describe("Used, 0 to 100"),
  windowMinutes: z.number().nullable().describe("How long the window is"),
  resetsAt: ms.nullable().describe("When it resets"),
});

// Plan limits are an account's, not a machine's (contracts, section 2): the
// identity a reading belongs to.
const Account = nstr.optional().describe("The identity it was read for (claude:…, chatgpt:…, openai:api-key), the one signed in now; null when XenoNet doesn't know it");
// With hosts on, a reading of the same account from another machine that's
// newer than this one's takes its place, and says where it came from.
const ReadOn = z.string().optional().describe("The host it was read on, when that's another machine");

const Forecast = obj({
  pct: z.number().describe("Used, 0 to 100"),
  perHour: z.number(),
  eta: ms.nullable().describe("When it runs out at this pace"),
  resetsAt: ms.nullable(),
  hitsBeforeReset: z.boolean(),
});

export const LiveSnapshot = define(
  "live/snapshot",
  1,
  obj({
    at: ms,
    sessions: z.array(LiveSession).describe("Sessions with anything in the last 24 hours"),
    series: obj({ claude: z.array(z.number()), codex: z.array(z.number()) }).describe("Tokens a second over the last five minutes, oldest first"),
    tpm: z.number(),
    total24h: z.number(),
    today: z.number(),
    limits: obj({
      codex: obj({ primary: CodexWindow.nullable(), secondary: CodexWindow.nullable(), observedAt: ms, identity: Account, host: ReadOn })
        .nullable()
        .describe("Codex's rate limits, as its last token count reported them, and when"),
      claude: z
        .union([
          obj({
            fiveHour: z.number().nullable(),
            sevenDay: z.number().nullable(),
            resets: obj({ fiveHour: ms.nullable().optional(), sevenDay: ms.nullable().optional() }),
            observedAt: ms,
            source: z.enum(["anthropic", "claude-desktop"]),
            identity: Account,
            host: ReadOn,
          }),
          obj({
            fiveHour: z.null(),
            sevenDay: z.null(),
            observedAt: ms.nullable(),
            stale: z.literal(true),
            reason: nstr,
            identity: Account,
            host: ReadOn,
          }).describe("No reading recent enough: when the last one was, and why there's none since"),
        ])
        .nullable()
        .describe("Claude plan usage, in percent"),
      others: z
        .array(obj({ app: App, identity: nstr, host: z.string(), reading: z.record(z.string(), z.unknown()) }))
        .optional()
        .describe("With hosts on: readings of accounts signed in only on other machines, the newest of each, as limits.claude and limits.codex have them"),
    }),
    limitForecast: obj({
      codex: obj({ ...Forecast.partial().shape, windowMinutes: z.number().nullable().optional() }).optional(),
      claudeFiveHour: Forecast.nullable().optional(),
      claudeSevenDay: Forecast.nullable().optional(),
    }),
    costs: obj({
      today: z.number().describe("Dollars"),
      byApp: obj({ claude: z.number(), codex: z.number() }),
      perHour: z.number().describe("Dollars an hour at this minute's pace"),
      unpriced: z.array(z.string()).describe("Models used today that XenoNet has no price for"),
      checked: z.string().describe("When the prices were last checked against the providers' (YYYY-MM-DD)"),
    })
      .optional()
      .describe("What today's calls would cost at API prices (core/pricing.js), in dollars"),
    collab: z.array(obj({ a: z.string(), b: z.string(), kind: z.literal("file"), files: z.array(z.string()) })).describe("Sessions that touched the same files lately"),
    collisions: z
      .array(
        obj({
          a: z.string(),
          b: z.string(),
          files: z.array(z.string()),
          paths: z.array(z.string()),
          ts: ms,
          branches: z.tuple([nstr, nstr]),
          sameBranch: z.boolean(),
          titles: z.tuple([z.string(), z.string()]),
        }),
      )
      .describe("Sessions on the map that both edited a file in the last ten minutes"),
    resources: z.array(obj({ server: z.string(), ts: ms })).max(8),
    status: obj({ watching: z.boolean(), files: count, lines: count, errors: count }),
  }),
  {
    title: "Live state",
    description: "What the agents are doing now: each session's live state, token rates, plan usage and its forecast, and the sessions working on the same files.",
  },
);

// ---------- activity ----------

const event = (kind, fields, description) => obj({ sessionId: z.string(), app: App, kind: z.literal(kind), ts: ms, host: z.string().optional().describe("The host it's from, for a session on another machine"), ...fields }).describe(description);

export const LiveActivity = define(
  "live/activity",
  1,
  z.discriminatedUnion("kind", [
    event(
      "tool",
      {
        tool: z.string(),
        category: Category,
        detail: z.string(),
        files: z.array(z.string()),
        server: nstr.describe("The MCP server, for an MCP tool"),
        agentId: nstr.optional().describe("The subagent that called it"),
        pages: z.array(z.string()).max(6).optional().describe("The Agent Wiki pages a wiki write names"),
      },
      "A tool call",
    ),
    event("prompt", { detail: z.string() }, "A prompt, the start of a turn"),
    event("message", { detail: z.string() }, "Something the agent said"),
    event("turn-start", {}, "A Codex turn starting"),
    event("turn-end", { duration: z.number().nullable(), interrupted: z.literal(true).optional().describe("You stopped it") }, "A turn ending"),
    event("needs-you", { detail: z.string() }, "A question for you, or a plan to approve"),
    event("tool-error", { agentId: nstr.optional() }, "A tool call that failed"),
    event("refusal", { detail: z.string() }, "The provider refused the turn (core/refusals); the detail is its own words or category"),
    event("subagent-start", { agentId: z.string(), label: z.string() }, "A subagent starting"),
    event("subagent-stop", { agentId: z.string(), label: z.string() }, "A subagent finishing"),
    event("hook", { detail: z.string().describe("The hook's event name") }, "A Claude Code hook"),
    event("ship-it", { detail: z.string(), projectId: z.string() }, "A nudge to commit: a turn ended in a repo with uncommitted work (not kept in the history)"),
    obj({
      kind: z.literal("identity"),
      app: z.enum(["claude", "chatgpt", "gh"]),
      identity: nstr,
      before: nstr,
      ts: ms,
    }).describe("A sign-in that changed (not kept in the history)"),
  ]),
  {
    title: "Activity",
    description: "One thing that happened in a session, as the activity feed, the replay and the history get it.",
  },
);
