# Candidate C: subtract first, then reshape what remains

Architect C, 2026-11-08. Starting constraint: begin from the ~177k lines of Rust (plus ~50k lines of shipped JS) that exist, decide what goes, keep what already serves the goals, and only then add the smallest typed core that reaches them. The service keeps working at the end of every phase. Section 10.5 says plainly where subtraction isn't enough.

## 0. The subtraction ledger

Sizes are measured line counts unless marked "est."

**Delete**

| What | Size | Phase | Replaced by |
|---|---|---|---|
| JS core and its twins: `core/` (209 files), `server/`, JS `cli/`, JS `mcp/` | ~50k JS | 1 | Golden files and JSON Schema generated from Rust become the contract |
| `xenonet-node` (napi twins), `parity.json`, twin unit tests, the `test:rust` job | 15.9k Rust, ~25k JS tests (est.) | 1 | Nothing |
| Electron's second installer and updater (`core/components/*.js` imported by `electron/`), `launcher.cjs` embedded by `components/install.rs` | in `core/` | 1 | The one Rust installer (`xenonet setup`, `xenonet update`) |
| SSH hosts prototype (`xenonet-hosts`, `hosts.json`, `core/hosts/`, `XENONET_HOSTS`), and with it the second host-id namespace | 4.6k Rust + JS | 1 | Network membership (network.md open question 4) |
| zod as the schema source; the runtime JSON-Schema checker (`schema/check.rs`, `jsregex.rs`) | 2.2k | 2 | serde types, `schemars` export |
| The string-dispatched `Calls` protocol (`engine/peer.rs`, `engine/js.rs`, `jsre.rs`, `wire-engine/js.rs` `Arg`, `world.rs`, `service/src/cores/`) | ~1k plus glue | 2 | Direct typed calls |
| JS emulation (`nodepath.rs`, `jsstr.rs`, `jstext.rs`, `jsv.rs`) | 2.1k | 2 | `std::path`, `regex`, serde |
| `wire-engine`, `wire-app`, `wire-map` as crates | 29k | 2 | Dissolved: logic moves to its owner, pass-through is deleted (about half, est.) |
| Five authorization rule sites: delivery grants and selectors (`delivery.json`), actor capability lists, `reach` start/wake enums, `operator::check`, assistant grants, `xenonet_gateway_` tokens, A2A tokens | — | 3 | One crate, `xenonet-authz` (Biscuit) |
| The map's `relay` frame kind, `RelaySink`, relay batches, the 32 MiB relay memory | — | 5 | Consoles read the local replica of presence |
| HTTP long-poll link, signed HTTP requests, the gateway-set clock offset, the single gateway SQLite (`link.rs`, `request.rs`, `gateway.rs`, `gwstore.rs`, `hoststore.rs`) | ~2.4k | 5 | iroh QUIC links, a hybrid logical clock, mailboxes |
| Polling: 8 s map rescan, 5 s and 3 s observers, 2 s presence tick | — | 4–5 | File watchers and tailers, presence deltas |
| About 12 state files: `engine.sqlite`, `net.sqlite`, `gateway.sqlite`, `a2a.sqlite`, four `delivery*.json`, `network.json`, `roster.json`, `services.json`, the authored half of `state.json` | — | 4 | One `xenonet.sqlite`, plus the rebuildable search index and key files |

**Keep (moved, not rewritten).** The seal-and-sign construction (`frame.rs`, `keys.rs`, `codec.rs`) goes to `xenonet-id`, head tag bumped to `/2`. Roster-chain verification (`roster.rs`) goes to `xenonet-log::chain`, its quorum table becoming one `Policy`. Tickets (`ticket.rs`) go to `xenonet-id::ticket`. Also kept: the three-services-in-one-process tests (later under turmoil), the vendor registry (typed, in `xenonet-adapter`), drift (`lenient.rs`, `drift.rs`, `fields.rs`, to `xenonet-adapter::drift`), the doctor as an aggregator of `Check`s, the SQLite migration discipline (to `xenonet-store`), all 23 fuzz targets, the blocks tailer and scanner (to `xenonet-adapter::io`), A2A inbound and the MCP servers as surfaces, and the shape of today's `Net`/`Sink` seam (small, typed, frame-blind) as the engine's `Transport` trait.

## 1. Shape in one paragraph, and the module map

After subtraction, XenoNet's core is eight typed crates built around two replicated, signed structures. **Chains** are quorum-signed membership logs, generalized from today's roster. One chain each describes a person (their device keys), an org, a network, a project and a Xenolith space. **Feeds** are each machine's gapless, sequenced log of signed operations: messages, receipts, claims and task events. One authorization crate decides every incoming operation, using Biscuit tokens checked against facts from the chains. One transport carries everything: iroh QUIC between machines, with any number of relays and mailboxes and no central gateway. Vendor platforms plug in behind one `Platform` trait. The console, CLI, MCP and A2A are clients of one typed local API. Sessions are still derived from vendor files, because those files belong to the vendors, but adapters push events into an incremental index, so nothing polls. Everything XenoNet itself authors is stored, signed and owned by a key.

```
xenonet-id*       <- (none)                    keys, typed signatures, the kept sealing, tickets, HLC, did:key
xenonet-log*      <- id                        chains (quorum-signed membership) and feeds (per-author sequence)
xenonet-authz*    <- id, biscuit-auth          "may P do A on R under this token, given these facts?"
xenonet-net*      <- id, iroh                  membership-gated push transport; defines Admission
xenonet-model*    <- id                        ids, entities, relations, ops, the action vocabulary; no IO
xenonet-adapter*  <- model                     the Platform trait, typed registry, drift, tailing and watching
xenonet-engine    <- model, authz, adapter     coordination guarantees; defines Store and Transport
xenonet-store     <- engine, model, log        one SQLite, single writer; implements Store and the session index
xenonet-service   <- all of the above          composition root; implements Admission and Transport; no logic
xenonet-api       <- model                     typed local RPC and client: the only crate surfaces import
surfaces          <- api                       xenonet-cli, xenonet-mcp, xenonet-a2a, the console's HTTP/SSE bridge
adapters/*        <- adapter, model            claude-code, codex, grok-build, hermes, opencode, acp, git
xenonet-doctor    <- adapter                   runs every Check that crates and adapters expose
* = usable alone by another project
```

The engine defines `Store` and `Transport` and depends on neither implementation, so another project can run it over its own. `xenonet-net` defines `Admission` and never learns what a network *means*. The win isn't the crate count. Today five hub crates have a fan-in of 10 to 19 and the `wire-*` crates exist only to glue a JS-shaped port. After the change, no core crate has more than three internal dependencies, and vendor and surface code sits in leaf crates.

**Reuse.**
- *Adopted:* iroh 1.x (transport, relays, discovery), biscuit-auth 6 (tokens), rmcp 3.x (MCP 2026-07-28), a2a-rs (the A2A edge), agent-client-protocol (ACP inside adapters), schemars (schemas), and quint-connect, turmoil, proptest and Kani (verification).
- *Borrowed:* Zanzibar tuples (facts), A2A's task states (`TaskState`), NANDA's small signed address records, did:key rendering, hybrid logical clocks, Nostr- and SSB-style signed author feeds.
- *Built, because nothing fits:* chains and feeds (`xenonet-log`, grown from the roster) and the envelope head (kept from today's frames).
- *Skipped:* CBOR/COSE, Automerge, libp2p, the UCAN crates, OpenFGA and SpiceDB servers (reasons in 10.1 and 10.2).

## 2. Domain model

```rust
// ───────────── crates/xenonet-id (standalone) ─────────────
pub mod key; pub mod seal; pub mod ticket; pub mod hlc; pub mod did;

/// Ed25519 public key. Every principal's id is its key; a machine's is also its iroh EndpointId.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId([u8; 32]);
pub struct SealKey([u8; 32]);                       // X25519 public, for sealing to this key's owner
pub struct PublicKeys { pub sign: KeyId, pub seal: SealKey }
pub struct KeyPair { /* zeroized; stored where the caller chooses: 0600 file, OS keychain */ }
pub struct Signature([u8; 64]);
pub struct Digest([u8; 32]);                        // SHA-256, as today

/// Domain separation is a type, not a string at each call site. Each variant is a tag
/// prefix, as `xenonet-roster/1` and `xenonet-net/1` are today.
#[derive(Clone, Copy)]
pub enum Purpose { ChainEntry, FeedEntry, Envelope, AddressRecord, PresenceProof, Ticket }

impl KeyPair {
    pub fn generate() -> Self { todo!() }
    pub fn public(&self) -> PublicKeys { todo!() }
    pub fn sign(&self, p: Purpose, msg: &[u8]) -> Signature { todo!() }
}
impl KeyId {
    pub fn verify(&self, p: Purpose, msg: &[u8], sig: &Signature) -> bool { todo!() }
    pub fn to_did(&self) -> String { todo!() }      // did:key:z6Mk…, format adopted, no library
}

/// Kept from net/frame.rs: ephemeral X25519, HKDF-SHA256 (info = eph pub, recipient key id, head),
/// ChaCha20-Poly1305 under a single-use key with a zero nonce, head as associated data.
pub struct Sealed { pub eph: SealKey, pub ct: Vec<u8> }
pub fn seal(to: &PublicKeys, head: &[u8], plain: &[u8]) -> Sealed { todo!() }
pub fn open(me: &KeyPair, head: &[u8], s: &Sealed) -> Result<Vec<u8>, OpenError> { todo!() }

/// Hybrid logical clock. Replaces "a host adopts the gateway's clock offset (up to a day)".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hlc { pub wall_ms: u64, pub logical: u32 }
impl Hlc {
    pub fn tick(&mut self, now_ms: u64) -> Hlc { todo!() }
    pub fn witness(&mut self, seen: Hlc, now_ms: u64) -> Result<Hlc, SkewTooLarge> { todo!() }
}

// ───────────── crates/xenonet-log (standalone) ─────────────
pub struct ChainId(pub Digest);                    // hash of the genesis entry
pub struct Height(pub u64);

/// What a chain's ops mean and who must sign them. Today's roster table
/// ("add a window host: founder or two window hosts") is one implementation.
pub trait Policy {
    type Role: Clone + Ord + Serialize + DeserializeOwned;
    type Op: Clone + Serialize + DeserializeOwned;
    /// May `entry` extend `state`? Checks signers, quorum, and any authority it carries.
    fn admit(&self, state: &State<Self::Role>, entry: &Entry<Self::Op>) -> Result<(), Refused>;
    fn apply(&self, state: &mut State<Self::Role>, op: &Self::Op);
}
pub struct Entry<Op> {
    pub chain: ChainId, pub height: Height, pub prev: Digest, pub op: Op, pub at: Hlc,
    pub authority: Option<Vec<u8>>,                // opaque here; a policy may require a Biscuit
    pub signers: Vec<(KeyId, Signature)>,          // up to 4, as today
}
pub struct State<R> { pub members: BTreeMap<KeyId, R>, pub revoked: BTreeSet<Digest>, pub head: Digest, pub height: Height }
pub enum Offered { Extended, Duplicate, NeedsApproval, Fork { at: Height } }

pub struct Chain<P: Policy> { policy: P, entries: Vec<Entry<P::Op>>, state: State<P::Role> }
impl<P: Policy> Chain<P> {
    pub fn genesis(policy: P, founder: &KeyPair, op: P::Op, at: Hlc) -> Self { todo!() }
    pub fn verify_all(policy: P, entries: Vec<Entry<P::Op>>) -> Result<Self, Refused> { todo!() }
    pub fn offer(&mut self, e: Entry<P::Op>) -> Result<Offered, Refused> { todo!() }
    pub fn state(&self) -> &State<P::Role> { todo!() }
    pub fn since(&self, h: Height) -> &[Entry<P::Op>] { todo!() }
}

/// One author's entries on one topic, numbered without gaps. This is where per-sender
/// FIFO and catch-up come from. Feeds are signed, so any holder can pass them on.
pub struct Topic(pub Digest);                      // opaque here
pub struct FeedId { pub author: KeyId, pub topic: Topic }
pub struct Seq(pub u64);
pub struct FeedEntry<B> { pub feed: FeedId, pub seq: Seq, pub at: Hlc, pub body: B, pub sig: Signature }
/// The highest contiguous seq per feed. 50 machines means 50 entries per topic.
pub struct Frontier(pub BTreeMap<FeedId, Seq>);
impl Frontier { pub fn missing_from(&self, theirs: &Frontier) -> Vec<(FeedId, Seq)> { todo!() } }
```

```rust
// ───────────── crates/xenonet-model (standalone; pure; schemars for JSON Schema) ─────────────
use xenonet_id::{Digest, Hlc, KeyId, Signature};

// Principals: they act, and each has a key or a machine that vouches for it.
pub struct PersonId(pub KeyId);    // root key: kept offline, printed as a recovery phrase
pub struct DeviceId(pub KeyId);    // the person's key on one machine, listed in the person's chain
pub struct MachineId(pub KeyId);   // one XenoNet service (one OS user on one host)
pub struct ActorId(pub KeyId);     // a bot or program that holds its own key
pub struct AgentId { pub machine: MachineId, pub local: Ulid }  // a harness session, minted on first sight

pub enum Principal {
    Person { id: PersonId, via: DeviceId },       // device-signed, with a user-presence proof
    Machine(MachineId),
    Actor(ActorId),
    Agent { id: AgentId, attested: Attestation }, // its machine signs; authority never exceeds the machine's
}
pub enum Attestation { HarnessSigned, ProcessChecked, Unchecked }

// Groups. Each one is a chain, and membership is many-to-many by construction.
pub struct OrgId(pub Digest);
pub struct NetworkId(pub Digest);
/// Minted, never a path. The chain id hashes the genesis without its signers, so a repo with a
/// remote founded as (owner, RepoKey) gets the same id on all of one person's machines without them
/// talking; anything else gets a random id on first sight. Another person's copy joins by Share or Merge.
pub struct ProjectId(pub Digest);
pub struct SpaceId(pub Digest);

/// Who can hold a role. A group can hold a role in another group (an org in a project,
/// a network as a project's reader): Zanzibar's usersets.
pub enum Subject { Person(PersonId), Machine(MachineId), Actor(ActorId), Org(OrgId), Network(NetworkId) }

pub enum OrgRole { Owner, Admin, Member }
pub enum NetworkRole { Admin, Member, Mailbox }
pub enum ProjectRole { Owner, Maintainer, Contributor, Reader }
pub enum SpaceRole { Curator, Writer, Noter, Reader }

/// Constructors refuse invalid pairs: an Admin is a Person, a Mailbox is a Machine,
/// a space has exactly one Curator (its policy refuses a second).
pub struct NetworkMember { subject: Subject, role: NetworkRole }
impl NetworkMember { pub fn new(s: Subject, r: NetworkRole) -> Result<Self, Invalid> { todo!() } }

pub enum PersonOp  { Found, AddDevice { device: DeviceId, on: MachineId }, RemoveDevice(DeviceId), Revoke(Digest) }
pub enum OrgOp     { Found { name: Label }, Add(Subject, OrgRole), Remove(Subject), Revoke(Digest) }
pub enum NetworkOp { Found { name: Label, admin: PersonId }, Add(NetworkMember), Remove(Subject),
                     Relays(Vec<RelayUrl>), Settings(NetworkSettings), Revoke(Digest) }
pub enum ProjectOp { Found { repo: Option<RepoKey> }, Share(Subject, ProjectRole), Unshare(Subject), Merge(ProjectId) }
pub enum SpaceOp   { Found { about: SpaceAbout }, Share(Subject, SpaceRole), Unshare(Subject), MoveCurator(MachineId) }
pub enum SpaceAbout { Project(ProjectId), Person(PersonId), Org(OrgId), Actor(ActorId), Machine(MachineId) }
pub struct NetworkSettings { pub detail: Detail, pub start: Option<StandingGrantRef> }
pub enum Detail { Full, TitlesOnly, Counts }

// Places. A project has many checkouts; a machine holds many projects.
pub struct Checkout { pub project: ProjectId, pub machine: MachineId, pub root: LocalPath, pub repo: Option<RepoKey> }
pub struct LocalPath(PathBuf);     // never leaves its machine
pub struct RelPath(String);        // '/'-separated, normalized, no "..": what claims use
pub struct RepoKey(String);        // normalized remote, kept from ids.js

// Accounts are vendor sign-ins seen on machines. They link to persons only by a statement.
pub struct PlatformId(SmolStr);    // from the registry; the core never matches on a value
pub struct Account { pub platform: PlatformId, pub native: String }
pub enum Link { Inferred { evidence: Vec<String> }, Stated { by: PersonId, sig: Signature } }
pub struct AccountLink { pub account: Account, pub person: PersonId, pub link: Link }

// Sessions: derived from vendor files, indexed by the store.
pub struct VendorRef { pub platform: PlatformId, pub native_id: String }
pub struct Session {
    pub id: AgentId, pub vendor: VendorRef, pub at: Option<(ProjectId, RelPath)>,
    pub ran_as: Option<Account>, pub status: Status, pub title: String, pub last: Hlc,
    pub ext: VendorExt,            // adapter-owned fields (permission mode, desktop id…), opaque to the core
}
pub enum Status { Working, Waiting(Wait), Idle, Ended }
pub enum Wait { Question, Permission(TaskId), Input }

// Coordination: what crosses machines.
pub struct MessageId(pub Ulid); pub struct ClaimId(pub Ulid); pub struct TaskId(pub Ulid);
pub struct Text<const MAX: usize>(String);

/// Information, never authorization. No field of a Message is read by authz,
/// and no op that needs authority can be built from one.
pub struct Message { pub id: MessageId, pub to: Recipients, pub text: Text<4000>, pub reply_to: Option<MessageId> }
pub enum Recipients { Agent(AgentId), Actor(ActorId), Person(PersonId), Select(Selector) }
pub struct Selector { pub project: Option<ProjectId>, pub machine: Option<MachineId>,
                      pub platform: Option<PlatformId>, pub status: Option<Status> }

/// An advisory lease. Two machines can each grant an overlapping claim while apart.
/// Once they're in contact, both see the overlap and the same winner (earliest Hlc, then key).
pub struct Claim { pub id: ClaimId, pub project: ProjectId, pub paths: Vec<RelPath>,
                   pub holder: AgentId, pub until: Hlc, pub note: Option<Text<200>> }

/// Handoffs, starts and permission questions share A2A's task states.
pub struct Task { pub id: TaskId, pub kind: TaskKind, pub state: TaskState, pub asker: Principal, pub on: MachineId }
pub enum TaskKind { Handoff { to: Recipients, brief: Text<4000> }, Start(StartSpec), Permission(Question) }
pub enum TaskState { Submitted, Working, InputRequired, AuthRequired, Completed, Failed, Canceled, Rejected }
pub struct StartSpec { pub platform: PlatformId, pub project: ProjectId, pub prompt: Text<8000>, pub limits: RunLimits }
pub struct Question { pub run: AgentId, pub tool: String, pub summary: Text<2000> }
pub enum Answer { Allow, Deny }

/// Presence is a stream of deltas, with a full snapshot on (re)connect.
pub struct PresenceDelta { pub machine: MachineId, pub upserts: Vec<SessionCard>, pub gone: Vec<AgentId> }
pub struct SessionCard { pub id: AgentId, pub platform: PlatformId, pub project: Option<ProjectId>,
                         pub status: Status, pub title: Option<String> /* None under TitlesOnly/Counts rules */ }

/// Everything a machine writes to its feed.
pub enum Op {
    Send(Message),
    Receipt { of: MessageId, state: ReceiptState },
    Claim(Claim), Release(ClaimId),
    Task(TaskId, TaskEvent),
    Answer { task: TaskId, answer: Answer },      // refused unless `by` is Principal::Person
}
pub enum ReceiptState { Stored, Surfaced, Held, Refused(RefusalCode) }

/// An op as it travels: what, by whom, in which network, under what authority.
pub struct Act { pub op: Op, pub by: Principal, pub network: NetworkId, pub authority: Option<TokenBytes>, pub at: Hlc }
/// The machine always signs. A person's act also carries the device signature and presence proof.
pub struct SignedAct { pub act: Act, pub machine_sig: Signature, pub person: Option<(DeviceId, Signature, PresenceProof)> }

/// The one place ops map to authz's vocabulary.
pub enum Action { MessageSend, MessageDeliver, ClaimMake, TaskStart, TaskAnswer, ChainAdmit, ChainWiden, Publish, Read }
impl Op { pub fn action(&self) -> Action { todo!() } pub fn resource(&self) -> ResourceRef { todo!() } }
```

**Stored or derived, and where**

| Thing | Stored or derived | Where |
|---|---|---|
| Person, org, network, project and space membership; revocations; relay lists; network settings | Stored, signed | Chains, in `xenonet.sqlite`, replicated to members |
| Messages, receipts, claims, task events, answers | Stored, signed | This machine's feeds (its own) and replicas of the feeds it receives |
| Checkouts, accounts | Derived by adapters (git, vendors) | Index tables, rebuilt on reconcile |
| Account-to-person links | `Inferred` is derived; `Stated` is stored, signed by the person | Index; stated links go in the person's chain |
| Sessions | Derived from vendor files, pushed as adapter events | Session index (incremental); `AgentId`↔`VendorRef` mapping is stored, so ids are stable |
| Other machines' presence | Ephemeral | Memory plus last snapshot; never queued |
| Map layout, names, pins, themes | Surface-owned | The console's own file, outside the core |
| Keys | Secret | Key files or the OS keychain (`KeyPair` doesn't care which) |
| Xenolith pages and notes | Stored | Xenolith's Markdown folder; notes and page versions as feeds (section 6) |

## 3. Protocols

**Layers.** iroh QUIC (hop authentication by endpoint key, encryption on live paths, hole punching, relay fallback), then the **envelope** (signed, and sealed when it may rest at a mailbox), then the **feed** (per-author, per-topic sequence), then the **act** (typed op, principal, authority). Each layer knows only the one below it.

**Envelope.** This is today's head format with a new tag. It's line-based, signed whole and fuzzed already:

```
xenonet-net/2
<scope> <envelope id> <topic> <from key> <to: key | members> <seq> <hlc> <class: live|kept> <sealed: 0|1>
<base64url SHA-256 of body>
```

The body is the JSON of a versioned typed `Act` (`{"v":2,...}`), whose schema is generated by `schemars`. One enum, `Op`, replaces today's three hand-synced kind lists (`FRAME_KINDS`/`HOST_KINDS`/`QUEUED_KINDS` in net, `Kind` in base, `QUEUED_KINDS` in JS). Whether an op is kept or live becomes a method on it.

**Transport.** One iroh 1.x `Endpoint` per machine, under its machine key, with ALPNs `xenonet/link/2` (envelopes and frontier exchange), `xenonet/join/2`, `xenonet/svc/2` (published loopback services as QUIC streams, replacing `svc` frames), plus whatever other crates register (Xenolith's `xenolith/sync/2`). A machine in several networks keeps one endpoint and one key. The envelope's `scope` names the network, and `Admission` checks membership of that scope for every envelope.

**Finding each other.** Chains give every member's key. Addresses come from whichever sources the person turns on: LAN mDNS (the fast path), small signed **address records** (endpoint id, relay URL, direct addresses, TTL; borrowed from NANDA) exchanged in-scope and kept by mailboxes, optional DNS/pkarr, or fixed addresses. A Tailscale or WireGuard address is just another direct address.

**Relays, mailboxes and front doors.** Today "gateway" means three things; here they are three named roles.
- A **relay** is an iroh-relay server: any number, listed in the network chain (`Relays`) and per machine, self-hostable. It sees endpoint ids and ciphertext only.
- A **mailbox** is a member machine with the `Mailbox` role. It keeps sealed `kept` envelopes for away members under today's gateway quotas (10k envelopes, 50 MB, one week per recipient). Kept envelopes go to k=2 mailboxes by default, and the recipient pulls from any. A mailbox can't read or forge what it holds; if it drops or delays something, the seq gap shows.
- A **front door** is a surface, not part of the network: MCP Streamable HTTP and A2A for outside actors, at an address the person chooses, or none.

**Joining.** `xenonet net invite --network studio --role member`, run where an admin is present, issues a Biscuit ticket authority ("admit one machine as Member of N for 10 minutes", rooted at the admin's device key) and prints a non-secret invite (scope plus the inviter's `EndpointAddr`) and a separate 10-character code. On the new machine, `xenonet net join <invite>` asks for the code. The `xenonet/join/2` handshake runs today's HMAC proofs both ways, with today's lockouts. The inviter appends `Add(Machine, Member)` to the network chain, signed and carrying the ticket authority. The joiner verifies the chain from genesis, pinning the inviter by its proof as today. A second network is the same command; the machine's key and other networks are untouched.

**Delivery and ordering.**
- **On the wire:** FIFO per (author, topic), gapless seq, at-least-once. Envelopes go into the store exactly once, deduplicated by (author, seq).
- **Catch-up:** on connect, peers swap `Frontier`s, and the missing entries come from the author or from any member holding them (feeds are signed, so anyone may pass them on).
- **Receipts:** `Stored`, `Surfaced`, `Held`, `Refused`.
- **Into an agent's context:** at-least-once, with the message id visible. A message is acked when the turn that showed it ends (the adapter's turn-end event). This replaces today's at-most-once "marked seen as returned".
- **Ordering:** no global order. The HLC orders things for display, and `reply_to` gives causality within a conversation.

**How each thing travels**

| Act | Class | Route |
|---|---|---|
| `Send` | kept | The recipient's machine, or the members matching a selector (25-recipient cap kept) |
| `Receipt` | kept | The sender's machine |
| `Claim`, `Release` | kept | Machines with a checkout of the project (`Route::Project`), not the whole network |
| Presence | live | Members, as deltas; snapshot on connect; detail as `NetworkSettings.detail` says |
| `Task` events (handoff, start, permission question) | kept | The target machine, plus the asker's machine |
| `Answer` | kept | The asking machine; valid only from a `Person` |
| Chain entries | kept | All members; pulled whenever heads differ |

At 50 machines direct fan-out is fine: 1,000 sessions changing about once a minute is roughly 16 presence deltas a second into each machine, batched per second, and 10,000 messages an hour is about 3 a second. iroh-gossip takes over fan-out once its 1.0 wire is stable.

**Where MCP, ACP and A2A sit.** **MCP** is how agents and actors reach the engine: stdio or HTTP locally, Streamable HTTP with a Biscuit bearer at the front door, targeting the 2026-07-28 stateless revision through rmcp 3.x. **ACP** sits inside adapters: one generic `acp` adapter starts and drives sessions and relays permission prompts for every platform that speaks it (Grok Build, Gemini CLI, Goose, Hermes, Claude Code through the Agent SDK). **A2A** sits at the edge (a2a-rs): inbound tasks map to `Task`s, the Agent Card is signed by the machine key, outbound comes later. It is never the internal bus.

```rust
// ───────────── crates/xenonet-net (standalone) ─────────────
// Membership-gated push transport on iroh 1.x. It knows scopes, peers, topics and envelopes,
// not messages, claims or sessions.
pub struct Scope(pub Digest);                            // a network id, opaque here
pub struct EnvelopeId(pub Ulid);
pub enum Class { Live, Kept }                            // Live is never stored for an away peer
pub enum Dest { Peer(KeyId), Peers(Vec<KeyId>), Members }

/// What the caller's chains say. The service implements it; another project brings its own.
pub trait Admission: Send + Sync + 'static {
    fn members(&self, scope: &Scope) -> Vec<Member>;     // key, seal key, mailbox or not
    fn relays(&self, scope: &Scope) -> Vec<RelayUrl>;
    /// On the inviter, once a joiner's proofs check out: the chain entries to hand it.
    fn admit(&self, scope: &Scope, joiner: &PublicKeys, ticket: &TicketHash) -> Result<Vec<Bytes>, Refused>;
}
pub enum Discovery { Lan, Records, Dns(DnsConfig), Fixed(Vec<SocketAddr>) }   // any mix; none assumed
pub struct NetConfig { pub key: KeyPair, pub discovery: Vec<Discovery>, pub mailbox: Option<MailboxLimits> }
pub struct Delivered { pub id: EnvelopeId, pub scope: Scope, pub from: KeyId, pub topic: Topic, pub seq: Seq, pub at: Hlc, pub body: Bytes }
pub struct Invite { pub scope: Scope, pub inviter: iroh::EndpointAddr }      // not secret; the Code is typed apart

pub struct Net { /* iroh Endpoint and Router, links, outbox, frontiers */ }
impl Net {
    pub async fn bind(cfg: NetConfig, adm: Arc<dyn Admission>) -> Result<Net, NetError> { todo!() }
    /// Returns at once. Kept envelopes go to the peer if connected, else sealed to k mailboxes.
    pub fn send(&self, scope: &Scope, to: Dest, topic: Topic, class: Class, body: Bytes) -> Result<EnvelopeId, NetError> { todo!() }
    /// Push: per-author order, deduplicated, gaps filled from the author or any holder.
    pub fn subscribe(&self, topic: Topic) -> impl Stream<Item = Delivered> + Send { todo!() }
    pub fn ack(&self, from: KeyId, topic: Topic, upto: Seq) { todo!() }
    pub fn peers(&self, scope: &Scope) -> Vec<PeerState> { todo!() }              // online, Direct | Relayed, rtt
    pub fn accept(&self, alpn: &'static [u8], h: impl ProtocolHandler) { todo!() } // other protocols share the endpoint
    pub async fn invite(&self, scope: &Scope) -> Result<(Invite, Code), NetError> { todo!() }
    pub async fn join(&self, invite: Invite, code: Code) -> Result<Vec<Bytes>, NetError> { todo!() }
}

// ───────────── crates/xenonet-engine ─────────────
pub trait Store: Send + Sync {
    fn append_own(&self, act: &SignedAct) -> Result<Seq, StoreError>;
    fn accept_remote(&self, from: MachineId, act: &SignedAct) -> Result<Accepted, StoreError>; // New | Duplicate
    fn inbox(&self, agent: &AgentId) -> Vec<Pending>;
    fn mark(&self, msg: MessageId, state: ReceiptState) -> Result<(), StoreError>;
    fn overlapping(&self, project: &ProjectId, paths: &[RelPath]) -> Vec<Claim>;
    fn sessions(&self) -> &dyn SessionIndex;             // indexed lookups; no scan on a send
    fn ground(&self) -> &dyn xenonet_authz::Ground;      // chain facts and revocations
}
pub trait Transport: Send + Sync {
    fn publish(&self, net: &NetworkId, to: Route, act: &SignedAct, class: Class) -> Result<(), TransportError>;
    fn incoming(&self) -> mpsc::Receiver<(MachineId, SignedAct)>;
}
pub enum Route { Machine(MachineId), Members, Project(ProjectId) }   // Project: machines with a checkout

/// Built by the service from local authentication. Surfaces never construct one.
pub struct Caller { pub principal: Principal, pub token: Option<Token> }

pub struct Engine<S: Store, T: Transport> { store: S, net: T, platforms: Platforms, policy: PolicySet }
impl<S: Store, T: Transport> Engine<S, T> {
    pub fn send(&self, c: &Caller, m: Message, via: Option<NetworkId>) -> Result<SendReceipt, Refusal> { todo!() }
    pub async fn inbox(&self, c: &Caller, wait: Option<Duration>) -> Vec<Pending> { todo!() }
    pub fn peers(&self, c: &Caller, s: &Selector) -> Vec<SessionCard> { todo!() }
    pub fn claim(&self, c: &Caller, p: ProjectId, paths: Vec<RelPath>, ttl: Duration) -> Result<ClaimOutcome, Refusal> { todo!() }
    pub fn release(&self, c: &Caller, id: ClaimId) -> Result<(), Refusal> { todo!() }
    pub fn task(&self, c: &Caller, kind: TaskKind, on: MachineId) -> Result<TaskId, Refusal> { todo!() }
    pub fn answer(&self, c: &Caller, task: TaskId, a: Answer) -> Result<(), Refusal> { todo!() }  // Person only
    /// Every remote act: verify its signature, authz::decide here at the receiver, store, route to a platform.
    async fn on_remote(&self, from: MachineId, act: SignedAct) { todo!() }
}
pub enum ClaimOutcome { Held(Claim), Overlaps { yours: Claim, theirs: Vec<Claim>, winner: ClaimId } }
```

## 4. Authorization

**Principals and how each is authenticated**

| Principal | Authenticated by |
|---|---|
| Person | A device key, used only after a user-presence check (OS auth: Touch ID, Windows Hello, polkit or a passphrase, whichever the platform has). This replaces the window key |
| Machine | Its key: the iroh handshake plus envelope signatures |
| Actor | Its own key: requests are signed, and the Biscuit names that key, so a stolen token alone is useless |
| Agent | Vouched for by its machine. Local evidence (a harness-signed hook, a process check, or nothing) becomes an `Attestation`, which receivers treat as a Datalog fact. An agent's authority is always narrower than its machine's |

```rust
// ───────────── crates/xenonet-authz (standalone) ─────────────
// Biscuit v6 tokens and Datalog; facts in Zanzibar's tuple shape. No network or vendor words.
pub struct Token(biscuit_auth::Biscuit);                 // opaque; never shown to an agent

/// Strings at this boundary because Datalog is; callers build them from typed enums (model::Action).
pub struct Request<'a> { pub principal: &'a str, pub action: &'a str, pub resource: &'a str, pub context: Vec<Fact> }

/// What the token doesn't carry: membership facts, trusted roots, revocations.
pub trait Ground {
    fn facts(&self, req: &Request) -> Vec<Fact>;          // member(G,S,R), share(P,N,role), checkout(P,M), …
    fn root_for(&self, issuer: &str) -> Option<biscuit_auth::PublicKey>;
    fn is_revoked(&self, revocation_id: &[u8]) -> bool;
}
pub enum Decision { Allow { under: Vec<Vec<u8>> }, Hold { ask: String, why: String }, Refuse { why: String } }
pub struct Grant { pub to: String, pub may: Vec<(String, String)>, pub until_ms: u64, pub max_hops: u8 }
impl Grant { pub fn new<'a>(to: &str, may: impl IntoIterator<Item = (&'a str, &'a str)>, until_ms: u64) -> Self { todo!() } }

pub fn issue(root: &xenonet_id::KeyPair, issuer: &str, g: &Grant) -> Token { todo!() }
pub fn narrow(t: &Token, g: &Grant) -> Token { todo!() }   // attenuation: offline, can only narrow
pub fn decide(t: Option<&Token>, r: &Request, ground: &dyn Ground, p: &PolicySet) -> Decision { todo!() }
pub struct PolicySet { /* default-deny Datalog allow, hold and refuse rules, versioned */ }
```

**Issuing and delegating.** Authority is rooted in persons (over what they founded or administer) and in chains (whose admins they list). Routine machine actions rest on membership facts, not tokens. At setup the person grants each machine a standing Biscuit ("agents on M may message members of N and claim in projects checked out on M, until T"). From it the machine mints 24-hour, renewable agent tokens, each narrowed to one `AgentId`. Delegation is Biscuit attenuation: offline, narrowing only, with `max_hops` capping the chain. Facts take Zanzibar's tuple shape: `member(G, S, R)`, `share(P, network:N, "reader")`, and the rule `member(N, x) ⇒ reader(P, x)`.

**Revoking.** A `Revoke(revocation id)` op goes into the issuer's chain (the person's or the network's), spreads like any chain entry, and every verifier checks it. This is honest about offline machines: one that hasn't caught up can accept a revoked token until its TTL ends, which is why agent tokens are short and standing grants are renewable.

**Checking.** Every incoming act is decided at the **receiver** by `authz::decide(token, request, ground, policy)`; a sender-side check is a courtesy. This fixes today's gap where a receiver trusted the sender's self-reported strength. The outcomes are `Allow`, `Hold` (ask the person: today's needs-you, generalized as accept/hold/refuse) and `Refuse`. Local surfaces go through the same check, and read routes need a credential from the component socket handshake, which closes "any local program reads transcripts". A Biscuit check costs tens to hundreds of microseconds per state-changing act, cached per (revocation id, action, resource); presence is checked once per connection, not per delta.

**"Messages are information, never authorization", in the types:**
1. `Message` has no field authz reads.
2. `Answer`, `ChainAdmit`, `ChainWiden` and `Publish` need `Principal::Person`, proved by a device signature the receiver verifies. A machine or agent can't assert it.
3. Tokens never appear in text an agent sees. A grant handed to an agent goes to its adapter, outside the agent's context.
4. Adapters surface messages in a framing that marks them as untrusted information.

**Only the person can:** found a network, org, project or space; admit a machine, actor or person (directly, or by issuing a ticket); widen any role, start policy or wake policy; publish (share a project or space outside its members, open a front door, publish a service); revoke; answer a permission prompt, unless they gave a standing grant for it; change relays or mailboxes. All of these are person-signed acts. The service refuses to produce a person signature for a caller authenticated as an agent, as today's "refused from under an agent" does.

## 5. Vendor and surface adapters

```rust
// ───────────── crates/xenonet-adapter (standalone trait crate; depends on model only) ─────────────
pub struct PlatformInfo { pub id: PlatformId, pub name: String, pub entry: RegistryEntry } // typed apps.json row

pub trait Platform: Send + Sync + 'static {
    fn info(&self) -> &PlatformInfo;
    /// Push. Watch the platform's files, sockets and hooks and emit events. The core never polls a vendor.
    fn watch(&self, out: EventSink) -> Result<Watch, AdapterError>;
    /// A full re-read, at start and to reconcile after missed events.
    fn reconcile(&self) -> Result<Vec<Event>, AdapterError>;
    /// Hook payloads arrive at one generic route and are decoded here.
    fn decode_hook(&self, _raw: &[u8]) -> Result<HookOutcome, AdapterError> { Err(AdapterError::Unsupported) }
    fn checks(&self) -> Vec<Box<dyn Check>>;                 // the doctor's rows for this platform
    fn deliver(&self) -> Option<&dyn Deliver> { None }        // capabilities are typed, not a "has" list
    fn start(&self) -> Option<&dyn Start> { None }
    fn connect(&self) -> Option<&dyn Connect> { None }
}
pub enum Event { SessionSeen(Session), SessionChanged { v: VendorRef, status: Status, title: Option<String> },
                 SessionEnded(VendorRef), TurnEnded { v: VendorRef, shown: Vec<MessageId> },
                 Account(Account), Checkout(LocalPath, Option<RepoKey>), Drift(drift::Record) }
pub trait Deliver {
    fn ways(&self, s: &Session) -> Vec<Way>;                  // in preference order
    fn surface(&self, s: &Session, m: &Surfaced, way: Way) -> Result<Outcome, AdapterError>;
}
pub enum Way { InTurn, Channel, Prompt, Resume, Terminal }
pub trait Start {
    fn start(&self, spec: &StartSpec, auth: RunAuth) -> Result<RunHandle, AdapterError>;
    fn answer(&self, run: &RunHandle, q: &Question, a: Answer) -> Result<(), AdapterError>;
}
/// Returns the config edits it would make; the service applies them after the person agrees.
pub trait Connect { fn plan(&self, mode: ConnectMode) -> Result<ConfigPlan, AdapterError>; }
pub struct Surfaced { pub id: MessageId, pub from: String, pub text: String, pub framing: Framing }
pub mod drift;   // kept from xenonet-schema: Lenient<T>, Record, fields
pub mod io;      // kept from xenonet-blocks: Tailer, Scanner, Watcher (notify)
```

**What moves out of the core.** `engine/connect.rs` (1,775 lines that write Claude, Codex and Grok configs), `engine/toml.rs`, `delivery/{tmux,resume,opencode,channel}.rs`, the vendor branches of `procs.rs` and `verify.rs`, hook payload decoding, `sources/{claude,codex,grok,…}.rs`, and from `actors/` the observers for Hermes, Grok Bot and Muse. Each goes to `adapters/<platform>`. The doctor's 33 hard-coded checks become `Check`s owned by the crate or adapter that knows the thing. Adding a platform means writing one crate that implements `Platform`, plus one registry entry. CI checks that no core crate contains a registry id literal, so "claude" can't creep back into the core.

**Surfaces stay out the same way.** `xenonet-api` is the one typed local protocol: requests, responses and a pushed `Event` stream over a Unix socket or named pipe, every connection authenticated. The CLI, the MCP servers, the A2A edge and the console's HTTP/SSE bridge are clients of it. A CI crate-graph check fails the build if a core crate depends on a surface crate. Console-only state (layout, themes, demo, Spotify) moves out of `xenonet-app` into the console.

## 6. Xenolith

Xenolith keeps its folder format and stays a separate program. It depends on four standalone *libraries* (`xenonet-id`, `-log`, `-authz`, `-net`), never on XenoNet the app.

- **Space = chain.** `SpaceOp::Found { about }` binds a space to a real entity (`ProjectId`, `PersonId`, `OrgId`, `ActorId` or `MachineId`) instead of free text like `project:atlas`. Sharing a project's space follows from the userset rules: whoever reads project P reads the space about P, unless the space chain narrows it.
- **Home = Curator.** The space policy allows exactly one `Curator` machine. That's today's home/satellite rule ("a second curator on a space is a bug") made unrepresentable. `MoveCurator` is the signed handover that `home move` lacks today.
- **Notes** are per-author feeds. **Pages** are the curator's feed of page versions. With a single writer, no CRDT is needed.
- **Grants** are Biscuit tokens (`ctx.read`, `ctx.append`, `ctx.write`, `ask`), replacing Xenolith's own token store and its ten-year machine grants. **Pairing** is the same invite-and-code flow (`xenonet-id::ticket`), so Xenolith's `pair.rs` and its copies of x25519 and private-file code are deleted.
- **Sync** is the `xenolith/sync/2` ALPN on an `xenonet-net` endpoint. When XenoNet runs, Xenolith registers on the service's endpoint; otherwise it binds its own. `svc` forwarding is no longer needed for it, and the port-47850 collision goes away.

## 7. Usage sketch

**An agent on the laptop asks a session on the desktop to take over a crate.** This is the MCP surface's `send` handler. The caller never sees a token.

```rust
// crates/xenonet-mcp/src/tools/send.rs
async fn send(api: &xenonet_api::Client, args: SendArgs) -> Result<ToolText, ToolError> {
    // The connection was authenticated at the handshake: the service matched this MCP
    // session to AgentId via the Claude Code adapter's signed hook pin.
    let to = api.resolve(&args.to).await?;                 // "@desktop codex fix-ci" -> Recipients::Agent(AgentId)
    let r = api.send(SendReq { to, text: args.text, reply_to: args.reply_to, via: None }).await?;
    Ok(ToolText::sent(&r))                                 // per machine: Queued | Stored | Held(reason)
}
// Inside the engine: pick the network both machines share in which the caller's token
// allows message.send (or refuse "ambiguous; pass via"), append Act{Send} to this machine's
// feed, publish as kept. On the desktop, net checks envelope and admission, then the engine
// verifies the act and calls authz::decide(MessageDeliver), stores it, and surfaces it with
// Codex's first way (InTurn at the next hook, or Resume if idle and wake allows).
// The receipts Stored and then Surfaced come back over the same feeds.
```

**A machine already in "home" joins "studio".**

```rust
// crates/xenonet-cli/src/commands/net_join.rs
let invite = xenonet_net::Invite::parse(&arg)?;            // not secret: scope + inviter EndpointAddr
let code = prompt::code()?;                                 // read from the terminal, never argv
let admitted = api.net_join(invite, code).await?;           // service: net.join -> verify chain with NetworkPolicy
println!("Joined {} as member. Other networks: {}.", admitted.name, admitted.others.join(", "));
// Nothing about "home" changes: same MachineId, a second chain in the store,
// a second scope in Admission, presence for studio filtered by studio's Detail setting.
```

**Another project uses only `xenonet-log` and `xenonet-authz`.** Here a CI fleet keeps a signed list of runners and gives each one narrow deploy rights.

```rust
use xenonet_log::{Chain, Policy, Entry, State, Refused};
use xenonet_authz::{issue, narrow, decide, Grant, Request};

struct Fleet;                                   // their own roles and ops; no XenoNet words
impl Policy for Fleet { type Role = RunnerRole; type Op = FleetOp;
    fn admit(&self, s: &State<RunnerRole>, e: &Entry<FleetOp>) -> Result<(), Refused> { todo!() }
    fn apply(&self, s: &mut State<RunnerRole>, op: &FleetOp) { todo!() } }

let mut fleet = Chain::genesis(Fleet, &owner_key, FleetOp::Found, now);
let deploy = issue(&owner_key, "fleet", &Grant::new("runner:*", [("deploy", "env:staging")], in_days(7)));
let one_job = narrow(&deploy, &Grant::new("runner:r12", [("deploy", "env:staging/app-a")], in_hours(1)));
```

## 8. Migration order

Every phase ships, keeps the service working, and deletes before it adds.

**Phase 1: the JS world goes** (no protocol change)
- *First* record the HTTP API, MCP tools and network frames as golden files from the Rust service; they replace the twins as the contract.
- *Delete* `core/`, `server/`, JS `cli/` and `mcp/`, `xenonet-node`, `parity.json`, the twin tests and `test:rust`, Electron's JS installer, `launcher.cjs`, and the SSH hosts prototype (leaving one host-id namespace).
- *Move* the dozen modules the page imports from `core/` into `src/shared/`, or generate them as JSON from Rust.

**Phase 2: a typed core, no more `Calls`** (wire unchanged, networks keep working)
- *Add* `xenonet-model` (serde, `schemars` writes `schemas/`, zod gone) and the `xenonet-adapter` trait.
- *Delete* `Calls`/`Arg`/`peer.rs`/`world.rs` (direct calls instead), the JS emulation, the runtime schema checker.
- *Dissolve* the `wire-*` crates and the `base`/`app` grab-bags; move vendor code to `adapters/*` and doctor rows with it. The session index replaces `Engine::agents()`'s O(n²) pass.

**Phase 3: one authorization**
- *Add* `xenonet-log::chain` (the roster generalized; today's network roster becomes one `NetworkPolicy`), person chains with device keys, and `xenonet-authz`.
- *Re-express, then delete:* actor tokens and capability lists, delivery grants and selectors, `reach`, `operator`, assistant grants, gateway and A2A tokens.
- *Check at the receiver;* local reads need a credential. Message bodies gain `authority` (body schema v2; frame head unchanged).

**Phase 4: many-to-many ids, one store**
- *Add* feeds, and stored ids: `ProjectId` with checkouts as rows, `AgentId`↔`VendorRef`, `OrgId`, `SpaceId`. Claims become (ProjectId, RelPath), with a one-time mapping from path ids.
- *Several networks:* `networks/<id>/`, one gateway link each, still on the old transport.
- *Consolidate* every state file into `xenonet.sqlite`. Adapters push events from watchers; the rescan becomes reconciliation.

**Phase 5: the transport**
- *Add* `xenonet-net` on iroh beside the gateway link for one release (iroh first, gateway fallback), with mailboxes (the gateway queue code moves there), relay lists, presence deltas, the HLC and envelope head `xenonet-net/2`.
- *Then delete* the HTTP link, signed requests, the clock offset, `gateway.sqlite`, `relay` frames and `svc` frames (now `xenonet/svc/2` streams). The three-service tests move under turmoil.

**Phase 6: Xenolith and the edges**
- Xenolith adopts the four libraries and deletes its own pairing, tokens and crypto copies; spaces bind to model ids.
- The generic ACP adapter; A2A outbound.

## 9. Verification plan

| # | Property | Tool |
|---|---|---|
| 1 | **Chain safety.** No two honest machines accept different entries at one height of one chain. Every accepted entry meets its policy's quorum at its parent. The founder can't be removed. Tickets are single-use and expire | Quint `chain.qnt`, with the mailbox and relay as an adversary that drops, delays, duplicates and invents keyless frames; `quint-connect` drivers against `Chain::offer`; proptest-state-machine on `verify_all` |
| 2 | **Delivery.** A kept act from a reachable author reaches every intended member at least once, enters the store at most once, and in per-author order; a gap is always detected | Quint `delivery.qnt` plus `quint-connect`; turmoil seeds (a mailbox dies mid-transfer, partitions, reconnect storms, skew) |
| 3 | **Attenuation and revocation.** A narrowed token never allows what its parent refuses. A revoked id is refused once its `Revoke` is known. No message content changes a decision | proptest against a plain-Rust reference model; an exhaustive unit test over the action × principal × role matrix (per grounding 05, not model-checked) |
| 4 | **Join.** The admitted machine gets exactly the role the ticket named and no wider. A code is useless without its proof. Expiry arithmetic can't overflow | Quint (in `chain.qnt`); Kani harness for ticket and HLC arithmetic; a one-off Verifpal model of `xenonet/join/2` |
| 5 | **Claims converge.** Overlapping claims are visible to both holders once in contact, and every machine computes the same winner | Quint `coord.qnt`; proptest that the resolver is commutative and idempotent |
| 6 | **One answer.** A permission task takes at most one effective answer, only from a person with authority over that machine; a timeout means deny | Quint `coord.qnt` plus `quint-connect` |
| 7 | **Codecs.** Envelope, feed and chain decoding never panics; encoding round-trips; a sealed body opens only for its recipient and only under its head | cargo-fuzz (the existing 23 targets, retargeted); Kani on envelope decode bounds; proptest round trips |
| 8 | **Presence converges.** After quiet, every connected member's view of a machine equals that machine's snapshot | turmoil plus proptest |

The specs live in `crates/*/formal/`. One `formal` CI job stays under 5 minutes, with larger bounds nightly. A protocol change edits the spec first.

## 10. Rationale

### 10.1 Tradeoffs accepted
- **JSON stays on the wire**, inside today's fuzzed head format. CBOR plus COSE would be churn without a property gained (coset's API "may change"). Revisit it only if outside interop needs it.
- **Claims are advisory.** Exclusive locks would need a majority online, and each machine has to work alone.
- **Agents are vouched for by their machine** rather than holding keys, because hook-based harnesses can't sign.
- **There's a revocation window on offline machines,** bounded by short token lifetimes.
- **The remote map shows session cards,** not today's relayed full graph.
- **One release runs two transports.**

### 10.2 Alternatives rejected
- A big-bang rewrite: months without a working service, and the built crypto, chain and tests are good.
- libp2p: heavier and still 0.x. iroh dials by the key we already have.
- Automerge for everything: single-writer feeds suffice, and authz over shared CRDTs is unsolved (Keyhive is pre-alpha).
- UCAN: its crates aren't 1.0. Biscuit gives the same delegation, plus Datalog.
- OpenFGA or SpiceDB servers: they need an always-on authority. We borrow only their tuple shape.
- Raft for claims or chains: it needs a quorum online.
- Keeping the HTTP gateway: it's poll-shaped, a single point, and it sets every host's clock.

### 10.3 Open questions for the owner
1. Retire the SSH hosts prototype in phase 1? (Recommended: yes.)
2. Re-express cloud-assistant read grants as Biscuit read tokens, or drop them?
3. Person root key: print a recovery phrase at first setup and keep daily use on device keys? (This answers network.md question 6.)
4. Is it acceptable that the remote map becomes session cards, losing the full remote graph?
5. Agents' default reach: keep today's open local default as an explicit, revocable standing grant?
6. The default mailbox count (2), and whether any member machine may volunteer as a mailbox, or only ones the person names.
7. Should a network start titles-only (network.md question 1)?

### 10.4 Biggest risks
1. **Phase 2 is wide** (typing touches every crate). Without the phase-1 golden files it would stall half-typed.
2. **iroh churn:** the gossip wire is pre-1.0 and public relays are reaching end of life, so we ship our own relay and fan out directly until gossip 1.0.
3. **User-presence proofs differ by OS.** On Linux they may fall back to a passphrase, which is weaker against a local computer-use agent.
4. **Biscuit on hot paths** needs caching. Presence must not be checked per delta.
5. **Mixing old and new ids** during phase 4. The rule is one id system at a time, cut over once.

### 10.5 Where subtraction isn't enough
These are the deeper changes I'd still make, and when.
- **One-gateway routing (phase 5).** This can't evolve inside HTTP long-poll. It's a replacement with a one-release overlap, and only the queue logic survives (as the mailbox).
- **Path-based ids (phase 4).** No additive path keeps two id systems without breaking the "two ways to do one task" rule. It's a single breaking cut with a one-time mapping.
- **Derived-not-stored model.** I would *not* store vendor sessions canonically, since the transcripts are the vendor's. Instead I store every XenoNet-authored fact and the `AgentId` mapping, and index the rest incrementally. This meets goal 4 (cheap reads every turn). Sessions on an offline machine are only as fresh as its last presence.
- **Agent keys (phase 6+, optional).** Real per-session keys are possible only where the adapter runs the agent loop (ACP, Agent SDK). Hook-based sessions stay machine-vouched. That is goal 3 met as "a credential of its own", not a key.
- **Strict claims.** If Will wants exclusivity, a project names one arbiter machine that grants leases (as Xenolith's curator does). That trades availability for that project.
- **Several people on one OS account** can't be separated. XenoNet is per OS user, so a shared host is several `MachineId`s.

### 10.6 Screening my own design
- **Shallow modules:** `xenonet-model` risks being data-only; its constructors (`NetworkMember::new`, `Text<MAX>`, `RelPath`) carry the invariants. `xenonet-api` must carry resolution and events, not mirror the engine.
- **Information leakage:** `Envelope` and `Frontier` live only in `-net` and `-log`; the engine sees `Act`. The JSON body schema is the one remaining leak, versioned by `schemars`.
- **Temporal decomposition:** crates are named by job; join, presence and mailbox are handlers inside `xenonet-net`.
- **Pass-through layers:** the `wire-*` crates and `Calls` go; the service is a composition root with no logic.
- **Split ownership:** chains own membership, authz owns decisions, the engine owns coordination state, adapters own vendor facts.
- **Two ways to do one task:** one installer, one token type, one transport, one host id, one schema source.
- **Hand-synced lists:** frame kinds collapse into `Op`, vendor lists come from the registry plus adapters, zod↔Rust becomes `schemars`.
