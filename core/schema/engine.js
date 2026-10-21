// engine/*: what the engine (core/engine) says about agents and keeps for
// them: the agents at work, their claims on files, their messages, and who
// has been in touch directly. Each comes in the forms the engine uses
// today: a row of engine.sqlite, and what its API and events send. An
// actor that registered (core/actors) is one of the agents, as app "actor"
// (contracts, section 8).
import { define } from "./registry.js";
import { z, obj, ms, count, nstr, openEnum } from "./common.js";

const AgentRef = z.string().describe("An agent: claude:<session id>, codex:<thread id> or grok:<session id>, or an actor, actor:<slug>");
const App = z.enum(["claude", "codex", "grok", "actor"]);

const HeldClaim = obj({
  pattern: z.string().describe("An absolute path, a folder (ending in /) or a glob"),
  note: nstr,
  until: ms,
}).describe("A claim, as an agent's claims list it");

export const EngineClaim = define(
  "engine/claim",
  1,
  z.union([
    HeldClaim,
    obj({ id: count, agent: AgentRef, pattern: z.string(), note: nstr, created_at: ms, expires_at: ms }).describe("As engine.sqlite keeps it"),
  ]),
  {
    title: "A claim",
    description: "A file, folder or glob an agent has said it's working on, until a time.",
  },
);

export const EngineAgent = define(
  "engine/agent",
  1,
  obj({
    id: AgentRef,
    app: App,
    title: z.string().max(90),
    project: nstr,
    branch: nstr,
    projectId: nstr.optional().describe("The map node it lives in"),
    projectKind: nstr.optional(),
    cwd: nstr.optional(),
    top: nstr.optional().describe("Its working tree's top"),
    repo: nstr.optional().describe("Its repo's shared git folder, the same for every worktree"),
    status: z.enum(["working", "idle", "ended"]).optional(),
    ended: z.boolean().optional(),
    needsYou: z.enum(["question", "permission"]).nullable().optional(),
    reader: z.enum(["experimental"]).nullable().optional().describe("Read from its own records by an experimental reader (core/native), so no message reaches it"),
    lastActive: ms.optional(),
    focus: nstr.optional().describe("What it says it's doing"),
    editing: z
      .union([
        z.array(
          obj({
            path: z.string(),
            at: ms,
            observed: z.boolean().optional().describe("An actor's edit XenoNet inferred from what it observes; the actor didn't report it"),
          }),
        ),
        count,
      ])
      .optional()
      .describe("Files it edited in the last half hour; the console's snapshot sends how many"),
    claims: z.array(HeldClaim).optional(),
    reachable: z.boolean().optional().describe("In touch directly lately, so messages reach it"),
    overlap: z.array(z.string()).optional().describe("From peers: files you've both edited lately"),
    delivery: z.string().optional().describe("From send: when it will see the message"),
    host: z.string().optional().describe("In a XenoNet network, the host an agent on another machine is on; its id is @<host>:<id>"),
    seen: ms.nullable().optional().describe("For an agent on another host: when that host was last heard from"),
    online: z.boolean().optional().describe("For an agent on another host: whether its host's link is up; offline since seen when it isn't"),
    repoKey: nstr.optional().describe("For an agent on another host: the normalized git remote of its folder, which clones of one repo share"),
    waitsOnYou: z.boolean().optional().describe("For an agent on another host: a question or a stuck state waits on the person"),
    question: nstr.optional().describe("For an agent on another host, in the console's snapshot only: the text of what waits on the person, as its host's presence carries it; null when it shares titles only"),
    lastMessage: nstr.optional().describe("For an agent on another host, in the console's snapshot only: what it said last, at most 280 characters"),
    stuck: z.array(z.string()).nullable().optional().describe("For an agent on another host, in the console's snapshot only: why it seems stuck"),
  }),
  {
    title: "An agent",
    description:
      "A session at work, as the engine sees it. The brief form (id, title, app, project, branch) names an agent in messages and answers; the agents list and peers send the whole view; the console's snapshot sends the brief form with status, focus, claims, reachability and how many files it's editing.",
  },
);

const Brief = obj({ id: AgentRef, title: z.string(), app: App, project: nstr, branch: nstr });

// How XenoNet knows who sent a message (core/engine/attest.js), written from
// its own checks, never from what the sender said.
export const MessageEnvelope = obj({
  schema: z.literal("message/envelope/v1"),
  mid: z.string(),
  at: ms,
  sender: obj({ id: z.string(), kind: openEnum(["agent", "actor", "you"]), name: nstr, product: nstr }),
  auth: obj({
    method: openEnum(["session", "token", "window", "grant", "gateway", "host"]).describe("How it proved who it is; host: another host of the network signed the frame that brought it"),
    process: openEnum(["signed", "checked", "unchecked", "n/a"]).describe("What XenoNet saw of the program that sent it"),
    strength: openEnum(["strong", "partial", "weak"]),
  }),
  client: obj({ exe: z.string(), name: z.string() }).nullable().describe("The program it came from, with the home folder as ~, when XenoNet saw it"),
  transport: z.string().describe("this machine, tunnel, gateway, or network: from another host of a XenoNet network"),
  host: z.string().optional().describe("For transport network: the host it came from"),
  original: obj({ kind: z.string(), method: z.string(), strength: openEnum(["strong", "weak"]) }).optional().describe("For transport network: how the host it came from checked the sender; a weak check stays weak"),
  hops: count.optional().describe("For transport network: the messages before it in its thread, across hosts"),
  line: z.string().describe("The same in a sentence: verified: …"),
});

export const EngineMessage = define(
  "engine/message",
  1,
  z.union([
    obj({
      id: count,
      from: AgentRef,
      target: z.string(),
      text: z.string(),
      at: ms,
      how: openEnum(["session", "token", "window", "grant", "gateway"]).nullable().optional().describe("How XenoNet knows who sent it (its envelope's method); null for one sent before XenoNet kept that"),
      to: z.array(
        obj({
          id: AgentRef,
          seen: z.boolean(),
          host: nstr.optional().describe("For an agent on another host: its host"),
          receipt: nstr.optional().describe("For an agent on another host: what its host last said of it (stored, held, delivered, seen or refused); null before it said"),
        }),
      ),
    }).describe("In the console's snapshot, GET /api/engine/state"),
    obj({
      id: count,
      from: Brief,
      to: z.string().describe('"you", or what it was sent to'),
      text: z.string(),
      at: ms,
      replyTo: count.nullable(),
      envelope: MessageEnvelope.nullable().optional().describe("How XenoNet knows who sent it; null for one sent before XenoNet kept that"),
    }).describe("In an agent's inbox"),
    obj({
      id: count,
      from: AgentRef,
      to: z.array(AgentRef),
      target: z.string(),
      text: z.string(),
      ts: ms,
      verified: z.string().optional().describe("How XenoNet knows who sent it, in a sentence"),
      strength: openEnum(["strong", "partial", "weak"]).optional(),
    }).describe("The agent-message event"),
    obj({
      id: count,
      sender: AgentRef,
      target: z.string(),
      scope: nstr.describe("For a message to a repo, the repo's git folder or project"),
      body: z.string(),
      reply_to: count.nullable(),
      created_at: ms,
      envelope: nstr.optional().describe("How XenoNet knew who sent it, as message/envelope/v1 JSON (core/engine/attest.js); null before it kept that"),
      host: nstr.optional().describe("The host of a XenoNet network it came from; null for one made on this machine"),
      mid: nstr.optional().describe("Its id across the network (its envelope's mid), for one that went to or came from another host"),
    }).describe("As engine.sqlite keeps it"),
  ]),
  {
    title: "A message between agents",
    description: 'A message from one agent to another, to everyone in a repo (target repo:<name>) or to all ("all"), at most 4,000 characters.',
  },
);

export const EngineContact = define(
  "engine/contact",
  1,
  obj({
    agent: AgentRef,
    app: App,
    cwd: nstr,
    via: z.enum(["hook", "mcp"]),
    at: ms,
    ended_at: ms.nullable().describe("When its SessionEnd hook said it ended"),
    pid: count.nullable().describe("Its process, to tell when it's gone"),
  }),
  {
    title: "A contact",
    description: "When XenoNet last heard from an agent directly, through a hook or a tool call, as engine.sqlite keeps it: how it knows messages reach the agent.",
  },
);
