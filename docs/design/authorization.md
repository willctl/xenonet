# Authorization: who may do what

Status: built, phase 3 of the rewrite ([direction.md](direction.md),
"Order of work"). This is how XenoNet decides who may do what, as the code
stands. The goals it answers to are in [goals.md](goals.md); the network it
runs over is in [network.md](network.md).

## In short

- **The receiver decides.** Every act that reaches a machine, from its own
  surfaces, another machine, the actor gateway or A2A, is decided there by
  one function, `xenonet_authz::decide`, on facts that machine holds
  itself. What a sender says of its own strength is never an input.
- **Four kinds of principal:** a machine, an agent (a harness session), an
  actor (a bot, a cloud assistant, a program of yours) and the person.
- **The person is a key chain.** Each machine holds the person's chain of
  device keys. An act as the person carries a device's signature over that
  one act, checked against the receiver's own copy of the chain.
- **Presence is the OS's own check.** An act that widens what anyone may
  do needs a presence block: a device signature made after Touch ID on a
  Mac, or Windows Hello, polkit or a passphrase elsewhere, which are marked
  weaker. One prompt per act; the block stands for the signature too.
- **Every actor holds one grant:** a Biscuit token rooted at its machine,
  naming its rights, the machine and network it acts in, and when it ends.
  Scopes the receiver keeps beside it say which sessions and projects it
  reaches and what it may Allow.
- **Delegation is the person's:** an entry in their chain, naming a set of
  their devices, the work and the kinds of prompt it covers, its limits
  and its end.
- **The local API takes a credential,** and every route has one row in one
  rules table that's checked before the handler runs.
- **Message text can't authorize.** No request type holds text, and the
  type that carries text converts to nothing the authorizer takes.

## The crates

| Crate | Holds | Depends on |
|---|---|---|
| `xenonet-id` | Keys and signatures bound to a purpose, the `Signer`, `KeyStore` and `Vault` traits, presence blocks and their check, did:key | nothing of XenoNet's |
| `xenonet-log` | `Chain<P: Policy>`, the network policy (`xenonet-roster/2`) and the person policy (`xenonet-person/1`) | `xenonet-id` |
| `xenonet-authz` | `decide`, `Request` and its vocabulary, `Ground`, tokens and their roots, `Information<T>` | `xenonet-id` |
| `xenonet-keys` | The platform keys and checks: the Secure Enclave and Touch ID, Windows Hello, polkit and a passphrase, and `Fake` | `xenonet-id`, `xenonet-os` |

Only the composition root uses `xenonet-keys`. The installed XenoNet and
XenoNet Dev's `dev` instance get the machine's own keys (`System`); the
demo, the tests and every other instance get `Fake`, which never prompts
or touches the hardware (`crates/xenonet-service/src/compose/keys.rs`).
The network crate (`crates/xenonet-net`) holds the person on the machine
(`identity.rs`) and its facts for a decision (`ground.rs`); the runtime
(`crates/xenonet-runtime/src/ground.rs`) adds what only the engine holds.

## The principals

| Principal | Who | Proven by |
|---|---|---|
| Machine | One XenoNet service: one OS user on one host | At a local door, being this service; from the network, the frame's signature by a member's host key |
| Agent | A harness session (Claude Code, Codex, Grok Build…) on a machine | Its machine vouches for it with an attestation: `Unchecked`, `ProcessChecked` or `HarnessSigned`. From another machine, the attestation is that machine's statement |
| Actor | A registered program, a bot such as Grok Bot or Hermes, a cloud assistant | Its grant, a token minted at its own machine |
| Person | The person, at one of their devices | A device signature over the act, from a device their chain holds unrevoked |

A machine has one person in this first cut. The person chain and the
chain policy hold many devices and are ready for more persons; nothing
else assumes one network.

## The person: their chain and devices

**The chain** (`xenonet-person/1`, `crates/xenonet-log/src/person.rs`) is
the person's root key and the device keys it, or their devices, add:

| op | Signed by |
|---|---|
| `found` | the root key, alone, as entry 0 |
| `add-device`, `remove-device` | the root key with a device's presence block, or two devices each with its own |
| `revoke` a token or a device | one device with its presence block, or the root key with a device's |
| `delegate` that issues or widens a delegation | one device with its presence block, or the root key with a device's |
| `delegate` that narrows one, `undelegate` | one device, or the root key; no presence |

A person's id is `prs_` and the base32 of the first 20 bytes of the
SHA-256 of entry 0. A device's id is its key's, `xns1_…`. Each device
entry records how that device proves presence, `biometric` or `weaker`,
when it's added; a presence block never says so itself. A device that was
removed or revoked can't sign, and doesn't come back: a machine that
returns makes a new key.

**Revoking a device** is for a lost machine: Network settings › Your
devices, then Revoke, which calls `POST /api/net/devices/revoke` from the
window (`xenonet net revoke-device` says so and does nothing). This
machine's device signs the `revoke` entry with its presence; it never
revokes itself, so the person revokes a machine from another one. The
entry is pushed to the network's machines, and each refuses what the
revoked device signs from then on: its act signatures and its presence
blocks alike. The network state lists the person's devices
(`person.devices`), each with how it asks and whether it can sign.

**On each machine** the person lives in `<state>/identity/`:

| File | Holds |
|---|---|
| `person.json` | The person's chain |
| `persons.json` | Other people's chains this machine holds to check a network's chain |
| `root.json` | The root seed, sealed to the device key where the platform can (opening it asks the person), else in an owner-only file marked weaker |
| `device-sign.key`, `machine-sign.key` | The device key and the machine signing key: Secure Enclave blobs on a Mac, key files elsewhere |
| `presence-seen.json` | The presence blocks taken in the last two minutes, so a block counts once across restarts |
| `token-roots.json` | This machine's token root certificates |

The person is founded from XenoNet's window, when the first network is
made or the first act as them needs a device signature. Joining a network
pairs the new machine's device into the inviter's person when the inviter
holds the root key; a machine whose person was never paired with another
device takes the inviter's. Person entries travel as messages to the
reserved target `xenonet:person`, pushed on append. Every act's authority
names the head of the chain its sender held (`personHead`), and a receiver
with an older head asks for the rest.

**The machine signing key** roots the machine's tokens. On a Mac with a
Secure Enclave it lives there, without biometry: it can't leave the Mac,
though any program of the account can ask it to sign. Elsewhere it's a
file. It signs network chain entries, feed entries and address records.
The **host key** (`<state>/network/host.key`) is the transport key: it
authenticates the machine's connections (QUIC, and today's frames) and
holds the X25519 key envelopes are sealed to. iroh needs it raw, so it is
always a file, and it signs no entry. The network chain binds the two, and
a `rekey` entry replaces a stolen transport key.

## Presence

A presence block is a device's signature over what it approves (`answer`,
`admit`, `widen`, `publish`, `revoke`, `pair`, `delegate` or `grant`), the
SHA-256 of the one act, a random nonce and the time
(`crates/xenonet-id/src/presence.rs`). It can't be lifted onto another act,
since the act's hash is in what's signed. A live receiver takes each block
once, within 60 seconds of its time, and remembers it across restarts. A
chain checks the blocks in its entries for what they sign, never their
age, since a machine catching up reads entries that were fresh when made.

| Platform | How the device asks | Strength |
|---|---|---|
| macOS with a Secure Enclave | The device key is a Secure Enclave key made with biometry, so the enclave asks Touch ID for every signature, whoever holds its blob. No passphrase or PIN instead | biometric |
| Windows | Windows Hello before the file key signs. Hello always lets a PIN stand in, which an agent that can type could enter | weaker |
| Linux and the BSDs | polkit (`pkcheck`, XenoNet's own action once a package installs it, else pkexec's), or a passphrase read from the terminal with echo off | weaker |

A weaker device is allowed, and the decision says so in its note. The
doctor shows how this machine's device asks (`network.presence`:
biometric, weaker or none) and where the root key is (`network.root`:
sealed to the device key, or in an owner-only file, which it warns of).

**One prompt per act.** An act that needs the person there carries the
presence block alone: the block is the device's signature over the act's
hash, so it stands for the act signature and the person is asked once.
Any other act as the person carries the device's act signature, which on a
Mac asks Touch ID too, and elsewhere asks nothing.

**What asks.** Only an act the person starts at XenoNet's window: every
method that may ask takes `at_window` and refuses without asking
otherwise. Making a key, loading one and asking what's available never
show anything. What each act asks on a Mac, where the root key is sealed
to the device key:

| Act | Prompts |
|---|---|
| Setting up your identity (the first network, or the first act as you) | 1 |
| Making a network | none beyond setting up your identity |
| Adding a machine: signing it in when it joins | 1 to admit, and 2 more to pair its device (the presence, and opening the root) when this machine holds the root |
| Approving an entry that admits or widens | 1 |
| Widening a machine's start or wake, making a window host, moving the gateway | 1 for each admin that signs |
| Issuing or widening your delegation | 2 (the presence, and the entry's signature) |
| Narrowing or ending your delegation | 1 (the entry's signature) |
| Setting a machine to `delegate` when no delegation names its device | the widening's 1, and 2 for the delegation |
| Starting a session as you, here or on another machine | 1 |
| A message as you to another machine | 1 for each machine it goes to |
| A message as you to this machine's sessions | none |
| Allowing a permission prompt, here or from another machine | 1 |
| Denying one | none |
| Issuing or widening an actor's grant: allowing a registration, adding an actor, a new token, a gateway token, an allow scope, a sessions scope, pairing or widening a remote connection | 1 |
| Publishing a service to the network | 1 |
| Revoking another of your devices (a lost machine) | 1 |
| Narrowing, revoking or removing a grant or an actor; narrowing a machine's start or wake; removing a machine; taking a service off; leaving | none |

On Windows and Linux the root key is in a file, so pairing asks once, and
a device's act signature asks nothing: there only presence blocks ask.

## The network chain

A network's chain is the roster (`xenonet-roster/2`,
`crates/xenonet-log/src/network.rs`, schema
`schemas/network/roster-entry/v2.json`): its machines, their keys, their
settings and who signed each entry. `found` names the network's owner,
the person it's theirs. Each `add` entry carries the host's transport
keys, whether it's an admin (a window host), the person's device on it
(none on a machine that can't ask them) and its machine signing key, which
roots its tokens. A `settings` entry holds a machine's start and wake.
Entries that admit or widen name the owner's chain head they were made
under (`personHead`).

| op | Signed by |
|---|---|
| `found` | the founder, once, as entry 0, naming the owner |
| `add` a machine that isn't an admin | any admin, with presence (`admit`) |
| `add` an admin | the founder, or two admins, each with presence (`admit`) |
| `window` on | the founder, or two admins, each with presence (`widen`) |
| `window` off, `remove` an admin | the founder, or two admins |
| `gateway` | the founder, or two admins, each with presence (`widen`) |
| `remove` a machine that isn't an admin | any admin, or the machine itself |
| `remove` the founder | nobody |
| `settings` that narrow | any admin, or the machine itself |
| `settings` that widen (start to `network` or `delegate`, wake to `network`) | the founder, or two admins, each with presence (`widen`) |
| `relays` that add a relay | the founder, or two admins, each with presence (`widen`) |
| `relays` that only drop relays | the founder, or two admins |
| `mailboxes` that add a mailbox | any admin, with presence (`widen`) |
| `mailboxes` that only drop mailboxes | any admin, or each mailbox it drops |
| `rekey`: a member's new transport keys | the member itself, with presence (`admit`) |

An admin's signature counts toward an admit or a widening only with a
presence block from that admin's device, the one its `add` entry named,
and only when the owner's chain had that device active at the head the
entry names. A live receiver refuses a head older than the one it holds.
An entry signed rightly but by too few waits at the gateway as a proposal
for the founder or a second admin to approve. Signers sign with their
machine signing keys, never their host keys. A relay URL in the chain
carries no credential: a relay's auth token is a setting of the machine
that uses it, since the chain is kept forever and sent to every member.

The facts a decision reads from it: each member's role (member, admin,
founder), its settings, its device and its machine signing key.

## Token roots

Biscuit signs only with a key in memory, and the machine signing key may
never leave a secure element. So each service start makes a token root in
memory the first time it mints, and the machine signing key certifies it:
"this root speaks for this machine from `from` to `until`", for a year
(`crates/xenonet-net/src/roots.rs`). The certificates are public: they're
kept in `<state>/identity/token-roots.json`, so tokens outlive a restart,
and travel with an act that uses them. A root's private half never touches
a disk. A root is made again once it has less than 330 days left.

A receiver checks a token against the roots it holds, and each root's
certificate against its machine's signing key: its own from its identity,
another machine's from that machine's `add` entry in the network chain.

## The actor grant

Every actor holds one grant: a local actor, a bot such as Grok Bot or
Hermes, a cloud assistant. It's a Biscuit v6 token rooted at the actor's
machine (`crates/xenonet-authz/src/token.rs`), written as
`xenonet_actor_` and the base64url of the token. Its authority block holds
exactly these facts, and parsing refuses any other:

- `actor(slug)` and `machine(id)`: whom it speaks for;
- `network(id)`: the one network it may act in, or none for any network
  this machine is in;
- one `right(action)` for each action it may do;
- `expires(date)`, with the check that the time is before it.

Rights are read from the authority block alone, so a block a holder
appends can only narrow. XenoNet keeps a token's revocation id and the
capability words it was minted for, never the token. A token is shown
once, travels as `Authorization: Bearer`, and never goes in a URL.

**Rights** come from the capability words the person allowed
(`crates/xenonet-actors/src/tokens.rs`):

| Word | Rights |
|---|---|
| every actor | `read` |
| `spawn-sessions` | `start` |
| `message-agents` | `message`, `handoff` |
| `edit-repos` | `claim` |
| `answer-prompts` | `answer`, which lets it Deny; what it may Allow is its allow scope |
| a gateway token | `use-gateway` too |

Other words (running commands, browsing…) say what an actor does
elsewhere; they allow nothing in XenoNet but reporting it.

**Scopes** follow the goals' hosts, identities and sessions. Hosts are in
the token: the machine that minted it and the network it names. The rest
is a row in `grant_scopes` in `engine.sqlite`, keyed by the token's first
revocation id:

| Column | What it scopes | Used by |
|---|---|---|
| `sessions` | A selector of the sessions it reaches (`project:`, `identity:`, `machine:`, `by:`…), with its end | Delivery: `peers`, `send`, broadcasts, `who_has` |
| `projects` | The projects a cloud assistant reads | The remote connections |
| `allows` | Its allow scope: sessions (`all` or a list), kinds of prompt (`read`, `edit`, `command`, `web`, `tool`) and an end | Answers |
| `identities` | Reserved for the accounts it may act as; no code yet | |
| `intents` | Reserved for phase 4's intents; no code yet | |

**Issuing and widening** a grant (its rights or any scope) is the act
`register-actor`, which needs the person's presence at the window. The
presence block is bound to the actor, its rights and the hash of its
scopes, so a presence for one grant can't stand for a wider one.
Narrowing, revoking and removing need nothing.

**Setting the scopes.** The Actors panel shows each actor's sessions scope
and, for one that answers prompts, its allow scope, and its Scope… view
sets and narrows both: the sessions scope with `POST /api/delivery/grant`
(which always asks, since selectors can't be compared) and `POST
/api/delivery/revoke`, the allow scope with `PUT /api/actors/<slug>/allows`
(which asks only when it widens) and `DELETE`. `GET /api/actors/tokens`
lists each actor's allow scope (`actor/tokens/v1`'s `allows`).

**Allow scopes.** An actor whose grant has `answer` may Deny any prompt it
sees. It may Allow only a prompt inside its allow scope, never for a run
whose agent is unchecked, and the decision is never wider than the scope.

**Gateway tokens** are the grant with `use-gateway`, narrowed with
`check if door("gateway")`, and written as `xenonet_gateway_…`. They work
only at the actor gateway, its MCP endpoint and A2A alike, which are
decided at the gateway door. An actor token without `use-gateway` doesn't
work there, and a gateway token doesn't work at a local door.

**Page cursors** for a cloud assistant's lists are the grant narrowed to
one read, its filters' hash and five minutes, holding where the last page
ended as data that narrows nothing. A cursor can't be made up or changed,
it ends with the grant, and one from before the grant's scope changed is
refused.

**Revocation** is one `revoked` table in `engine.sqlite`, by revocation
id, read through `Ground::revoked`. Revoking a token's first id takes back
it and everything narrowed from it. Revocations are kept 400 days, past a
root's year. A token unused for 30 days is revoked. Locally a token must
also be the one its actor holds now.

**Acts on another machine.** An actor's own token is decided at its own
machine. For an act that goes on (a message, a start), its machine mints a
token for that act's right alone, in the network, for eight days (past the
week the outbox keeps a message), and sends it with its root's
certificate (`authority.token` and `authority.root`). The receiver checks
the certificate against the sender's machine signing key in the network
chain and decides the act as that actor.

## Delegation

Delegation is a standing grant the person signs to a set of their own
devices: a `delegate` entry in their chain
(`crates/xenonet-log/src/person.rs`). It holds:

- `id`: `dlg_…`; a `delegate` with an id the person already has replaces
  that one;
- `devices`: a list, or all of them, those paired later too;
- `work`: `start`, `answer`, or both;
- `prompts`: the kinds of prompt an Allow under it may cover;
- `limits`: one set today, `routine-in-folder`, the delegation judge's
  rules, which whoever answers under it checks before asking `decide`;
- `until`: when it ends.

Issuing or widening one needs the person's presence; narrowing it or
ending it (`undelegate`) needs only a device's signature. It covers work
only when it names both this machine's device and the device the work came
from: the person's signing device, or the device of the machine the agent
or actor is on.

- **Starts.** A machine runs a delegated start only when its start setting
  is `delegate` and a delegation for starts covers the run. The person
  starts delegated runs without the setting.
- **Answers.** For a run started under a delegation, the machine itself
  allows a prompt of a kind a delegation answers, after its limits pass.
  Everything else comes to the person.
- **The standard delegation.** Setting a machine to `delegate` from the
  window, when no delegation names its device, issues one: every device,
  starts and answers, read, edit, command and tool prompts, the routine
  limits, for a year.

The set is named as devices, not machines, so the intents phase 4 adds
(made on one device, worked on the others) take the same grant.

## `decide` and its rule table

`decide(token, request, ground, presence) -> Allow | Hold | Refuse`
(`crates/xenonet-authz/src/decide.rs`). A request is a closed struct:
principal, resource (a machine, a network, a run, a session here), act,
door (local, network, gateway, A2A), time and the network its envelope
named. The action is the act's, so the two can't disagree. `Ground` is the
facts the receiver holds: members and admins, each machine's settings, the
person's devices and delegations, revocations, certified token roots,
consent rows, allow scopes and the prompts waiting. An `Allow` carries an
`Authorized`: who, what, to what, the act's hash and the authority it
rests on (membership, a token, the person's signature, their presence, a
delegation, an allow scope), with a note when it's weaker.

A request goes through these steps in order, and the first refusal is the
answer:

1. **The resource** fits the action and is this machine's: a machine or
   run here, a network this machine is in, a session here, a prompt
   waiting here.
2. **The door.** At a local door, only this machine's principals and the
   person. From the network, a member of the network the envelope names,
   never this machine itself. At the gateway and A2A, only this machine's
   actors.
3. **Who.** The person's device signature is for this act, from an
   unrevoked device of this machine's person. An actor holds a token for
   exactly them, whose root is still held and certified, with no id
   revoked, whose Datalog grants the action's right (and `use-gateway`, at
   the gateway or A2A) and whose own checks pass. Nobody else brings a
   token.
4. **The rule table**, one row per action and one column per kind of
   principal:

```text
                 machine   agent     actor     person
  message        reach     reach     reach     reach
  wake           wake      wake      wake      reach
  start          -         start     start     start
  answer         answer    -         answer    answer
  claim          reach     claim     reach     reach
  handoff        reach     reach     reach     reach
  read           read      read      read      reach
  admit          -         -         -         presence
  widen          -         -         -         presence
  publish        -         -         -         presence
  register-actor -         -         reach     presence
  use-gateway    -         -         reach     -
  revoke         -         -         -         presence
```

| Cell | Means |
|---|---|
| `-` | refused |
| `reach` | steps 1 to 3 are enough (an actor's token has the right) |
| `wake` | the machine's wake setting is `network`, or the session's consent allows it; actors need consent for actors |
| `start` | start `off` refuses everyone; `you` lets only the person; `network` and `delegate` let agents and actors too, never an unchecked agent; a delegated run needs start `delegate` (the person needs none) and a delegation of the person's for starts that names this machine's device and the one the start came from |
| `claim` | reach, but an unchecked agent claims only on its own machine |
| `read` | the map: reach. Transcripts, search and state: here, an agent process-checked at least; from another machine, an admin member, never an unchecked agent |
| `presence` | the person, with a presence block for this act from the device that signed it; a weaker device is allowed and the decision says so. `register-actor` is issuing an actor's grant or widening it; narrowing and revoking need no act |
| `answer` | A Deny only narrows, so it needs no presence: the person; this machine for its own window; a window host (an admin) relaying its window's; an actor whose token has the right. An Allow widens, and never for a run whose agent is unchecked: the person, with a presence block for it; an actor, inside its allow scope here; this machine itself, for a run started under a delegation of the person's that answers that kind of prompt and names this machine's device and the device the run was started from |

5. **Hold.** Start, answer, admit, widen and publish for another machine (a
   principal not of this machine, or the person over the network) hold
   instead of allowing when governance was last synced from another member
   more than ten minutes ago, or never. This machine acting on its own is
   exempt, so a laptop on a plane still answers its own prompts, and so is
   a Deny.

Admits and widenings of the network are decided by the network chain's
policy, which holds the same rule: an admin's signature with the person's
presence. Publishing a service is the act `publish` on the network, by the
person at this machine's own door, with a presence block bound to the
publication: its name and the hash of its address, paths and callers, so
a presence for one publication can't stand for a wider one
(`crates/xenonet-net/src/node.rs`, `decide_publish`). It's done from the
window (Network settings › Publish a service); `xenonet svc publish` says
so and does nothing. Taking a service off needs nothing.

**How it's tested** (`crates/xenonet-authz/tests/authz/`): an exhaustive
matrix of 86,400 decisions (every action by every kind of principal, here
and from another machine in every role, under every start and wake
setting, consent, delegation and governance age) against a separately
written statement of the rules; property tests that a
narrowed token never allows what its parent refuses, that a revocation
holds once it's held, and that an allow scope is never decided wider than
it is; and token and presence cases: a changed or cut token, a root
that isn't held, certified or in its time, a presence for another act or
device, a Deny without one. `xenonet-id`'s own tests hold that a block
approves the one act it names, once and for a minute.

## `Information<T>` and the compile-fail tests

Message, handoff, ask and prompt text is information, never authority.
The types hold that:

- `Request`, `Act` and `Principal` have no field for text. Ids are short
  words from a fixed alphabet, and a run's folder can't hold a control
  character, so a message's lines can't reach a decision through a
  request. Callers parse requests from the signed act and the envelope's
  header, never from a body.
- `Information<T>` has `new` and `read` and nothing else: no `From`,
  `Into`, `Deref`, `AsRef` or `Clone`. Logged, it says what it is.
- `Authorized`, `PersonSigned` and `VerifiedPresence` have private fields
  and no public constructor: only `decide` makes an `Authorized`, and only
  a checked signature or block makes the other two.

`crates/xenonet-authz/tests/ui/` holds the proof as trybuild compile-fail
tests: information isn't authorized, isn't a request, an act or a
principal, isn't a token and can't be read as one, and an `Authorized`
can't be built outside the crate or changed after `decide` made it.

The runtime wraps text in `Information` where it comes in from another
machine or an actor (`crates/xenonet-runtime`): a message's body
(`network.rs`, `message_in`), an ask's question and input (`asks.rs`,
`ask_in`), a handoff's goal and notes (`handoff_in`), and a start's
prompt, which is taken out of the request before the start is decided and
handed to the run only once it was allowed (`starts.rs`, `apart`,
`run_here`). It's read where it's stored and shown, never where a request
is made. The engine's own crate keeps plain text: it stays out of
`xenonet-authz`.

## The local API: the credential and the rules table

**The credential.** Every `/api/` route takes one
(`crates/xenonet-http/src/routes/rules.rs`):

- the API credential: each start writes `<runtime>/api-<port>.cred`,
  owner-only, with a new value each time; it's never logged and is removed
  at shutdown. The command, the hooks, the MCP servers, the relay and the
  e2e harness send it as `x-xenonet-credential`;
- the window key: Desktop gets it in the component service's hello and
  sets it as an HttpOnly, SameSite=Strict cookie,
  `xenonet-window-<port>`, or sends it as `x-xenonet-window`;
- an actor's token, as `Authorization: Bearer`, decided as the actor.

Anything else is 401 `{code: "credential"}`, a wrong credential too,
whatever else came with it. The file keeps out other OS users and
programs that can't read it. An agent on the same account can read it, so
the file alone never reads what matters.

**The rules table** (`RULES`) has one row per route: its method, its path
(`*` one segment, `**` the rest), who may call it, the authz act it is, and
whether it's the window's alone when it names another machine (`?host=`,
or an id that starts with `@`). A path with no row is refused, so a new
route can't ship unguarded. Callers:

| Who | Means |
|---|---|
| `Open` | no credential; the route checks for itself: `/api/health`, the Spotify sign-in's return (its state is its check), and Claude Code's HTTP hooks, which can't carry a header and are taken only from the process of the session they name |
| `Tray` | the tray app's key, from your own process |
| `Local` | any credential |
| `Program` | the file's or the window's credential; `Authorization` belongs to the service being called (`/api/svc/…`) |
| `Hook` | a credential; the handler checks the process of the session the hook names |
| `Checked(scope)` | a read that matters: the window, the service's own reads, a session in `x-xenonet-agent` whose process passes the check, or your own terminal, decided through authz with the process check as the agent's attestation. Never an actor's token: actors read through their grants |
| `You` | the window, or your own terminal (not from under an agent or an actor product); never an actor |
| `Window` | the window's credential from the window's own process; never an actor |
| `Session` | a session named in `x-xenonet-agent`, its process checked |
| `Actor` | an actor's token, decided by the handler as the actor, with the right the row's act names |

Reads, messages, claims and handoffs are decided at the local door for the
caller found; starts and answers by the engine, with the run or the prompt
they're for. A session whose process check passed because nothing could
be seen is decided as unchecked. Routes that ask the person (Allow, a
start as you, issuing a grant, adding a machine) are `Window` rows, or
check that the window called, so no prompt starts from anywhere else.

## Refused from under an agent

- **The window's alone:** adding a machine (its ticket), every act that
  asks the person (setting up the identity, pairing, admitting, widening,
  delegation, a start or message as you, Allow, publishing a service,
  revoking a device), issuing or widening an actor's grant, gateway
  tokens, allow scopes and sessions scopes (the Actors panel's Scope…),
  pairing and turning on remote connections, and answering permission
  prompts (the window or an actor with the right).
- **Yours, from the window or your own terminal:** creating, joining and
  leaving a network, removing and replacing machines, approving, moving
  the gateway, a machine's settings, taking a service off, revoking and
  removing actors, connecting your agents, and `xenonet service install`.
- **Reads that matter** (transcripts, search, the engine's state, delivery,
  other machines' state) from an unchecked caller.
- XenoNet's control channel never presses Allow, Deny or the buttons that
  answer as you outside the demo.

## What stays weaker

- **Presence off a Mac.** On Windows and Linux the device key is a file. A
  program of the account that reads it can sign as the person and make
  presence blocks without any prompt; the check stops only XenoNet's own
  paths, and an agent that can type could pass polkit, a passphrase or a
  PIN. Those devices are recorded as weaker and every decision on them
  says so.
- **The machine signing key.** On a Mac any program of the account can ask
  the Secure Enclave to sign with it; elsewhere it's a file, and the TPM
  isn't used. Either way a program running as the account can certify a
  root and mint tokens for this machine's actors. Running the service as
  its own OS user is the hardening for that, and isn't built.
- **The host key** is a file. A program of the account that copies it can
  authenticate as this machine, send frames as it and speak for its
  agents, and open what is sealed to it, until the person rekeys it. It
  signs no chain entry, feed entry or address record: those take the
  machine signing key.
- **No nonce on start and message acts.** A device's signature over a
  start or a message as you covers the act and its target, not a nonce or
  a time, so the receiver relies on the frame's own replay checks.
- **Pairing needs the root's machine.** The root key isn't copied to other
  devices yet, so only the machine holding it pairs a device. A machine
  added by another admin joins as a plain member and does nothing as the
  person. There's no recovery phrase yet.
- **Revocation is local.** An actor's grant is revoked on its own machine,
  where its acts start; a receiver doesn't read the person chain's token
  revocations yet, and a per-act token lasts its eight days. Store-order
  revocation comes with feeds in phase 4.
- **An actor's Allow stays on its machine.** Allow scopes are rows on the
  actor's own machine, so from another machine an actor can only Deny.
- **The process check.** Where XenoNet can't see the calling process, a
  `You` row passes on the credential alone and a session is unchecked.
  Only a run XenoNet started asks a permission prompt, and the ask keeps
  what the process check found of its caller: one it couldn't see is
  unchecked, so only the person may Allow it.
- **Governance freshness** is the last chain sync from any member, as the
  gateway's link reports it.
- **Hardware keys** (FIDO2) for the widest acts aren't built.

## Alignment check

- **Which goal?** Goal 3 first: one authorizer, explicit and narrow
  authority that can be revoked, and the person in charge of what admits,
  widens or publishes by presence. Goal 6: `xenonet-id`, `-log` and
  `-authz` are usable alone. Goal 9: the matrix, the property tests and
  the compile-fail tests. Goal 8: Biscuit, adopted, not invented.
- **Intents, hosts, identities and sessions.** Every act names the host it
  runs on (a machine or a run there), and a grant's hosts are in its
  token. Sessions are what a grant reaches and what an allow scope covers.
  Identities have a reserved scope and reach grants today through the
  sessions selector's `identity:` terms. Intents have a reserved scope,
  and delegation names devices so that an intent made on one device can be
  worked on the others in phase 4.
- **One-of-anything.** One person per machine, as Will decided for the
  first cut; the person chain and `Policy` hold many. Tokens name their
  network, so nothing assumes one.
- **Who, under what authority.** Every act carries its authority (a
  device signature, a presence block, or a token and its root's
  certificate), and the receiver checks it against its own chains.
  Nothing trusts a sender's strength.
- **Poll or push.** Nothing new polls. Person entries are pushed on
  append, and asked for by head when an act names a newer one.
- **Vendor or surface knowledge in a core crate.** None: capabilities are
  actions, and the platform keys sit in `xenonet-keys`, outside the core.
- **Reuse.** biscuit-auth 6, ring, trybuild and proptest. The chain is
  XenoNet's own, as the direction says, since nothing small fits.
- **Properties.** A narrowed token never allows more; a revocation holds;
  a presence block counts once, for one act, for a minute; no message
  text reaches a decision; an allow scope is never decided wider than it
  is. Each has a test named above.

## Related

- [goals.md](goals.md): what XenoNet is for, and the alignment check.
- [direction.md](direction.md): the target shape, the order of work and
  Will's decisions.
- [network.md](network.md): the network these acts cross.
- [actors/interface.md](actors/interface.md): how actors register and get
  their grants.
- [cloud-assistants.md](cloud-assistants.md): the remote connections'
  reads.
- [../guide/network.md](../guide/network.md),
  [../guide/delegation.md](../guide/delegation.md),
  [../guide/actors.md](../guide/actors.md): the same, for the person.
