# Guests

This proposes a way to run XenoNet in a guest: a virtual machine, a WSL2
distribution or a container, apart from the computer you look at. The agents
can run there too. It is a design for review, and nothing in it is built yet.

Two things in the repo already do part of this by hand. `vm-lab.md` and
`scripts/lab/` build Linux VMs on a Mac to test `hosts.md`. The Development
guide's "Developing on another computer" joins a dev checkout in a VM to the
installed XenoNet with two forwarded ports. Each needs its own runbook. This
replaces both with one set of commands that works on every operating system
and every kind of guest.

**In short:** make a guest something XenoNet can create, update and reach.

- A guest is a Linux environment with an SSH server on a loopback address,
  with key login only. WSL2, Docker, Lima, a VM you made and a cloud box are
  each a way to get one.
- Over SSH, one provisioner installs XenoNet. A **runtime guest** gets the
  Service and Tools release archives, so it needs no checkout and no Node. A
  **dev guest** gets a checkout and runs XenoNet Dev.
- XenoNet keeps one SSH bridge to each running guest. Nothing listens on a
  network, and the guest appears as a host on the map.
- `xenonet guest update` updates a guest by running `xenonet update` in it.
- Every guest has a name that never changes, `<name>.xenonet.lan`, which
  reaches it from your computer.
- XenoNet never touches your own keys or sign-ins. A guest gets keys of its
  own, and you sign in to Claude Code and Codex inside it.

## Why

- **Locked-down computers.** Security software can block the tools XenoNet is
  developed with. XenoNet can still be installed there, but not worked on. A
  Linux guest next to it can.
- **Isolation.** Agents can work somewhere that can't see your home folder,
  your SSH keys or your other projects. XenoNet still shows what they do.
- **A clean second machine.** A guest you can destroy and make again tests a
  XenoNet change against a fresh Linux, or against a second host.
- **Computers you can't change.** A shared computer or a build box may allow
  a VM or container and nothing else.

A guest needs no new kind of XenoNet. It runs the released XenoNet in the same
three components that `components.md` describes.

## What a guest is

Every guest has these parts, whichever backend made it.

| Part | What it is |
|---|---|
| Name | You choose it, such as `vm1` or `sandbox`. It is never the computer's own name, as with `xenonet hosts self`. |
| Flavor | `runtime` or `dev`. |
| Backend | What made it and how it starts and stops: `wsl`, `docker`, `lima`, or `ssh` for one you bring. |
| Address | `<name>.xenonet.lan`, which resolves to your own computer's loopback and so to the bridge. See [Names](#names). |
| SSH endpoint | An address and port, a user, and the keys XenoNet made for it. The address is on loopback except for the `ssh` backend. |
| XenoNet port | The port XenoNet listens on in the guest, from 47850 up, one for each guest. |
| State | One of the four below. |

**States.**

| State | Meaning | Moves to |
|---|---|---|
| `created` | The record exists. Provisioning is not finished. | `stopped` when `create` finishes |
| `stopped` | The guest is not running. | `running` on `up` |
| `running` | The bridge is up and the guest's XenoNet answers. | `stopped` on `down`, `unreachable` when either check fails |
| `unreachable` | The guest should be running and isn't answering. `status` says which check failed. | `running` when the bridge reconnects |

The XenoNet port is the same number on both sides of the bridge. XenoNet answers
only to its own address and returns 421 for any other Host header, so a
forwarded port must keep its number. That is why each guest needs a port of
its own beside the 47840 your XenoNet uses. The guest's XenoNet starts with
`XENONET_PORT` set to it. A dev guest keeps XenoNet Dev's usual 47842 for the
first one, so `xenonet connect --here --port 47842` works as it does today.

XenoNet keeps guests in the same file as hosts, `hosts.json` in its state
folder, as a host entry with an extra `guest` field. A guest is a host that
XenoNet also knows how to build, so there is one list of other machines to
keep, show and check.

## The two flavors

### Runtime guest

A runtime guest is for isolation and for a second machine. It runs what a
release installs.

- The provisioner downloads the Service and Tools archives for the guest's
  operating system and CPU from one release, checks them against
  `SHA256SUMS.txt`, and runs `./bin/xenonet setup` for each, as
  `components.md` says. Each archive is the `xenonet` binary for that
  system, so the guest needs no Node.
- The Service runs as a systemd user unit in the guest. Lingering keeps it
  running when nobody is signed in. Where there is no systemd, as in most
  containers, the backend's `up` starts it.
- Claude Code and Codex are installed inside, at the versions this computer
  runs unless you choose others. You sign in to them inside the guest. XenoNet
  copies no credential files in, as `vm-lab.md` already requires.
  `--no-agents` skips them.
- The bridge forwards the guest's XenoNet port to the same port on your
  computer. The guest can't see anything of yours. You open
  `http://127.0.0.1:<port>` for the guest's own console.
- On the map, the guest is a host, read-only, through `xenonet relay`, as
  `hosts.md` designs. Hosts are still a prototype behind `XENONET_HOSTS=1`, so
  the browser console works first.

### Dev guest

A dev guest is for working on XenoNet where the computer won't let you. It runs
XenoNet Dev from a checkout, as the by-hand setup does today.

- The provisioner installs git, Node from its official tarball (checked
  against `SHASUMS256.txt`), Electron's libraries for end-to-end runs and a
  virtual display. It clones the repository and installs its packages, and
  runs Electron's own installer explicitly, because npm's script allow-list
  skips it.
- XenoNet Dev runs as a user service. `XENONET_UPSTREAM` points it at your XenoNet
  through the bridge, so the dev page shows your real sessions, read-only.
  The bridge forwards your XenoNet's 47840 into the guest, and the guest's XenoNet
  Dev port to the same port on your computer.
- `xenonet guest connect <name>` runs `xenonet connect --here --port 47842`, so
  your agents can talk to XenoNet Dev beside the installed one. It asks first,
  because it changes the agents' settings.
- `xenonet guest update <name>` pulls the checkout, reinstalls and restarts.

One guest can later hold both flavors. The first version keeps them apart.

## Backends

A backend has one job: end with a Linux user, an SSH server listening on
loopback with key login only, and a way to start and stop the guest. The
provisioner and the bridge don't know which backend made it.

| Backend | Where | What it does | What to know |
|---|---|---|---|
| `wsl` | Windows | Uses an existing WSL2 distribution or imports a fresh one, then adds a user and the SSH server. | WSL stops a distribution that has no open `wsl.exe`, and its SSH server with it, so XenoNet keeps one open while the guest is up. WSL already publishes a guest port on the host's loopback, so the bridge leaves the dev port's forward out. |
| `docker` | Anywhere Docker runs | Builds a small image (a base, an SSH server, git and curl), runs it with the SSH port published on loopback only, and keeps the guest's home folder in a named volume. | Resets with one command, and nothing is mounted. There is no systemd, so `up` starts the services. The agents' sign-ins live in the volume. |
| `lima` | macOS, Linux | Builds on `scripts/lab`: one golden copy per distribution, cloned for each guest. | Apple silicon runs arm64 guests. See decision 1. |
| `ssh` | Anywhere | You bring a Linux machine with an SSH login, such as a Hyper-V VM, a cloud box or a spare computer. XenoNet doesn't create it. | The only backend that can leave loopback. `create` refuses a guest that listens on a network unless you give the address on purpose. |

A backend is a small module with `create`, `up`, `down`, `destroy` and
`endpoint`. A fake one drives the tests. `scripts/lab` becomes the `lima`
backend's first user, so the lab and the feature are one system.

## Commands

```sh
xenonet guest create <name> [--backend wsl|docker|lima|ssh] [--dev] [--no-agents]
xenonet guest list                 # name, flavor, backend, state, versions
xenonet guest up|down <name>       # start or stop the guest, and open or close the bridge
xenonet guest update <name>        # xenonet update in a runtime guest; pull in a dev guest
xenonet guest status <name>        # what answers, on which ports, and what to do next
xenonet guest connect <name>       # dev guests: connect your agents to XenoNet Dev
xenonet guest shell <name>         # an ssh login, for the one-time sign-ins
xenonet guest names [--print|--install|--remove]   # make the guests' names resolve here
xenonet guest destroy <name> [--keep-data]
```

With no `--backend`, `create` picks the one that fits the computer and says
why: `wsl` on Windows, `lima` on a Mac, `docker` on Linux. With
`--backend ssh` it takes an address.

Like `xenonet hosts add`, `create`, `connect`, `shell` and `destroy` are yours.
XenoNet refuses them from under an agent, because they make accounts, keys and
long-running services. An agent can run `list`, `status`, `up`, `down` and
`update` for a guest you already created. In tests, XenoNet Dev's `simulate`
stands in for a guest, as it does for hosts.

Each command prints what it will change before it changes it. `create` shows
the user, the SSH server's address and port, the keys' paths, the services it
will install and the ports it will use. It records each finished step in the
guest's entry. A second `create` of the same name skips the finished steps,
checks the rest and changes nothing that is already right, so a `create` that
stopped halfway picks up where it was.

## Why SSH

XenoNet already runs `ssh <host> xenonet relay` for hosts, with forwarded ports,
restricted keys and host-key checks. A guest that offers SSH gets all of it,
and every backend looks the same from there.

The cost is a new SSH server on loopback in WSL and Docker guests. The
alternative for those two is to run commands through `wsl.exe` or
`docker exec` and skip the server. That removes the listener but needs a
second transport, and the dev guest's forwarded ports would need a TCP relay
running inside the guest. The first version uses SSH only, and keeps the
backend interface open to an exec transport later.

Two other designs were set aside.

- **Prebuilt guest images** with XenoNet baked in start faster. They add an
  image build for each backend to every release, they drift from the
  release they were built with, and they still need the bridge and the
  keys. A `docker` image could come later on top of this design.
- **Runbooks only**, the way it works today. Each setup is hand work, with
  mistakes nobody catches until they fail. The npm script allow-list that
  skipped Electron's installer and the passphrase quoting in PowerShell both
  cost time that a provisioner pays once.

## The bridge

XenoNet keeps one `ssh -N` process for each running guest, with an ssh config of
its own, as hosts do. It never reads or changes `~/.ssh/config`.

- **Forwards.** A runtime guest forwards its XenoNet port to your computer. A
  dev guest also forwards your XenoNet's 47840 into the guest, for
  `XENONET_UPSTREAM`. Every forward binds `127.0.0.1`.
- **Keys.** XenoNet makes three, with `ssh-keygen` and an argument list, never a
  shell line. A quoted empty passphrase typed in PowerShell becomes two quote
  characters, and the login then fails in batch mode while the server accepts
  the key.
  - The **admin key** gives a shell as the guest's user. Only XenoNet uses it,
    for `create`, `update`, `status` and `shell`.
  - The **bridge key** is limited in the guest's `authorized_keys` to the
    forwards above, with `restrict`, `port-forwarding`, `permitopen` and
    `permitlisten`, and gives no shell.
  - The **console key** is limited to `xenonet relay`, as `hosts.md` specifies.
  All three stay in XenoNet's state folder, readable by your account only.
- **Host keys.** XenoNet records the guest's host key at `create` and refuses
  a changed one, as it does for hosts. A rebuilt guest asks you.
- **Staying up.** XenoNet restarts a dropped bridge after 2 seconds, then
  less often, up to a minute apart, as hosts do. While it is down, the guest
  reads as unreachable with its last snapshot. A dev guest's hooks on your
  computer stay out of the way, as the Development guide says.
- **Where it runs.** The Service on your computer runs the bridges, so they
  start with it and don't depend on Desktop.

## Names

A guest's console and API are at `http://<name>.xenonet.lan:<port>`. The name
comes from the guest's name and nothing else. It doesn't depend on an
address, a port or when the guest was made, so a bookmark, a script or a
note keeps working after a rebuild. A guest name must be a valid DNS label:
lowercase letters, digits and hyphens, up to 63 characters. `create` refuses
anything else.

A guest name resolves to `127.0.0.1` on your computer. The bridge's local end
is there, so the name reaches the guest without any traffic leaving the
computer.

**Making the name resolve.** A computer finds `.lan` names only if something
tells it to. The system resolver doesn't know `.xenonet.lan`, and Windows' resolver
doesn't resolve `.localhost` names either, though browsers do. XenoNet adds the
names to the hosts file, in a block between two marker lines that it owns and
rewrites whole:

```
# xenonet guests begin
127.0.0.1 vm1.xenonet.lan
127.0.0.1 sandbox.xenonet.lan
# xenonet guests end
```

Writing the hosts file needs an administrator. `xenonet guest names --print`
shows the block and the command that applies it, `--install` writes it when
you ran it elevated, and `--remove` takes the block out. `create` and
`destroy` run the same logic and, when they can't write the file, print what
to run and carry on. Until the name resolves, `127.0.0.1:<port>` works, and
`status` says which of the two is missing.

**What the guest accepts.** XenoNet answers only to its own address and returns
421 for any other Host header, which stops DNS rebinding. The guest's XenoNet
therefore needs the name added to the Host and Origin names it accepts. The
provisioner passes it exactly, `<name>.xenonet.lan:<port>`, in an environment
variable the Service reads at start. It is one name for one guest, never a
pattern, so a hostile page can't make the guest answer for a name that you
did not give it.

**The port stays in the address.** The first version does not listen on port
80, so the address includes `:<port>`. A reverse proxy that routes by name
would drop the port, but it adds a listener that the design otherwise avoids.

**Changing the suffix.** `.lan` is not reserved. A home or office network may
have real `.lan` names, and a hosts entry wins on this computer only. The
suffix is a setting, `xenonet.lan` by default. `.localhost` needs no
resolver setup in most browsers, and `.internal` is reserved for private
use. Don't put these names in a network's DNS: pointing them at
`127.0.0.1` there would send every other computer to itself.

## Isolation

Every guest meets these rules, and `xenonet guest status` and the doctor check
them.

- Nothing is mounted from your computer: not your home folder, not
  `~/.claude`, not `~/.codex`, not your SSH keys. Where a backend mounts
  something by default, as WSL mounts the Windows drives, `create` turns it
  off inside the guest or says it can't.
- There is no SSH agent forwarding, and no credential is copied in. A runtime
  guest gets release archives and a dev guest clones the public repository,
  so neither needs a sign-in of yours.
- The guest's XenoNet, its SSH server and every forward listen on 127.0.0.1
  only. `status` checks this from both sides.
- The guest answers to its own name and to 127.0.0.1, and to no other Host
  header.
- The guest's user is named for the guest, not for you, and its git identity
  is a placeholder unless you give one. Nothing from your account name
  reaches the guest's paths, its `passwd` or a commit made in it.
- Reading a guest follows the host relay's read-only contract: titles,
  states and counts, with paths under the guest's home written as `~`.
- A guest can reach the internet, because agents need to. Whether it can
  reach your own network depends on the backend. WSL and Docker share the
  computer's, and `lima` can be told not to.

A guest does not protect you from an agent that you sign in there and let run
commands. It keeps that agent away from your files and credentials, not from
the guest's own.

## Updates

- **Runtime guest.** `xenonet guest update` runs `xenonet update all` in the
  guest. That downloads, checks and installs as `components.md` says, and keeps
  the old version until the new one starts. A guest can lag your own XenoNet,
  because both speak compatible API versions. The doctor warns when a guest's
  versions fall out of range.
- **Dev guest.** `git pull --ff-only`, the install and a restart.
- **The guest's own system.** XenoNet doesn't patch the operating system. It
  says when the guest's packages are old and gives the command.
- **Desktop.** A guest runs the Service and Tools only.

## Platforms and gaps

- The release builds Service and Tools for Linux x64 only. A `lima` guest on
  Apple silicon and a Docker guest on a Mac are arm64.
- A Windows computer can host guests. Windows containers and Windows VMs as
  guests are out of the first version.
- macOS guests need Apple hardware and a lot of disk, as `vm-lab.md`'s Phase
  B says. They are also out of the first version.
- The SSH server inside a WSL distribution is a new listener on loopback.
  `create` says so, and leaves a distribution that already runs one alone.
- Writing the hosts file needs an administrator on every system, and
  security software may block it. The names are a convenience. Every command
  and the map use the port address, and they work without the names.
- Hosts are still a prototype. A guest shows first in its own console in a
  browser, and on the map once hosts graduate.

## Tests

- **Unit.** The provisioner, bridge supervision, the registry and command
  parsing run against a fake backend that records what it was asked and
  answers from a script. No unit test needs a VM.
- **The lab.** `npm run lab` keeps checking the relay over real SSH. It moves
  to the `lima` backend.
- **End to end.** The demo instance gets a `simulate` step that adds a guest
  with a made-up state, so the page and the console's tools can be driven and
  screenshotted without one.
- **Real backends.** Each gets a short checklist, run by hand before a
  release, because CI can't start WSL or nested VMs. The `docker` backend can
  run in CI on Linux.
- **Made-up names.** Tests, fixtures and docs use `vm1`, `sandbox` and
  addresses from the documentation ranges only.

## What stays out of this repository

This is a public project, so nothing about the computer that built or tests it
goes in. That means no computer, user or distribution names, no paths, no
addresses outside the documentation ranges, no ports that a person's setup
chose, no key names, no screenshots of a real guest, and no mention of the
security software that made someone want a guest. A person's guests, keys and
ssh config live in their own XenoNet state folder. Setup notes about one
computer belong in that person's own notes. When the docs need a reason, they
say "a computer whose security software blocks development tools" and stop.

This document, the guide page and every test follow that rule. The build
already stops when a packaged file contains your home folder's path. That
check should also scan the docs and tests for host names.

## Phases

1. **The `ssh` backend, the provisioner and the bridge.** `create`, `list`,
   `up`, `down`, `update`, `status`, `names` and `destroy` for a machine you
   bring, both flavors, the registry, the isolation checks and the fake
   backend. This is the core, and every later backend makes something it
   can use.
2. **`wsl`.** The backend Windows users need first, and the one the by-hand
   setup used.
3. **`docker`.** The image, the volume and a CI run on Linux.
4. **`lima`.** The lab moves in. Linux arm64 builds land here if decision 1
   says yes.
5. **In the app.** A Guests panel for the same commands, a doctor check for
   each guest, the map showing a guest as a host, and `xenonet mcp console`
   tools that list and report on guests.

Each phase ships on its own and leaves the earlier commands working.

## Decisions to confirm

Each decision has a default. Phase 1 starts with the defaults unless you
change one.

1. **Linux arm64.** Default: add Service and Tools for Linux arm64 to the
   release before phase 4. Until then a `lima` guest on Apple silicon, and a
   Docker guest on a Mac, run as dev guests from a checkout. That works, and
   is heavier than a release.
2. **One list.** Default: guests are host entries with a `guest` field, so
   there is one registry. The `guest` commands read it without `XENONET_HOSTS`.
   Only the relay and the map need the flag.
3. **Ports.** Default: XenoNet ports 47850 to 47899, one for each guest. `create`
   refuses a port that something else is using.
4. **Agents inside.** Default: a runtime guest installs Claude Code and Codex
   when it is created, and `--no-agents` skips them. A sandbox is for running
   agents, and the install adds about 0.35 GB.
5. **Under an agent.** Default: `list`, `status`, `up`, `down` and `update` are
   allowed, so an agent can bring up the dev guest it works in. `create`,
   `connect`, `shell` and `destroy` stay refused.
6. **Sign-ins in a Docker guest.** Default: `destroy` names what the volume
   holds, including the agents' sign-ins, and deletes it after you confirm.
   `--keep-data` keeps the volume.
7. **Names.** Default: `<name>.xenonet.lan` through a XenoNet-owned block in the
   hosts file, never written without an administrator, and a `names --print`
   that gives the lines to apply by hand. The suffix can be changed. The port
   stays in the address until there is a reason to add a listener.
