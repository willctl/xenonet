# Sync between Xenoliths: `xenolith/sync/2`

Status: built in 2.0. The [README](../README.md#sync-between-machines) says
how to use it; this is how it works. "Not built yet" at the end lists what's
left.

Xenolith runs on its own. Between machines it uses XenoNet's libraries
(`xenonet-id`, `-log`, `-authz` and `-net`), never XenoNet's app, and it uses
a running XenoNet only through XenoNet's optional forward (shared mode,
below). The code is `rust/crates/net` (xenolith-net: the endpoint, the hello,
join/1, the worker, shared mode) and the core's `sync.rs` (what sync keeps in
the state folder and what its entries do to the folder), `chains.rs` (the
chain per space) and `xenonet.rs` (what shared mode holds of XenoNet).

## Spaces are chains

Each space is a chain of signed entries, xenonet-log's `xenolith-space/1`
policy (`crates/xenonet-log/src/space.rs`), kept in the state folder under
`spaces/`, never in the shared context folder, whose files anyone with the
folder can write. Entry 0 names what the space is about, its owner, its
first curator, a name and a nonce, and the space's id (`spc_…`) is derived
from that entry's hash, so nobody chooses it.

- **The owner** is a person (`prs_…`, through XenoNet, signing with one of
  their devices and their presence for what widens) or, standalone, this
  Xenolith's owner signing key, a file. A key-owned space is marked weaker:
  it can't be shared into a XenoNet network or bound to an entity, and it
  moves to a person only (`move-owner`), never to another key.
- **The curator** is one context key, the Xenolith that files the space's
  notes into pages. There is exactly one at a time.
- **Members** are the chain's subjects at a level: `read` (pages and the
  log), `append` (notes too), `write` (replacing and removing pages too) or
  `admin`. A subject is a context key with the endpoint id its share names
  (own mode), or a XenoNet network (shared mode).

Owner acts that widen or bind (`share`, `bind`, `move-owner`) are
sequenced: the owner signs, and the current curator appends the entry with
its own signature, so two acts made on two devices apart never fork the
chain. `unshare`, `take` and `move-curator` are side entries, valid at any
place: an `unshare` takes effect at each holder as soon as it holds it, so
narrowing never waits on the curator. A `take` or `move-curator` hands the
chain to a new curator at the next epoch; of competing ones for one epoch,
the lowest entry id wins on every member, with nothing frozen. A curator
that signs two different entries at one place stops its run there: reads
and notes go on, and only the owner's `take` moves the chain on.
`formal/space.qnt` ([formal/README.md](../../formal/README.md)) models these
rules.

A space founded here is curated here. The first space of a kind and name
keeps `spaces/<kind>/<name>/`; a second one of that name, from another
Xenolith, gets `<name>~<six characters of its id>/`
([FORMAT.md](../FORMAT.md#spaces)). Two spaces about one thing stay side by
side, never merged. Each folder Xenolith curates gets a signed
`xenolith-space.json` that tells a reader which chain it is; Xenolith decides
nothing by it.

## Keys

Three files in `<state>/keys/`, readable by the owner alone: the **context
key** (`context.key`), which signs what this Xenolith adds to a space (its
notes, as curator its pages and chain entries, and its hello); the
**transport key** (`transport.key`), its own endpoint's key, for QUIC alone;
and the **owner signing key** (`owner-signing.key`), the standalone owner.
A key file that can't be read is an error; Xenolith never makes a new key
over it, since a new key is a new identity to every peer.

## The endpoint (own mode)

`xenolith-service` runs an iroh endpoint keyed by the transport key, on UDP
47851 by default (`sync.bind` in `config.json`). It opens only once a space
is shared with another Xenolith or followed from one, or an invite or a join
needs it, and only once the upgrade from 1.x has run. It uses no relay and
no mDNS unless `sync.relays` (https URLs) or `sync.mdns` names them, and
never n0's by default. `sync.endpoint: "off"` keeps it closed.

**Admission.** A share names the member's context key and its endpoint id.
A curator admits the endpoints its chains share a space with; a member
admits its curator (from the chain, or, before the chain arrives, from the
follow record a join wrote) and the chain's other members. A connection
from any other endpoint is refused before anything is read. join/1 is
answered only while an invite is open.

## The hello

The first thing on a connection is each side's hello, signed by its context
key under the tag `xenolith-sync-hello/1`, over both endpoint ids, a nonce,
the time, the spaces it lists and the connection's own keying material
(RFC 5705's exporter, which isn't sent: the receiver derives the same bytes
from its end). Each side checks that the hello's context key is the one its
chains bind to the endpoint the connection comes from. A hello taken from
one connection fails on any other, and a nonce seen in the last ten minutes,
or a time more than five minutes off, is refused too.

The hello lists the spaces the sender holds whose chain admits the
receiver, each with its genesis hash and a digest of what the sender holds
of it, so a member that holds nothing of a space shared with it hears of it,
and a space that doesn't admit the peer is never named to it. It also says
where the sender's endpoint is and, from a curator, where the other members
were last seen, as hints: QUIC checks the endpoint id at every dial.

## What travels

Sync runs xenonet-net's link/2 engine with Xenolith's admission, hello and
groups (a space is a group). Each Xenolith writes one feed per space, signed
entries in its author's order, kept in `<state>/sync/out/`, and applies each
entry of another's feed once (`<state>/sync/feeds/`):

| Kind | From | To |
|---|---|---|
| `ctx.note` | a member at append or above | the curator alone |
| `ctx.propose` | a member at write, from an edit of its copy | the curator alone |
| `ctx.page`, `ctx.remove`, `ctx.log` | the curator | every member |
| `ctx.forget` | the curator | every member |

Notes are raw input, before the curator's secret and lint passes, so they go
to the curator alone; a member at read gets pages, never another member's
notes. An entry names its text by SHA-256 and the text travels beside it,
kept in `<state>/sync/blobs/` by that hash, up to 700 KiB; a page past that
stays on its machine, and the log says so.

**On the curator,** a note goes into the space's inbox for the curator to
file, as a local one does. A replace or removal against the page as it is
now becomes a note the curator applies (a removal at once); against an older
version it becomes a proposal, a held change of kind "sync" that waits for
the person's OK. A peer's note is `agent`. The one exception is shared mode:
a note keeps `user` only when its author is bound to a machine whose device
is an active device of the space's person owner, and it's stamped
`person@<machine>`.

**On a member,** the curator's pages are written into its copy, with what
they replace kept in `.history/`, and the last seven days of the curator's
log are kept. A watcher on the copy turns the person's edits into
`ctx.propose` entries, and an edit not yet proposed goes out before a
curator page that would replace it. A note sent to the curator moves to
`.curator/sent/` ([FORMAT.md](../FORMAT.md#a-notes-status)). Everything goes
out as it happens: there is no pass on a timer.

**Page epochs.** Every page, removal, log and forget entry names the curator
epoch it was made under. A member writes only entries of the epoch its
chain is at, so a superseded curator's pages don't land, and a curator holds
back entries from an epoch its own chain hasn't reached until the chain
does.

**Forget.** `xenolith forget` on a curator drops the texts that hold the
forgotten text from its blobs and sends a `ctx.forget` naming their hashes;
every member drops those texts too. The entries stay and still serve,
without the texts.

## Holders

Every member keeps the curator's entries it took (`<state>/sync/held/`), so
with the curator away one member fills another. A member whose hello shows
more of the curator's feed is asked, one at a time, for ten seconds each,
and answers through xenonet-net's `holder::serve_in` with its own copy of the
chain deciding: only to a member at read or above, only the curator's feed,
never a note, within per-peer rate limits. A requester whose chain differs
from the holder's waits until the holder has taken the requester's chain, so
a holder that is behind learns of an `unshare` before it serves. Space
entries travel only over authenticated QUIC between admitted members and
are never sealed for rest: a space has no mailboxes.

## join/1

`xenolith/join/1` is how one Xenolith joins spaces another curates in own
mode. It is xenonet-net's join with Xenolith's `JoinTarget`, so the ticket,
the proofs and their order are XenoNet's join/2's.

- **The invite** (`xlj1_…`) isn't secret: a 128-bit ticket id, the inviter's
  endpoint id, its relay if it uses one, and the SHA-256 of the offer (the
  owner's key id, the curator's context key id, and each offered space's id
  and genesis hash).
- **The code** is 26 characters of Crockford base32 (130 bits), given
  another way than the invite, read from a prompt or stdin, never an
  argument. It works once, for ten minutes; five wrong codes retire it. The
  inviter keeps only its hash.
- **The proofs.** The joiner proves first, over the ticket, the offer's
  hash, both endpoint ids (the inviter's being the one it dialled) and the
  key and endpoint it asks to be added with. A wrong code gets a refusal
  with no proof in it; a right one gets the inviter's proof, the offer and
  the chains, each with the share the curator appended for the joiner.
- **The joiner checks** that the offer is the one the invite names, that
  each chain checks from its genesis, is the space offered with the owner
  and curator offered, and shares the space with its own key at its own
  endpoint. Then it follows what the person chose (`--only`; all offered by
  default; `--all` also follows what the curator shares with it later),
  each in a folder of its own.

`xenolith invite` and `xenolith join` go through the service with the owner
key, and a share is signed by the standalone owner key. That is weaker than
presence: an unsandboxed agent running as the person can read both files.

## Shared mode, over XenoNet's forward

On a machine where XenoNet runs, Xenolith also syncs over XenoNet's own
endpoint with the members of the networks the machine is in, with no
invite. XenoNet carries `xenolith/sync/2` for it through `forward(alpn)`
([network.md](../../docs/design/network.md#forwardalpn)). It runs beside the
own endpoint, per peer: shares to a key go over the own endpoint, network
members over the forward.

- **The socket.** Xenolith reaches XenoNet's forward socket in XenoNet's
  runtime folder (`forward-<port>.sock`; `sync.xenonet.socket` names
  another, `"sync": {"xenonet": "off"}` turns shared mode off). It checks
  that the socket file is this account's and no one else may use it, and
  that the process answering on it runs as this account, before anything
  goes to it. It watches XenoNet's runtime folder and the grant's file and
  attaches whenever both are there, again after XenoNet restarts. On
  Windows XenoNet's socket is a named pipe, which isn't built, so shared
  mode is off there.
- **The grant.** Without a grant, Xenolith asks for one on the socket
  (`collect`), proving it holds its context key by signing XenoNet's
  challenge. The person allows it at XenoNet's window, with the networks it
  may reach ticked and their presence. The grant is an actor grant under
  the machine's token root naming `forward(xenolith/sync/2)` and
  `key(<context key id>)`, for 30 days. It comes back over the socket only,
  into `<state>/keys/xenonet.grant`, owner-only, never through a window, a
  command's output or a log. Xenolith collects a new one when under ten
  days are left, which revokes the one before. Taking it back at the window
  ends shared mode at once. `xenolith sync grant` (the grant on stdin) keeps
  one made another way.
- **The chains.** XenoNet hands copies of its networks' chains, the person
  chains of their owners and its own, and its token roots' certificates.
  Xenolith verifies them from genesis itself and keeps them in
  `<state>/xenonet/chains.json`. XenoNet's word counts for one thing only:
  which endpoint id a forwarded connection comes from.
- **Binding a peer to its machine.** Over the forward, the hello's endpoint
  ids are the two machines' XenoNets', and its keying material is what
  XenoNet's header gives for the connection it terminates. The hello also
  carries a copy of the sender's grant, narrowed so no XenoNet door takes it
  (`check if false`, for an hour), and its root's certificate. The receiver
  binds the hello's context key to a machine only when that machine owns the
  connection's endpoint by the network chains, its machine key certified the
  root, and the grant names that context key. A grant without `key(id)`, a
  copy used over another machine's connection, or a root another machine
  certified binds nothing.
- **Who is in a space.** A bound peer is in a space only through a share
  into a network both machines are in, never through a share to a key,
  which is the own endpoint's. A member that holds nothing of a space takes
  its chain only from its curator, and only when the space's person owner
  owns a network both are in. A space shared into one network is never
  named to a machine in another.
- **Signing at the window.** Over its forward session Xenolith says which
  spaces it curates, and XenoNet's window offers what the person can sign
  for each: Make it yours (`move-owner` from the owner key to the person),
  Share into a network (first those the space's project is shared into,
  marked "from the project") and Stop sharing. Xenolith drafts the entry
  (`chains::draft`, or `xenolith spaces draft`), XenoNet reads the draft
  back and signs it with the person's device only if it does exactly the act
  asked (`xenonet_net::space_sign`), with their presence where it widens,
  and Xenolith appends it as the curator (`xenolith spaces sequence`).
- **Narrowing.** XenoNet tells Xenolith what each network sees of the
  machine's projects. When a project stops being shared into a network,
  Xenolith holds its space back from that network at once; Stop sharing,
  which needs no presence, records it in the chain.

XenoNet terminates QUIC, so it reads what it forwards: the same machine and
account, which reads the folder anyway. Xenolith decides again on its own
copies of the chains and each peer's signed hello.

## Threads

The endpoint runs on a tokio runtime of its own, on one thread that never
waits on a file: everything that reads or writes one goes through a channel
to the worker thread (`engine.rs`), which sends what goes back through the
links. A third thread watches the spaces' folders. The hook, the stdio
server and the tray never start or link any of it: only `xenolith-service`
takes xenolith-net.

## Not synced

Per machine, never copied: `config.json`, `access.json` and its tokens, the
keys, the owner key, the curator's sign-in, the audit chain and the tray's
settings. Ask runs on the machine where it's asked, on that machine's copy.

## Not built yet

- A ticked list to confirm before following, and the relay and mDNS
  questions at `xenolith invite`; invites and joins in the window. An open
  invite is lost when the service restarts.
- A `rekey` of a member's endpoint; a member whose transport key changed
  joins again.
- The curator's `ctx.snapshot` of the live pages, so a new member gets the
  live set rather than every entry; a forget that also redacts members'
  `.history/`.
- A superseded curator sending its unfiled inbox to the new one, and a test
  of two takes in a partition with both curators filing.
- A stored feed incarnation checked against the feed after a state folder
  is restored.
- Shared mode on Windows (the named pipe); per-remote limits on forwarded
  connections; revocations and project ids over the socket, and chains
  pushed on change; Xenolith taking XenoNet actor grants with ctx rights,
  so an actor holds one grant for both; one presence for a batch of moves to
  the person, and spaces founded with the person as owner.
- Binding a space to its project's id: the chain takes a `bind` only from a
  person owner, and the window doesn't offer it, so `xenolith spaces bind`
  says what it would do and a space stays tied by its name. Machine and
  actor owners aren't checked when a space binds.
- Ask from another machine, run on the curator; Xenolith's MCP on the
  2026-07-28 revision; its owner decisions on XenoNet's platform keys.

## Related

- [README: Sync between machines](../README.md#sync-between-machines) and
  [From 1.x to 2.0](../README.md#from-1x-to-20)
- [FORMAT.md](../FORMAT.md): the folder, the space marker and the `~` folders
- XenoNet's [network.md](../../docs/design/network.md) (link/2, join/2,
  holders, `forward(alpn)`), [authorization.md](../../docs/design/authorization.md)
  (the grant, forward grants, space rows) and
  [shared-context.md](../../docs/guide/shared-context.md)
