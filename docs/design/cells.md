# Cells and eidolons

Two tools that sit beside XenoNet in this repository: **Adytum** keeps
sealed cells on a machine, and **Eidolon** keeps an entity's state as one
sealed thing that opens in a cell and moves between machines. XenoNet stays
the only way across machines and the only place that decides who may do
what. Neither tool opens a network listener beyond localhost.

```
XenoNet   who may do what, across machines (signed acts, decide, grants)
   |      drives both through their --json commands and MCP servers
 Eidolon  an entity's state, sealed, opened in a cell, moved
   |      links Adytum's library crates
 Adytum   cells on one machine (VMs and containers)
```

## Where the code is

- `adytum/crates/*` and `eidolon/crates/*`, members of the root workspace,
  held to the same lints, gate and lockfile as XenoNet's crates.
- The layer rule (`scripts/rust-layers.mjs`, `TOOLS`): Adytum's crates take
  `xenonet-text`, `xenonet-os`, `xenonet-store` and `xenonet-keys` of
  XenoNet's, and nothing of Eidolon's. Eidolon's take those, `xenonet-id`
  and Adytum's. No crate of XenoNet's or Xenolith's depends on either, tests
  included. Moving them to repositories of their own is moving two folders.

## Vocabulary

- A **cell** is a VM or a container with one purpose, made and removed by
  Adytum. A **wall** is an opaque label every cell carries. Adytum never
  learns what a label means; XenoNet maps identities to walls.
- An **eidolon** is a set of **facets** (a folder, a browser profile). It is
  **sealed** (encrypted chunks in a vault) or **open** (laid out in one cell).
  It has one **seat**: the machine that may open it now.
- A **holder** is a machine on the eidolon's holder list, named by its
  Ed25519 key.

## Adytum

| Crate | Job |
|---|---|
| `adytum-model` | Typed ids (`cell_`, `img_`, `wall_`), names, the cell spec, image pins, refusals. Serde, no IO |
| `adytum-policy` | Every isolation rule as a pure function over a spec and what already exists |
| `adytum-driver` | The trait a cell driver implements (`create`, `start`, `exec`, `stream`, `copy_in`, `copy_out`, `stop`, `remove`, `list`, `clone`) and its errors; the one place text becomes a cell name |
| `adytum-exec` | A program and an argument list, never a shell string: streamed output, a deadline, a cancel, the process group stopped, only the named variables, and a duplex byte stream to a child's stdin and stdout |
| `adytum-lima` | Lima cells: create, start, run, copy, stop, remove, from a pinned image, by running `limactl` through `adytum-exec` |
| `adytum-oci` | Podman and docker cells, the same trait as Lima |
| `adytum-ledger` | A record before each create and remove, settled after, compared with what exists at every start. It keeps its own `adytum.sqlite` under `ADYTUM_HOME`, not XenoNet's store, because a table added there would change XenoNet's schema hash and make every XenoNet machine set up again |
| `adytum-doctor` | What is wrong, each problem with its fix |
| `adytum-cli` | The `adytum` binary |
| `adytum-testkit` | A fake `limactl` and `podman` that replay golden files |

### The rules (`adytum-policy`)

Each rule's first test is its refusal.

- A port is published on 127.0.0.1 only. A spec's automation port is never
  published at all.
- A sealed cell has no mounts from the host.
- Adytum touches only cells it named: every driver refuses a name outside
  `adytum-*`.
- Walls: two cells with different walls never share a network, a mount
  source, a clone source or a secret.
- A cell that holds an open eidolon is not cloned.

### Running a command

`adytum-exec` is the only place a child process starts. A command is a
program and a list of arguments. The child gets an empty environment plus
the variables the caller names (and `PATH` and `HOME`, named the same way),
runs in its own process group, and is stopped with the whole group when the
deadline passes or the caller cancels. Output streams as it comes. A duplex
mode hands the caller the child's stdin and stdout as one byte stream, which
is how Eidolon moves bytes into a cell and how the host reaches a browser's
automation socket without a forwarded port.

### Drivers

A driver implements one trait: `create`, `start`, `exec`, `stream`,
`copy_in`, `copy_out`, `stop`, `remove`, `list`. `adytum-lima` writes a Lima
template from the spec (image by location and digest, no host mounts for a
sealed cell, port forwards on 127.0.0.1 only) and runs `limactl` with
`--tty=false`. Its golden files pin what `limactl` is asked and what it
answers, per pinned Lima version.

## Eidolon

| Crate | Job |
|---|---|
| `eidolon-model` | Typed ids (`eid_`, `fct_`), walls, facet specs, holders, the seat record and its `State` (`Sealed`, `Opening`, `Open`, `Retired`) |
| `eidolon-policy` | One seat, the holder list, the wall, what a facet may leave out |
| `eidolon-vault` | Chunks named by a keyed hash of their content, encrypted at rest |
| `eidolon-pack` | A facet's files into chunks, the manifest that lists them, and the signed head that names the manifest's chunks; and back |
| `eidolon-facet` | The facet trait and the `folder` kind |
| `eidolon-move` | Have-and-want over any byte stream, resumable, key wrapped to the receiver, the seat passed |
| `eidolon-bidi` | A WebDriver BiDi client |
| `eidolon-browser` | The browser facet: profile rules, the systemd units, a browser kept alive in a cell |
| `eidolon-view` | The person's window for sign-ins: a screen share on 127.0.0.1 |
| `eidolon-mcp` | The browser facet's tools for agents, over stdio or over HTTP on localhost |
| `eidolon-cli` | The `eidolon` binary, and the machine's file key store |
| `eidolon-testkit` | A fake BiDi server |

### Sealed

- Files are cut into content-defined chunks, so an edit in the middle of a
  file changes the chunks around it and no others.
- A chunk's name is an HMAC-SHA256 of its plaintext under the eidolon's
  name key, so the vault doesn't show which eidolons share a file. Its body
  is compressed, then sealed with ChaCha20-Poly1305 under the eidolon's
  content key, the nonce taken from the name. A chunk whose body doesn't
  open, or opens to a plaintext with another name, is refused.
- Both keys come from one 32-byte eidolon key, kept under the eidolon's name
  in the machine's key store (`xenonet_id::store::KeyStore`), never in a file
  beside the vault. On this platform the store is a file store, one
  owner-only file per secret under `<home>/keys`, because no operating system
  store is used yet; `eidolon machine init` says so.
- The manifest (in `eidolon-pack`) lists each facet's files (path, mode,
  size, chunk names) and is itself sealed into chunks. The head names those
  chunks and is signed by a machine key. An eidolon opens only if some machine
  on the holder list signed its head, because after several moves the signer
  can be neither the current holder nor the sender of the last move.

### One seat

The seat is a signed record: the eidolon, a generation number, and the
holder that may open it. A move writes the next generation, naming the
receiver, signed by the sender. The sender then deletes its eidolon key, so
its vault can no longer open the eidolon even though its chunks remain. A
receiver refuses a seat record that isn't the next generation, isn't signed
by the current seat, or names a machine off the holder list.

An eidolon is `Sealed`, `Opening`, `Open` in one cell, or `Retired`. `Opening`
is written before the first file is placed in a cell, so a kill during an open
is found by the next command, which clears the cell or finishes the open.
A sender that cannot tell whether the receiver took the eidolon keeps it
closed until the person settles it (`move-settle`).

### Moving

Over any byte stream (a pipe, `adytum-exec`'s duplex mode, SSH, later
XenoNet):

1. The sender offers the manifest's chunk names.
2. The receiver answers with the names it lacks.
3. The sender sends those chunks as they sit in its vault, already sealed.
   The receiver checks each and keeps it, so a broken transfer resumes from
   what arrived.
4. The sender wraps the eidolon key to the receiver's X25519 key and sends
   it with the new seat record. The receiver checks the record, stores the
   key and acknowledges. Only then does the sender delete its key.

A move killed before the acknowledgement leaves the seat with the sender;
after it, with the receiver. There is never a moment with two seats or none.

### The browser facet

Firefox runs in its own cell, kept alive by systemd units, with no program
of Eidolon's in the guest. The cell is set up on its first open: Firefox from
Mozilla's package repository, a virtual display and a screen share. Its automation is WebDriver BiDi on a socket in
the guest, which the host reaches as a byte stream over Adytum's exec
channel; the port is never forwarded. Ready means the protocol answered.
Before sealing, the browser is stopped so its profile is quiet, and caches
are left out. Eidolon does not hide automation from sites.

The person signs in through a window (`eidolon view`) on 127.0.0.1 with a
secret per session, shown once. The window is three units: the browser
without its automation socket, a virtual display and a screen share. Opening
it stops the automation browser, and closing it starts that browser again on
the same profile. While it is open, automation is off. The host reaches the
screen share as a byte stream over Adytum's exec channel, as it does the
automation socket; nothing is forwarded.

Agents get the browser through `eidolon mcp <eid>`, an MCP server on standard
input and output, or `eidolon mcp <eid> --http 127.0.0.1:<port>`, the same
tools over HTTP on the loopback address only. The HTTP form makes a token,
writes it once to standard error and checks it on every request; it answers
only on `/mcp/` and refuses a request that carries an `Origin` header. XenoNet
publishes that address as a service, so a session on another machine calls it
through the network's own door with the token as its `Authorization` header.
Files for uploads and downloads pass through one folder on the host,
`$EIDOLON_HOME/files/<eid>/`. An upload must resolve to a regular file inside
it, and a fetched download lands in its `downloads` folder.

## The alignment check

- **Which goal?** Goal 3, secure by construction: a wall is a machine
  boundary, so a session can't act as an identity that isn't in its cell.
  Goal 1: an identity's state moves to the machine where the work is. Goal 7:
  each tool is one command with a doctor.
- **Intent to outcome.** A session given an identity's task works in a cell
  holding that identity's eidolon, on whichever host holds it; the browser
  facet lets it post, upload and download as that identity.
- **One-of-anything?** None. Any number of cells, walls and eidolons per
  machine; an eidolon's holder list names any number of machines. The one
  seat is deliberate: one live copy of a signed-in session.
- **Who, under what authority?** Adytum and Eidolon act only for the person
  on their own machine. Across machines the request is a XenoNet act, decided
  by the receiving machine. An eidolon's seat record is signed by the
  machine key that held it, and the holder list bounds who may receive it.
- **Push or poll?** The browser's readiness and events are BiDi messages,
  never sleeps or polls. Moves send only missing chunks.
- **Vendor knowledge in the core?** None: Adytum knows Lima and OCI through
  drivers; Eidolon knows browsers through one facet kind.
- **Existing protocols.** WebDriver BiDi for the browser, Lima and OCI
  runtimes for cells, content-defined chunking and have-and-want as rsync
  and git do it, ring and x25519-dalek for the cryptography. Eidolon's own
  format covers only the manifest and the seat record.
- **How we know.** Refusal tests for every rule, golden files for the
  drivers, kill tests for the ledger and the move, and a live gate
  (`ADYTUM_LIVE=1 npm run cargo -- test -p eidolon-cli --test live`) that
  proves its own teardown.

## Related

- [goals.md](goals.md), [rust-core.md](rust-core.md) (the standards both
  follow), [authorization.md](authorization.md) (where cross-machine
  decisions are made).

## Designed, not built

Two ways XenoNet and the cells could meet that nothing implements yet.
Neither changes XenoNet's authorization rules; each says where it would
meet them.

### A move as a XenoNet act

Today `eidolon move` runs over a byte stream the person names: a pipe, or
a command such as `ssh` that Eidolon starts and speaks to. Between two
machines of one network the same move could be a XenoNet act instead, so
the person needs no shell access between them and the receiving machine
decides.

The sender's XenoNet signs an act for the move and sends it to the
receiver as it does any act from another machine: it names the eidolon,
its wall, the generation the move would write, and the sender's holder
entry (public keys only). The receiver's `decide` takes it at the network
door from a member, as it does a start from another machine, and then
needs the person: a machine receiving an identity widens what that
machine holds, so the nearest existing rows are `admit` and `widen`, with
a presence block bound to the act's hash. The ask shows in the receiver's
window ("tower wants to send you an eidolon for wall W: Allow?"), and
Touch ID answers it. A decision to hold, because governance is stale, or
a refusal, ends the move before a chunk is sent. Whether this is a new
act kind in the table or an existing one with a new resource is a
decision for authorization.md when it is built; this note doesn't pick.

Once the receiver allows, the move itself needs a long, two-way byte
stream, which `svc/2` is not (its requests and answers are 8 MiB at most).
The stream that fits is `forward(alpn)`: Eidolon attaches as the actor
for one registered ALPN, say `eidolon/move/1`, with the grant that names
it, and XenoNet carries the stream between the two machines over the
member connection ([network.md](network.md), "forward(alpn)"). Registering
the ALPN is a line of data in the composition root, as Xenolith's is. The
forward is Unix only, so a move to or from Windows waits for its named
pipe.

In Eidolon only the stream's source changes: the `move` and `receive`
commands would read and write the forward's socket, where now they read
and write a pipe. The protocol doesn't change: have and want, chunks sent
as sealed in the vault, the key wrapped to the receiver's X25519 key, the
seat record for the next generation, and the sender deleting its key only
after the receiver's acknowledgement. The seat doesn't change either:
Eidolon still checks the record's generation, the signature of the current
seat and the holder list on its own, so there are two separate decisions,
XenoNet's on whether this machine takes the eidolon at all and Eidolon's
on whether the record is valid, and neither stands in for the other.
XenoNet terminates QUIC and so reads the forwarded bytes, which are sealed
chunks and a wrapped key, nothing it can open. A move killed at any point
leaves one seat, as it does over a pipe.

### A cell as a sandbox kind on a start

The daily-use plan's slice 4 adds a `sandbox` field to a start: the
request, the signed act and what the adapter is given all carry it, and a
start from anyone but the person with it missing or off is refused. The
field there is on or off with allowed domains, mapped to each harness's
own sandbox. A cell could be a second kind of value in that field: the
start names an Adytum cell as the sandbox, and the session runs inside it.

The cell would hold the project folder and the identity's eidolon. The
start names an identity; the receiving machine opens that identity's
eidolon in a new cell on the identity's wall, with the project folder as a
folder facet, and starts the harness inside the cell through Adytum's exec
channel. What the session can reach is then what the cell holds: a machine
boundary, not a harness setting, which is the point of goal 3. The browser
facet, if the eidolon has one, is in the same wall and so reachable.

A cell is a stronger sandbox than a harness's, and it costs more: the
image needs the harness installed, a cell takes seconds to a minute to
start, and the session's hooks and MCP servers inside it must reach
XenoNet's local door without a published port (Adytum publishes ports on
127.0.0.1 only, and never the automation port). That last piece has no
answer yet and may need a socket carried over the exec channel, as the
browser's is. Which harnesses can run inside a cell at all is an
adapter question.

This waits on slice 4's field. Until a start can carry a sandbox, the
signed act has nowhere to name a cell, and XenoNet has no sandbox of its
own to extend. When the field exists, the cell kind adds a variant to it,
a refusal for an adapter that can't run in a cell, and a doctor row for
Adytum being present, which `xenonet doctor` already reports.
