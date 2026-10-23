# Multi-host workspaces

Research for merge point A, workstream 4 ("Roots and hosts"). It compares
ways for one XenoNet console to show the projects and sessions of several
machines, recommends one, and plans a prototype for Wave 2. Section 2 of
`contracts.md` fixes host-qualified ids; this builds on it and proposes a
few amendments ([Contract notes](#contract-notes)). Sources are linked
where they're used and listed at the end.

**In short:** build a hub that federates XenoNets over SSH.

- Every machine keeps running its own XenoNet on 127.0.0.1, unchanged.
- The console you look at runs `ssh <host> xenonet relay`: a small, read-only
  command that streams that host's snapshots and events back over the SSH
  connection's standard input and output, much as Zed's remote development
  and MCP's stdio transport do.
- Nothing listens on a network interface, XenoNet never handles a key, and
  every host's API keeps its Host and Origin checks as they are.
- On the map, this machine stays at the center as today and each host is a
  constellation of its own. A dashed bridge joins checkouts of the same repo
  on different machines.
- Merging repos into one node (option 2) and several projects folders on
  one machine (option 1) both remain possible later, on top of this.

## What's built

The recommendation was taken at merge point A and is built as a
prototype: read-only, and shown on a console started with
`XENONET_HOSTS=1` (being a host needs no flag). As of Wave 3:

- **`xenonet relay`** (`core/hosts/relay.js`) sends a host's map, live
  state, engine and events over the SSH connection's standard input and
  output: titles, states and counts unless the console asks for
  `--detail`, and no home folder.
- **The hub** (`core/hosts/hub.js`, `link.js`, `merge.js`) keeps a link
  to each host and its last snapshot, cached by its key, and serves the
  merged view with every remote id qualified and only the fields the
  schemas declare.
- **`xenonet hosts`** adds, removes, tests and reconnects hosts, and names
  this XenoNet as one. XenoNet reaches them with an ssh config of its own.
- **The map** (`src/hosts.js`, `src/hosts-draw.js`) draws each host as a
  constellation on an outer orbit, bridges checkouts of one repo, and has
  a chip per machine. What's drawn of the hosts you're not looking at is
  capped to a frame budget, which the benchmark's `hosts` part measures:
  every host node, and at most 24 of the hosts' other nodes, what waits
  on you first, within 2 ms median and 3 ms p95 a frame with two
  demo-sized hosts.
- **The doctor** has a check per host, and beside them one for two hosts
  that share a key (`core/hosts/doctor.js`).
- **The VM lab** (`npm run lab`, `vm-lab.md`, `scripts/lab/README.md`)
  has two Linux VMs to try it in: its Phase A.
- **Run against the lab** in Wave 3: the relay over real SSH on both VMs,
  through the lab's ssh config and its console key restricted to the
  relay, with the privacy checks; and `npm run lab -- check`, nine checks
  in each VM, eighteen in all (the XenoNets answer, the Mac's loopback is
  blocked, the internet isn't, XenoNet listens on 127.0.0.1 only, nothing is
  mounted from the Mac, the console key gets the relay and not a shell,
  only made-up names, and root needs the VM's password).
- **Adding hosts to a console is yours.** XenoNet refuses `xenonet hosts add`
  and `remove` from under an agent, so the owner adds the lab's VMs to a
  console; an agent checking the page uses XenoNet Dev's `host-add`
  simulation, a second XenoNet on the same machine, in memory only.

Not built yet: a tailnet listener or mutual TLS, anything written across
machines, the merged repo view and collisions across machines, Windows
hosts, a macOS host in the lab (its Phase B), and streams that resume.
The sections headed "As built", near the end, say where the build differs
from the design that follows.

## Where XenoNet starts from

What the code does today, which every option has to work with:

- **One server per machine** (`server/index.js`), on 127.0.0.1 only:
  - It answers only `Host: 127.0.0.1:<port>` or `localhost:<port>` (421
    otherwise, against DNS rebinding).
  - It refuses requests other than GET that carry a foreign `Origin`.
  - GET routes have no other check: any program on the machine can read the
    map, the live state and transcripts.
- **What the page reads:** `GET /api/graph`, `GET /api/live` and one SSE
  stream, `/api/events`. The stream's events are `graph`, `live`,
  `activity`, `tokens`, `pulse`, `agent-message`, `doctor` and `theme`.
  They carry no `id`, so a dropped stream can't resume, and the page fetches
  everything again.
- **Node ids:**
  - `core` is the projects folder, and the center of the map.
  - Places: `project:<path key>`, `zone:<path key or name>` and
    `native:codex:<id>`.
  - Sessions: `claude:<uuid>` and `codex:<uuid>`.
  - Shared things: `resource:<server>` and `wiki:<slug>`.

  Path keys are lowercased absolute paths, so the same folder on two
  machines gets the same id. Session ids are UUIDs and don't collide.
- **What only makes sense on its own machine:**
  - absolute paths, and `~`, which the page expands with the one home
    folder that `core.home` gives it;
  - the sign-in timelines that decide which account a session ran as
    (`identities.signins` in `state.json`);
  - the layout, dismissed alerts and git status;
  - the engine's process checks: `core/engine/verify.js` looks up the
    process on the other end of a socket, which can't be done for a socket
    from another machine;
  - every action that opens something: `claude://`, `codex://`, VS Code,
    the terminal.
- **The map** is a radial tree rooted at `core`: `3d-force-graph` with
  `dagMode("radialout")`, rings 72 units apart.
- **Matching by remote already exists.** `buildGraph` places a session
  outside the projects folder by its git remote (`normalizeRemote`), and
  `projectRelations` finds local projects that share a remote.

### Sizes

Measured on made-up data only: a scratch script over the demo world (40
seconds of its driver) and the benchmark's synthetic graphs, in one run on
the development Mac.

| What | Raw | gzip |
|---|---|---|
| Demo world graph (33 nodes) | 41 KB | 4.6 KB |
| Demo live snapshot (14 sessions) | 13.7 KB | 2.9 KB |
| Demo SSE stream, per second | 19 KB | 1.3 KB (compressed as one stream) |
| Graph of 50 projects and 2,700 sessions over 90 days | 1.44 MB | 108 KB |
| The same, unarchived sessions of the last 7 days only (218 nodes) | 122 KB | 9.8 KB |
| Graph of 400 projects and 4,000 sessions (the bench's "large") | 2.37 MB | 188 KB |

- `live` events dominate the stream: 37 in 40 seconds, 12.7 KB/s raw. The
  server sends a full live snapshot at most every 500 ms while anything
  happens, and every 2 seconds regardless.
- `graph` events came about every 7 seconds, and each changed 3 to 12
  nodes.
- Parsing the large graph takes 3.8 ms, and qualifying every id in it
  0.5 ms (medians of 20 runs). Merging is cheap; what limits a merged map
  is how many nodes the page can draw.

## The options

### 1. Several roots side by side

One machine reads more than one projects folder, such as `~/Projects`,
`~/Work` and `~/src`.

- **On the map.** With one folder, nothing changes. With more, `core`
  becomes "this machine", and each folder is a node on the first ring with
  its projects and groups around it. Sessions outside every folder become
  zones, as today. The picker the root lane is building gains "Add a
  projects folder" and "Remove".
- **Data model.** A new node kind, `root:<path key>`, and
  `workspace.roots` in `state.json` in place of the single
  `workspace.root`. Project ids are absolute path keys already, so nothing
  collides. Layout keys, pins, identities and the engine don't change.
  Codex's picks are already kept per root (contracts section 5).
- **Transport and security.** None: it's one machine.
- **Identities, and folders that go away.** Identities don't change. A
  folder on an unmounted drive shows as missing until it's back.
- **Reuse, bandwidth.** The scan, your rules, the git loop and the root
  lane's picker. No network at all.
- **Does it generalize to hosts?** Only as a picture. The data would have
  to come through network mounts of each machine (SMB, NFS, or sshfs, which
  needs macFUSE on a Mac), and that breaks XenoNet in three places:
  - Watching files on network file systems "can be unreliable, and in some
    cases impossible" ([Node's `fs.watch` caveats](https://nodejs.org/docs/latest-v24.x/api/fs.html#availability)),
    so the tailer would fall back to polling.
  - `git status` every 20 seconds over a network mount is slow.
  - Transcripts aren't in the projects folder at all. They're in each
    machine's `~/.claude` and `~/.codex`, and the Agent SDK and
    `codex app-server` each read one configured home at a time.

  Sign-in timelines, the process checks and the engine stay out of reach
  either way.
- **Verdict.** Worth doing for one machine once the root lane has landed,
  and its map treatment (folders as constellations around one center) is
  what hosts need too. It isn't a way to see other machines.

### 2. One virtual root merged by git remote

Every machine's projects are matched by their remote,
`repo:<normalized remote>`, so the same repo on three machines is one node,
and each session carries the host it ran on.

- **On the map.**
  - One center, the workspace, and one constellation per repo, whatever
    machine its sessions ran on. Each session wears its host's color or
    glyph.
  - Selecting `xenonet` gathers its sessions from every machine.
  - Two sessions on different machines editing the same file of the same
    repo get the collision arc they'd get on one machine: a merge conflict
    to come.
  - Everything without a remote stays per machine, on small per-host nodes
    beside the merged repos: local-only repos, groups (a folder of repos is
    a folder on one disk), zones, scratch folders, Codex desktop projects,
    the shared context and MCP resources.
- **Data model.**
  - `repo:<remote>` nodes, each with a list of checkouts: host, path,
    branch, dirty count, ahead and behind, worktrees, git identity.
    Everything else is qualified as `@<host>:<id>`.
  - Layout keys for repos have no host, so a repo keeps its place whichever
    machines have it.
  - Identities: a repo's owner is shared, but "pushes as" and "commits as"
    belong to each checkout and can differ by machine.
  - The engine gains the most: "everyone in this repo" spans machines. That
    means messages and claims crossing machines, which is exactly the write
    path the process checks can't vouch for.
- **Transport.** It needs one, in practice option 3's. Merging is a view of
  the data, not a way to get it.
- **Hard parts.**
  - The repo key has to come out the same on every machine, and
    `normalizeRemote` doesn't resolve SSH host aliases. `git@gh-work:me/app`
    becomes `gh-work/me/app`, while a machine that uses
    `git@github.com:me/app` gets `github.com/me/app`. Forms like
    `ssh://git@github.com:22/me/app` keep their port. Only the machine whose
    `~/.ssh/config` defines the alias can resolve it (`parseSshAliases` in
    `core/identities.js` already reads it).
  - Every action needs "on which machine?", and per-checkout state needs an
    inspector of its own.
  - It hides where work happens, which is the first thing you want to know
    about a VM or a test box.
- **A host that goes offline.** It takes some sessions out of a merged
  node, and that's awkward to show per host inside one node.
- **Verdict.** Valuable as a view, but too big and too lossy as the map.
  Three parts are worth keeping:
  - the repo key, computed on each host with its aliases resolved;
  - collisions across machines;
  - gathering a repo's sessions from every machine.

### 3. A hub that federates headless XenoNets

Each machine runs XenoNet's server, with or without a window. The console's
XenoNet subscribes to the hosts you add: it pulls a snapshot from each, then
follows its events, and merges them into its own map.

- **On the map.**
  - This machine stays at the center, exactly as today.
  - Each host is a node on the first ring, with its id and its state
    ("vm1 · live", "studio · last seen 14:02"), and its own projects,
    groups, zones and sessions around it.
  - Remote alerts join the alert strip with their host's name
    ("vm1 · Fix the parser needs you"), and the tray and menu bar count
    them.
  - Selecting a remote project gathers its sessions, as a local one does.
- **Data model.** Remote ids are qualified as contracts section 2 says;
  local ids, layout keys and pins don't change. The details are under
  [the recommendation](#ids-and-the-graph).
- **Headless.** `node server/index.js` already runs without Electron (the
  server tests start it that way). A headless host needs nothing more than
  a way to start at login (launchd or systemd), or the relay's `--start`
  (below).
- **Verdict.** The architecture to build:
  - a host's server doesn't change; the relay is a small command beside it;
  - the console does the merging;
  - each machine stays the only reader of its own files.

  How the console reaches a host is the sub-question this option turns on.

#### How a console can reach a host

| Transport | Authentication | New listener | Host header | Notes |
|---|---|---|---|---|
| **SSH running a command**: `ssh host xenonet relay`, frames over stdio | SSH keys and `known_hosts`, all handled by `ssh` | None | The relay is a local client on the host: `127.0.0.1:<port>`, as today | Works where the SSH server forbids forwarding. Zed's remote development also runs over `ssh` connections of its own |
| SSH port forwarding: `-L` to a port or a Unix socket, or `-W` over stdio | The same | `-L` to a port: any local process can use it. To a Unix socket: guarded by `StreamLocalBindMask`. `-W`: none | The console must send the host's `127.0.0.1:<port>` itself, or the host answers 421 | Needs forwarding allowed by the host's SSH server |
| Tailscale: `tailscale serve`, or a listener on the tailnet address | Tailnet identity (Serve's `Tailscale-User-Login` header, or `tailscale whois` on the peer) plus a pairing token | Yes, on the tailnet | Arrives under the tailnet name or address, not 127.0.0.1 (Serve's docs don't say whether it passes `Host` through), so it needs a route set with its own allowlist | A new tailnet allows all traffic between its devices, so any of them could reach the port |
| Mutual TLS with pinned self-signed certificates, Syncthing's model | Certificate fingerprints, exchanged once | Yes, on the LAN | As for Tailscale | XenoNet would keep a private key. No way through NAT. macOS firewall prompt |
| Plain HTTP with a pairing token | The token | Yes | — | Readable by anyone on the network: ruled out |

The sources for each row are under [Security](#security).

### Considered and set aside

- **Network mounts of each host.** Option 1's only route to other machines;
  see above.
- **Syncing through shared storage.** Each host writes its snapshot to
  iCloud Drive, Dropbox or a git repo, and the console reads it there. It
  needs no listener, but it's seconds to minutes late, it conflicts, and a
  third party keeps a copy of what your agents do.
- **One central XenoNet that every agent uses over the network.** The agents
  on each machine would call a remote `xenonet` MCP server. But the process
  checks can't see across machines, transcripts still need a reader on
  each machine, and every machine's engine would stop when the console
  does.

## Comparison

| | 1. Several roots | 2. Merged by remote | 3. Hub of headless XenoNets | **Recommended: 3, with host constellations and repo bridges** |
|---|---|---|---|---|
| Sees other machines | No; only through network mounts, which break watching | Yes, through 3's transport | Yes | Yes |
| The map | One center, a constellation per folder | A constellation per repo across machines, plus per-host leftovers | This machine at the center, a constellation per host | 3, plus dashed bridges between checkouts of one repo |
| New ids | `root:<path key>` | `repo:<remote>`, and `@host:` for the rest | `@host:` for everything remote | 3's, with the repo key as a field |
| Layout and pins | Unchanged | Shared by repo; `@host:` for the rest | Remote positions under `@host:` ids, on the console | As 3 |
| Identities | Unchanged | Merged by id; timelines stay per host | Merged by id; timelines stay per host | As 3 |
| Engine | Unchanged | Repos span machines: writes across machines | Read-only display at first | As 3 |
| Transport | None | As 3 | SSH running the relay | SSH running the relay |
| New listener | None | None | None | None |
| A host offline | A missing folder | Sessions drop out of merged nodes | The host dims and holds its last snapshot | As 3 |
| Reuses | The root lane's scan and picker | Graph merging, plus much new UI | SSE events, GET routes, snapshot shapes, demo worlds for tests | As 3 |
| Size of the work | Small, after the root lane | Large | Medium | Medium; bridges add little |
| Main risk | None for one machine | Wrong merges; actions that need a machine | The map's size; needs an SSH server on hosts | As 3 |

## Recommendation

Option 3 over SSH, with each host as its own constellation and bridges
between checkouts of the same repo.

### Why

1. **It reaches other machines without changing how XenoNet reads one.**
   Each host's own XenoNet stays the only reader of its files, transcripts,
   sign-ins and process table, so every feature keeps working there, and
   the console only merges.
2. **Nothing new listens anywhere.**
   - Every API stays on 127.0.0.1 with its Host and Origin checks.
   - XenoNet keeps no token or key: SSH does authentication, host
     verification and encryption.
   - A key line in the host's `authorized_keys` can limit the console's key
     to running the relay.
3. **It reuses what's there.** The GET routes, the SSE events and the
   snapshot shapes travel as they are. A second demo world (one per port)
   stands in for a second machine in tests, with no VM at all.
4. **One machine works exactly as today, and a host's XenoNet serves the
   relay like any local client.** A console going away costs a host
   nothing.
5. **It's the README's peer-to-peer plan in its simplest form.** Any XenoNet
   can watch any other you can SSH into, in both directions, with no server
   in the middle.
6. **It leaves room for the rest.** Repo bridges come now. A merged repo
   view, collisions and messages across machines, and a tailnet listener
   for machines without an SSH server can come later.

What it costs:

- **Hosts need an SSH server you can reach.**
  - Linux usually has one.
  - A Mac needs Remote Login (System Settings › General › Sharing), which
    Apple notes "can make it less secure"
    ([Apple](https://support.apple.com/guide/mac-help/allow-a-remote-computer-to-access-your-mac-mchlp1066/mac)).
  - Windows needs the OpenSSH Server optional feature
    ([Microsoft](https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_install_firstuse)).
  - Tailscale SSH can stand in on Linux, and on a Mac running Tailscale's
    open-source variant
    ([Tailscale](https://tailscale.com/kb/1193/tailscale-ssh)).
- **Hosts need the `xenonet` command**, and Node 22.13 or later or the app's
  runtime.
- **The map grows with each host.** Remote sessions are limited to recent
  ones for now.

### The design

#### Topology

- **A host** is any machine running XenoNet, with a window or without.
- **A console** is the XenoNet whose window you're looking at. It holds a
  list of hosts and runs one link to each: a single `ssh` process.
- **Data goes one way, host to console;** the console only asks to read.
  A console never passes on what it receives, so two machines can watch
  each other without a loop, and trust doesn't pass from one machine to
  the next.
- **Hosts never depend on a console.** If the console goes away, nothing
  changes on the host.

#### The relay

The console starts, for each host:

```sh
ssh -T -a -x -o BatchMode=yes -o StrictHostKeyChecking=yes \
    -o ClearAllForwardings=yes -o ConnectTimeout=10 \
    -o ServerAliveInterval=15 -o ServerAliveCountMax=3 \
    [-C] -- vm1 xenonet relay --port 47840 --since 7d
```

- **Where it connects.** `xenonet relay` finds the XenoNet on its own machine
  (`core/client.js`, as `xenonet agents` does) and talks to it at
  `127.0.0.1:<port>`.
- **What it writes.** One JSON object per line on standard output:

```text
← {"t":"hello","schema":"host/hello/v1","protocol":1,"version":"0.4.0",
   "host":{"id":"vm1","key":"7c1e…"},"platform":"linux","demo":false,"now":1791234567000}
← {"t":"snapshot","graph":{…},"live":{…},"identities":{…},"engine":{…}}
← {"t":"event","name":"live","data":{…}}     graph, live, activity, tokens, pulse, agent-message
→ {"t":"get","id":7,"path":"/api/session/claude:4f1c…/transcript?limit=120"}
← {"t":"res","id":7,"status":200,"body":{…}}
← {"t":"ping","now":1791234582000}           every 15 seconds when nothing else is sent
```

- **Reads on demand.** `get` frames read through the relay, and only from
  an allowlist: a session's transcript and stats, history, collision
  diffs, the engine's state, identities, and the graph and live state
  themselves. The relay never forwards POST or DELETE, and anything else
  gets a 403 from the relay itself.
- **Under backpressure** it keeps only the newest `graph` and `live`
  frames, since each supersedes the last. After a gap it sends a fresh
  snapshot.
- **What it filters.** `--since 7d` drops archived sessions and sessions
  not updated in the last 7 days, before sending; all projects stay.
  `--metadata` drops conversation text (below).
- **`--start`.** If no XenoNet answers on the host, the relay starts one
  (`node server/index.js`) and stops it when it exits. That suits a VM with
  no service set up.
- **Why not port forwarding.** The relay only asks SSH to run a command,
  the most basic thing SSH does. Port forwarding can be turned off on the
  server: `AllowTcpForwarding` and `DisableForwarding` in
  [sshd_config](https://man.openbsd.org/sshd_config), or the policy flag
  `AllowLocalPortForwarding` for
  [Tailscale SSH](https://pkg.go.dev/tailscale.com/tailcfg#SSHAction),
  which [a 2022 forum thread](https://forum.tailscale.com/t/does-port-forwarding-work-with-tailscale-ssh/2664)
  shows refusing it. Nothing in the relay depends on it.
- **Framing.** One JSON object per line, the same as
  [MCP's stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).
  [Zed](https://zed.dev/docs/remote-development) runs its remote protocol
  over `ssh` connections it starts itself (sharing one with
  `ControlMaster`) rather than over forwarded ports, and
  [VS Code's Remote-SSH](https://code.visualstudio.com/docs/remote/ssh)
  also puts a server of its own on the remote machine.

#### Ids and the graph

The console merges each host's view into its own. The relay sends what the
host's `/api/graph` returns, filtered by `--since` and with paths as `~`.

- **Qualified ids.** Every remote id becomes `@<host>:<local id>`, as
  contracts section 2 says: `@vm1:claude:4f1c…`,
  `@vm1:project:~/projects/atlas`. The console qualifies them itself, and
  drops any id that already starts with `@`, so a host can't pass on
  another host's nodes or pose as this machine.
- **The host node.** The host's `core` becomes the host node: id
  `@vm1:core`, kind `host`. It carries the host id, the state, when the
  console last heard from it, the clock offset, the version and the
  platform, and whether the host is a demo. A `host` link joins it to the
  console's `core`, so the radial tree stays one tree.
- **Repo key.** Each project, local or remote, carries
  `repoKey: "github.com/willctl/xenonet"`: its remote, with no prefix. A
  merged view's id for the repo would be `repo:<repoKey>`,
  `repo:github.com/willctl/xenonet`. Its host computes the key, with its own
  SSH aliases resolved (contract note 1).
- **Bridges.** A `same-repo` relation joins the checkouts of one repo on
  different machines. It sits beside `wikilink` and `mentions` in
  `graph.relations`.
- **What the console drops from remote nodes.**
  - The host's own layout (`fx`, `fy`, `fz`, `pinned`): the console applies
    its own `state.json` layout under the qualified ids.
  - Every action that would open something on this machine. Remote
    sessions keep Preview and Stats, read through the relay.
- **Remote state, marked.** Remote nodes carry `host`, and `stale` when
  their host isn't live. `graph.hosts` lists the hosts and their states.
- **Live state.**
  - Remote sessions are qualified.
  - The token series is summed after shifting it by the clock offset.
    Tokens per minute and today's tokens add up.
  - Plan limits merge per account (below).
  - Collisions are qualified per host; collisions across machines come
    later.
- **Events.** Each SSE event from a host is passed on with its ids
  qualified: `activity.sessionId`, `pulse.nodeId`, `tokens.sessionId`, and
  `agent-message` `from` and `to`. A host's `doctor` results feed its state
  in `/api/hosts`, not the console's doctor strip.
- **Yours, kept on the console.** Dismissed alerts, identity pins and
  layout positions for remote nodes live in the console's `state.json`,
  under the qualified ids. These maps take any key, so they need no new
  shape.

#### The map

- **Hosts on the first ring.** This machine is the center as today, and
  once a host is added, its label gains this machine's host id ("studio ·
  ~/Projects"). Each host's node sits on the first ring, labelled `vm1`,
  with a ring showing its state.
- **Bridges.** A dashed `same-repo` line joins checkouts of the same repo.
- **Paths** under the host's home show as `~`, as local ones do (see
  [Security](#security)).
- **The inspector** says where a node lives: "on vm1 · live", or "on vm1 ·
  last seen 14:02".
- **Remote alerts** join the strip, the tray and the menu bar count, with
  the host's name. They chime like local ones while the host is live.
- **Filters.** A chip per host, like the identity chips, shows one
  machine's work.
- **Layout.** In the radial tree, hosts share the center's rings: a host's
  projects sit on the ring of this machine's sessions. Making each host a
  small radial system of its own, on an outer orbit, would read better.
  `3d-force-graph`'s `dagNodeFilter` can take a host's subtree out of the
  radial layout and let a force of XenoNet's own place it. That's UI work
  for after the prototype.

#### Identities across hosts

- **Accounts merge by id.** Account ids are global already (contracts
  section 2): `claude:<account>:<org>`, `chatgpt:<workspace>`,
  `github:<host>/<login>`, `git:<hash of the email>`. The console unions
  each host's `graph.identities` by id:
  - session and project counts add up;
  - `signedIn` becomes the list of machines it's signed in on ("here and on
    vm1");
  - a handle comes from whichever machine knows it.
- **`openai:api-key` isn't an account.** Two machines with different API
  keys would merge into one identity, so the console qualifies it:
  `@vm1:openai:api-key`.
- **Which account a session ran as is decided on its host.** It depends on
  that machine's sign-in timeline, so the console takes each remote
  session's `ranAs` as its host worked it out, and never applies its own
  timeline to it.
- **Yours stays on the console.**
  - Names and hidden identities apply to every host's records.
  - Pins on remote sessions and projects are kept under their qualified
    ids.
  - `Hide or show identities` masks remote identities as it masks local
    ones.
  - A name given on a host fills in where the console has none.
- **Plan limits are per account, not per machine.** Claude's limits are
  "shared across Claude and Claude Code"
  ([Anthropic](https://support.claude.com/en/articles/11145838)), and
  Codex's "local messages and cloud chats share your plan's usage
  allowance" ([OpenAI](https://learn.chatgpt.com/docs/pricing)). So one
  account signed in on three machines has one limit, read three times.
  - Today the live snapshot's `limits` don't say which account they belong
    to, so each reading should carry its identity (contract note 4).
  - The console keeps the freshest reading per account, and shows accounts
    signed in only elsewhere as bars of their own.
- **Flags come through as they are.** A push that signs in as one of your
  accounts into a repo another owns is flagged on its host, from that
  host's gh and git config.
- **The person.** `actor:you` on a host is the same person, so the console
  should treat `@vm1:actor:you` as `actor:you` (question 6).

#### The engine across hosts

- **Read-only in the prototype.** A host's `/api/engine/state` (its
  agents, what each says it's doing, claims, the last day's messages)
  comes with its snapshot, and `agent-message` events follow. The console
  shows them in that host's constellation, under qualified ids.
- **Remote ids can't act locally.** `isAgentId` doesn't match a qualified
  id, so a remote agent can't act as a local one.
- **Later, awareness.** Hosts exchange who edits which file, and what's
  claimed, as a repo key and a path inside the repo. The edit warning then
  gains "an agent on vm1 is changing the same file in its clone: expect a
  merge conflict", like today's worktree warning.
- **Later, messages across machines.** Routed by the console, framed as
  coming from an agent on another machine, rate-limited, opted into per
  pair of machines, and still unable to approve anything.
- **Both are writes** into a machine whose process checks can't see the
  sender. They're information, never authority.

#### Hosts that go offline

- **States.**

  | State | When |
  |---|---|
  | `connecting` | The link is starting |
  | `live` | Frames are arriving |
  | `stale` | No frame for 45 seconds: three missed pings |
  | `offline` | The connection ended. The console reconnects with backoff, from 2 to 60 seconds, with jitter |
  | `error` | Needs you: the host key, a refused key, no relay, another protocol |

- **Cache.** The console keeps each host's last snapshot (graph, live
  state, identities, the engine's summary) in its cache folder, by host
  key, written at most every 30 seconds. Offline hosts are still on the map
  after a restart. Transcripts and stats are never cached.
- **What the map shows.** The host node greys and says since when. Its
  constellation stays in place, dimmed and still: no pulses, no beams.
  - Each session's activity is worked out again by the console's clock from
    its last update, so nothing looks busy forever: active, then recent
    after 10 minutes, then idle after a day.
  - A question that was waiting stays in the strip as "as of 14:02, vm1
    offline", without a chime and outside the tray count.
  - Collisions that involve the host are dropped.
- **Back online.** A fresh snapshot replaces the cached one in one step,
  and only the sessions that changed meanwhile pulse.
- **Forgetting a host.** `xenonet hosts remove` deletes its cache, and asks
  before removing its layout positions and pins. A host that comes back
  with another key keeps one snapshot, under the new key.
- **Clocks.** The hello carries the host's clock. The console takes the
  offset as the host's time minus the midpoint of the round trip, and
  shifts the host's timestamps by it. The host node shows an offset over a
  few seconds, and the doctor warns over two minutes.

#### Bandwidth and performance

- **Per host, the stream is about what a page gets locally.** For the demo
  that's 19 KB/s raw. `ssh -C` compresses the whole channel with zlib
  ([ssh](https://man.openbsd.org/ssh)), which brings it to about 1.3 KB/s.
- **Old sessions drive graph size.** The relay's `--since 7d` took the
  2,700-session graph from 1.44 MB to 122 KB, or 9.8 KB compressed.
- **On the console,** the merged graph goes out at most once a second, and
  merged live state at most every 500 ms, as now. The page's load grows
  with the sessions shown, not with the number of hosts.
- **Caps.** At most 5,000 nodes and 16 MB per frame from any host.
- **Later, if needed:**
  - Node-level changes in place of whole `graph` events: 3 to 12 nodes
    change per event in the demo.
  - Event ids, so a short drop resumes rather than resending a snapshot
    ([`Last-Event-ID`](https://html.spec.whatwg.org/multipage/server-sent-events.html),
    and MCP's rules for
    [resuming streams](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)).

#### Security

**The console**

- **Adding a host is yours to do.** `POST /api/hosts` is checked like
  connecting your agents: refused from under any agent (`verify.person`).
  As with connecting, that check works on macOS only for now.
- **The SSH target is validated.** It must match
  `^[A-Za-z0-9][A-Za-z0-9._@-]{0,252}$`, and `--` comes before it. A
  target starting with `-` would be read by `ssh` as an option, such as
  `-oProxyCommand=…`, which runs a command.
- **The remote command is built from fixed words and checked values only:**
  `xenonet` or an absolute path, and digits for the port. The host's shell
  parses that line.
- **SSH options.**
  - `BatchMode=yes` never prompts
    ([ssh_config](https://man.openbsd.org/ssh_config)), and
    `StrictHostKeyChecking=yes` makes an unknown or changed host key fail
    even if your own config would accept new ones.
  - `-T` asks for no terminal; `-a` and `-x` turn off agent and X11
    forwarding.
  - `ClearAllForwardings=yes` keeps forwards from your `~/.ssh/config` off
    this connection.
  - The first connection to a new machine is yours to make, once, in a
    terminal (`ssh vm1`), so you see its host key.
- **What hosts send is untrusted input.**
  - Frames are capped in size and parsed leniently against the schemas.
  - The console qualifies every id, and drops ids that start with `@`.
  - The console keeps only the fields the schemas declare, so a host
    can't add one the page doesn't expect (see Wave 3's notes below).
  - The page escapes remote text as it escapes transcripts.
  - Remote text never reaches an agent or a prompt: the engine doesn't
    relay it, and digests and Codex summaries stay local in the prototype.
- **Local programs on the console** can read the hosts' data through its
  API, as they can read this machine's today.
- **macOS 15 and later.** An app's child processes count as the app for
  local network privacy. So XenoNet's first SSH connection to a machine on
  the local network can bring up the "find devices on your local network"
  prompt for XenoNet. Addresses on a VPN, such as Tailscale's, don't count
  ([TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)).

**The host**

- **The relay is a local client of its own XenoNet**, like `xenonet agents`,
  so the Host and Origin checks hold as they are.
- **The relay forwards GETs from its allowlist only.**
- **It runs as you, under the SSH server.** The host's process checks see
  a process that isn't an agent, so even a request in an agent's name would
  be refused. The routes only you may use (connecting agents) would take it
  for you, which is why the relay forwards no writes at all.
- **No home folder leaves the machine.** The relay runs every payload
  through `tilde()` from `core/paths.js` before it goes, the same function
  that keeps your user name out of activity details. Paths, and the path
  keys inside ids, reach the console as `~/…` (contract note 2). A folder
  named for a person outside the home folder, such as a projects folder
  on Windows's side of WSL, goes with `~` for the name:
  `/mnt/c/Users/~/Projects`.
- **Least privilege, if you want it.** Give the console a key of its own,
  installed in the host's `~/.ssh/authorized_keys` as:

  ```text
  restrict,command="xenonet relay --port 47840" ssh-ed25519 AAAA… console
  ```

  That key can run the relay and nothing else: `restrict` turns off
  forwarding, the terminal and `~/.ssh/rc`, and `command` replaces
  whatever the client asks to run
  ([sshd](https://man.openbsd.org/sshd#AUTHORIZED_KEYS_FILE_FORMAT)).
  The client's own command is still in `SSH_ORIGINAL_COMMAND`, so the
  relay can take options it knows (`--since`, `--metadata`) from there.
  Write `xenonet` as an absolute path where it isn't on the PATH of commands
  run over SSH.
  Turning off forwarding alone "does not improve security unless users are
  also denied shell access"
  ([sshd_config](https://man.openbsd.org/sshd_config)).

**Other transports, if they come later**

- **A tailnet listener.**
  - Bind to the tailnet address only, never `0.0.0.0`, and serve only the
    host routes.
  - Require a pairing token: shown once and stored hashed, as actor
    registration does (contracts section 3).
  - Check the peer with `tailscale whois`, which returns the peer's
    machine and user ([Tailscale CLI](https://tailscale.com/kb/1080/cli)).
  - Accept only the MagicDNS name as Host, and refuse any request with an
    `Origin`: no browser has a reason to call it.
  - Leave the main API's checks as they are.

  Tailscale Serve strips incoming `Tailscale-User-*` headers to stop
  spoofing ([Tailscale](https://tailscale.com/kb/1312/serve)), but a
  program on the host can still send them straight to 127.0.0.1, so the
  token is what counts. A new tailnet allows all traffic between its
  devices ([Tailscale](https://tailscale.com/kb/1192/acl-samples)).
  [Grants](https://tailscale.com/kb/1324/grants) can narrow that to one
  port.
- **Mutual TLS.** Self-signed certificates, identified by a hash of the
  certificate as [Syncthing's device IDs](https://docs.syncthing.net/dev/device-ids.html)
  are. Node's TLS server can require and check client certificates
  (`requestCert`, `rejectUnauthorized`,
  [Node](https://nodejs.org/docs/latest-v24.x/api/tls.html)). XenoNet would
  then keep a private key in its state folder.
- **SSH port forwarding.** Send `Host: 127.0.0.1:<port>` explicitly
  (Node's `http.request` takes `headers` and `setHost`, and
  `createConnection` takes any stream,
  [Node](https://nodejs.org/docs/latest-v24.x/api/http.html#httprequestoptions-callback)).
  Forward to a Unix socket rather than a port, so other users can't use it.
- **Browsers.** Chrome 142's Local Network Access asks before a public
  site reaches loopback or the local network
  ([Chrome](https://developer.chrome.com/blog/local-network-access)). That
  helps, but XenoNet keeps its own Host and Origin checks, as MCP's
  specification asks of local HTTP servers
  ([MCP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)).

#### What crosses machines

| Data | Crosses by default | Notes |
|---|---|---|
| Projects, sessions and their state: names, titles, branches, counts | Yes | Paths as `~` |
| Live detail: a waiting question's text, the last message (280 characters), stuck reasons, files touched | Yes, unless the host's link says `--metadata` | The default is question 3 |
| Activity events and token counts | Yes | |
| Identities: account ids, handles, plans | Yes, unless identities are off on the host | Masked on the console like local ones |
| Messages between agents | Yes | As the console shows them today |
| Transcripts, stats, collision diffs | Only when you open one | Never cached |
| Tokens, keys, credentials | Never | XenoNet doesn't read them |

#### What's hard

- **The layout and the map's size.** Several large hosts pass the page's
  frame budget before the network matters. `--since` and the node cap hold
  it for now; host systems on an outer orbit come later.
- **Hosts need an SSH server and XenoNet installed.**
- **Windows hosts:**
  - drive letters and backslashes, so `tilde()` on JSON text must also
    match the escaped form (`C:\\Users\\…`);
  - default shells that don't expand `$HOME`.
- **Versions.** A console and its hosts can run different XenoNets. The
  hello carries a protocol number, and the console refuses one it doesn't
  know, naming the host's version.
- **Actions on remote nodes.** Opening `claude://` or VS Code on the
  console's machine would open the wrong thing, so the prototype offers
  only Preview and Stats. Later, a terminal tab could run `ssh -t vm1` in
  the project's folder, and VS Code's Remote-SSH can open a folder on a
  host; its URL form is documented only by third parties, so check it
  before using it.
- **Writes across machines.** Dismissing alerts, messages, claims and
  opening sessions all need a trust model the process checks can't give.
  Read-only comes first.

### Contract notes

Changes to `contracts.md` this design needs, for the orchestrator's OK:

1. **Repo keys are computed on the host.** First the host's SSH aliases
   are resolved (`parseRemote` with `parseSshAliases`), and `ssh://`, ports
   and user names are dropped; then `normalizeRemote` runs. The key travels
   as `repoKey` on project nodes, and `repo:` stays reserved for a merged
   view.
2. **Remote path keys are tilded on the host.** That gives
   `@vm1:project:~/projects/atlas`, not section 2's
   `@vm1:project:/users/will/projects/atlas`, so no home folder (a user
   name) crosses machines or lands in the console's layout keys. Local ids
   stay as they are.
3. **`openai:api-key` belongs to its host** and is qualified.
4. **Plan limits in the live snapshot name the identity they belong to.**
5. **The `host` schema** that section 1 foresees covers `hosts.json`
   (`host/list/v1`) and the relay's hello (`host/hello/v1`).
6. **Host ids.** A host names itself (`xenonet hosts self <id>`). A console
   uses that name unless it clashes, and may rename the host for itself.
   The host key, random and made once, recognizes a host across renames
   and different SSH aliases. Neither is ever taken from the machine's
   name.

## Prototype plan for Wave 2

One lane, `w2/hosts`, builds the recommendation's read-only core: hosts on
the map over SSH, with nothing written across machines.

### What it builds

| Part | Files | What |
|---|---|---|
| The relay | `cli/commands/relay.js`, `core/hosts/relay.js` | `xenonet relay [--port N] [--since 7d] [--metadata] [--start]`, as above |
| Host links | `core/hosts/link.js` | One per host. Starts the transport: `ssh`, or `local`, which runs `node cli/xenonet.js relay --port N` against another XenoNet on the same machine, for tests and for watching XenoNet from XenoNet Dev. Reads frames; keeps the state, backoff and clock offset; sends `get`s with timeouts |
| Merging | `core/hosts/merge.js` | Pure functions: qualify a host's graph, live state, identities and engine snapshot; host nodes and links; `same-repo` relations; activity from the clock offset; actions cut to Preview and Stats; the console's layout applied |
| Config and cache | `core/hosts/config.js`, `core/hosts/cache.js` | `hosts.json` in the state folder: this XenoNet's id and key, and its hosts. The id rules from contracts section 2, through `core/schema/ids.js` once the core lane lands it. The last snapshot per host in the cache folder |
| API | `server/routes/hosts.js`, mounted from `server/index.js` | `GET /api/hosts`; `POST /api/hosts` and `DELETE /api/hosts/:id`, yours only; `POST /api/hosts/:id/reconnect`. `/api/graph`, `/api/live` and `/api/events` serve the merged view. Transcripts and stats of `@` sessions go through the relay; every other route refuses `@` ids |
| Command | `cli/commands/hosts.js` | `xenonet hosts`, `xenonet hosts add <id> <ssh target> [--port] [--command]`, `xenonet hosts remove <id>`, `xenonet hosts self <id>` |
| Page | `src/hosts.js`, with small hooks in `src/nodes.js`, `src/scene.js` and `src/hud.js` | The `host` node with its label and state ring; dimmed, still nodes for stale and offline hosts; `same-repo` as a dashed line; "on vm1 · …" in the inspector; remote paths as `~` |
| Driving | `src/driver.js`, `mcp/console.js`, `core/script.js` | `hosts()` in the driver, a `hosts` console tool, `find` matching host nodes, and `simulate` kinds `host-offline`, `host-online` and `host-skew` |
| Doctor | `core/doctor/` | A check per host: reachable, relay found and speaking a known protocol, clock within two minutes |

Notes on the plan:

- `hosts.json` is a file of its own, beside `state.json`, so the lane
  doesn't touch `core/store.js`, which the core and root lanes share. It
  can move into `state.json` when the prototype graduates.
- Remote layout positions, pins and dismissed alerts use `state.json`'s
  existing maps, under qualified ids.
- The `local` transport runs only XenoNet's own relay; it can't run an
  arbitrary command.
- The page's part goes in a new module, with only registration lines in
  shared files, as contracts section 6 asks. That keeps it clear of the
  actors UI lane, which works in the same files in Wave 2.

### The flag

`XENONET_HOSTS=1`. Without it:

- the host routes answer 404, and no link starts;
- the merged view is the local view;
- the page never sees a host node.

With the flag off, `npm test` passes unchanged and the benchmark doesn't
move.

### Not in it

- The tailnet listener, mutual TLS, pairing codes.
- Anything written across machines: dismissed alerts, opens, handoffs,
  messages, claims, warnings across machines.
- The merged repo view, and collisions across machines.
- Search, replay, films, digests and Codex summaries across hosts.
- Node-level graph changes, and streams that resume.
- Windows hosts.

### Tests

**Unit tests**, in new files `test/hosts-*.test.js`:

- Ids: qualifying and parsing; the host id rules; ids that start with `@`
  dropped.
- Merging two made-up snapshots:
  - ids qualified, with the host node and its link;
  - a `same-repo` relation between two `atlas-api` checkouts;
  - the console's layout winning over the host's;
  - actions cut to Preview and Stats;
  - activity worked out with a 3-minute clock offset;
  - identities unioned, with `openai:api-key` qualified;
  - limits merged per account.
- Reading frames: lines split across chunks, a 20 MB frame, bad JSON,
  unknown frame types.
- The relay:
  - its allowlist refuses POST, `/api/dev/*`, and `..` or encoded paths;
  - `tilde()` covers a home path inside a string field.
- The link, against a fake relay script: no hello in time; exit, then
  offline, backoff and back; silence, then stale.
- The `ssh` arguments: target validation, `--`, the options, and no shell
  metacharacters in the remote command.
- The cache: written and read back.

**A server test**, `test/hosts-server.test.js`:

- It starts two demo servers on spare ports: two made-up worlds with the
  same repos.
- The console runs with `XENONET_HOSTS=1` and a `local` host pointing at the
  other.
- It checks that:
  - the merged graph has `@demo-b:core`, and the other world's projects
    and sessions under `@demo-b:` ids;
  - `same-repo` relations join the two `atlas-api`s;
  - remote live sessions arrive;
  - a remote transcript and stats come through the relay;
  - `POST /api/open` and `POST /api/ack` refuse remote ids;
  - after the other server is killed, the host is `offline` within 10
    seconds, its nodes kept and marked stale;
  - after a restart it's `live` again.

**An end-to-end test**, `test/e2e/hosts.test.js`, a new file the
orchestrator runs at merge point B. On the test instance, with a second
demo server as a host:

- the host node shows;
- a remote project gathers its sessions;
- the inspector says where they are;
- going offline dims the host, and coming back restores it;
- no page errors.

**Benchmarks.** `node scripts/bench.mjs graph refresh tail server` before
and after:

- with the flag off, nothing should move;
- with it on and one demo host, report the time to emit a merged graph.

### In the VM lab

Using the guests from the VM lab plan:

1. **Set up.** A Linux guest, reachable by SSH from the Mac, with Node
   22.13 or later and a copy of the branch, runs `node server/index.js
   --demo`: headless, with made-up data. The Mac's XenoNet Dev test instance,
   with `XENONET_HOSTS=1`, adds it with `xenonet hosts add vm1 <ssh alias>`.
2. **Connect and watch for ten minutes.** Events should arrive within a
   second. Measure bytes per second on the SSH channel, with `-C` and
   without.
3. **Suspend the guest.** The host goes stale, then offline, and dims.
   Resume it: back to live within a minute, with no duplicate nodes.
4. **Set the guest's clock three minutes ahead.** Activity and "last seen"
   stay right, and the doctor warns.
5. **A restricted key.** The console's key, installed with
   `restrict,command=`, still works; a shell with that key doesn't.
6. **Failures say what to do:** an unknown host key, a refused key,
   `xenonet` not found, an unknown protocol.
7. **Two guests at once,** if the lab has two.
8. **Optional, after your sign-ins:** the same over Tailscale (the MagicDNS
   name as the target, with OpenSSH or Tailscale SSH), and a macOS guest
   with real Claude Code and Codex sessions.

On the development Mac today:

- OpenSSH 10.2 (the client);
- Tailscale 1.98.8, installed (the Homebrew command and the app) but not
  running;
- UTM 4.7.5 with Ubuntu guests, and Lima 2.1.2 with no machines;
- Remote Login off, which is fine: the Mac is the console.

### Done when

- The unit and server tests pass.
- The end-to-end file is written.
- The benchmark doesn't move with the flag off.
- VM lab scenarios 1 to 6 pass, with their numbers.

### As built: the relay, the hub and xenonet hosts (Wave 2, step 1)

The first of the lane's two steps built the relay, `xenonet hosts` and the
hub; the page's part (host nodes on the map, the driver, the console tool
and the end-to-end test) is the second. Where it differs from the plan
above, or settles what the plan left open:

- **What crosses by default** follows merge point A: titles, states and
  counts. The relay's opt-in is `--detail`, in place of `--metadata`;
  `xenonet hosts add --detail` asks for it.
- **The protocol** adds two frames: `time` (a console asks, the relay
  answers with its clock, for the offset from the quickest round trip) and
  `bye` (why a relay stops: no XenoNet running there, the XenoNet stopped, or
  one too old to be a host). A host's XenoNet stopping ends the relay, so the
  console sees the host offline and connects again by itself.
- **Being a host needs no flag.** `GET /api/hosts/self` says who a XenoNet
  is as a host (its key is made the first time anything asks) and gives the
  relay the folder `~` stands for; `POST` names it, yours only. Showing
  hosts is what `XENONET_HOSTS=1` turns on.
- **Only a XenoNet's own view leaves it.** The relay reads `/api/graph`,
  `/api/live` and `/api/events` with `?hosts=0`, which a hub answers with
  its own machine's view, and drops anything qualified besides.
- **`repoKey`** is the key without a prefix (`github.com/willctl/xenonet`),
  as contracts section 2 has it, so a merged view's id is `repo:<repoKey>`.
  The relay computes it on the host; the hub computes this machine's.
- **XenoNet's own ssh config** is `ssh_config` beside `hosts.json`, written
  from it, `0600`; links run `ssh -F` with it. `xenonet hosts add <id>
  [user@host]` takes `--ssh-port`, `--identity`, `--known-hosts`,
  `--host-key-alias`, `--xenonet-port` (the XenoNet's port there; `--port`
  already picks which XenoNet the command talks to), `--command`, `--since`,
  `--detail`, `--start` (the relay's, for a machine with no service set up)
  and `--local`.
- **Keys.** A host that answers with another key than the one it had is
  refused until it's added again, except a demo world, which is made afresh
  with every start. A demo console shows demo hosts only, so nothing real
  reaches its screenshots.
- **Caps** besides 5,000 nodes and 16 MB a frame: frames nested at most 64
  levels (deeper would overflow the stack when the merged map is
  stringified), 1,000 live sessions, 200 collisions, 20,000 relations and
  500 identities per host. Every node is checked against `graph/v1` for its
  kind (again only when it changes), plan limit readings are rebuilt from
  the fields that fit, and a host's nodes always hang from its host node in
  one tree.
- **XenoNet Dev's simulations** on the server: `host-add` (another XenoNet on
  this machine, in memory only, since adding a host for real is refused
  from under an agent), `host-offline`, `host-online` and `host-skew`.

### As built: the map (Wave 2, step 2)

The lane's second step put hosts on the page: `src/hosts.js` (the layout's
forces, the words for how a host stands, the machines' chips and the
inspector's and alert strip's parts) and `src/hosts-draw.js` (the host node
and the bridges), with small hooks in `scene.js`, `nodes.js`, `hud.js`,
`alerts.js`, `main.js` and the rest. With no host on the map nothing in
them runs, and one machine draws as before. Where it differs from the plan
above, or settles what it left open:

- **Each host is a system of its own on an outer orbit.** The plan left
  that for after the prototype; merge point A made it the map. The force
  graph's radial layout leaves remote nodes alone (`dagNodeFilter`), and
  each host node is held in the floor's plane on an orbit as far out as
  this machine's deepest ring, plus 120, plus the widest host's radius
  (further with three hosts or more, so neighbours stay clear), in order of
  host id: one host to the right, two either side. A host's nodes keep to
  rings 0.62 × 72 apart around their host node, drawn there by a force of
  XenoNet's own with the radial layout's strength. The host link pulls on
  neither end and is drawn faint, carrying no flow (a host's work shows in
  its own constellation), and the force graph's centering counts only this
  machine's nodes, so hosts stay where they are. New remote nodes start on their
  ring, not at the center. A host node you drag and drop takes its
  constellation along; in the free layout a host's nodes float around
  their host node.
- **The host node** is a smaller core with a tilted ring in the color of
  how it stands: live as the status light's "connected", stale in amber,
  offline and connecting greyed, an error in the danger color. Three arcs
  on the ring turn while it's live and pulse while it connects. Its id is
  under it, and under that how it stands ("LIVE", "SEEN 14:02"). It's drawn
  in the core's color, and this machine's core reads "studio · ~/Projects"
  once hosts are on the map, if this XenoNet named itself.
- **Bridges** are dashed and rise a little over the gap. They're faint at
  rest, so a dozen shared repos don't net the map over, and lit while
  either end is selected, hovered or in the selection's branch. Bridges
  between two hosts are drawn too.
- **A host that isn't live** keeps its constellation in place, dimmed to
  0.42 and still: no ambient motion, no flow or packets, no question
  beacons. Its questions stay in the strip "as of 14:02, vm1 not live",
  with no chime or notification, outside the tab's count and the tray's.
- **Read-only.** A remote session offers Preview and Stats; nothing remote
  opens in an app, is dismissed, handed off, given a terminal, a project
  rule or an identity pin. The inspector says where a node lives ("on vm1 ·
  live · read-only"); the feed, the active sessions, the palette and the
  map's search mark remote ones with their host. A remote question's text
  stays on its host unless the link has `--detail`, so the strip says where
  to read it. A remote session's MCP servers and context pages are looked up
  on its own host.
- **Machines.** A chip per machine under Filters (this one by the name it
  gave itself), with how it stands and its sessions on the map, shows only
  those machines' work.
- **Plan limits.** An account signed in only on another machine
  (`limits.others`) gets bars of its own under Token use, headed by the
  app and the machine it was read on, never by the account.
- **The tray and the menu bar.** `/api/summary` adds every live host's
  waiting questions and stuck sessions, each with `host`, and counts its
  working sessions, reviews, collisions and tokens; the tray shows a remote
  question on the map rather than asking to open it.
- **Driving.** The driver's `hosts()` and `filterHosts()`, `find` with kind
  `host`, a remote node's brief with its `host` and `stale`, and
  `state().hosts`; the console's `hosts` tool, and `simulate` with
  `host-add`, `host-offline`, `host-online` and `host-skew`;
  `xenonet console hosts[:<ids>]`; and `test/e2e/hosts.test.js`, against a
  second demo XenoNet added with `host-add`.
- **Frame time.** In a hidden window with `?raf-shim`, as the benchmark's
  page part measures it (time in animation-frame callbacks per 16 ms frame,
  20 s, two runs each, a heavily loaded machine): the demo alone (33 nodes)
  2.3 to 2.6 ms median and 2.7 to 3.5 ms p90; with two demo hosts (99
  nodes, 24 bridges) 5.6 to 5.8 ms median, 6.4 to 6.5 ms p90 and 7.2 ms
  p99, and 17 MB more heap. Step 1's page with the same two hosts on one
  radial map measured 5.6 to 5.7 ms median and 6.9 to 7.5 ms p90, so the
  constellations, host nodes and bridges cost nothing measurable: what a
  host costs is its nodes, about 0.05 ms a frame for each.
- **Left for later:** a layout for many hosts (past five or six, the orbit
  grows wide and the camera pulls far back), labels that collide inside a
  dense constellation, collisions and the merged repo view across machines,
  flow to a remote host's MCP servers, and whether `@vm1:actor:you` is you.

### As built: hostile hosts and privacy (Wave 3, step 1)

The Wave 3 review found places where a hostile host, or a host's own
data, got further than this design meant. What changed:

- **Only the fields the schemas declare.** Every schema object is loose,
  so a host's node passed `graph/v1` with fields of its own, and the merge
  kept them: a project node with an `app` reached the page's markup
  unescaped. The console now copies what a host sends with only what the
  schemas declare, at every level (`core/hosts/declared.js`): nodes against
  `graph/v1` for their kind, live sessions, activity, identities, and the
  engine's agents and messages, which are checked against
  `engine/agent/v1` and `engine/message/v1` too. Collisions and shared
  files are rebuilt field by field. The copies are remembered per host
  with the check, so a host resending its map costs what it did; the
  first map of a 4,400-node host takes about 12 ms more. The page's part
  of the fix, in the page lane, escapes those fields where it writes them
  into markup.
- **Names every object has.** A node kind or an identity id such as
  `toString` or `__proto__` is looked up as the map's own: no kind at all,
  where it used to make every merge throw. The relay refuses such a query
  key with a 403, where it used to exit, and a read it fails to answer
  gets a 500 and leaves it running.
- **Subagents' tasks.** A subagent's label is the start of what it was
  asked to do, so without `--detail` it crosses as "subagent", in the live
  state and on its events.
- **Folders named for a person.** Besides the home folder, the relay
  scrubs the folders named for a person that the map's places are in (the
  projects folder, the folders outside it where sessions ran, Codex's
  projects): `/Users/<name>` or `/home/<name>`, after a drive if there is
  one. They go with `~` for the name alone, so a projects folder on
  Windows's side of WSL crosses as `/mnt/c/Users/~/Projects`, and can't be
  taken for the home folder's. Nothing but the hello goes before the first
  snapshot: the console takes no event before one, and until it has read a
  map the relay doesn't know these folders, or which sessions the map
  leaves out.
- **Accepting a host's key.** The printed command is `ssh -F
  ~/'Library/Application Support/XenoNet Dev/ssh_config' vm1`, with the `~`
  outside the quotes, where the shell expands it; ssh doesn't expand `~`
  in `-F`.
- **The cache.** A host that comes back with another key has its snapshot
  kept under the new one and the old file deleted. Removing or replacing a
  host deletes both, except a file another host's link uses too.

### As built: the frame budget, shared keys and the relay's copies (Wave 3, step 2)

Merge point B gave hosts a frame budget and a cap on the remote nodes
drawn beyond what you're looking at, measured by a part of the
benchmark; the rest was scheduled.

- **The budget.** The hosts you're not looking at add at most 2 ms median
  and 3 ms p95 a frame, measured with two demo-sized hosts beside the demo
  world. At the end of Wave 2, those two hosts drawn in full added about
  3.2 ms: what a host costs is its nodes, about 0.05 ms a frame each.
- **The cap** (`capHosts` in `src/hosts.js`). Every host node is drawn,
  and beyond those at most 24 of the hosts' nodes, shared between the
  hosts in turn. A host's share goes first to its sessions that wait on
  you or are stuck, then to its places (where something ran lately
  first), then to its active sessions, then to the rest, the most recent
  first, and a node is drawn with what it hangs from. A host is drawn in
  full while the selection is one of its nodes, or while its chip is
  picked: that's looking at it. The nodes left out stay in the layout,
  hidden as the filters hide nodes, so looking at a host draws them in
  place with nothing moving, and selecting a remote project draws its
  host in full before its sessions are gathered. A capped host's chip
  says how much of it is drawn, and the driver's `hosts()` has that on
  each chip. The page tells the machines' filter what you're looking at
  as it selects, and what waits on you as the live state comes; until it
  does, nothing is capped.
- **The benchmark's `hosts` part** (`node scripts/bench.mjs hosts`).
  Two demo XenoNets on spare ports, added to a hidden test instance with
  `host-add`, and windows that take turns with the chips picked three
  ways: only this machine's, so no host is drawn; none, as the map starts,
  which is capped; and every machine's, which draws both in full. It
  reports what the capped and full ways add over none, how many of the
  hosts' nodes each drew, and whether the capped way is within the budget.
  Frame time is measured at merge points, not in a lane's own windows.
- **Two hosts with one key.** A XenoNet makes its key once, at random, so
  two hosts share one only when one XenoNet was added twice or one
  machine's state folder was copied from the other's, as setting up a
  new Mac from an old one's backup does. They'd share one snapshot in the
  cache, each writing over the other's. The doctor warns about it
  (`core/hosts/doctor.js`), saying which hosts and what to do in either
  case; XenoNet Dev's simulated hosts, which keep nothing in the cache,
  don't count.
- **The relay's copies.** The relay used to clone each map, shape the
  clone, then copy it again to write the home folder as `~`. It now
  picks what goes from the map as XenoNet gave it and copies only that,
  once, writing `~` as it copies, so what `--since` leaves out is never
  copied at all. The live state, the engine's and events go the same way.
  On the benchmark's made-up map of 400 projects and 4,000 sessions, a
  graph event's shaping went from 14 to 16 ms to about 6 ms with
  `--since 7d`, and from 21 to 12 ms with every session kept.

## Questions for the owner

Answered at merge point A (`next-phase.md`), and kept here as they were
asked: a hub over SSH with constellations and bridges; SSH only for now;
titles, states and counts by default, transcripts when opened; read-only
through Wave 2; hosts name themselves and tilde their paths; Lima, with
two Linux VMs first. Still open: whether `@vm1:actor:you` is you (6), and
the tailnet path, which hasn't been tried (8).

1. **Which option?** The recommendation is a hub over SSH, with host
   constellations and repo bridges. Or would you rather have the merged
   repo view as the main map?
2. **SSH as the only way in, for now?** It means an SSH server on each
   host: Remote Login on a Mac, OpenSSH Server on Windows. Or should Wave 2
   also try the tailnet listener, for machines without one?
3. **What crosses machines by default?** Everything the console shows,
   including a waiting question's text and the last message? Or
   metadata only (titles, states, counts), with transcripts only when you
   open one?
4. **Read-only through Wave 2?** No dismissing, opening or messaging
   across machines.
5. **Host ids and paths.** Do hosts name themselves (the console can
   rename), and are remote paths tilded on the host, which changes the
   example in contracts section 2?
6. **The person.** Does `actor:you` on another host count as you on the
   console?
7. **Which guests?** For the prototype's VM lab runs: the Ubuntu guests
   already in UTM, or a new Lima machine? And do you want a macOS guest
   with Claude Code and Codex signed in before merge point B?
8. **Tailscale.** Sign in on the Mac and a guest to test the tailnet path?
   The Personal plan is free for up to 6 users with unlimited devices
   ([Tailscale](https://tailscale.com/pricing)).

## Sources

- OpenSSH manuals: [ssh(1)](https://man.openbsd.org/ssh),
  [ssh_config(5)](https://man.openbsd.org/ssh_config),
  [sshd_config(5)](https://man.openbsd.org/sshd_config),
  [sshd(8), the authorized_keys format](https://man.openbsd.org/sshd#AUTHORIZED_KEYS_FILE_FORMAT).
- Tailscale:
  - [Serve](https://tailscale.com/kb/1312/serve) and
    [its command](https://tailscale.com/kb/1242/tailscale-serve);
  - [the CLI: `whois` and `cert`](https://tailscale.com/kb/1080/cli);
  - [Tailscale SSH](https://tailscale.com/kb/1193/tailscale-ssh), and the
    [`SSHAction` policy fields](https://pkg.go.dev/tailscale.com/tailcfg#SSHAction);
  - the [default allow-all policy](https://tailscale.com/kb/1192/acl-samples)
    and [grants](https://tailscale.com/kb/1324/grants);
  - [pricing](https://tailscale.com/pricing);
  - a [community forum thread](https://forum.tailscale.com/t/does-port-forwarding-work-with-tailscale-ssh/2664)
    on forwarding through Tailscale SSH (2022, secondary).
- MCP, [transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports):
  stdio, the Origin and DNS rebinding rules, and resuming streams.
- WHATWG HTML,
  [server-sent events](https://html.spec.whatwg.org/multipage/server-sent-events.html).
- Chrome, [Local Network Access](https://developer.chrome.com/blog/local-network-access).
- Node.js: [`http.request` options](https://nodejs.org/docs/latest-v24.x/api/http.html#httprequestoptions-callback),
  [TLS](https://nodejs.org/docs/latest-v24.x/api/tls.html),
  [`fs.watch` caveats](https://nodejs.org/docs/latest-v24.x/api/fs.html#availability).
- Apple: [TN3179, local network privacy](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy),
  and [Remote Login](https://support.apple.com/guide/mac-help/allow-a-remote-computer-to-access-your-mac-mchlp1066/mac).
- Microsoft, [OpenSSH Server for Windows](https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_install_firstuse).
- Earlier tools that work this way:
  [Syncthing's device IDs](https://docs.syncthing.net/dev/device-ids.html),
  [VS Code Remote-SSH](https://code.visualstudio.com/docs/remote/ssh),
  [Zed's remote development](https://zed.dev/docs/remote-development).
- Plan limits:
  [Claude Code with a Pro or Max plan](https://support.claude.com/en/articles/11145838),
  and [Codex pricing](https://learn.chatgpt.com/docs/pricing).
