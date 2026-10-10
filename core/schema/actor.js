// actor/*: actors, the persistent entities beside the session graph: the
// person at the console, always-on agents, and custom actors that register
// themselves (contracts, section 3, and docs/design/actors/interface.md).
// GET /api/actors sends a list, and the `actors` and `actor-event` events
// send lists and events as they change. Registered actors post reports and
// get answers about themselves; XenoNet's own window posts the person's.
//
// Their enums are open sets: a value a client doesn't know is drawn as the
// default and isn't an error. The bodies programs send are checked a part
// at a time where the interface says so (an event that doesn't fit is
// dropped, not the call), so their parts are exported too.
import { define } from "./registry.js";
import { z, obj, ms, count, nstr, openEnum } from "./common.js";

const ACTOR_ID = /^(@[a-z][a-z0-9-]{1,31}:)?actor:[a-z0-9][a-z0-9-]{0,31}$/;
const SLUG = /^[a-z0-9][a-z0-9-]{0,63}$/;
// An actor's own slug, and an instance's name, which follows the same rule
// (contracts, section 2).
const ACTOR_SLUG = /^[a-z0-9][a-z0-9-]{0,31}$/;

const ActorId = z.string().regex(ACTOR_ID).describe("actor:<slug>, or @<host>:actor:<slug> from another host; the person at the console is actor:you");
const Via = openEnum(["observed", "attributed", "registered", "remote", "demo"]).describe("observed: XenoNet reads its state; attributed: XenoNet traces work here back to it; registered: it reports itself; remote: it reads through a connection you approved; demo: made up");
const Capability = openEnum(["spawn-sessions", "edit-repos", "run-commands", "message-people", "message-agents", "schedule", "browse", "use-computer"]);
const Presence = openEnum(["working", "idle", "away", "offline", "unknown"]);
const Relation = openEnum(["works-on", "spawned", "edited", "messaged", "watches", "reviews"]);

export const ActorEvent = define(
  "actor/event",
  1,
  obj({
    schema: z.literal("actor/event/v1"),
    id: z.string().describe("Unique for the actor, such as wren:41"),
    actor: ActorId,
    at: ms,
    kind: openEnum(["presence", "status", "spawned", "edited", "ran", "messaged", "said", "finished", "failed"]),
    target: nstr.describe("A graph node id, an actor id, or null"),
    detail: z.string().max(200).nullable().describe("What and where, written by XenoNet from facts: never the text of a message or a conversation, and never a secret"),
    via: Via,
  }),
  {
    title: "An actor's event",
    description: "Something an actor did: came or went, changed what it's doing, spawned a session, edited, ran a command, messaged someone, said something, finished or failed.",
  },
);

const Product = obj({
  id: z.string().regex(SLUG).describe("A slug: hermes-agent, grok-bot, muse, dots"),
  vendor: nstr,
  version: nstr,
});

export const ActorSnapshot = define(
  "actor/snapshot",
  1,
  obj({
    schema: z.literal("actor/snapshot/v1"),
    id: ActorId,
    kind: openEnum(["agent", "human"]),
    name: z.string().min(1),
    product: Product.nullable().describe("What it is; null for a custom actor"),
    host: nstr.describe("Null on this machine, else the host id, and the id is then qualified"),
    via: z.array(Via).describe("How XenoNet knows of it"),
    presence: Presence.describe("away: it missed two heartbeats"),
    presenceSince: ms.nullable(),
    lastSeen: ms.nullable(),
    heartbeatMs: z.int().positive().nullable(),
    status: z.string().max(140).nullable().describe("What it says, or XenoNet infers, it's doing"),
    needsYou: obj({
      kind: openEnum(["question", "permission", "review"]),
      text: z.string().max(140),
      since: ms,
    })
      .nullable()
      .optional()
      .describe("When it waits on you, for the alert strip; null or left out otherwise"),
    capabilities: z.array(Capability),
    targets: z
      .array(
        obj({
          id: z.string().describe("A graph node id"),
          relation: Relation,
          since: ms,
          lastAt: ms,
        }),
      )
      .describe("What it's acting on"),
    recent: z.array(ActorEvent).max(20).describe("Its last events, newest last"),
    counts: obj({ events24h: count, spawned24h: count, messages24h: count }),
    identity: nstr.describe("The identity it acts as (claude:…, chatgpt:…, github:…), if XenoNet knows it"),
    remote: obj({
      connections: count.describe("Its remote connections, active or not"),
      active: z.boolean().describe("Whether one is active: approved, not revoked or expired"),
      bridge: openEnum(["connected", "not_connected"]).describe("Whether an adapter is connected through one now; it says nothing of whether the actor itself is at work"),
      accountLink: openEnum(["verified", "owner_asserted", "unverified"]).describe("How sure XenoNet is of the account it acts as: verified by its provider, your word only, or neither"),
      permissions: z.array(openEnum(["read"])).describe("What it may do through them, apart from its capabilities elsewhere"),
      transports: z.array(openEnum(["openai-secure-mcp-tunnel", "local-stdio"])).optional().describe("The routes its active connections take: a tunnel such as OpenAI's Secure MCP Tunnel, or a local MCP client"),
    })
      .optional()
      .describe("Left out for an actor with no remote connection"),
  }),
  {
    title: "An actor",
    description: "An actor as the console draws it: who or what it is, how XenoNet knows of it, whether it's there and what it's doing, and what it's acting on.",
  },
);

export const ActorList = define(
  "actor/list",
  1,
  obj({
    schema: z.literal("actor/list/v1"),
    at: ms,
    actors: z.array(ActorSnapshot),
  }),
  {
    title: "The actors",
    description: "Every actor XenoNet knows of, as GET /api/actors and the actors event send them.",
  },
);

export const ActorEvents = define(
  "actor/events",
  1,
  obj({
    schema: z.literal("actor/events/v1"),
    at: ms,
    events: z.array(ActorEvent).describe("Oldest first"),
  }),
  {
    title: "Actors' events",
    description: "What actors did since a time, at most the last 7 days, for replay and digests: GET /api/actors/events?since=<ms>, which leaves out the person at the console's.",
  },
);

export const ActorRegistration = define(
  "actor/registration",
  1,
  obj({
    schema: z.literal("actor/registration/v1").optional(),
    name: z.string().min(1).max(64),
    slug: z.string().regex(ACTOR_SLUG).optional().describe("Without one, XenoNet makes one from the name"),
    product: obj({ id: z.string().regex(SLUG), vendor: nstr.optional(), version: nstr.optional() }).nullable().optional(),
    capabilities: z.array(Capability).optional(),
    heartbeatMs: z.int().positive().optional(),
    pid: z.int().positive().optional().describe("Its process, which XenoNet checks on macOS"),
  }),
  {
    title: "Registering an actor",
    description: "What a program sends to POST /api/actors/register to ask to be an actor. It's answered at once, and its token works once you allow it in XenoNet's window.",
  },
);

export const ActorCredentials = define(
  "actor/credentials",
  1,
  obj({
    schema: z.literal("actor/credentials/v1"),
    id: ActorId,
    token: z
      .string()
      .regex(/^xenonet_actor_[0-9a-f]{64}$/)
      .describe("Shown once: XenoNet keeps only its SHA-256"),
    state: openEnum(["waiting", "allowed"]),
    expiresAt: ms.optional().describe("While it waits: when the request lapses. Left out once it's allowed"),
    ignored: z.array(z.string()).describe("Capabilities asked for that XenoNet doesn't know"),
  }),
  {
    title: "An actor's credentials",
    description: "The answer to registering, to adding an actor from XenoNet's window and to a new token: the actor's id and its token, once.",
  },
);

export const ActorGatewayCredentials = define(
  "actor/gateway-credentials",
  1,
  obj({
    schema: z.literal("actor/gateway-credentials/v1"),
    id: ActorId,
    token: z
      .string()
      .regex(/^xenonet_gateway_[0-9a-f]{64}$/)
      .describe("Shown once: XenoNet keeps only its SHA-256. It works only at XenoNet's actor gateway, as Authorization: Bearer"),
    url: z.string().nullable().optional().describe("The gateway's address, when XenoNet knows it: where the program sends its MCP requests"),
  }),
  {
    title: "An actor's gateway credentials",
    description: "The answer to making a gateway token in XenoNet's window: the actor's id and a token that works only at XenoNet's actor gateway, once.",
  },
);

export const ActorRequests = define(
  "actor/requests",
  1,
  obj({
    schema: z.literal("actor/requests/v1").optional(),
    at: ms,
    requests: z
      .array(
        obj({
          id: z.string().min(1).describe("The request's own id: POST /api/actors/requests/<id> answers it"),
          actor: ActorId.describe("The actor it would be"),
          name: z.string().min(1).max(64),
          product: Product.nullable().describe("The product it says it is; null for a program of your own"),
          capabilities: z.array(Capability).describe("What it asks to be allowed"),
          heartbeatMs: z.int().positive().nullable(),
          pid: z.int().positive().nullable().describe("The process it named, if any"),
          asker: obj({
            program: z.string(),
            parents: z.array(z.string()).describe("Its parents, up to the first app or terminal"),
            folder: nstr,
          })
            .nullable()
            .describe("Who asked, with ~ for the home folder; null where XenoNet couldn't look"),
          joins: z.boolean().describe("It would join an actor XenoNet already knows, such as a product's observed one"),
          at: ms.describe("When it asked, in epoch milliseconds"),
          expiresAt: ms.describe("When it lapses unanswered, in epoch milliseconds"),
        }),
      )
      .describe("Oldest first"),
  }),
  {
    title: "Registrations waiting for you",
    description: "What GET /api/actors/requests answers XenoNet's own window, with its key: each program waiting for an Allow, what it asks, who asked and when it lapses.",
  },
);

export const ActorTokens = define(
  "actor/tokens",
  1,
  obj({
    schema: z.literal("actor/tokens/v1"),
    at: ms,
    tokens: z
      .array(
        obj({
          actor: ActorId.describe("An actor that reports itself"),
          state: openEnum(["works", "expired", "none"]).describe("works; expired: 30 days unused, until you give it a new token; none: the demo's made-up actor has none"),
          expiresAt: ms.nullable().describe("When it stops working unless it's used before then; null unless it works"),
          usedAt: ms.nullable().describe("When it was last used: exactly the first time, and to within the hour after that; null for a token never used, or none"),
          gateway: obj({
            state: openEnum(["works"]),
            expiresAt: ms.describe("When it stops working unless it's used before then"),
            usedAt: ms.nullable().describe("When it was last used, to within the hour; null for one never used"),
          })
            .optional()
            .describe("Its gateway token, while it has one that works (server/actor-gateway.js)"),
        }),
      )
      .describe("Oldest actor first"),
  }),
  {
    title: "Registered actors' tokens",
    description: "What GET /api/actors/tokens answers XenoNet's own window, with its key: for each actor that reports itself, whether its token works, when it expires unless it's used, and when it was last used. Never a token, nor its hash.",
  },
);

// ---------- what a registered actor reports ----------

export const ReportEvent = obj({
  kind: openEnum(["spawned", "edited", "ran", "messaged", "finished", "failed"]),
  target: z.string().optional().describe("A graph node id"),
  path: z.string().optional().describe("An absolute path, mapped to the project or folder holding it"),
  at: ms.optional().describe("When it happened; now when left out"),
  ref: z
    .string()
    .regex(/^[A-Za-z0-9._:-]{1,64}$/)
    .optional()
    .describe("Its own id for the event, so a retry never counts twice"),
  count: z.int().min(1).max(10000).optional(),
  paths: z.array(z.string()).max(20).optional().describe("For edited: absolute paths"),
  app: openEnum(["claude", "codex", "other"]).optional().describe("For spawned, with a session not on the map yet"),
  platform: openEnum(["telegram", "discord", "slack", "whatsapp", "signal", "matrix", "mattermost", "email", "sms", "imessage", "teams", "other"]).optional().describe("For messaged"),
  subject: openEnum(["session", "task", "job", "turn", "command"]).optional().describe("For finished and failed"),
}).describe("Something it did, as facts: XenoNet writes the detail");

export const ReportTarget = obj({
  id: z.string().optional().describe("A graph node id"),
  path: z.string().optional().describe("An absolute path, mapped to the project or folder holding it"),
  relation: Relation.optional(),
}).describe("Something it's acting on");

export const ActorReport = define(
  "actor/report",
  1,
  obj({
    schema: z.literal("actor/report/v1").optional(),
    instance: z.string().regex(ACTOR_SLUG).optional().describe("Which of its processes this is: main when left out"),
    presence: z.enum(["working", "idle", "offline"]).optional().describe("offline says goodbye; only XenoNet decides away and unknown"),
    status: z.string().optional().describe("What it's doing: cut to 140 characters on one line, and an empty string clears it"),
    cwd: z.string().optional().describe("Where it's working, an absolute path"),
    pid: z.int().positive().optional(),
    events: z.array(ReportEvent).max(50).optional(),
    targets: z.array(ReportTarget).optional(),
    remove: z.array(ReportTarget).optional(),
    replace: z.boolean().optional().describe("targets is the whole list"),
  }),
  {
    title: "An actor's report",
    description: "What a registered actor posts with its token: a heartbeat (POST /api/actors/heartbeat), what it did (POST /api/actors/events) or what it's acting on (POST /api/actors/targets).",
  },
);

export const ActorMe = define(
  "actor/me",
  1,
  obj({
    schema: z.literal("actor/me/v1"),
    id: ActorId,
    state: openEnum(["waiting", "allowed"]),
    presence: Presence,
    heartbeatMs: z.int().positive().nullable(),
    unread: count.describe("Engine messages waiting for it"),
    accepted: count.optional(),
    dropped: z.array(obj({ index: count, reason: z.string() })).optional().describe("The parts of the report XenoNet left out, and why"),
  }),
  {
    title: "An actor, as it sees itself",
    description: "The answer to an actor's own calls (GET /api/actors/me, and its reports) and to the person's reports: whether it's allowed, its presence, its unread messages, and what XenoNet took.",
  },
);

// ---------- the person at the console ----------

export const ConsoleDid = obj({
  what: openEnum(["looked", "read", "opened", "handed-off", "started", "terminal"]).describe("looked: a selection that held 3 s; read: a preview or stats; opened: in its app; handed-off and started: a session; terminal: one opened in a folder"),
  node: z.string().min(1).max(1024).describe("The graph node it was done to"),
  at: ms.optional(),
  to: openEnum(["claude", "codex"]).optional().describe("For handed-off: the app it went to"),
  app: openEnum(["claude", "codex"]).optional().describe("For started: the app the session starts in"),
}).describe("Something you did, as a node id and an action's name: never text");

export const ActorConsole = define(
  "actor/console",
  1,
  obj({
    schema: z.literal("actor/console/v1").optional(),
    presence: z.enum(["working", "idle", "away"]).optional(),
    did: z.array(ConsoleDid).max(20).optional(),
  }),
  {
    title: "The person at the console",
    description: "What XenoNet's own window reports of you to POST /api/actors/you, with the key only the window holds: your presence, and what you did as node ids and action names. Never what you type.",
  },
);
