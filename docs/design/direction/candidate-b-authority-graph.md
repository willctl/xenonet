# Candidate B: an authority graph and typed streams

Architect B, 2026-11-08. Premise given: the primitive is an **authority graph** (principals and relations as Zanzibar-style tuples) carried as **Biscuit facts** in capability tokens, plus **typed message streams** with per-stream ordering, over a **multi-network, multi-relay** fabric of **CBOR + COSE_Sign1** envelopes. Checked against `docs/design/goals.md` and the five grounding notes.

## 1. Shape in one paragraph

Every thing that can act or be acted on is a *principal* with an Ed25519 key: person, organization, machine, network, actor, service, session (sessions get a key too, minted by their machine), and *account*, the one principal kind without a key (an external login a person vouches for). What they may do to each other is a set of **tuples** `(object, relation, subject)` kept in a **signed, hash-chained graph log**, one log per network, replicated to every member machine: today's roster chain, generalized so it carries every relationship and so its signer rules are Datalog over the graph instead of "founder or two window hosts". Every request carries a **Biscuit token** whose blocks are facts from that graph plus attenuations; any machine authorizes it offline against its replica. Everything that moves is an entry in a **typed stream** (messages in a thread, presence of a machine, claims in a project, a permission question and its answer, a graph entry, a context-doc change), each stream with its own ordering and retention rule, pushed over **iroh** QUIC between machines and buffered by **relays** (a role any member machine can hold, several per network) that forward sealed envelopes they cannot read. The console, CLI and MCP servers are surfaces that call `xn-coord`; vendors plug in behind one `Adapter` trait; Xenolith is one more stream kind over one more object kind (`space`).

### Module map

| crate | one job | depends on | standalone? |
|---|---|---|---|
| `xn-ids` | key ids (`did:key` rendering), typed principal/object ids, content hashes | — | yes |
| `xn-model` | the domain: principals, objects, relations (tuples), stream kinds and their payload types; no IO | `xn-ids` | yes |
| `xn-envelope` | CBOR + COSE_Sign1 envelopes; sealing; `Unverified → Verified → Opened<T>` typestate | `xn-ids`, `coset`, `ciborium`, `ring`, `x25519-dalek` | yes |
| `xn-authz` | Biscuit tokens: mint, attenuate, verify; the Datalog rule set; revocation; `Decision` | `xn-model`, `biscuit-auth` | yes |
| `xn-graph` | the signed graph log (entries, fork rule, set reconciliation) and its materialized tuple view; a `GraphStore` trait | `xn-model`, `xn-envelope`, `xn-authz` | yes |
| `xn-streams` | typed streams: ids, `Order`/`Retention` classes, per-publisher sequence + prev hash, cursors, dedupe; `StreamStore` trait | `xn-model`, `xn-envelope` | yes |
| `xn-net` | membership-aware transport: iroh endpoints, networks as topics, relays, queues for offline members, join handshake; delivers envelopes, nothing else | `xn-envelope`, `xn-graph` (read-only `Membership` trait), `iroh` | yes (with `xn-graph` or any `Membership` impl) |
| `xn-coord` | coordination rules: send/inbound policy, claims conflicts, handoffs, starts, permission questions, wake decisions; the one API surfaces call | `xn-streams`, `xn-authz`, `xn-graph`, `xn-adapter` (traits only) | with the three below it |
| `xn-adapter` | the vendor SPI: `Adapter`, `SessionSource`, `Deliver`, `Connect`, `AccountSource`; the registry manifest type | `xn-model` | yes |
| `xn-context` | Xenolith: spaces, pages as Automerge docs, change stream, space keys | `xn-streams`, `xn-authz`, `automerge`, `samod` | yes |
| `xn-sqlite` | one SQLite file per machine; implements `GraphStore`, `StreamStore`, `TokenCache`, `ContextStore` | the four store traits, `rusqlite` | no (glue) |
| `xn-edge-mcp`, `xn-edge-a2a`, `xn-edge-acp` | protocol edges: MCP tools in a `io.xenonet` extension namespace (rmcp 3), A2A 1.0 in/out (a2a-rs), ACP session control (agent-client-protocol 5) | `xn-coord`, `xn-adapter` | no (edges) |
| `adapter-claude`, `adapter-codex`, `adapter-grok`, `adapter-hermes`, … | one crate per platform, implementing `xn-adapter` | `xn-adapter` | each alone |
| `xenonet-service`, `xenonet-cli`, `xenonet-desktop-ipc` | surfaces: wire the crates, hold no rules | everything above | no |

Arrows go one way, down the table. Nothing below `xn-coord` names a vendor, a port, a folder or a window. `xn-model` and `xn-envelope` are the only crates that know the wire's *content types*, and `xn-envelope` knows them only as opaque `(content_type, bytes)`.

## 2. Domain model

Stored vs derived, in one sentence each:

- **Graph log** (stored, signed, replicated per network): durable relationships between long-lived principals and objects. Thousands of tuples, changes per day, never per message.
- **Session certificates** (stored on the issuing machine, carried in envelopes): a session's key, `runs_on`, `works_in`, `acts_as`. Sessions are too many and too short-lived for the graph; their machine vouches for them.
- **Stream stores** (SQLite, per machine): the entries of each stream this machine publishes or subscribes to, with cursors. Messages, claims, questions, receipts, graph entries, context changes.
- **Presence** (derived, ephemeral): last entry per publisher of a `Presence` stream; never queued, never stored past its TTL.
- **Sessions, projects, accounts as observed** (derived by adapters): what the vendor files say, read leniently with drift records as today; only what a principal *asserts* (a cert, a tuple, a stream entry) becomes a fact others see.
- **Context docs** (Automerge, per space, per machine that reads the space): CRDT state; the change stream is the sync.

```rust
// ─── xn-ids ───────────────────────────────────────────────────────────────
/// An Ed25519 public key; displayed as did:key:z6Mk…, parsed from that or
/// from today's xnk1_… ids (same bytes, new rendering).
pub struct KeyId([u8; 32]);
pub struct Hash([u8; 32]);                    // SHA-256, base64url when shown
pub struct ContentType(pub &'static str);     // "xn/message.v1" …

macro_rules! key_id_newtype { /* PersonId(KeyId) … with no cross-conversion */ }
key_id_newtype!(PersonId, OrgId, MachineId, NetworkId, ActorId, ServiceId, SessionId, SpaceId);

/// Projects are not keyed: a project is the set of clones of one repository
/// (or a folder that several machines agree is one thing), so its id is a
/// content id: hash of the normalized remote, or of a founder-signed name.
pub struct ProjectId(Hash);

/// The one principal without a key of its own: an external login.
pub struct AccountRef { pub provider: Provider, pub external_id: String }
pub enum Provider { Claude, Codex, GitHub { host: String }, GitEmail, Other(String) }

// ─── xn-model ─────────────────────────────────────────────────────────────
/// Anything that can sign.
pub enum Principal {
    Person(PersonId), Org(OrgId), Machine(MachineId), Network(NetworkId),
    Actor(ActorId), Service(ServiceId), Session(SessionId),
}
/// Anything a tuple can be about. Every Principal is also an Object.
pub enum Object {
    Principal(Principal),
    Account(AccountRef),
    Project(ProjectId),
    Space(SpaceId),
    Stream(StreamId),
}
/// The right-hand side of a tuple: one principal, or everyone who holds a
/// relation on another object (Zanzibar's userset: "members of org O").
pub enum Subject { Principal(Principal), Userset { object: Box<Object>, relation: Relation } }

/// Closed set. Adding a relation is a model change, reviewed as one.
pub enum Relation {
    // ownership and membership
    Owner, Admin, Member,
    // machines and networks
    MemberMachine, Relay, Device /* machine of a person */,
    // where work happens
    RunsOn, WorksIn, Clone /* project ↔ machine path */,
    // identity
    Holds /* person holds account */, ActsAs /* session acts as account */,
    // capabilities over objects
    MayRead, MayWrite, MayMessage, MayClaim, MayStart, MayAnswer, MayPublish,
    // negative facts
    Revoked,
}
pub struct Tuple { pub object: Object, pub relation: Relation, pub subject: Subject }

/// A durable change to the graph: who asserted it, under what token.
pub struct GraphEntry {
    pub network: NetworkId,
    pub n: u64,
    pub prev: Option<Hash>,                   // None only for the founding entry
    pub op: GraphOp,
    pub at: Timestamp,
    pub asserted_by: Principal,
    pub authority: TokenRef,                  // the Biscuit that let them (hash; body in the envelope)
}
pub enum GraphOp { Found { network: NetworkId, by: PersonId }, Add(Tuple), Remove(Tuple), Rotate { who: Principal, new_key: KeyId } }

/// What a machine says about a session it runs. Signed by the machine's key,
/// valid for `ttl`; renewed while the session lives. Not a graph entry.
pub struct SessionCert {
    pub session: SessionId,
    pub runs_on: MachineId,
    pub works_in: Vec<ProjectId>,
    pub acts_as: Vec<AccountRef>,
    pub platform: PlatformId,                 // "claude-code", from the adapter manifest; opaque to the core
    pub issued: Timestamp, pub ttl: Duration,
}

pub struct Network { pub id: NetworkId, pub name: String, pub founded_by: PersonId }
pub struct Space { pub id: SpaceId, pub name: String, pub owner: Principal }
```

Invalid states made unrepresentable: an id of one kind cannot be used as another (`SessionId` is not a `MachineId`; there is no `String` id anywhere); a `GraphEntry` cannot exist without the `authority` that produced it; a `SessionCert` names exactly one machine, so a session is never "local" by omission; there is no `actor:you` constant, the person is a `PersonId` whose devices are tuples; `Relation` is closed so no stringly capabilities; `Subject::Userset` makes "all members of org O may read space Z" one tuple, not a hand-synced list.

## 3. Protocols

**Envelope.** One COSE_Sign1 structure, CBOR-encoded, signed by the sending principal's Ed25519 key. The protected header carries what today's nine-line head carries, as labels: `alg EdDSA`, `kid` (sender `KeyId`), `xn.net`, `xn.stream`, `xn.seq`, `xn.prev`, `xn.ct` (content type), `xn.at`, `xn.to` (recipient `MachineId` or `Any`), `xn.tok` (token hash; the full token rides in the unprotected header the first time a receiver sees it, then by hash), `xn.cert` (session cert, same rule). The payload is either plain CBOR or a `COSE_Encrypt0` sealed to the recipient machine with the same scheme as today (ephemeral X25519 + HKDF-SHA256 + ChaCha20-Poly1305, protected header as AAD, single-use key so the fixed nonce stays safe) wrapped in COSE's structure. Every envelope is version-tagged by its content type; unknown types are refused, as now. Adopted: `coset`, `ciborium`; kept: the key bytes, the sealing math, the known-answer fixtures (re-encoded once).

**Transport.** `iroh` 1.x: a machine's `MachineId` is its iroh endpoint id, so dialing a machine is dialing its key; hole punching and relay fallback come with it. XenoNet does not use iroh's public relays: a *relay* is a role held by one or more member machines of a network (tuple `(network, Relay, machine)`), and `xn-net` runs iroh-relay there plus XenoNet's store-and-forward queue. Direct machine-to-machine QUIC when reachable; via any relay of that network otherwise. One iroh endpoint per machine, every network it belongs to a topic on it. Adopted for the data path; rejected: libp2p (heavier, 0.x), NATS (daemon), own HTTP(S) long-poll (today's; kept only as a fallback `Transport` impl for hosts behind HTTP-only egress).

**Finding each other.** Members learn addresses from the graph (`Relay` tuples carry an address attribute) and from iroh discovery (DNS/Pkarr, optional, user's choice). A machine in several networks has one endpoint and several graph replicas.

**Joining.** Invitations replace tickets: an admin device mints a Biscuit `invite` token (`check if time($t), $t < expiry; check if use_count 1`), hands it out of band (short code renders the token's hash + a 50-bit secret, as today). The joiner dials any relay, presents the invitation and its new machine key; the relay checks the token offline and *proposes* an `Add((network, MemberMachine, M))` entry signed by the joiner; an admin device signs it on its next connection (or the inviter pre-signs it into the invite, in which case the relay appends at once). The joiner holds a pinned relay key bounded by the invitation, like today's TOFU-with-ticket. Second proof when the invite was not minted on that relay: kept.

**Relay/gateway.** A relay forwards envelopes it can neither read nor forge, buffers queued stream kinds per `(network, recipient machine)` with the same bounds as today (count, bytes, age; overflow drops oldest and emits a `Gap` to both sides), serves graph-log catch-up, and nothing else: no tickets, no tokens, no actor rows. Several per network; a sender uses any; a recipient drains all of them and dedupes by envelope id. No relay is required: two machines on a LAN exchange directly, and a network of one relay that goes down is a network that queues locally until a relay or the peer is back.

**Streams and their semantics.** `StreamId = (network, kind, scope, publisher)`. Within one stream, entries carry `seq` and `prev`, so a subscriber has a total order per publisher and detects loss. Across publishers there is no order, only `reply_to` causality. Each kind declares its class:

| kind | scope | order | retention | notes |
|---|---|---|---|---|
| `Message` | thread | per publisher | queued, 7 d | at-least-once, exactly-once into an agent by id |
| `Receipt` | thread | per publisher | queued | stored / delivered / seen / held / refused |
| `Presence` | machine | last-writer-wins | live, TTL 30 s | full set on subscribe, deltas after: fixes the 64 KiB and O(hosts²) problem |
| `Claim` | project | add-wins set | queued, TTL per claim | holder + path + expiry; conflicts computed by the reader |
| `Question` / `Answer` | run | per publisher | queued | permission prompts; one `Answer` wins, by graph authority then time |
| `Start`, `Handoff` | project | per publisher | queued | request / outcome pairs with a typed state machine |
| `Graph` | network | hash chain | forever | the authority log itself |
| `Context` | space | Automerge | forever | sync messages for one space's docs |
| `Map`, `Svc` | machine | per publisher | live | console relay and published services, as today, unqueued |

Push, not poll: a subscriber opens one QUIC stream per `(network, kind, scope)` selector and receives entries as they are published; offline time is covered by the relay queues and `cursor` resume. Where MCP, ACP and A2A sit: at the edge, in `xn-edge-*`, each a translation to `xn-coord` calls; no edge type appears below `xn-coord`.

## 4. Authorization

**Principals and roots.** A person's root key is created once, kept offline (printed recovery, owner's choice), and used only to pair devices and to recover. Each device (machine) holds a Biscuit minted by the root with the block `device(M, P)` and `check if right($r), $r in {admin, invite, publish}` as the person decides per device: today's "window host" becomes `(person, Device, machine)` plus `(network, Admin, person)`. Orgs are roots too, with persons as `Admin`/`Member`. Networks are founded by a person, so a network's authority chains to that person's root; a second admin is a tuple, so the founder is no longer irremovable by construction, only by the rule the owner picks (the default rule keeps one admin always).

**Issuing and delegating.** All authority is a Biscuit: root block from a root key, attenuation blocks appended by holders. A machine attenuates its device token into a session token: `session(S); runs_on(S, M); check if operation($op), $op in {send, claim, read_presence, read_context}` plus `works_in(S, P…)`. A session attenuates further when it delegates work to a started session, and can only narrow. Actors and services hold tokens a person minted with exactly the facts they need. Delegation across machines is appending a block and handing the token over; "no wider than given" is Biscuit's semantics, not XenoNet's code.

**Checking.** A receiver builds an authorizer from: the token; the tuples of the relevant network's graph replica, loaded as facts `rel(object, relation, subject)`; the session cert as facts; `time`, `operation`, `resource`. The rule set lives once, in `xn-authz`, and is the same on every machine. Example: a message may be injected into session R if `rel(R, runs_on, here)` and either the sender and R `works_in` a common project or a `MayMessage` tuple exists; a claim stands if the claimant `works_in` the project; a start on machine M needs `rel(M, MayStart, sender-or-userset)`; an answer to a permission question needs `MayAnswer` and a fresh `human_present` block.

**Messages are information, never authorization.** Encoded three ways. (1) The only input to an authorizer is the token and the graph; `Opened<Message>` has no accessor that yields a `Token` or a `Tuple`, and the `Grant` stream kind does not exist: the graph changes only through `GraphEntry`, which needs an admin token, never through a `Message`. (2) When `xn-coord` hands a message to an adapter it is wrapped in `Information<Message> { from: Provenance, policy: Accept | Hold }`, where `Provenance` is derived from the verified envelope on the receiving side, never copied from the sender's self-report (closing today's "strong you" hole). (3) Per-recipient inbound policy `accept / hold / refuse` (borrowed from Claude Code's cross-session rule) is a tuple attribute on the session cert's platform mode, computed from both ends.

**Revocation.** Every Biscuit block carries a revocation id; the graph has `Remove` entries and `(x, Revoked, token-ref)` tuples; authorizers consult the local replica's revocation set. Propagation is graph sync, so an offline machine honours a revocation only when it reconnects; the bound is the token's TTL (session tokens hours, device tokens long-lived but revocable by any other admin device of the person). That is the honest limit: there is no instant global revocation without an always-on authority, and the goals forbid requiring one.

**What the person alone can do.** Anything that admits, widens or publishes: found a network, add or remove a machine, grant `Admin`, `MayStart`, `MayAnswer`, `MayPublish`, pair a device, approve an actor, publish a service, widen a space's readers. Each needs a token chaining to a person root *and* a `human_present(at)` block signed by a device within 60 s. The block is minted by the surface that proved a human is there (the desktop's window, a terminal confirmation on Linux); the core only checks it. Surfaces that cannot prove it cannot mint it.

## 5. Vendor and surface adapters

One trait set in `xn-adapter`, one crate per platform, loaded from a manifest (today's `apps.json` entries become `AdapterManifest` values; the doctor's vendor rows and `connect` are derived from them, not hand-listed):

```rust
pub trait Adapter: Send + Sync {
    fn manifest(&self) -> &AdapterManifest;              // id, name, capabilities, config files it touches
    fn sessions(&self) -> Box<dyn SessionSource>;         // observe live sessions → SessionObservation events (push)
    fn deliver(&self) -> Option<Box<dyn Deliver>>;        // inject Information<Message>, wake, start, answer a prompt
    fn connect(&self) -> Option<Box<dyn Connect>>;        // install hooks / MCP config; report what it would change
    fn accounts(&self) -> Option<Box<dyn AccountSource>>; // which AccountRefs this platform is signed into
}
pub trait SessionSource { fn watch(&self, sink: Box<dyn Fn(SessionObservation)>) -> Result<Watch, AdapterError>; }
pub trait Deliver {
    fn inject(&self, s: &SessionId, info: Information<Message>) -> Result<Delivery, AdapterError>;
    fn start(&self, req: StartRequest) -> Result<StartOutcome, AdapterError>;
    fn answer(&self, q: &QuestionId, a: Answer) -> Result<(), AdapterError>;
}
```

The core sees `PlatformId` as an opaque string from a manifest. The console, CLI, MCP and desktop IPC call `xn_coord::Coord` methods that take and return `xn-model` types; they hold keys for `human_present` and nothing else. ACP is one `Deliver` implementation (`adapter-acp`) that drives any ACP-speaking agent (Grok Build, Hermes, the Claude Code ACP adapter), so new platforms that speak ACP need no adapter of their own.

## 6. Xenolith

A space is an `Object::Space` with `Owner`, `MayRead`, `MayWrite` tuples (usersets allowed: "members of project P may write"). Pages are Automerge documents (Markdown text + metadata), synced as the `Context` stream over the same `xn-net`, each change envelope signed by the writing session's token which must carry `MayWrite(space)`. Confidentiality across relays: a space has a symmetric space key, sealed to each reader machine's key when the reader is admitted, rotated when a reader is removed (removal is a graph entry, rotation follows it). The curator is an `Actor` with `MayWrite` on the spaces it files into. Strain: per-change authorization inside a CRDT is unsolved (Keyhive pre-alpha), so authority is per space, not per page or per line; a writer admitted to a space can edit any page in it, and revocation is forward-only (past reads stay read).

## 7. Usage sketch

```rust
// (a) An agent on machine A sends to a session on machine B, from the MCP edge.
let coord: &Coord = ctx.coord();
let me: Authority<Session> = ctx.session_authority()?;         // its Biscuit + cert, from the service
let to = coord.resolve(&me, Target::Title("payments refactor"))?; // → Principal::Session(…) with runs_on B
let ticket = coord.send(&me, Outgoing::message(to, Thread::reply_to(prev), body))?;
// xn-coord: policy (rate, loop, size) → xn-streams: append to Message stream (thread, publisher = me)
// → xn-net: seal to B's machine key, sign with session key, push direct or via any relay of the network.
// B: verify → authorize(token, graph) → Information<Message>{policy} → adapter.deliver().inject().

// (b) A new machine joins a second network, from the CLI.
let invite: Invite = Invite::parse(code)?;                        // token hash + secret, out of band
let net = xn_net::join(&machine_key, invite, relay_hint).await?;  // dials relay, proves, proposes Add(MemberMachine)
coord.graph(net.id).await_entry(net.join_entry)?;                 // pending until an admin device signs, or immediate if pre-signed
// Nothing about the first network changes: a second replica and a second topic on the same endpoint.

// (c) Another project embeds only the authorization crate.
let authz = xn_authz::Rules::default();
let graph = my_tuple_source.tuples(scope);                        // any Iterator<Item = Tuple>
let decision = authz.check(&token, graph, Request { op: Op::Write, resource: Object::Space(id), at: now })?;
match decision { Decision::Allow(trace) => …, Decision::Deny(reason) => … }
```

## 8. Migration order

Each phase ships, deletes before it adds, and keeps the three-service tests green on the new wire.

1. **Types and wire.** Add `xn-ids`, `xn-model`, `xn-envelope`. Replace JSON-line frames with COSE envelopes (re-encode `test/fixtures/net/` known answers once). Delete `schemas/network/*`, the JS network twins, `core/schema/network.js`, the zod-generated frame schemas. Keep: key bytes and `xnk1` parsing, sealing, replay windows, `net_*.rs` harness.
2. **Authority.** Add `xn-authz`. Biscuit tokens replace actor bearer tokens, the window key, gateway tokens, assistant grants and tickets; the roster's signer table becomes Datalog; delivery grants and selector scopes become tuples. Delete `delivery.json` grants, `actors/limits` capability strings, `operator.rs` rules, `reach.rs` enums, self-reported auth strength on the wire. Keep: token-as-hash storage, actor approval in the window (now `human_present`).
3. **Graph and fabric.** Add `xn-graph`, `xn-net` on iroh with the HTTP link as fallback transport. Roster chain → graph log; gateway → relay role, N per network, machine in N networks. Delete `hosts.json`/SSH prototype, the second host-id namespace, `gateway.sqlite` tickets, `network.json`'s single-network shape. Keep: queue bounds, `Gap`, clock-skew handling.
4. **Streams and coordination.** Add `xn-streams`, `xn-coord`, `xn-sqlite`. Engine methods become typed `Coord` calls; presence becomes deltas; claims scoped by `ProjectId`; questions/answers typed. Delete `peer.rs` string dispatch, `Arg`/JS emulation (`nodepath`, `jsstr`, `jsregex`, `jstext`, `jsv`), `wire-engine`, `wire-map`, `xenonet-node` twins and the 50k lines of JS they mirror; `engine.sqlite` migrates once or is rebuilt (prototype: rebuilt).
5. **Adapters and edges.** Add `xn-adapter` and one crate per platform; `connect.rs`, `verify.rs`, `procs.rs`, `starts.rs` split along the SPI; registry drives doctor and connect. Add `xn-edge-acp` (ACP for starts and prompts), move A2A to `xn-edge-a2a` with outbound, MCP tools into the `io.xenonet` extension namespace on rmcp 3. Delete every `"claude"`/`"codex"` literal outside `adapter-*`.
6. **Xenolith on the model.** Add `xn-context`; Xenolith's workspace joins the root workspace; spaces become graph objects; sync moves from the opaque `svc` to the `Context` stream. Delete Xenolith's private copies of private-file, X25519, SHA-256 and token code; keep `svc` for third-party services.

## 9. Verification plan

| # | property | tool |
|---|---|---|
| 1 | Graph fork safety: no two members accept different entries at the same height; a relay that drops, reorders, duplicates or invents (without keys) cannot cause it | Quint `graph.qnt` + `quint-connect` driver in `xn-graph/tests` |
| 2 | Authority soundness: every tuple in any member's view traces to an entry whose `authority` token was valid against the view at height n−1 | Quint invariant + proptest over entry sequences |
| 3 | Attenuation monotone: for every (parent, child) token pair produced by `attenuate`, `authorizes(child) ⊆ authorizes(parent)` over the closed `Relation`×`Op` matrix | exhaustive unit matrix + proptest; Alloy 6 for the relation schema (userset cycles, many-to-many) |
| 4 | Revocation convergence: once a revocation is in one connected member's view, every connected member denies that token within one sync round; no member ever accepts it after seeing it | Quint liveness (bounded) + turmoil seeds with partitions |
| 5 | Delivery: a queued entry reaches every subscribed member at least once after reconnect; never a non-member; per-publisher order holds; duplicates are dropped by id | Quint `leadsTo` + turmoil (relay dies mid-batch, two relays, skew) |
| 6 | Claims: after convergence no two sessions hold overlapping claims in one project's view unless a conflict is surfaced to both | Quint `coord.qnt` + `proptest-state-machine` |
| 7 | One answer: a permission question takes exactly one `Answer`, from a principal with `MayAnswer` and a fresh `human_present`; late or unauthorized answers are refused | Quint + unit matrix |
| 8 | Codec and handshake: envelope decode never panics up to max size; invite single-use and expiry arithmetic never overflow; join handshake authenticates both ways and keeps the invite secret from an active relay | Kani harnesses + cargo-fuzz; Verifpal model of join (one-off) |

CI: one `formal` job under five minutes (quint run + bounded verify, seeded turmoil, fuzz smoke); nightly for larger bounds and 100k seeds. Specs live in `crates/*/formal/` next to the types they model; a protocol change edits the spec first.

## 10. Rationale

**Tradeoffs accepted.**
- Tokens on every envelope (by hash after first sight) cost bytes and a cache; bought: offline authorization on any machine with no authority server.
- Sessions out of the graph: cross-machine references to a session need its cert in hand, so `peers` is served from presence, not from the graph; bought: the graph stays small and rarely written, so replication is cheap.
- Revocation is eventual, bounded by TTL; bought: no single point of failure.
- Per-publisher ordering only; bought: no consensus, no global clock; `reply_to` gives agents the causality they need.
- A closed `Relation` enum means a new kind of authority is a release, not a config change; bought: the authorizer rule set is finite and testable exhaustively.

**Alternatives rejected.**
- One signed log of everything: one ordering domain makes presence churn fight messages for sequence numbers and bloats catch-up; typed streams let each kind pick retention and order.
- OpenFGA/SpiceDB server: needs an always-on authority; the graph replica plus Biscuit does the same offline.
- UCAN: right model, no 1.0 crate; Biscuit v6 is shipped and does attenuation plus Datalog.
- Macaroons: no public-key verification, so relays could not check invites offline.
- Keeping the HTTP gateway as the primary transport: hub-required and one SQLite; kept only as a fallback `Transport`.
- Automerge for everything: a CRDT for messages or claims hides the ordering question rather than answering it; used only where text merges (Xenolith).
- Person key on every device: simple, but one stolen laptop is the person; device delegation costs a pairing step and buys revocable devices.

**Where the premise strains (honestly).**
- *Token size and refresh.* A session token with cert and three blocks is around 1 KB; fine for 10k messages/hour, but presence at 1,000 sessions must not carry tokens per session: presence is published by the machine, under the machine's token, listing sessions by id. Refresh: session tokens expire in hours and are re-minted by the machine silently; device tokens are long-lived, so revocation, not expiry, is the control. If a person has no second device, revoking the first means using the offline root.
- *Revocation propagation.* Eventual, through graph sync. A partitioned machine keeps honouring a revoked actor until it reconnects. Mitigation is TTL on everything an actor holds and `Hold` as the default inbound policy for weakly attested senders. This is a real gap for a stolen device, and the owner should know it.
- *Graph consistency across machines.* A hash chain per network means one writer at a time per network: two admins adding machines concurrently fork, and one proposal is refused and retried, as today. At 50 machines and human-rate changes that is fine; at machine-rate graph writes it would not be, which is why sessions and presence are kept out of it. Multiple networks are independent chains; a machine in several has several replicas and nothing joins them, so an org spanning networks is modelled by the same `OrgId` appearing in each, not by one global graph.
- *Who hosts the authority graph.* Every member, as a replica; relays also serve catch-up. There is no authority host, which means there is no place to ask "what is true now" except the local replica, and two members can disagree for a sync round. The design accepts that and makes every decision reproducible from (token, replica height), which is also what makes it model-checkable.
- *Space confidentiality.* A per-space symmetric key is group messaging without MLS; forward secrecy is absent and rotation on removal is best-effort. SLIM/MLS later if spaces outgrow a few dozen readers.

**Open questions for the owner.**
1. Person root offline with printed recovery, or a root held in the first device's keychain and copied by pairing?
2. Default for a founded network: one admin always kept (founder irremovable, as today), or any admin removable by two others?
3. Should `human_present` be requirable for *answering permission prompts* by default, or only for admitting and widening? It decides whether delegated starts can run unattended.
4. Direct machine-to-machine on a LAN without any relay: on by default, or opt-in per network?
5. Space keys: accept no forward secrecy for Xenolith now, or wait for an MLS crate?

**Biggest risks.** iroh's ecosystem is half stable (gossip wire changes; public relays EOL, so the relay role must ship) and is the largest new dependency; `coset` says its API may change; Biscuit Datalog rule sets can be subtly permissive, which is why property 3 is exhaustive; the migration deletes the JS contract before all behaviours have Rust tests, so phase 4 needs the console e2e kept green throughout; and adding `Relation` variants is tempting and each one widens the authorizer.

**Self-screen.** *Shallow modules:* `xn-ids` is thin but it is the one thing every crate shares, and it keeps `xn-model` out of `xn-envelope`; `xn-sqlite` is glue by design. *Information leakage:* wire labels live in `xn-envelope`; `xn-model` payload types are CBOR-derived but carry no header fields; the one leak is `ContentType` strings known to both, kept as constants in `xn-model`. *Temporal decomposition:* none; crates are by concern, not by phase. *Pass-through layers:* `xn-coord` could become one if it only forwards to streams; its job is policy and the inbound wrap, and it is where to watch. *Split ownership:* each store trait is owned by the crate whose types it stores; `xn-sqlite` owns only the file. *Two ways to do one task:* the HTTP fallback transport is a second path; it exists for egress-restricted hosts and is a `Transport` impl, not a second protocol. *Hand-synced lists:* the vendor registry, the doctor's rows and `connect` all derive from `AdapterManifest`; the one remaining list is `FRAME_KINDS`-like `ContentType` constants, checked by a test that every stream kind has one.
