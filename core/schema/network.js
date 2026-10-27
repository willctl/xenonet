// network/*: one network across machines (docs/design/network.md). A
// network is one gateway and the hosts that joined it; each host's service
// keeps one outbound link to the gateway, and hosts send each other signed,
// sealed frames through it. network/frame/v1 is a frame on the wire; the
// others are the bodies the engine sends and receives through the seam
// (xenonet_base::network), and what a host's own API answers its window.
import { define } from "./registry.js";
import { z, obj, ms, count } from "./common.js";
import { HOST_ID } from "./ids.js";

// A host's key id: xnk1_ and the base32 of the first 20 bytes of the
// SHA-256 of its two public keys (Ed25519, then X25519).
export const KEY_ID = /^xnk1_[a-z2-7]{32}$/;
// A network's id, made once by its founder.
export const NETWORK_ID = /^net_[0-9a-f]{32}$/;
export const FRAME_ID = /^frm_[0-9a-f-]{36}$/;
const B64URL = /^[A-Za-z0-9_-]*$/;

const HostId = z.string().regex(HOST_ID).describe("The host's id in the network, picked by the person: never the machine's own name");
const KeyId = z.string().regex(KEY_ID).describe("What people compare: xnk1_ and the base32 of the first 20 bytes of the SHA-256 of both public keys");
const B64 = z.string().regex(B64URL).describe("base64url, no padding");
const Mid = z.string().regex(/^msg_[0-9a-f-]{36}$/).describe("The message's id across the network (its envelope's mid)");
const Strength = z.enum(["strong", "weak"]).describe("strong: checked by the window key, or by process checks on macOS; weak: claimed, not checked");
export const STARTS = ["off", "you", "network"];
export const WAKES = ["consent", "network"];
// Who may start and wake sessions on a host (docs/guide/network.md), and
// the widenings to network another window host asked for, which wait for
// the founder or a second window host.
const Reach = obj({
  start: z.enum(STARTS).describe("Who may start sessions there: nobody, you (from a window host's console), or the network (you, and the sessions and actors on window hosts)"),
  wake: z.enum(WAKES).describe("Who may wake its sessions for a message: your consent per session, or any sender in the network, within the budgets"),
  pending: z
    .array(obj({ id: z.string().min(1).max(64), setting: z.enum(["start", "wake"]), value: z.literal("network"), by: HostId.describe("The window host that asked"), at: ms }))
    .max(4)
    .describe("Widenings to network another window host asked for, waiting ten minutes at most for the founder or a second window host to approve"),
});
const Can = z.array(z.string().max(32)).max(16).describe("What the host's XenoNet takes beyond messages: start (a start request) and answer (an answer to a run's permission prompt). An older XenoNet says nothing");

export const FRAME_KINDS = ["hello", "ping", "gap", "roster", "removed", "presence", "claims", "message", "receipt", "handoff", "relay", "operator", "svc"];
// Frames the gateway keeps for a host that's away; the rest are dropped for
// it, since a full set goes when it's back.
export const QUEUED_KINDS = ["message", "receipt", "handoff", "operator", "roster"];

export const NetworkFrame = define(
  "network/frame",
  1,
  obj({
    v: z.literal(1),
    id: z.string().regex(FRAME_ID),
    seq: count.optional().describe("Set by the gateway, per recipient, within its epoch: what a host resumes after"),
    kind: z.enum(FRAME_KINDS),
    from: HostId.describe("The host that sent it, whose key signed it"),
    to: z.union([HostId, z.literal("*")]).describe("The host it's for, or * for every host (presence and claims, sealed once per host)"),
    at: ms.describe("When the sender made it, by its clock"),
    sealed: z.boolean().describe("Whether body is sealed to the recipient's X25519 key; frames between hosts always are"),
    body: B64.describe("The sealed bytes, or the JSON of an unsealed gateway frame"),
    sig: B64.describe("Ed25519 over the head: xenonet-net/1, network id, id, kind, from, to, at, sealed (1 or 0) and the base64url SHA-256 of the body bytes, joined by newlines"),
  }),
  {
    title: "A network frame",
    description: "One line of a link's NDJSON: signed by the host that sent it and, between hosts, sealed for the host it's for. The gateway sees the head and never the body.",
  },
);

const RemoteAgent = obj({
  id: z.string().min(1).max(200).describe("The agent's id on its own host (claude:<uuid>, codex:<id>, actor:<slug>): receivers qualify it as @<host>:<id>"),
  app: z.string().min(1).max(32).nullable(),
  title: z.string().max(200).nullable(),
  state: z.string().max(32).nullable().describe("What the live state says: working, waiting, idle…"),
  doing: z.string().max(280).nullable().describe("What it said it's doing (focus)"),
  repoKey: z.string().max(300).nullable().describe("The normalized git remote of its folder (origin, else the first remote), or null when it has none"),
  branch: z.string().max(200).nullable(),
  cwd: z.string().max(500).nullable().describe("Its folder, with the home folder as ~"),
  lastActive: ms.nullable(),
  waitsOnYou: z.boolean().describe("A question or a stuck state waits on the person"),
  // Detail (docs/design/network.md, section 7): left out when the sending
  // host shares titles only, and never shown to agents.
  question: z.string().max(320).nullable().optional().describe("While it waits on the person: the text of its question, or of the permission it asks for, with the home folder as ~"),
  lastMessage: z.string().max(280).nullable().optional().describe("What it said last, its last 280 characters, with the home folder as ~"),
  stuck: z.array(z.string().max(280)).max(8).nullable().optional().describe("Why it seems stuck, with the home folder as ~"),
});

export const NetworkPresence = define(
  "network/presence",
  1,
  obj({
    schema: z.literal("network/presence/v1"),
    counter: count.describe("Grows with every presence a host sends; a receiver drops one older than the newest it has"),
    full: z.boolean().describe("true: every agent on the host; false: only the agents listed changed, and removed went"),
    agents: z.array(RemoteAgent).max(1000),
    removed: z.array(z.string().max(200)).max(1000).describe("Agents that went since the last presence"),
    you: z.boolean().describe("The person's last input was in this host's console: they're here"),
    youAt: ms.nullable().optional().describe("When the person last worked at this host's console (input in the last two minutes), to the half minute, by the gateway's clock: the consoles chime only where it's latest, or everywhere once it's five minutes old"),
    roster: B64.describe("The hash of the sender's roster head, so hosts that see different rosters notice"),
    reach: Reach.optional(),
    can: Can.optional(),
  }),
  { title: "A host's presence", description: "Which agents are on a host, what each is doing, and whether the person is there." },
);

export const NetworkMessage = define(
  "network/message",
  1,
  obj({
    schema: z.literal("network/message/v1"),
    mid: Mid,
    sender: z.string().min(1).max(200).describe("The sender's id on its own host; the receiver qualifies it"),
    senderKind: z.enum(["agent", "actor", "you"]),
    auth: obj({ method: z.string().max(32), strength: Strength }).describe("How the sender's own host checked it"),
    target: z.string().min(1).max(200).describe("The recipient's id on the receiving host, unqualified"),
    body: z.string().min(1).max(16000),
    replyTo: Mid.nullable(),
    scope: z.string().max(64).nullable().describe("direct, repo or all: how the sender addressed it"),
    kind: z.enum(["message", "handoff"]).optional(),
    hops: count.describe("Messages in its thread so far, across hosts, for the hop limit"),
    at: ms,
  }),
  { title: "A message between hosts", description: "One message from an agent, actor or the person on one host to an agent or actor on another." },
);

export const NetworkReceipt = define(
  "network/receipt",
  1,
  obj({
    schema: z.literal("network/receipt/v1"),
    mid: Mid,
    agent: z.string().min(1).max(200).describe("The recipient, unqualified: xenonet:start for a start request"),
    state: z
      .enum(["stored", "delivered", "seen", "held", "refused", "started", "ended"])
      .describe("stored: the receiving engine has it (the sender may drop it from its outbox); held: it waits because the session bypasses permission checks. For a start request: started, refused, then ended"),
    reason: z.string().max(280).nullable(),
    at: ms,
    session: z.string().max(200).nullable().optional().describe("For started and ended: the new session's id (null for Codex until it names its thread)"),
    pid: count.optional().describe("For started: its process"),
    folder: z.string().max(1000).optional().describe("For started: its folder, with the home folder as ~"),
    app: z.enum(["claude", "codex", "grok"]).optional(),
    exit: z.int().nullable().optional().describe("For ended: its exit code, null when it was stopped"),
    minutes: count.optional().describe("For ended: how long it ran"),
    signIn: z.boolean().optional().describe("For ended: Claude Code there needs a sign-in for XenoNet's runs"),
  }),
  { title: "A receipt", description: "What happened to a message on the host it went to." },
);

export const NetworkClaims = define(
  "network/claims",
  1,
  obj({
    schema: z.literal("network/claims/v1"),
    full: z.boolean(),
    claims: z
      .array(
        obj({
          repoKey: z.string().min(1).max(300),
          path: z.string().min(1).max(1000).describe("Inside the repo, with / between parts"),
          agent: z.string().min(1).max(200),
          branch: z.string().max(200).nullable(),
          kind: z.enum(["claim", "edit"]).describe("claim: the agent claimed it; edit: XenoNet saw it change"),
          until: ms.nullable(),
        }),
      )
      .max(5000),
    released: z.array(obj({ repoKey: z.string().max(300), path: z.string().max(1000), agent: z.string().max(200) })).max(5000),
  }),
  { title: "Claims across clones", description: "What a host's agents claimed or are changing, by repo remote and path inside the repo, so agents on other hosts are warned." },
);

export const NetworkHandoff = define(
  "network/handoff",
  1,
  obj({
    schema: z.literal("network/handoff/v1"),
    id: z.string().regex(/^hof_[0-9a-f-]{36}$/),
    op: z.enum(["offer", "accept", "taken", "lapsed"]).describe("offer: from the maker; accept: from a session that wants it; taken or lapsed: the maker's answer"),
    from: z.string().max(200).describe("The maker, unqualified, on the frame's sender"),
    to: obj({ agent: z.string().max(200).nullable(), repoKey: z.string().max(300).nullable(), host: HostId.nullable() }),
    goal: z.string().max(2000).nullable(),
    done: z.string().max(8000).nullable(),
    next: z.string().max(8000).nullable(),
    files: z.array(z.string().max(1000)).max(200),
    branch: z.string().max(200).nullable(),
    commit: z.string().regex(/^[0-9a-f]{7,64}$/).nullable(),
    by: z.string().max(200).nullable().describe("For accept and taken: the session that took it, qualified"),
    at: ms,
  }),
  { title: "A handoff", description: "Work one session leaves for another, perhaps on another machine; taken by the first that accepts it." },
);

export const NetworkOperator = define(
  "network/operator",
  1,
  obj({
    schema: z.literal("network/operator/v1"),
    action: z.enum(["message", "ticket", "remove", "replace", "settings", "answer"]),
    args: z.record(z.string(), z.unknown()).describe("For settings: { start?, wake? } or { approve } with a waiting widening's id. For answer: { ask, decision }, decision allow or deny"),
    checked: z.enum(["window", "person"]).describe("window: the window key; person: a terminal, by process checks"),
    strength: Strength,
    nonce: B64,
    at: ms,
  }),
  { title: "An action for the person", description: "Something the person did in a console on one host, for another host to carry out." },
);

// The map between hosts (docs/design/network.md, section 7). A window
// host's console subscribes to another host's map; that host answers with
// the SSH relay's own frames (host/hello/v1, snapshot, event, res, time,
// ping, bye), a list of them in each body, at most one body every half
// second for each subscription.
export const RELAY_OPS = ["open", "get", "time", "renew", "close"];
const SubId = z.string().regex(/^sub_[0-9a-f]{32}$/).describe("The subscription, made at random by the subscribing host");

export const NetworkRelay = define(
  "network/relay",
  1,
  z.union([
    obj({
      schema: z.literal("network/relay/v1"),
      op: z.enum(RELAY_OPS).describe("From the subscribing host. open: start, with a hello and a snapshot; get: one read from the relay's allowlist, as GET; time: the host's clock; renew: still watching (every 20 s; a subscription lapses after 60 s without one), with fresh for a new snapshot after a gap; close: stop"),
      sub: SubId,
      id: count.min(1).optional().describe("For get and time: the console's question, which the answer's res or time frame names"),
      path: z.string().min(1).max(2048).optional().describe("For get: the path and query to read"),
      fresh: z.boolean().optional().describe("For renew: send a fresh snapshot, since frames were lost"),
    }),
    obj({
      schema: z.literal("network/relay/v1"),
      sub: SubId,
      seq: count.min(1).describe("1, 2, 3… within the subscription: a hole means frames were lost, and the console asks for a fresh snapshot"),
      frames: z.array(obj({ t: z.string().min(1).max(16) })).max(1000).describe("The relay's frames, in order"),
    }),
  ]),
  { title: "The map between hosts", description: "A console's subscription to another host's map, and that host's answers: the SSH relay's frames, sealed between the two hosts." },
);

// Services through the network (docs/design/network.md, "Services through
// the network"): a program on one host calls /api/svc/<host>/<name>/<path>
// on its own XenoNet, which sends the request to <host> in a svc frame;
// that host checks it, passes it to the service it published, and sends
// the answer back the same way. Live only: never kept for a host that's
// away, since a request may carry the service's own credential.
export const SVC_METHODS = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
const SvcId = z.string().regex(/^svq_[0-9a-f]{32}$/).describe("The request, made at random by the calling host; its answer names it");
const Headers = z.array(z.tuple([z.string().min(1).max(64), z.string().max(8192)])).max(64);

export const NetworkSvc = define(
  "network/svc",
  1,
  z.union([
    obj({
      schema: z.literal("network/svc/v1"),
      op: z.literal("request").describe("From the calling host"),
      id: SvcId,
      name: z.string().regex(HOST_ID).describe("The service, as its host published it"),
      method: z.string().regex(/^[A-Z]{1,16}$/).describe(`A host passes on ${SVC_METHODS.join(", ")}`),
      path: z.string().max(4096).describe("The path after /api/svc/<host>/<name>, with its query: the publishing host normalizes it and checks it against the paths it publishes"),
      headers: Headers.describe("Only the caller's Authorization, Content-Type, Accept and User-Agent cross; the publishing host adds Host and Forwarded itself"),
      body: B64.describe("The request's body, at most 1 MiB"),
    }),
    obj({
      schema: z.literal("network/svc/v1"),
      op: z.literal("answer").describe("From the publishing host"),
      id: SvcId,
      status: z.int().min(200).max(599),
      headers: Headers.describe("The service's answer's headers, without Set-Cookie and hop-by-hop headers"),
      body: B64.describe("The answer's body, at most 4 MiB"),
      code: z.string().regex(/^[a-z-]{1,32}$/).optional().describe("Set when XenoNet answered rather than the service: not-published, not-allowed, rate, path-refused, too-big, timeout…"),
    }),
  ]),
  { title: "A service between hosts", description: "A request to a service another host published to the network, and its answer, sealed between the two hosts." },
);

export const ROSTER_OPS = ["found", "add", "remove", "window", "gateway"];

export const NetworkRosterEntry = define(
  "network/roster-entry",
  1,
  obj({
    n: count,
    prev: B64.describe("base64url SHA-256 of the previous entry's canonical bytes; empty for entry 0"),
    network: z.string().regex(NETWORK_ID),
    op: z.enum(ROSTER_OPS),
    host: obj({
      id: HostId,
      sign: B64.describe("Ed25519 public key"),
      seal: B64.describe("X25519 public key"),
      keyId: KeyId,
      window: z.boolean().describe("A XenoNet window the person uses: this host may act for them"),
    }),
    gateway: obj({ address: z.string().max(300), version: count }).nullable().describe("For op gateway: where the network's gateway is now"),
    at: ms,
    signers: z.array(obj({ host: HostId, sig: B64 })).min(1).max(4),
  }),
  { title: "A roster entry", description: "One link in a network's chain of hosts: who joined, who left, who's the gateway, and which hosts signed it." },
);

const LinkState = z.enum(["off", "connecting", "live", "offline", "refused", "removed"]);

export const NetworkState = define(
  "network/state",
  1,
  obj({
    schema: z.literal("network/state/v1"),
    network: obj({ id: z.string().regex(NETWORK_ID), name: z.string().max(64) }).nullable().describe("null: this machine isn't in a network"),
    self: obj({ id: HostId, keyId: KeyId, window: z.boolean(), founder: z.boolean(), gateway: z.boolean() }).nullable(),
    gateway: obj({
      address: z.string().max(300),
      keyId: KeyId,
      kind: z.enum(["tunnel", "lan", "loopback"]),
      link: LinkState,
      since: ms.nullable(),
      reason: z.string().max(280).nullable(),
      skewMs: z.int().nullable(),
    }).nullable(),
    hosts: z.array(
      obj({
        id: HostId,
        keyId: KeyId,
        window: z.boolean(),
        founder: z.boolean(),
        gateway: z.boolean(),
        online: z.boolean(),
        seen: ms.nullable(),
        agents: count,
        waitsOnYou: count,
        you: z.boolean(),
        youAt: ms.nullable().optional().describe("When the person last worked at its console, by this machine's clock"),
        platform: z.string().max(32).nullable(),
        self: z.boolean().optional().describe("This machine's own row"),
        reach: Reach.nullable().optional().describe("Who may start and wake sessions there, as its presence said; null when its XenoNet doesn't say"),
        can: Can.optional(),
      }),
    ),
    tickets: z.array(obj({ host: HostId, window: z.boolean(), expires: ms })).describe("Tickets that wait; the ticket itself is shown once, where it was made"),
    outbox: obj({ frames: count, bytes: count, oldest: ms.nullable() }),
    rosterHead: B64.nullable(),
    rosterMismatch: z.array(HostId).describe("Hosts whose presence shows another roster head"),
    titlesOnly: z.boolean(),
    shareLoad: z.boolean().optional().describe("Whether this machine sends its load (CPU, GPU, memory and what the parts are) to the window hosts' consoles that watch its map; on unless turned off"),
    start: z.enum(STARTS).optional().describe("Who may start sessions on this machine; you unless widened"),
    wake: z.enum(WAKES).optional().describe("Who may wake this machine's sessions for a message; consent unless widened"),
    queues: z.array(obj({ host: HostId, frames: count, bytes: count })).optional().describe("On the gateway's host: what waits at the gateway for each host"),
    epoch: B64.optional().describe("On the gateway's host: its queues' epoch, which hosts resume within"),
    joinClosedUntil: ms.nullable().optional().describe("On the gateway's host: when the join route opens again, after 200 bad tickets in an hour; null while it's open"),
    proposals: z
      .array(
        obj({
          id: B64.describe("The entry's id, the hash of what it says apart from its signers: what POST /api/net/approve takes"),
          op: z.enum(ROSTER_OPS),
          host: HostId,
          window: z.boolean(),
          gateway: z.string().max(300).nullable().describe("For op gateway: the address it moves to"),
          signers: z.array(HostId).describe("The hosts that signed it so far"),
          at: ms,
        }),
      )
      .optional()
      .describe("Roster entries signed by too few, waiting for the founder or a second window host to approve them"),
    removed: obj({
      network: z.string().regex(NETWORK_ID).optional(),
      host: HostId.optional(),
      by: z.array(HostId).describe("The hosts that signed the removal"),
      at: ms,
      n: count.describe("The removal's place in the roster"),
    })
      .optional()
      .describe("This machine was removed from the network"),
    lan: z.string().max(300).optional().describe("The LAN address this machine's gateway listens on, when it's a LAN-only gateway"),
    simulated: z.boolean().optional().describe("XenoNet Dev's made-up network (simulate net-…)"),
  }),
  { title: "The network, as a host sees it", description: "GET /api/net/state: the network this machine is in, its gateway and link, the other hosts, and what waits." },
);
