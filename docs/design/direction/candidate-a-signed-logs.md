# Candidate A: XenoNet on signed per-owner logs

Architect A, 2026-11-08. Starting constraint: the primitive is a signed,
per-owner, append-only log; everything else is a projection over logs.

## 1. Shape in one paragraph

Every durable fact is an entry in a log that exactly one key writes: a
machine's device key, a person's root key, or a self-hosted actor's key. An
entry is a COSE_Sign1-signed CBOR header, hash-chained within its log, plus a
detachable body that is clear, sealed to one device (HPKE), or encrypted
under the writer's key for a space. Logs are grouped by **space** (a network,
a project, a Xenolith context layer, an org): the unit of membership,
replication and encryption. Member devices, and any **keepers** the owner
picks, dial each other by Ed25519 key over iroh, swap log heads and pull what
they lack; iroh relays forward packets they can't read. Each device verifies,
authorizes and folds what it holds into SQLite projections (membership,
grants, inbox, claims and the rest). A **live plane** carries what must not
be logged: presence beacons, wake-ups, map streams, published services. Anything that needs exactly one
outcome (a handoff taken, a prompt answered, an invite used) is decided by
an append to the log of the device that owns the resource. The gateway stops
being an authority. Its jobs become three optional roles any device can
take, any number of times: **relay** (connectivity), **keeper**
(store-and-forward replica) and **edge** (HTTP front door for MCP and A2A
actors).

### Module map

| Crate | One job | Depends on | Alone? |
|---|---|---|---|
| `xenonet-id` | Keys; COSE_Sign1 sign/verify; HPKE seal/open; `did:key` and `xnk1_` rendering | none | yes |
| `xenonet-log` | Single-writer signed hash-chained logs: append, verify, heads, ranges, pruning, equivocation proofs; SQLite store | id | yes |
| `xenonet-net` | Replicate logs and carry live streams over iroh: discovery, relays, reconciliation, push, beacons, PAKE join | id, log | yes |
| `xenonet-authz` | Rights, Biscuit proofs, the space-policy fold, the check, `Information<T>` | id | yes |
| `xenonet-model` | Entities, fact kinds (NSID-versioned), admission, projections as pure folds | id, log (types), authz | yes |
| `xenonet-adapter` | Adapter traits, the app registry (`AppSpec`), drift records | model (types) | for adapter authors |
| `xenonet-api` | Typed commands, queries, events and the `Api` trait; no logic | model (types) | for surface authors |
| `xenonet-engine` | The node: ingest, project, arbitrate, route, wake; implements `Api` | all above | no |
| `adapters/*` | One per platform: claude, codex, grok, acp, opencode, hermes, numbat | adapter | |
| `surfaces/*` | `-service` (local HTTP/socket), `-mcp`, `-cli`, `-edge` (MCP over HTTP, A2A), `-map` (console projections) | api | |
| `xenonet` (bin) | Composition root | everything | |

A crate depends only on crates above it. No core crate names a vendor,
surface, port or path. `authz` doesn't depend on `log`, and `net` moves logs and bytes,
never facts.

## 2. Domain model

- **A principal is a key** (shown as `did:key`). An org is a governance
  space; its id is that space's id.
- **A log has one writer, which holds its key.** Sessions, and remote
  actors reaching an edge with a bearer token, get **custodial** keys held
  by their device; their facts are entries in that device's log with
  `acting` naming them.
- **A space has exactly one owner**, a person or an org (joint ownership is
  an org). Its id is the hash of its signed `Genesis`, which pins owner and
  policy.
- **Relations are facts, so all are many-to-many:** `Admit`,
  `Controls(person, device)`, `MemberOf`, `RepoLink`, `AccountLinked`. One
  device key serves any number of spaces.
- **Apps are an open set** (`AppId` from the registry). **Paths never
  travel:** `RepoKey` plus a relative `RepoPath`.

```rust
// xenonet-model::ids
pub struct DeviceId(pub SigningPub);     // also the device's iroh EndpointId
pub struct PersonId(pub SigningPub);     // the person's root key
pub struct AgentId(pub SigningPub);      // custodial unless Custody::SelfHeld
pub struct ActorId(pub SigningPub);
pub struct SpaceId(pub EntryHash);       // hash of the space's Genesis entry
pub struct OrgId(pub SpaceId);           // an org is a governance space

pub enum Principal { Person(PersonId), Org(OrgId), Device(DeviceId), Agent(AgentId), Actor(ActorId) }
pub enum Owner { Person(PersonId), Org(OrgId) }          // never a device, never "nobody"
pub enum SpaceKind { Network, Project, Context, Org }
pub enum Role { Owner, Operator, Member, Keeper, Edge, Guest }
pub enum Custody { SelfHeld, HeldBy(DeviceId) }

pub struct AppId(SmolStr);               // "claude", "codex", "grok", … from the registry
pub struct NativeId(SmolStr);            // the vendor's run id
pub struct AgentAddr { pub device: DeviceId, pub app: AppId, pub native: NativeId } // renders @laptop:claude:6f1…
pub struct RepoKey(SmolStr);             // normalized git remote
pub struct RepoPath(SmolStr);            // relative, normalized, no `..`
pub struct Account { pub provider: SmolStr, pub subject: SmolStr } // github/login, claude/acct:org

// xenonet-model::facts: what an entry body holds once opened.
pub struct Body { pub authority: Authority, pub fact: Fact }
pub struct Authority {
    pub acting: Principal,               // who did it
    pub proof: ProofRef,                 // under what authority (a Grant entry or inline Biscuit)
    pub method: AuthMethod,              // how the writing device checked `acting`
    pub strength: Strength,              // recorded for display; rights come from `proof` only
}
pub enum Fact {
    Gov(Gov),             // space streams, clear bodies
    Identity(IdentityFact), // the person root's own stream, clear
    Coord(Coord),         // group-encrypted or sealed bodies
    Context(ContextFact), // Xenolith
}
pub enum Gov {
    Genesis { kind: SpaceKind, owner: Owner, policy: PolicySpec, name: SmolStr },
    Invite { invite_key: SigningPub, role: Role, expires: Millis },
    Admit { who: Principal, bundle: KeyBundle, role: Role, invite: Option<EntryRef> },
    Remove { who: Principal, reason: SmolStr },
    SetRole { who: Principal, role: Role },
    Grant { to: SigningPub, proof: Proof },
    Revoke { ids: Vec<RevocationId> },
    Cosign { entry: EntryRef },
    KeyShare { epoch: EpochId, sealed: Vec<(DeviceId, Sealed)> },
    Keeper { device: DeviceId, streams: KeeperScope },
}
pub enum IdentityFact {
    Controls { device: DeviceId, operator: Option<SigningPub> }, // root certifies a device and its operator key
    Uncontrols { device: DeviceId },
    MemberOf { org: OrgId },
    AccountLinked { account: Account },
    Succeeds { old: SigningPub },                     // re-keyed device, signed by its person
}
pub enum Coord {
    AgentBorn { agent: AgentId, addr: AgentAddr, custody: Custody },
    AgentEnded { agent: AgentId },
    Message(Message),
    Seen { upto: EntryRef },                          // receipt and outbox cursor in one
    Claim { repo: RepoKey, path: RepoPath, branch: Option<SmolStr>, until: Millis },
    Release { claim: EntryRef },
    RepoLink { repo: RepoKey },
    Handoff(Handoff),
    Accept { handoff: EntryRef },
    Taken { handoff: EntryRef, by: Principal },       // only the maker's log decides
    StartRequest(StartRequest),
    Started { request: EntryRef, agent: AgentId }, Ended { run: EntryRef, exit: i32 },
    Refused { request: EntryRef, reason: Refusal },
    Ask { run: EntryRef, question: Information<Text> },
    Answer { ask: EntryRef, decision: Decision },     // needs right(run, answer)
}
pub struct Message {
    pub to: Target,
    pub body: Information<Text>,          // no proof field: a message can't carry authority
    pub reply_to: Option<EntryRef>,       // also a causal dep
}
pub enum Target { Agent(AgentAddr), Actor(ActorId), Person(PersonId), Repo(RepoKey), Selector(Selector) }
pub struct Handoff { pub to: Target, pub goal: Text, pub done: Text, pub next: Text,
                     pub files: Vec<RepoPath>, pub branch: Option<SmolStr>, pub commit: Option<SmolStr>, pub lapses: Millis }
pub struct StartRequest { pub app: AppId, pub folder: FolderRef, pub prompt: Information<Text>, pub expires: Millis }
pub enum ContextFact {
    Note { pages: Vec<SmolStr>, body: Text, source: NoteSource },
    PageVersion { slug: SmolStr, body: Text, base: Option<EntryRef> },
    PageRemoved { slug: SmolStr },
    CuratorSet { device: DeviceId },      // a Gov-like fact, owner-signed
}

// Live plane, never logged: a signed, group-encrypted beacon per (device, space).
pub struct Beacon { pub device: DeviceId, pub counter: u64, pub ttl_ms: u32, pub heads: HeadsDigest,
                    pub agents: Vec<AgentPresence>, pub person_here: bool, pub can: Vec<Kind> }
pub struct AgentPresence { pub addr: AgentAddr, pub state: AgentState, pub title: Option<String>,
                           pub focus: Option<String>, pub repo: Option<(RepoKey, SmolStr)>,
                           pub waits_on_person: bool, pub last_active: Millis }

// Projections only ever see facts that passed admission.
pub struct Effective<'a> { meta: EntryMeta<'a>, fact: &'a Fact, by: &'a Authorized } // private fields
pub fn admit<'a>(e: &'a Entry<Verified>, body: &'a Body, view: &PolicyView, past: &dyn CausalPast)
    -> Result<Effective<'a>, Ineffective> { todo!() }
pub trait Projection: Send {
    type View;
    fn kinds(&self) -> &'static [Kind];
    fn apply(&mut self, e: Effective<'_>);
    fn view(&self) -> &Self::View;
}
```

**Stored or derived:**

| What | Where |
|---|---|
| Every `Fact` | Signed logs: `entries` (headers) and `bodies` tables in each device's SQLite. The source of truth |
| Membership, roles, grants, revoked set, epochs; inbox, claims, handoffs, runs, identities, projects | SQLite projections, rebuildable from logs |
| Agent directory and live state | Memory: beacons merged with `AgentBorn`; rebuilt within seconds |
| Xenolith folder (`FORMAT.md`) | Markdown on disk, a projection |
| Sessions, transcripts, usage | Derived per device by adapters, as today; never logged |
| Secret keys | Owner-only file or OS keystore; never logged |

An agent gets an `AgentId` and `AgentBorn` lazily: the first time it sends,
claims, is granted something or is addressed from another device. Most of
1,000 live sessions never touch a log.

## 3. Protocols

**The envelope is the log entry.** COSE_Sign1 (`coset`) with `kid` = the
writer; the payload is the CBOR `Header`: log (writer, stream), `seq`,
`prev`, `at` (advisory), `kind` (an ATProto-style NSID with a version,
`net.xenonet.coord.message#1`), `deps` (causal references into other logs)
and the body's hash, length and encoding. The signature covers the body's
hash, so bodies can be pruned without breaking the chain. Replicas store and
forward kinds they don't know and just don't project them, so mixed versions
stop being refusals.

Bodies are `Clear` for governance (keepers see membership, as today's
gateway sees the roster), `Group { epoch }` for space content, under the
writer's sender key shared to members by `KeyShare` (Signal's Sender Keys,
Matrix's Megolm), or `Sealed { to }` for mail, asks and starts: HPKE base
mode (RFC 9180), which `frame.rs` already is in all but its labels. Secrets
(token answers, service credentials) stay on the live plane.

**Transport and reach.** iroh 1.x QUIC. A device key is its `EndpointId`, so
the handshake proves the peer, and signed-HTTP headers, nonces and the
gateway's clock offset go. ALPNs: `xenonet/{sync,live,join,svc,map}/1`.
Discovery and relays are the person's choice per space: mDNS, DNS or pkarr
(n0's or their own), static addresses, addresses in beacons and `Admit`;
relays off, iroh's public ones, or `iroh-relay` on any member. A Tailscale
address is just another direct address.

**Sync**, borrowing SSB's epidemic broadcast trees and iroh-docs' key model:

1. Peers swap `Heads` for the logs `Interest` lets each serve the other:
   members get their space streams, a device the direct streams addressed
   to it, a keeper what the owner's `Keeper` fact names.
2. Each asks for `(log, from_seq)`. The stream stays open.
3. Every local append pushes a `Have`, with the entry inline under 64 KiB.
4. A fork travels as a self-proving `Equivocation`.

For prefix-closed logs a heads vector is exact set reconciliation, a few KB
for 60 writers. A keeper holding pruned ranges switches to range-based
reconciliation (RBSR, as in iroh-docs and negentropy).

**Delivery and ordering:**

| What | Carried as | Order | Guarantee |
|---|---|---|---|
| Admit, remove, grant, revoke, cosign | Space stream, clear | Per writer; causal by `deps` | All members converge on one `PolicyView` |
| Message to an agent, actor or person | Direct stream to its device, sealed | FIFO per device pair; replies after their message | At-least-once transfer, exactly-once ingest by `(log, seq)`, at-most-once into context (`Seen`) |
| Repo or `all` broadcast | Space stream, group-encrypted | Per writer | The same, without per-recipient receipts |
| Claim, release, repo link | Space stream | Per writer | Advisory; never blocks |
| Handoff | Offer and `Taken` in the maker's log; accepts in takers' logs | The maker decides | Exactly one taker |
| Start, ask, answer | Direct stream plus a live nudge | The target device decides | At most once; start expires in 2 min, ask denies at 10 |
| Presence, focus, person's whereabouts | Beacon per (device, space), group-encrypted | Counter | Best effort, TTL |
| Map, published services, wake | Live QUIC streams | Stream | QUIC's |

There is no outbox table. The outbox is the suffix of my direct stream to
you that your `Seen` hasn't covered; it waits there and in any keeper we
share. Claims go to the repo's project space, else to the device's network
spaces.

**Joining.** On an operator device, with the person present, `invite`
writes an `Invite` (one use, 10 minutes, at most 5 open) and shows either a
pasteable `xn1…` ticket (inviter key, relay hint, 128-bit secret; no server)
or a short code `KX7Q-49MZ-T2` with a rendezvous (a keeper, an edge or the
LAN). Both sides run SPAKE2 over `xenonet/join/1`, as magic-wormhole does:
mutual authentication, one guess per attempt, nothing to brute-force
offline. (Today's gateway stores enough to check tickets, so a broken-into
one takes part in every join.) The inviter, which must be online, appends
`Admit` and `KeyShare`; the joiner syncs governance and checks the genesis
hash against the space id PAKE confirmed. No trust on first use, and refused under an agent. **A second
network** is another invite for the same device key; `Interest` keeps the
spaces' logs apart.

**Starts and prompts.** A start is a sealed `StartRequest` plus a live
nudge; the target's adapter (ACP where the app speaks it) runs it and
appends `Started`, `Refused` or `Ended`. A run's prompt is an `Ask` sealed
to each of the person's operator devices, pushed live to those online (and
to an edge webhook if set). The run's device takes the first valid `Answer`;
after ten minutes it appends a deny.

**MCP, ACP and A2A sit at the edges, never as the bus.** MCP 2026-07-28
(`rmcp` 3.x) is how sessions reach their device and remote actors an edge;
XenoNet's tools form an extension namespace. ACP
(`agent-client-protocol` 5.x) lives inside one adapter that starts and
drives any ACP agent and relays its prompts. A2A 1.0 (`a2a-rs`) sits at the
edge; each client is an actor with a custodial key and a default-deny grant.

## 4. Authorization

| Principal | Holds | Authenticated by |
|---|---|---|
| Person | Root key, used rarely; an operator key on each device it certifies, in the OS keystore | Root signature, or the operator key behind a person-presence check (window, tray, OS prompt, strong process check) |
| Org | A governance space | Its policy's quorum of persons |
| Device | Its own key | The QUIC handshake; entry signatures |
| Agent | Custodial key on its device | The adapter's caller check (signed hooks, process chain, MCP session), kept as method and strength |
| Actor | Own key, or custodial at an edge | Its signatures, or a bearer token at the edge |

**Authority is a Biscuit v6 token** with Zanzibar-shaped facts:
`right("space:ab12", "message")`, `right("repo:github.com/x/y", "claim")`,
`right("device:laptop", "start")`, `right("run:…", "answer")`. Its root is
the space owner for space resources, the device for what runs on it, and the
person's root for acting as the person. Delegating adds a block bound to the
next holder (`check if holder($k), …`), and Biscuit blocks only narrow, so
"no wider than given" holds by construction. A grant is logged once as a
`Grant` fact and cited. By default a member device gets `message`, `claim`
and `handoff`; its agents get an attenuated copy at `AgentBorn`; an actor
gets nothing until the person allows a scope.

**The check runs at ingest on every device, and before every append.**
Verify the signature and chain; wait for `deps` (a new member's message
waits for its `Admit`); map the fact to its one required right; run the
authorizer with `holder`, `acting`, the time and the `PolicyView`. The entry
is `Effective` and projected, or `Ineffective(reason)`: kept for audit,
never projected.

**Revocation** (`Revoke`, by the issuer or owner) is strict and causal: a
revoked proof is ineffective for every entry outside the revocation's causal
past, so nothing can be backdated. On `Remove`, writers rotate sender keys.
Honest limits: a partitioned device honors a revoked proof until it hears,
and old content stays readable. So grants expire, and for `start`, `answer`,
`admit`, `widen` and `publish` the verifier must have synced governance with
a member or keeper in the last ten minutes, or it refuses.

**"Information, never authorization" is in the types.** Message, handoff,
ask and prompt text is `Information<T>`, which converts to nothing. The
authorizer's inputs are a closed struct built from header, proof and policy
view, never from a body. A message requires only `message`. Answering and
widening are fact kinds of their own, and the operator certification carries
`check if holder_kind("operator")`, which every attenuated token keeps, so
no agent can hold `answer`. `Deliverer` takes only `Framed`, whose preface
can't be dropped.

**The owner alone**, by root signature or the policy's quorum of operator
devices (default two), each with the person present, can: create a space;
widen past role defaults, including letting the network start or wake
sessions on a device; admit operators, keepers and edges; publish (edge,
A2A, services for non-members); change policy or owner; recover. Today's
founder becomes the person root, window hosts become operator devices, and
"two window hosts" becomes a `Cosign` quorum. A machine's own setup stays
with the person at it.

## 5. Vendor and surface adapters

A platform implements one trait of optional facets, so an app seen only in
transcripts implements two methods:

```rust
pub trait Adapter: Send + Sync {
    fn spec(&self) -> &AppSpec;                                  // from the registry data, the one list
    fn sessions(&self) -> Option<&dyn SessionSource> { None }    // transcripts → SessionEvent, with drift
    fn connector(&self) -> Option<&dyn Connector> { None }       // install hooks and MCP config
    fn deliverer(&self) -> Option<&dyn Deliverer> { None }       // put a Framed message before a session
    fn starter(&self) -> Option<&dyn Starter> { None }           // start and drive runs; relay prompts
    fn caller_check(&self) -> Option<&dyn CallerCheck> { None }  // which session is calling, how sure
    fn checks(&self) -> Vec<DoctorCheck> { Vec::new() }          // the doctor's rows for this app
}
```

The readers and drift system become `SessionSource`; `apps.json` generates
`AppSpec`; channel, resume, tmux and OpenCode delivery become `Deliverer`s.
One generic ACP adapter gives `Starter` and `Deliverer` to any ACP agent;
vendor code stays only for what ACP lacks (Claude Code's signed hooks, edit
events, transcripts). `SessionSource::watch` is a push stream; any polling
lives in the adapter. Surfaces call `xenonet-api` only; the map's layout,
zones and themes are a projection in `xenonet-map`, so the console never
shapes the model.

## 6. Xenolith

A context layer is a space of kind `Context`, with the same owners, grants
and sync. Notes are `Note` facts in each writer's log; any device with
`append` leaves them, offline included. Pages are `PageVersion` facts,
effective only from the device the owner's `CuratorSet` names, so "a second
curator on a space is a bug" becomes a policy rule. Pages stay single-writer
and need no CRDT; if two people ever edit one page at once, `PageVersion`
can carry Automerge changes instead. The folder, in `FORMAT.md`'s layout, is
the projection, so existing readers keep working. Pairing becomes `invite`,
`access.json` and the `ctx:*` tokens become grants (`read`, `append`,
`curate`, `ask`), and `/sync/v1` becomes `xenonet-net`. Ask from a satellite
is a sealed request to the curator's device, needing `ask`. Xenolith stays
standalone: it takes `xenonet-id`, `-log`, `-net` and `-authz`, never the
engine, and runs its own replicator under its own actor key.

## 7. Usage sketch

**An agent on studio messages a session on laptop**, from the MCP tool
handler:

```rust
let caller = api.identify(LocalCall::Mcp { app: AppId::new("claude"), session: native, meta })?; // adapter's CallerCheck
let sent = api.send(&caller, SendReq {
    to: Target::Agent("@laptop:claude:6f1…".parse()?),
    body: Text::new("Tests pass on studio. The migration is yours.")?,
    reply_to: None,
    space: None,            // the engine picks a shared space where the caller holds `message`
}).await?;                  // Sent { id: EntryRef, path: Delivery::Pushed | Delivery::Waiting }
let mut receipts = api.events(&caller, Topic::Receipts(sent.id)); // Event::Seen when laptop appends Seen
```

The engine checks the proof and appends a sealed `Message` to studio's
direct stream to laptop; the net pushes it; laptop verifies, authorizes and
projects it, and its Claude adapter's `Deliverer` wakes the session with the
framing.

**A machine already in `home` joins a second network:**

```rust
let code: JoinCode = "xn1qq…".parse()?;                  // or JoinCode::short("keeper.example.org", "KX7Q-49MZ-T2")
let me = Caller::person(PersonPresent::from_terminal(&local)?); // refused under an agent
let joined = api.join(&me, code).await?;
assert_eq!(api.spaces(&me).await?.len(), 2);             // same DeviceId, two spaces
```

**Another project embeds the log and the network**, say a CI fleet keeping
a signed audit trail, or takes authorization alone:

```rust
let key = xenonet_id::SecretKeys::load_or_create(&path)?;
let store = Arc::new(xenonet_log::SqliteStore::open("audit.db")?);
let mut w = xenonet_log::Writer::open(store.clone(), &key, StreamId::named(b"org.example.audit"))?;
w.append(Kind::new("org.example.audit.deploy", 1), BodyIn::Clear(&cbor), &[])?;
let net = xenonet_net::Net::bind(&key, NetConfig::lan_only(), store, Arc::new(AllPeers(fleet))).await?;
let ok = xenonet_authz::check(&view, &proof, &Request::new(&holder, &right, now))?;
```

## 8. Migration order

Each phase ships, and subtracts before it adds.

1. **Cut the JS contract.** Delete the JS core as contract (`core/engine`,
   `core/delivery`, their twins), the napi twin crate, `parity.json` and
   Electron's second installer. Split `xenonet-base` into `xenonet-id` (with
   `xenonet-net`'s keys) and the registry data. Nothing new; the engine is
   free to change shape.
2. **The log on one machine.** Add `xenonet-log`, model facts for
   messages, claims, handoffs and grants, and the projector. Engine
   tables for messages, deliveries, claims and handoffs become rebuildable
   projections. Delete their direct writes, `delivery*.json`, the
   string-dispatched `Calls`/`Arg` on these paths and the O(n²)
   `Engine::agents()`.
3. **Sync over iroh; dissolve the gateway.** Rebuild `xenonet-net` on
   iroh: sync, live plane, PAKE join, keepers. The roster's rules become
   the network space's policy, the first piece of `xenonet-authz`. Delete `gateway.rs`, `gwstore.rs`, the HTTP
   link, tickets, epochs, `seq`/`gap`, the clock offset, signed HTTP
   requests, relay-through-gateway and one-network-per-host files. Keep
   sign and seal (now in `xenonet-id`), the svc allowlist (now
   `xenonet/svc/1`), and the three-service tests as three nodes on an
   in-memory transport.
4. **Authority.** Add Biscuit proofs, person roots, operator keys,
   custodial keys and the ingest check. Delete actor capability lists and
   `actor_revoked`, selector grants, `reach.rs` enums, `operator.rs`
   frames, trust in a sender's self-reported strength, and the window key
   as the root of "you" (the window becomes a presence check).
5. **Adapters and edges.** Add `xenonet-adapter`, adapter crates taking
   `connect.rs`, `delivery/*`, `procs.rs`, `verify.rs` and `sources/*`, the
   ACP adapter, MCP 2026-07-28, `xenonet-edge` and `xenonet-map`. Delete
   vendor strings in core crates, the closed `App` enum, the doctor's
   hard-coded vendor rows and the actor gateway's port logic.
6. **Xenolith on the model.** Notes and pages as facts, the folder as a
   projection, pairing as an invite. Delete `/sync/v1`, `peers/*.token`
   and Xenolith's duplicated x25519, private-file and token code.

Kept throughout: the vendor readers and drift system, the registry data,
the console and its driver, the doctor's framework and the fuzz targets.

## 9. Verification plan

| # | Property | Tool |
|---|---|---|
| 1 | **Chain integrity.** Never two projected entries at one `(log, seq)`; any fork yields an `Equivocation` and freezes its writer | proptest-state-machine on `LogStore`; Kani on `Entry::verify`; cargo-fuzz on decode |
| 2 | **Convergence.** Replicas with the same entries compute identical `PolicyView`, inbox, claims and handoffs for every delivery order that respects `deps` | proptest with causal shuffles; Quint `sync.qnt` + quint-connect into `model::admit` and the folds |
| 3 | **Delivery.** A message to a member who is eventually connected to its sender or a keeper holding it is ingested exactly once, in stream order | Quint `delivery.qnt` (Apalache safety, TLC liveness nightly); turmoil seeds: partitions, keeper crash, reconnect storms, skew |
| 4 | **One outcome.** At most one `Taken` per handoff, effective `Answer` per ask, `Admit` per invite, each decided by the owning device | Quint `coord.qnt`; quint-connect against the engine's arbiter |
| 5 | **Governance safety.** Nothing is effective without its right in its causal past; removal and revocation cover all outside theirs; no self-widening; quorum ops need quorum | Quint `governance.qnt` via Apalache (3 devices, 2 operators, concurrent admit, remove, revoke); exhaustive decision-table unit test |
| 6 | **Attenuation.** Derived rights ⊆ parent's; expired or revoked proofs never authorize; no authorizer input from a body | proptest over random Biscuit chains; trybuild compile-fail: `Information<T>` can't reach `Authorized` |
| 7 | **Join.** Mutual authentication, no offline guessing by relays or keepers, one device per code, matching genesis | Verifpal one-off, rerun on handshake change; turmoil: inviter dies mid-join, two joiners race one code |
| 8 | **Total decoders.** Entries, beacons and sync messages never panic and refuse oversize input | cargo-fuzz; Kani |

Specs live in `crates/*/formal/`; one `formal` CI job under five minutes,
larger bounds and long runs nightly.

## 10. Rationale

**Tradeoffs accepted:**

- **Two planes.** Presence (1,000 agents on 50 devices) changes too often
  to log. Rule: what must survive or be audited is logged; what can be
  recomputed within seconds is a beacon. Beacons go straight to each online
  member; well past 50 devices they need iroh-gossip, whose 1.0 wire isn't
  out.
- **Exactly-once needs its arbiter online.** An accept or answer for an
  offline device waits and may lapse.
- **Custodial keys** don't stop a device acting as its sessions; they let
  authority be granted per session and named on every act.
- **Sender-key rotation on removal** costs writers × members `KeyShare`s
  (about 2,500 small blobs at 50 devices). Past a few hundred members it
  needs MLS's tree.
- **Governance metadata is clear to keepers**, as the roster is to today's
  gateway. **Wall clocks bound only expiries**, never order.

**Rejected:**

- A central gateway as router: single point of failure, metadata hub,
  clock authority.
- Automerge as the primitive: no per-entry authorship or authority;
  Keyhive is pre-alpha.
- A Matrix-style event DAG with state resolution: its complexity is the
  warning.
- MLS for space keys: commits need a total order, so a central delivery
  service.
- iroh-docs as the store: no stability promise; last-writer-wins loses
  append semantics.
- SSB or Hypercore directly: no maintained Rust; we borrow their ideas.
- libp2p (heavier, 0.x); NATS or Kafka (central daemons).
- UCAN tokens: crates not 1.0; Biscuit carries UCAN's holder-bound chain
  shape.

**Open questions for the owner:**

1. Where does a person's root key live: one machine, a passkey or hardware
   key, or a printed recovery kit?
2. Default join code: a pasted ticket (no server) or a short code (needs a
   rendezvous)?
3. Is the founding device every new space's default keeper, with "add
   another" offered?
4. How long do bodies of seen mail and released claims stay?
5. Are iroh's public relays ever on by default? They see who talks to whom.
6. Does a new member read a space's history, or only from its admission on
   (the default here)?
7. Are orgs needed in the first cut?

**Biggest risks:**

- **Event sourcing is real work:** fact evolution, projection versions,
  rebuild time. NSID versions and rebuildable projections are the plan.
- **The governance fold under concurrency is subtle.** Keep it tiny and
  model it before coding it.
- **A device restored from backup clones its key with an old head**, and
  an append before it catches up forks the log (SSB's old wound). A device
  opens its writers only after one sync round, or after the person accepts
  working offline; re-keying goes through `Succeeds`.
- **Append-only remembers mistakes.** Bodies prune and redaction runs
  before append, but headers stay.
- **iroh churn.** Only core iroh 1.x for now; no gossip or docs.
- **The local person-versus-agent check stays the weakest link:** a
  computer-use agent can click Allow. Quorum and fresh-sync rules bound
  what one device can widen.

**Screened against the smells.**

- *Shallow modules:* `log` and `net` hide chains, storage, discovery and
  reconciliation behind a few calls each; `api` is types and a trait.
- *Information leakage:* COSE, CBOR and NSIDs stay in `log` and `model`'s
  codec; the engine sees `Fact`.
- *Temporal decomposition:* crates split by knowledge, not by receive,
  verify, route.
- *Pass-throughs:* the engine owns arbitration, routing and waking.
- *Split ownership:* each fact kind has one module and one required right;
  policy lives only in `authz`.
- *Two ways to do one task:* messages, receipts, operator frames, start
  targets, roster entries and outbox rows become entries; the one
  deliberate pair is log versus beacon.
- *Hand-synced lists:* `FRAME_KINDS`, `HOST_KINDS`, `QUEUED_KINDS` and
  `CAN` go; kinds come from the `Fact` enum through one table, `can` from
  what a build projects, apps from the registry.

## Appendix: type sketch of the other core crates

```rust
// ───────────── xenonet-id ─────────────
pub struct SigningPub(pub [u8; 32]);          // Ed25519; a device's is also its iroh EndpointId
pub struct SealPub(pub [u8; 32]);             // X25519
pub struct Signature(pub [u8; 64]);
pub struct KeyBundle { pub sign: SigningPub, pub seal: SealPub, pub since: Millis }
pub struct SecretKeys { /* zeroized on drop; no Serialize, no Debug */ }
impl SecretKeys {
    pub fn generate() -> Self { todo!() }
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> { todo!() } // owner-only file or OS keystore
    pub fn bundle(&self) -> KeyBundle { todo!() }
    pub fn sign(&self, msg: &[u8]) -> Signature { todo!() }
}
pub fn verify(by: &SigningPub, msg: &[u8], sig: &Signature) -> Result<(), BadSig> { todo!() }
pub fn cose_sign1(k: &SecretKeys, content_type: &str, payload: &[u8]) -> Vec<u8> { todo!() }
pub fn cose_open1(bytes: &[u8]) -> Result<(SigningPub, Vec<u8>, Signature), BadSig> { todo!() }
pub struct Sealed { pub enc: [u8; 32], pub ct: Vec<u8> }   // HPKE base: X25519, HKDF-SHA256, ChaCha20-Poly1305
pub fn seal(to: &SealPub, info: &[u8], aad: &[u8], pt: &[u8]) -> Sealed { todo!() }
pub fn open(k: &SecretKeys, info: &[u8], aad: &[u8], s: &Sealed) -> Result<Vec<u8>, OpenError> { todo!() }
pub fn did_key(k: &SigningPub) -> String { todo!() }
pub fn short_id(b: &KeyBundle) -> String { todo!() }      // xnk1_…

// ───────────── xenonet-log ─────────────
pub struct StreamId(pub [u8; 32]);            // opaque: the model maps spaces and direct pairs to it
pub struct LogId { pub writer: SigningPub, pub stream: StreamId }
pub struct Seq(pub u64);
pub struct EntryHash(pub [u8; 32]);
pub struct EntryRef { pub log: LogId, pub seq: Seq, pub hash: EntryHash }
pub struct Kind { pub nsid: SmolStr, pub v: u16 }
pub enum BodyEnc { Clear, Group { epoch: EpochId }, Sealed { to: SigningPub } }
pub struct BodyRef { pub hash: [u8; 32], pub len: u32, pub enc: BodyEnc }
pub struct Header { pub log: LogId, pub seq: Seq, pub prev: Option<EntryHash>, pub at: Millis,
                    pub kind: Kind, pub deps: Vec<EntryRef>, pub body: BodyRef }
pub struct SignedHeader { pub header: Header, pub sig: Signature }
pub struct Unverified; pub struct Verified;
pub struct Entry<S> { head: SignedHeader, body: Option<Bytes>, _s: PhantomData<S> }
pub struct Head { pub seq: Seq, pub hash: EntryHash }
pub enum Reject { BadSig, Gap { have: Option<Seq>, got: Seq }, Forked(Equivocation), BodyMismatch, TooLarge, Frozen }
pub struct Equivocation { pub a: SignedHeader, pub b: SignedHeader }   // proves itself
impl Entry<Unverified> {
    pub fn decode(cose: &[u8], body: Option<Bytes>) -> Result<Self, DecodeError> { todo!() }
    pub fn verify(self, head: Option<&Head>) -> Result<Entry<Verified>, Reject> { todo!() }
}
impl Entry<Verified> { pub fn header(&self) -> &Header { todo!() } pub fn body(&self) -> Option<&[u8]> { todo!() } }
pub struct Heads(pub BTreeMap<LogId, Head>);
impl Heads { pub fn wants_from(&self, theirs: &Heads) -> Vec<(LogId, Seq)> { todo!() } }
pub enum BodyIn<'a> { Clear(&'a [u8]), Group { epoch: EpochId, key: &'a GroupKey, pt: &'a [u8] }, Sealed { to: &'a KeyBundle, pt: &'a [u8] } }
pub trait LogStore: Send + Sync {
    fn head(&self, log: &LogId) -> Option<Head>;
    fn heads(&self, streams: &[StreamId]) -> Heads;
    fn put(&self, e: Entry<Verified>) -> Result<Stored, Reject>;
    fn range(&self, log: &LogId, from: Seq, max: usize) -> Result<Vec<Entry<Verified>>, StoreError>;
    fn prune_bodies(&self, log: &LogId, below: Seq) -> Result<u64, StoreError>;
    fn equivocation(&self, writer: &SigningPub) -> Option<Equivocation>;
    fn subscribe(&self) -> broadcast::Receiver<Stored>;
}
pub struct Writer { /* store, key, log, head */ }
impl Writer {
    /// Continues from the latest head of our own log the store holds. The engine
    /// opens writers only after a first sync round, so a restored backup never forks.
    pub fn open(store: Arc<dyn LogStore>, key: &SecretKeys, stream: StreamId) -> Result<Self, OpenError> { todo!() }
    pub fn append(&mut self, kind: Kind, body: BodyIn<'_>, deps: &[EntryRef]) -> Result<EntryRef, AppendError> { todo!() }
}
pub struct SqliteStore { /* entries, bodies, heads, equivocations */ }

// ───────────── xenonet-net ─────────────
pub struct NetConfig { pub relays: Relays, pub discovery: Vec<Discovery>, pub bind: Bind }
pub enum Relays { Off, Urls(Vec<Url>) }
pub enum Discovery { LocalNetwork, Dns { origin: String }, Pkarr { url: Url }, Static(Vec<PeerAddr>) }
/// Who may receive which logs, and which streams to ask a peer for. The engine supplies it from authz.
pub trait Interest: Send + Sync {
    fn may_serve(&self, peer: &SigningPub, log: &LogId) -> bool;
    fn wants(&self, peer: &SigningPub) -> Vec<StreamId>;
}
pub enum SyncMsg { Heads(Heads), Want(Vec<(LogId, Seq)>), Entries(Vec<WireEntry>), Have(LogId, Head, Option<WireEntry>), Forked(Equivocation) }
pub struct Net { /* iroh::Endpoint, replicator tasks, live hub */ }
impl Net {
    pub async fn bind(key: &SecretKeys, cfg: NetConfig, store: Arc<dyn LogStore>, interest: Arc<dyn Interest>) -> Result<Net, NetError> { todo!() }
    pub fn nudge(&self, log: &LogId) { todo!() }                  // after a local append: eager push
    pub fn peers(&self) -> Vec<PeerState> { todo!() }             // connected, direct or relayed, last sync per stream
    pub fn live(&self) -> &Live { todo!() }
    pub async fn offer_join(&self, secret: JoinSecret, admit: Box<dyn FnOnce(KeyBundle) -> BoxFuture<'static, Result<JoinGrant, JoinError>> + Send>) -> Result<(), JoinError> { todo!() }
    pub async fn join(&self, code: JoinCode, mine: KeyBundle) -> Result<JoinGrant, JoinError> { todo!() } // SPAKE2
}
pub struct Live { /* beacon topics, open streams */ }
impl Live {
    pub fn beacon(&self, topic: StreamId, signed: Bytes, ttl: Duration) { todo!() }
    pub fn beacons(&self, topic: StreamId) -> BoxStream<'static, (SigningPub, Bytes)> { todo!() }
    pub async fn open(&self, peer: &SigningPub, alpn: &'static [u8]) -> Result<BiStream, NetError> { todo!() }
    pub fn accept(&self, alpn: &'static [u8]) -> BoxStream<'static, (SigningPub, BiStream)> { todo!() }
}
pub enum JoinCode { Ticket { inviter: SigningPub, relay: Option<Url>, secret: [u8; 16] }, Short { rendezvous: PeerAddr, code: ShortCode } }

// ───────────── xenonet-authz ─────────────
pub struct Resource(SmolStr);   // "space:…", "device:…", "repo:…", "run:…", "svc:<device>/<name>", "context:…"
pub enum Verb { Read, Message, Claim, Handoff, Start, Wake, Answer, Append, Curate, Ask, Call, Admit, Widen, Publish, Revoke }
pub struct Right { pub on: Resource, pub verb: Verb }
pub struct Proof(Vec<u8>);      // Biscuit v6
pub struct RevocationId(pub Vec<u8>);
pub enum HolderKind { Device, Operator, Agent, Actor }
pub struct GrantSpec { pub to: SigningPub, pub holder: HolderKind, pub rights: Vec<Right>, pub expires: Millis, pub delegable: bool }
pub fn issue(root: &SecretKeys, g: &GrantSpec) -> Proof { todo!() }
pub fn attenuate(p: &Proof, narrower: &GrantSpec) -> Result<Proof, Widening> { todo!() } // refuses any widening
pub struct PolicySpec { pub owner: SigningPub, pub quorum: u8, pub defaults: RoleDefaults }
pub struct EventId(pub [u8; 32]);
pub trait CausalPast { fn includes(&self, e: &EventId) -> bool; }
pub enum GovEvent { Admit { who: SigningPub, role: Role }, Remove { who: SigningPub }, SetRole { who: SigningPub, role: Role },
                    Grant { proof: Proof }, Revoke { ids: Vec<RevocationId> }, Cosign { of: EventId }, Policy(PolicySpec) }
pub struct PolicyView { /* members, roles, quorum, grants, revoked, pending cosigns */ }
pub fn fold(spec: &PolicySpec, events: &[(EventId, SigningPub, Vec<EventId>, GovEvent)]) -> PolicyView { todo!() } // deterministic
pub struct Request<'a> { pub holder: &'a SigningPub, pub acting: &'a SigningPub, pub right: &'a Right, pub at: Millis, pub past: &'a dyn CausalPast }
impl<'a> Request<'a> { pub fn new(holder: &'a SigningPub, right: &'a Right, at: Millis) -> Self { todo!() } } // acting = holder, empty past
pub struct Authorized { right: Right, chain: Vec<SigningPub> }   // no public constructor
pub enum Denied { NoRight, Expired, Revoked, NotMember, NeedsQuorum { have: u8, need: u8 }, Stale }
pub fn check(view: &PolicyView, proof: &Proof, req: &Request<'_>) -> Result<Authorized, Denied> { todo!() }
pub struct Information<T>(T);
impl<T> Information<T> { pub fn new(t: T) -> Self { todo!() } pub fn read(&self) -> &T { todo!() } } // and nothing else

// ───────────── xenonet-adapter (facets) ─────────────
pub struct AppSpec { pub id: AppId, pub name: String, pub facets: Facets, pub processes: Vec<ProcessMatch>, pub schema_versions: Vec<VendorVersion> }
pub trait SessionSource: Send + Sync {
    fn scan(&self) -> Result<Vec<SessionObs>, SourceError>;
    fn watch(&self) -> BoxStream<'static, SessionEvent>;     // push; any poll lives here
    fn drift(&self) -> &DriftLog;
}
pub trait Connector: Send + Sync { fn status(&self, env: &ConnectEnv) -> ConnectStatus; fn connect(&self, env: &ConnectEnv) -> Result<ConnectReport, ConnectError>; fn disconnect(&self, env: &ConnectEnv) -> Result<(), ConnectError>; }
pub struct Framed<'a> { preface: &'static str, from: &'a PeerLabel, body: &'a Information<Text> } // preface can't be omitted
pub trait Deliverer: Send + Sync { fn deliver(&self, s: &NativeId, m: Framed<'_>) -> Result<DeliveryPath, DeliverError>; }
pub trait Starter: Send + Sync { fn start(&self, r: &StartSpec) -> Result<RunHandle, StartError>; fn asks(&self, run: &RunHandle) -> BoxStream<'static, AskEvent>; fn answer(&self, run: &RunHandle, ask: AskId, d: Decision) -> Result<(), StartError>; }
pub trait CallerCheck: Send + Sync { fn identify(&self, call: &LocalCall) -> Option<(NativeId, AuthMethod, Strength)>; }

// ───────────── xenonet-api ─────────────
pub struct Caller { pub principal: Principal, pub method: AuthMethod, pub strength: Strength, pub person_present: Option<PersonPresent> }
impl Caller { pub fn person(p: PersonPresent) -> Caller { todo!() } }   // acting as the device's person, with presence
pub trait Api: Send + Sync {
    fn identify(&self, call: LocalCall) -> Result<Caller, ApiError>;
    async fn send(&self, c: &Caller, r: SendReq) -> Result<Sent, ApiError>;
    async fn inbox(&self, c: &Caller, wait: Duration) -> Result<Vec<Delivered>, ApiError>;
    async fn peers(&self, c: &Caller, scope: Scope) -> Result<Vec<Peer>, ApiError>;
    async fn who_has(&self, c: &Caller, repo: RepoKey, path: RepoPath) -> Result<Vec<Holder>, ApiError>;
    async fn claim(&self, c: &Caller, r: ClaimReq) -> Result<ClaimOutcome, ApiError>;
    async fn release(&self, c: &Caller, claims: Vec<EntryRef>) -> Result<(), ApiError>;
    async fn handoff(&self, c: &Caller, r: Handoff) -> Result<EntryRef, ApiError>;
    async fn accept(&self, c: &Caller, h: EntryRef) -> Result<AcceptOutcome, ApiError>;
    async fn start(&self, c: &Caller, r: StartReq) -> Result<StartOutcome, ApiError>;
    async fn answer(&self, c: &Caller, ask: EntryRef, d: Decision) -> Result<(), ApiError>;
    async fn invite(&self, c: &Caller, r: InviteReq) -> Result<JoinCode, ApiError>;
    async fn join(&self, c: &Caller, code: JoinCode) -> Result<Joined, ApiError>;
    async fn grant(&self, c: &Caller, r: GrantReq) -> Result<EntryRef, ApiError>;
    async fn revoke(&self, c: &Caller, r: RevokeReq) -> Result<EntryRef, ApiError>;
    async fn spaces(&self, c: &Caller) -> Result<Vec<SpaceSummary>, ApiError>;
    fn events(&self, c: &Caller, topic: Topic) -> BoxStream<'static, Event>;
}

// ───────────── xenonet-engine ─────────────
pub struct Node { /* store, net, keyring, projections, adapters, arbiter, clock */ }
impl Node { pub async fn open(cfg: NodeConfig, adapters: Vec<Box<dyn Adapter>>) -> Result<Node, OpenError> { todo!() } }
struct Ingest;   // verify → wait deps → open body → admit (authz) → project → wake
struct Arbiter;  // decides for what this device owns: handoffs it made, runs it hosts, invites it issued
struct Router;   // target → device via the directory; picks the space; appends; nudges
```
