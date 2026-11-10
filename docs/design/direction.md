# Direction: from the prototype to the system

Status: analysis and target shape, 2026-11-08. Written for the goals in
[goals.md](goals.md). Phases 1 and 2 of the order of work are done;
phase 3 is next. "Where the code stands" is the measure taken before
phase 1; the order of work says what each phase changed. Will decides the
open questions at the end as each phase reaches them.

## Where the code stands

Measured against the goals on 2026-11-08, from a read of the crates, the
schemas and the design docs. Sizes are lines of Rust under `crates/*/src`.

### What already serves the goals

- **The frame crypto and the roster chain** (`crates/xenonet-net`, 8.6k
  lines). Ed25519 signatures over a head that binds the sealed body;
  X25519 + HKDF + ChaCha20-Poly1305 sealing per recipient; a hash-chained
  roster with signer rules; one-time tickets with HMAC proofs both ways;
  replay windows; a gateway that relays and can neither read nor forge.
  Known answers in `test/fixtures/net/` and about fifty tests. The seam to the
  engine (`xenonet_model::ops::{Net, Sink}`) is small and typed. This is
  the one part of the core that is close to standalone.
- **Three services in one process** as the network's test harness
  (`crates/xenonet-service/tests/net_*.rs`), and 23 fuzz targets.
- **Lenient vendor readers with drift records** (`crates/xenonet-schema`,
  `crates/xenonet-sources`): vendor data is read against versioned
  schemas, differences are kept as shapes, never values.
- **The app registry** (`apps.json`, 26 apps) with capabilities per app,
  and the doctor.
- **The owner's controls**: admitting, widening and publishing are refused
  from under an agent; tokens are kept as hashes; secrets never enter an
  agent's context.

### Where it falls short

| Goal | Finding |
|---|---|
| 1 Collaboration across machines | Built for messages, claims, presence, handoffs, starts and permissions. Each relies on one gateway per network and one network per host (`crates/xenonet-net/src/files.rs`). Remote maps and transcripts reach only a window host. |
| 2 Many-to-many | Not there. No person, organization or team entity: an org is text inside an identity id, a GitHub org is an ordinary identity with `role: owner`. `actor:you` is one constant across every host. One projects folder, one shared context, one network per host. Project ids are lowercased absolute paths; session ids are vendor ids; nothing XenoNet mints is referable across machines except `repo:<remote>`. Sessions, projects and identity links are recomputed from vendor files on every refresh, not stored. |
| 3 Secure by construction | Four principal types (agent, actor, you, host), each with its own authenticator and its own rule sites (routes, policy, reach, operator, roster). Actors have an ACL-ish capability list plus one selector grant each. A receiving host takes the sender's self-reported auth strength, so a compromised window host speaks as "strong you" everywhere. Process checks pass unchecked off macOS. Read routes and the event stream answer any local process. |
| 4 Performance | Polling throughout: transcripts every 8 s, observers every 3 to 5 s, presence every 2 s, inbox and channels by long-poll. `Engine::agents()` dedupes in O(n²) and runs on every send, peers call and presence tick. |
| 5 Distributed and scalable | Presence is one sealed frame per host per tick, so O(hosts² × agents); by the frame cap of 64 KiB, a full presence with detail would be refused at roughly 50 to 100 agents per host, and 20 hosts would exhaust the 600 frames/min bucket (both inferred from the limits, not measured). One gateway, single SQLite, no failover by design, moved by hand. |
| 6 Modular | The engine is a line-by-line port of JS: cores call each other through `Calls::call(path, method, args)` with string names and JS-shaped values (`crates/xenonet-engine/src/peer.rs`), so there are no typed boundaries between engine, store, policy, delivery and actors. `xenonet-base` holds the vendor registry, the Hermes plugin, pricing and 2.8k lines of Node emulation, and every crate depends on it. Five leaf crates depend on `xenonet-app` for JS shims. `connect.rs` (1,775 lines of Claude, Codex and Grok config writing) lives inside the messaging engine. `"claude"` and `"codex"` appear in over a hundred Rust files. The `cli` crate depends on 19 crates, `wire-app` on 15. |
| 7 Seamless setup | Only Claude Code, Codex and Grok Build have `connect`; Codex still needs `/hooks` trust by hand, Hermes a config edit and a token file, Cursor a checkout and a hand-edited hooks file. The Mac app is unsigned (Open Anyway every version, no auto-update), Windows too. The plugin hard-codes port 47840 and breaks on Windows. The doctor's 33 checks are hand-listed, not derived from the registry. |
| 8 Reuse | MCP (rmcp) and inbound A2A are adopted. The wire format, the roster, tokens, delivery and sync are all XenoNet's own. ACP, which Grok Build, Hermes and the Claude Code adapter all speak, is not used to start or drive sessions. |
| 9 Verified | 951 Rust tests, 1,762 JS tests, 23 fuzz targets. No property tests, no model, no simulation: nothing checks the roster, the tickets, the outbox or the claims under reordering, loss or crashes. |
| 10 Xenolith follows | Xenolith is a separate workspace in the same repo, excluded from the root workspace, with its own copies of private-file, X25519, SHA-256 and token code. XenoNet reads its folder by format and syncs it as an opaque published service. Its spaces are free-text names, not XenoNet ids. |

### Dead weight

- About 50k lines of JS under `core/`, `server/`, `cli/` and `mcp/` ship in
  the Desktop package and are run only as the test contract through 206
  twin modules in `crates/xenonet-node`. New readers are written twice.
- Electron runs a second installer and updater in JS
  (`core/components/*.js`), and Rust embeds `launcher.cjs`.
- Schemas are written in zod in JS and generated into `schemas/`; Rust
  validates `serde_json::Value` against them at runtime and has no derived
  types. Changing a schema needs the Node toolchain.
- The SSH hosts prototype (`XENONET_HOSTS=1`) keeps a second host-id
  namespace beside the roster.

## How the target shape was chosen

Three architects designed the core independently from the same grounding
and goals, each from a different premise: every fact as an entry in a
signed per-owner log ([candidate A](direction/candidate-a-signed-logs.md));
an authority graph of Zanzibar-style tuples plus typed streams
([candidate B](direction/candidate-b-authority-graph.md)); subtract the
dead weight first, then reshape what remains
([candidate C](direction/candidate-c-subtract-first.md)). A fourth model
judged them against the goals. The research behind the reuse decisions is
in [survey-protocols.md](direction/survey-protocols.md) and
[survey-formal-methods.md](direction/survey-formal-methods.md).

The three agreed on more than they differed: iroh as the transport with
the gateway dissolved into roles; Biscuit tokens checked by the receiver;
one adapter trait with vendor code in leaf crates; MCP, ACP and A2A at the
edges; presence as a live stream, never stored; deleting the JS contract,
the string-dispatched engine and the SSH prototype; Quint, turmoil,
proptest and Kani for verification. They differed on the primitive. C is
the base, because it is the plan one owner can run with coding agents:
every phase ships, the deletions have sizes, and the transport overlaps
for one release. From A it takes the type-level guarantee that a message
can never become authority, causal revocation with a freshness rule, and
"the envelope is the entry". From B it takes the human-presence block, the
per-kind order and retention table, and Alloy for the relation schema. An
independent review of the synthesis then found four blocking problems (a
replayable presence block, a machine key that could not be both iroh's
endpoint key and hardware-backed, no chain policy table, and a phase 3
that contradicted the wire) and eleven smaller ones; all are folded in.

## The target shape

### One paragraph

XenoNet's core is eight typed crates around two signed, replicated
structures. **Chains** are quorum-signed membership logs, generalized from
today's roster: one each for a person (their device keys), an org, a
network, a project and a shared-context space. **Feeds** are each
machine's gapless, sequenced log of signed operations: messages, receipts,
claims, task events. One authorization crate decides every incoming
operation at the receiver, from a Biscuit token checked against facts the
chains provide. One transport carries everything: iroh QUIC between
machines, with any number of relays and mailboxes and no central gateway.
Platforms plug in behind one trait. The console, CLI, MCP servers and A2A
are clients of one typed local API. Sessions stay derived from vendor
files, pushed by watchers into an index. Everything XenoNet itself
authors is stored, signed and owned by a key.

### Crates

```
xenonet-id*       keys, typed signatures, sealing (kept from frame.rs), tickets, HLC, did:key
xenonet-log*      chains (Chain<P: Policy>) and feeds (per-author sequence); Entry<Unverified> → Entry<Verified>
xenonet-authz*    "may P do A on R under this token, given these facts?"; Information<T>; Authorized
xenonet-net*      membership-gated push transport on iroh; defines Admission (an implementation brings its own, usually xenonet-log); relays, mailboxes, join
xenonet-model*    ids, entities, relations, ops, the action vocabulary; no IO; schemas from schemars
xenonet-adapter*  the Platform trait, typed registry, drift, tailing and watching
xenonet-engine    coordination rules; defines Store and Transport, depends on neither implementation
xenonet-store     one SQLite per machine, single writer
xenonet-api       the typed local protocol; the only crate surfaces import
xenonet-service   composition root; no logic
surfaces/         cli, mcp, a2a edge, the console's HTTP and SSE bridge
adapters/         claude-code, codex, grok-build, hermes, opencode, acp, git
xenonet-doctor    runs every Check the crates and adapters expose
* usable alone by another project
```

No core crate has more than three internal dependencies. CI fails when a
core crate depends on a surface crate or contains a registry id literal.

### Domain model

- **Principals** have a key, or a machine that vouches for them: a
  person (root key kept offline, device keys on each machine), a machine
  (one XenoNet service, one OS user on one host), an actor (its own key),
  an agent (a harness session, `AgentId` minted on first sight, vouched
  for by its machine with an `Attestation` of how it was checked).
- **Groups** are chains, so membership is many-to-many by construction:
  org, network, project, space. A `Subject` of a role can be a person, a
  machine, an actor, an org or a network, which gives Zanzibar's usersets
  ("members of org O may read space S") in one entry. Every chain entry
  carries the authority it was made under (a Biscuit, or a device
  signature with a presence block), and each chain kind has a policy that
  says who may extend it:

  | Chain | Who signs an entry | Notes |
  |---|---|---|
  | Person | Adding or removing a device: the root key, or two existing devices each with a presence block. Revoking: any one device with presence. | The root is used rarely; losing every device means the root or the recovery kit |
  | Network | Adding a member or mailbox: one admin device with presence. Adding or removing an admin, moving relays, widening settings: the founder's root or two admin devices. Removing a member: one admin device. | Today's roster rules, with "window host" read as "admin device" |
  | Org | Adding a member: one admin. Adding or removing an admin: two admins, or the owner. | Orgs are an entry kind; the first cut builds none (question 5) |
  | Project | Sharing: one owner or maintainer device with presence. Merging two chains: an owner of each. | Readers and contributors cannot share |
  | Space | Sharing or moving the curator: the space's owner (the owner of the entity it is about). | Exactly one curator at a time |

  The table is the default for Will to confirm (question 1 below); the
  `Policy` trait holds it in one place per chain kind.
- **Projects** are minted ids, never paths. A repo with a remote founded
  as (owner, repo key) gets the same id on all of one person's machines
  without them talking; anything else gets a random id and joins by
  `Share` or `Merge`. Two persons who both found a project for one repo
  get two ids and merge them, so with orgs `Merge` is the common path, not
  the exception. A project has many checkouts; a machine holds many.
  Claims are (project, relative path).
- **Accounts** (vendor sign-ins) link to persons by inference or by a
  person-signed statement. The inference, which reads the vendors' own
  files, lives in the adapters; the core holds only the signed statement.
- **Sessions** are derived from vendor files and indexed incrementally;
  the `AgentId` to vendor-id mapping is stored, so ids are stable. A
  session XenoNet starts itself (over ACP or a headless run) gets its
  `AgentId` from the adapter that started it, mapped to the vendor id when
  a transcript appears.
- **Coordination** ops are `Send`, `Receipt`, `Claim`, `Release`, `Task`
  events (handoff, start, permission question, sharing A2A's task states)
  and `Answer`. Every op travels as one signed object: the envelope is the
  feed entry, so there is one signature and one sequence number per op.
  A feed's sequence is `(author, incarnation, n)`: the incarnation is
  persisted with fsync and bumped when a machine is restored from a
  backup or reinstalled, so a restored machine writes at once from a new
  incarnation instead of silently colliding with its old entries, and
  peers freeze an author only on a fork within one incarnation.
- **Presence** is a stream of deltas with a snapshot on connect, never
  queued or stored.

Each stream kind declares its order and retention:

| Kind | Scope | Order | Retention |
|---|---|---|---|
| Message, Receipt | thread | per author | kept, 7 days |
| Claim, Release | project | add-wins set | kept until expiry |
| Task events, Answer | run | per author | kept until the task ends, then 7 days |
| Chain entries | group | hash chain | forever |
| Presence | machine | last writer wins | live, TTL 30 s |
| Context notes and page versions | space | per author | forever |
| Map, published services | machine | stream | live |

### Protocols

- **Transport:** one iroh 1.x endpoint per machine under its machine key,
  which is also its iroh endpoint id. ALPNs `xenonet/link/2`,
  `xenonet/join/2`, `xenonet/svc/2`, and any other crate's (Xenolith's
  `xenolith/sync/2`). A machine in several networks keeps one endpoint; the
  envelope's scope names the network and `Admission` checks membership.
- **Roles, not a gateway:** a **relay** is an iroh-relay any member can
  run, listed in the network chain; a **mailbox** is a member machine the
  person names that keeps sealed envelopes for away members under today's
  gateway quotas, two per network by default; a **front door** is a surface
  (MCP over HTTP, A2A) at an address the person chooses, or none. A
  mailbox sees what today's gateway sees for the envelopes it holds (who,
  to whom, kind, size, time) and can drop or delay them, so it signs a
  `Held(author, seq)` receipt for each envelope it takes, and withholding
  shows as a held-but-undelivered gap. Sealed envelopes have no forward
  secrecy: a machine key taken later opens what a mailbox still holds, so
  the quotas keep a week at most and secrets never travel as kept
  envelopes.
- **Finding each other:** chains give keys; addresses come from LAN
  discovery, small signed address records exchanged in scope, DNS or
  pkarr if the person turns them on, or fixed addresses. A Tailscale or
  WireGuard address is just another address.
- **Envelope:** today's line head, tag `xenonet-net/2`, with scope, topic,
  author, recipient, sequence, hybrid logical clock, class (live or kept)
  and the body hash; the body is a versioned JSON `Act` whose schema comes
  from the Rust types. The sealing math and the known-answer fixtures stay.
  CBOR with COSE_Sign1 is an open question (below), not a change now.
- **Joining:** an admin's device issues a Biscuit ticket authority ("admit
  one machine as member of N for ten minutes"), prints a non-secret invite
  (the network id, which is the hash of its genesis entry, and the
  inviter's endpoint address) and a separate code. The joiner dials the
  inviter by the key in the invite, so the channel is already
  authenticated; today's HMAC proofs over the code, both ways, are enough,
  and a password-authenticated exchange would add nothing while the
  invite carries the key. The inviter must be online; the lockouts that
  sit on the gateway today move to it. It appends to the chain, and the
  joiner verifies the chain from the genesis the invite named. A second
  network is the same command.
- **Delivery:** FIFO per (author, topic), gapless, at-least-once on the
  wire, exactly once into the store by (author, seq), with gaps filled
  from the author or any member holding the feed. Into an agent's context:
  at-least-once with the id visible, acked when the turn that showed it
  ends. No global order; the HLC orders for display, `reply_to` gives
  causality.
- **Edges:** MCP (rmcp 3, the 2026-07-28 revision) is how agents and
  actors reach the engine, XenoNet's tools in one extension namespace. ACP
  (`agent-client-protocol`) lives in one generic adapter that starts,
  drives and relays prompts for every platform that speaks it. A2A 1.0
  (`a2a-rs`) sits at the front door; inbound first, outbound later.

### Authorization

- **Every incoming act is decided at the receiver** by one function:
  `decide(token, request, ground, policy)` → `Allow | Hold | Refuse`. The
  sender's check is a courtesy. The sender's self-reported strength is
  never trusted. Local surfaces present a per-surface credential issued
  at install, and the service checks the peer's uid; that keeps other
  users and stray programs out, but not an agent on the same OS account,
  whose MCP server is its own child. Reads that matter (transcripts,
  search, other machines' state) therefore need a Biscuit read scope,
  not just a connection.
- **Authority is a Biscuit v6 token.** Biscuit roots are keys, so: tokens
  a person issues root at one of their device keys, and the receiver maps
  the device to the person through the person chain; tokens a group
  issues carry the chain's admin keys as third-party blocks; the person's
  offline root key signs chain entries, never daily tokens. Facts take
  the tuple shape `member(group, subject, role)`, and the agent's
  attestation (`HarnessSigned`, `ProcessChecked`, `Unchecked`) is a fact
  too: policy refuses `start`, `answer` and claims on other machines from
  an `Unchecked` agent, and a standing grant names the attestation it
  requires. Routine machine actions rest on membership facts; a standing
  grant from the person lets a machine mint agent tokens of 24 hours,
  renewed while the session lives, each narrowed to one agent. Delegation
  is attenuation: offline, narrowing only, hop-capped.
- **A human is proven** by a device signature made after an OS
  user-presence check, carried as a presence block that only a
  presence-capable surface can mint. On macOS and Windows the check is
  biometric (Touch ID, Windows Hello) with no passphrase fallback, so a
  computer-use agent on the same account cannot pass it; Linux uses
  polkit or a passphrase and is marked weaker. When the person has
  enabled a hardware key, the widest acts (admit, widen, publish) also
  need its tap. The block signs the purpose, the hash of the one act it approves
  (or the receiver's nonce) and the time; the receiver accepts it once,
  and refuses it after 60 seconds. It cannot be lifted off one act and
  put on another. This replaces the window key. The person alone founds,
  admits, widens, publishes, revokes, and answers permission prompts,
  unless they issued a standing grant for answers; the receiver accepts
  an answer only under authority that chains to a person.
- **Revocation** is a chain entry. It takes effect on a machine the
  moment that machine holds it, and from then on a revoked proof is
  ineffective for every entry the machine had not already stored. The
  receiver's own store order decides what counts as before, never the
  entry's clock, which the author asserts and could backdate. For
  `start`, `answer`, `admit`, `widen` and `publish` on another machine's
  behalf, the verifier must have synced governance within ten minutes or
  it holds; a machine acting on its own runs is exempt, so a laptop on a
  plane still answers its own prompts.
- **"Information, never authorization" is in the types.** Message,
  handoff, ask and prompt text is `Information<T>`, which converts to
  nothing; the authorizer's inputs are a closed struct built from header,
  token and chain facts, never from a body; `Authorized` has no public
  constructor; the framing an adapter shows a session cannot be dropped. A
  compile-fail test keeps it so.

### The machine key

All three candidates and today's code root every agent's authority in
the machine key, kept in an owner-only file or the OS keychain. The agents
run as the same OS user with a shell, so any session can read that file
or run the binary the keychain trusts, and sign as any agent on the
machine. Presence checks guard the person's key; nothing guards the
machine's. iroh needs the raw Ed25519 secret of the endpoint key for its
QUIC handshake, so that key can never be hardware-backed. The design
therefore gives a machine two keys, as the roster already carries a
signing and a sealing key: a **transport key** in a file, which proves
the channel and nothing else, and a **machine signing key** behind a
`Signer` trait, which signs chain entries, feed entries and agent tokens.
The machine's chain entry binds the two. By Will's decision the signing
key is hardware-backed and non-exportable by default where the platform
has a secure element (the Secure Enclave on macOS, the TPM on Windows),
and a file elsewhere, with attestation recorded honestly either way and
used by policy as the authorization section says. A service running as
its own OS user is the fallback hardening for machines without a secure
element (question 2 below).

### Adapters and surfaces

A platform is one crate implementing `Platform`: `watch` (push events
from files, sockets and hooks; the core never polls a vendor),
`reconcile`, `decode_hook`, `checks`, and optional `deliver`, `start` and
`connect`. `connect` returns the config edits it would make; the service
applies them after the person agrees. The doctor's rows, `connect` and the
capability list derive from the registry entry plus the adapter. What
moves out of the core: `connect.rs`, `toml.rs`, the delivery ways,
the vendor branches of `procs.rs` and `verify.rs`, hook decoding and the
readers. The Hermes, Grok Bot and Muse observers are out already
(`adapters/hermes`, `adapters/grok-bot`, `adapters/muse`, each an
`Observer` over `xenonet_adapter::observe`'s typed host; the engine runs
their looks and answers their host).

Surfaces import `xenonet-api` only: a typed request and response protocol
with a pushed event stream over a Unix socket or named pipe, every
connection authenticated. Console-only state (layout, themes, demo,
Spotify) moves out of the core into the console.

### Xenolith

Xenolith stays a separate program that depends on four libraries
(`xenonet-id`, `-log`, `-authz`, `-net`), never on the app. A space is a
chain bound to a real entity (project, person, org, actor, machine)
instead of a free-text name; whoever reads project P reads the space about
P unless the space narrows it. The space policy allows exactly one
curator machine, which makes today's home-and-satellite rule
unrepresentable to break, and `MoveCurator` is the signed handover. Notes
are per-author feeds; pages are the curator's feed of versions, so no CRDT
is needed while pages have one writer. Grants are Biscuit tokens; pairing
is the same invite flow; sync is an ALPN on the shared endpoint. Xenolith's
copies of the private-file, X25519 and token code go.

## Reuse decisions

| Need | Decision | Why |
|---|---|---|
| Transport, NAT traversal, relays, discovery | Adopt iroh 1.x; run our own relay role | Dial by the Ed25519 key we already have; 1.x wire is stable; public relays are rate-limited with end-of-life dates |
| Tokens, delegation, offline checks | Adopt biscuit-auth 6 | Public-key verifiable offline, attenuation only narrows, Datalog policy; UCAN crates are not 1.0 |
| Relationship model | Borrow Zanzibar tuples and usersets; skip OpenFGA and SpiceDB servers | They need an always-on authority; a chain replica does the same offline |
| Agents reaching XenoNet | Keep MCP, move to the 2026-07-28 revision on rmcp 3; one extension namespace | Already in use; stateless handles and per-message versions fit |
| Starting and driving sessions | Adopt ACP in one generic adapter | Grok Build, Hermes and the Claude Code adapter speak it; vendor code stays only for what ACP lacks |
| Cross-vendor agent tasks | Keep A2A 1.0 at the front door via a2a-rs | The one LF-governed agent protocol with an official Rust SDK; request-shaped, so never the internal bus |
| Shared context pages | Single-writer feeds now; Automerge (samod) only if pages gain several writers | Authority inside a CRDT is unsolved (Keyhive is pre-alpha) |
| Schemas | serde types with schemars; delete zod | One source of truth, in the language that ships |
| Wire format | Keep the fuzzed line head and JSON body, versioned; CBOR/COSE open | A format change now buys no property; revisit for outside interop |
| Identity rendering | did:key, implemented in a few lines | Maps onto whichever agent-identity registry wins (A2A cards, ANS, SLIM) |
| Skip | libp2p, NATS, Matrix, Nostr, Zenoh, Willow-rs, SPIFFE, Tailscale-rs, Cap'n Proto RPC, AG-UI, MLS | Heavier, 0.x, needs a daemon, archived, or ties to one person's setup |

## Order of work

Each phase ships with the service working, and deletes before it adds.
Nothing starts until Will answers the open questions that gate it.

The rewrite is worked for speed, not ceremony (Will, 2026-11-08). Only
Will uses XenoNet, so there is no one to protect with a review queue.
Work lands on `main` directly, in small Conventional Commits, not as a
stream of pull requests; a pull request is for a change Will wants to
read before it lands, and nothing else. Checks run locally and before
the commit: `cargo test --workspace`, `npm test`, the console tools, and
the VM lab (`scripts/lab/`) for anything that spans machines, with each
VM running the build under test. CI runs after the push as a backstop;
no one waits for it, and what it finds is fixed in the next commit.
Releases are cut at the end of a phase, or when Will wants the installed
XenoNet on a new build, not per change; between releases the installed
service runs a local build.

1. **The JS world goes. Done (2026-11-08).** The JS core and its napi
   twins are gone: `core/`, `server/`, the JS `cli/` and `mcp/`,
   `crates/xenonet-node` with `parity.json`, the twin tests and the scripts
   that ran them. Golden files recorded from the JS replace the twins as
   the contract: the MCP servers' answers
   (`crates/xenonet-mcp/tests/golden/`), 42 HTTP routes and 60 command
   cases over a made-up home (`crates/xenonet-cli/tests/golden/`), the map's
   golden world (`crates/xenonet-state/tests/golden/`), and the network's
   known answers (`test/fixtures/net/`). The modules the page and Desktop
   import moved to `src/shared/` and `electron/lib/`. Desktop installs and
   updates its components through the binary (`xenonet components`, over
   `crates/xenonet-components`), and `launcher.cjs` lives in that crate.
   Left for later: the
   routes and help texts where the JS and the Rust already differed
   weren't frozen (the commit that froze them lists them).
2. **A typed core, no more string dispatch. Done (2026-11-09).** The
   data model is `xenonet-model`: typed ids, every document as a serde
   type, the operations' kinds and the action vocabulary; `schemas/` is
   written from its types, and its types enforce the bounds the schemas
   declare when untrusted input is read. `xenonet-adapter` holds the
   registry and the `Platform` trait; each platform is a crate in
   `adapters/` (Claude Code, Codex, Grok Build, OpenCode, Cursor, Gemini
   CLI, Copilot CLI, Hermes Agent, Grok Bot, Muse) with its process rules,
   ways in, connect, readers, observers and doctor checks, and
   `xenonet-platforms` installs them at the binary's start. `Calls`,
   `Arg`, `peer.rs`, `world.rs`, the JS emulation and the runtime schema
   checker are gone: every collaborator is a typed trait. The engine keeps
   a session index in place of the rebuild per call. The three `wire-*`
   crates and the `base` and `app` grab-bags are gone too, their code in
   `xenonet-text`, `xenonet-os`, `xenonet-api`, `xenonet-demo`,
   `xenonet-console`, `xenonet-runtime`, `xenonet-state`'s runtime and the
   domain crates, with the composition root in the service. No core crate
   depends on a surface or an adapter crate or names a platform, and
   `npm run lint:rust` fails if one does (`scripts/rust-layers.mjs`).
   Left for later: the core traits in `xenonet-api` still pass JSON (typed
   requests come with the local protocol); the "no core crate has more
   than three internal dependencies" and "surfaces import only
   `xenonet-api`" rules wait for phase 3; the session index takes the map
   and the store's contacts by push but still reads the live tailer and
   the actors' list on each call; the per-app line views stay in
   `xenonet-sources` as types the adapters name by format; the Pi
   integration's session reading is still in `xenonet-integrations`; and
   `xenonet-hosts` still tells Claude Code's plan readings from the
   registry's limits kind rather than through its adapter.
3. **One authorization.** Add `xenonet-log::chain` with today's
   `roster.rs` as its first `Policy`, so the network chain *is* the roster
   and one implementation runs the old transport and the new rules; add
   person chains with device keys, `xenonet-authz`, the presence block,
   the `Information<T>` types and the compile-fail test. The frame head
   stays `xenonet-net/1` and its kinds stay closed; the message body
   schema goes to v2 and gains the act's authority, which is how Biscuits
   travel until phase 5. Tokens in this phase name machines and networks
   only; project- and agent-scoped grants wait for the stored ids of phase
   4, so no token dies at that cutover. Revocation in this phase takes
   effect once the receiver holds the entry; the store-order refinement
   comes with feeds in phase 4. Re-express, then delete: actor tokens and
   capability lists, delivery grants and selectors, `reach`, `operator`,
   assistant grants, gateway and A2A tokens. Decide at the receiver; the
   local API gets its credential.
4. **Many-to-many ids, one store.** Add feeds and stored ids (project,
   agent, org, space) with checkouts as rows; claims become (project,
   relative path) with a one-time mapping; consolidate the dozen state
   files into one SQLite; adapters push from watchers and the rescan
   becomes reconciliation. One id system at a time, cut over once. Still
   one network per machine on the old transport, so the ids of this phase
   name that one network, and feeds give exactly-once dedupe and
   per-author order but fill gaps only through the gateway's sequence and
   the engine's resend; fetching a missing range from any holder arrives
   with the transport. This order (ids and store before transport) keeps
   the widest data change on the wire that already works, and lets the
   transport phase change one thing.
5. **The transport.** Add `xenonet-net` on iroh beside the gateway link
   for one release, with mailboxes (the gateway queue code moves there),
   relay lists, presence deltas, the HLC, several networks per machine and
   the `xenonet-net/2` head. Then delete the HTTP link, signed requests,
   the clock offset, `gateway.sqlite`, `relay` and `svc` frames. The
   three-service tests move under turmoil.
6. **Xenolith and the edges.** Xenolith adopts the four libraries and
   deletes its own pairing, tokens and crypto; spaces bind to model ids;
   the generic ACP adapter; A2A outbound; MCP 2026-07-28.

## Verification plan

| # | Property | Tool |
|---|---|---|
| 1 | Chain safety: no two honest machines accept different entries at one height; every entry meets its policy's quorum; tickets are single-use and expire | Quint `chain.qnt` with mailbox and relay as an adversary; quint-connect against `Chain::offer`; proptest-state-machine |
| 2 | Delivery: a kept act from a reachable author reaches every intended member at least once, enters the store at most once, in per-author order; gaps are detected | Quint `delivery.qnt`; turmoil seeds (mailbox dies mid-transfer, partitions, reconnect storms, skew) |
| 3 | Authority: a narrowed token never allows what its parent refuses; a revoked id is refused once held and for every entry not yet stored when it arrived; no message content changes a decision | Exhaustive unit matrix over action × principal × role; proptest against a reference model; the store-order rule in Quint `delivery.qnt`; trybuild compile-fail for `Information<T>` |
| 4 | Join: the admitted machine gets exactly the role the ticket named; a code is useless without its proof; expiry arithmetic cannot overflow | Quint; Kani on ticket and HLC arithmetic; a one-off Verifpal model of the handshake |
| 5 | Claims converge: overlapping claims are visible to both holders once in contact, and every machine computes the same winner | Quint `coord.qnt`; proptest that the resolver is commutative and idempotent |
| 6 | One answer: the run's machine is the arbiter; the first answer in its store order under authority chaining to a person is the one, later ones get a refused receipt; `Hold` becomes `Refuse` after ten minutes | Quint plus quint-connect |
| 7 | Codecs: envelope, feed and chain decoding never panic; encoding round-trips; a sealed body opens only for its recipient under its head | The 23 fuzz targets retargeted; Kani on decode bounds; proptest round trips |
| 8 | Presence converges after quiet; a restored or reinstalled machine writes from a new incarnation and never collides with its old entries; a fork within one incarnation is detected and freezes the author | turmoil plus proptest |
| 9 | Relation schema: no userset cycle or unintended inheritance across org, network, project and space roles | Alloy 6, run when the role or subject enums change |

One `formal` CI job under five minutes; larger bounds nightly. A protocol
change edits the spec first. Not verified formally: parsers and drift
(fuzz and golden files), the UI and hook plumbing (e2e), crypto primitives
(audited crates), migrations (round-trip tests).

## Open questions for Will

Questions 1, 2 and 5 are decided, so nothing gates phase 3. The rest
can wait for the phase that needs them.

1. **The person's root key, the presence check and the chain rules.**
   Decided by Will on 2026-11-08. The root key lives in the first
   device's OS keychain by default and is copied to other devices by
   pairing; a hardware key (FIDO2 or a passkey) can hold it instead when
   the person enables one. Since a lost first device is then the
   identity until a second device is paired, setup offers pairing a second
   device, or exporting a recovery phrase, as the next step. A human is
   proven by the OS user-presence prompt (Touch ID, Windows Hello,
   polkit), with a hardware-key tap added for the widest acts only when a
   hardware key is enabled. The chain policy table in the domain model
   stands. A computer-use agent on the same OS account is inside the
   threat model: on macOS and Windows presence is biometric only, with no
   passphrase fallback, so an agent that can type cannot pass it; Linux
   uses polkit or a passphrase and the doctor says it is weaker.
2. **The machine signing key.** Decided by Will on 2026-11-08: hardened
   by default. On macOS the key lives in the Secure Enclave and on Windows
   in the TPM, non-exportable, signing only through the service; an agent
   that reads every file the account owns still cannot sign as the
   machine. On Linux, and wherever the hardware is missing, the key is a
   file and attestation says so. Running the service as its own OS user
   stays an option behind `xenonet setup --harden` for machines without a
   secure element. The transport key stays a file on every platform.
3. **Out of contact.** May a machine that has not heard from any other
   member in ten minutes still start runs, admit, widen or answer remote
   prompts? Recommended: it holds them and answers only for itself.
4. **Unattended answers.** May a standing grant answer permission prompts
   with no person present, as today's delegation does? Recommended: yes,
   per machine, as an explicit person-signed grant with limits, and only
   for runs the asker's attestation is `HarnessSigned` or `ProcessChecked`.
5. **The first cut.** Decided by Will on 2026-11-08: one person with
   many machines and several networks. Phases 2 to 4 build the person,
   machine, network, project and space chains and no org chain; orgs are
   an entry kind the `Policy` trait admits later, not a redesign, and the
   org row of the chain table stays as the default for that day. Two
   persons sharing a project is not in the first cut, so `Merge` ships
   with the data model but is exercised only by tests until then.
6. **Wire format.** Keep the line head and JSON, or move to CBOR with
   COSE_Sign1 for outside interop? Recommended: keep, revisit in phase 6.
7. **Claims.** Stay advisory with a deterministic winner, or allow a
   project to name one arbiter machine for exclusive leases?
8. **Retire now** the SSH hosts prototype and the cloud-assistant read
   grants? Recommended: yes to both; the grants come back as Biscuit read
   tokens if wanted.
9. **The remote map.** Is it acceptable that other machines show as
   session cards from presence, not today's relayed full graph?
10. **Defaults.** Two mailboxes per network, named by the person
    (recommended) or any member may volunteer? Networks start titles-only?
    Direct LAN paths on by default?

## Risks

- Phase 2 is wide. Without the golden files from phase 1 it stalls half
  typed.
- iroh's gossip and docs crates are not stable; only core iroh 1.x is
  used, with direct fan-out until gossip 1.0.
- quint-connect is young (0.1.x); the Quint specs stand on their own if
  the bridge breaks.
- User-presence proofs differ by OS; Linux falls back to polkit or a
  passphrase and is weaker by design, which the doctor states.
- Secure Enclave and TPM signing need platform code (Security.framework,
  Windows CNG) that nothing in the workspace has today.
- Biscuit checks on hot paths need caching; presence is checked once per
  connection, never per delta.
- Mixing old and new ids during phase 4; one id system at a time.
