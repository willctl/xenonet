# The XenoNet lab

Linux VMs on the development Mac, so one XenoNet can be tested against
several machines. Each VM runs Claude Code, the Codex CLI and a headless
XenoNet, which joins a network with the Mac's
([network.md](../../docs/guide/network.md)). The VMs and the Mac share a
LAN, so the checks can test XenoNet's transport between machines: direct
paths, mDNS, your own relays, mailboxes and catch-up, which phase 5 of the
rewrite builds ([direction.md](../../docs/design/direction.md)). The lab
has two Linux VMs; a macOS guest isn't built.

```sh
npm run lab -- build ubuntu --ref main   # the golden copies, once
npm run lab -- build fedora --ref main
npm run lab -- clone vm1                 # the working VMs
npm run lab -- clone vm2
npm run lab -- check                     # each VM's checks, then the transport's
npm run lab -- status
```

It needs Lima 2.1 or later (the lab was built with 2.1.2), about 10 GB of
disk, and about 1.5 GB of downloads: the tools, once for each golden
copy. The OS images come from Lima's cache when they're there. Nothing
costs money; sessions in the VMs count against your Claude and ChatGPT
plans, as on the Mac.

## The VMs

| Lima instance | What | Host id | SSH on the Mac | Time zone |
|---|---|---|---|---|
| `xenonet-base-ubuntu` | Golden copy: Ubuntu 24.04, 2 vCPUs, 4 GiB, a 20 GiB disk | | | |
| `xenonet-base-fedora` | Golden copy: Fedora 44, the same size | | | |
| `xenonet-vm1` | Clone of the Ubuntu golden copy | `vm1` | 127.0.0.1:60851 | UTC |
| `xenonet-vm2` | Clone of the Fedora golden copy | `vm2` | 127.0.0.1:60852 | Pacific/Auckland |

- **Golden copies** are built, stopped and protected (`limactl protect`).
  Nobody signs in to them, so no clone ever carries a sign-in.
- **Working VMs** are copy-on-write clones of them, which take close to
  no disk at first. A clone makes its own SSH host keys on its first boot,
  and Claude Code makes its own machine ids there the first time it runs.
- **Host ids** are the ids the owner gives the VMs when joining them to a
  network. The transport checks look for `vm1` and `vm2`.
- **Time zones** come from a template parameter, `labTimezone`, which the
  VM's root setup applies at every boot: Lima's own `timezone` setting does
  nothing in plain mode.
- `scripts/lab/config.mjs` has the table, and `scripts/lab/lima/` the
  templates (`lab.yaml` holds what the two share).

## Commands

| `npm run lab -- …` | What it does |
|---|---|
| `status` | The lab's VMs, the XenoNets in the running ones, and each VM's address on the lab's LAN |
| `build ubuntu\|fedora [--ref R]` | Builds a golden copy: creates it, installs the tools and XenoNet at git ref R (HEAD by default), checks the demo XenoNet answers, then stops and protects it. A build that stopped halfway picks up where it was |
| `clone vm1\|vm2` | Clones a working VM, starts it, puts made-up repos in `~/Projects`, and starts its two XenoNets |
| `up [vm…]` / `down [vm…]` | Starts or stops working VMs |
| `push [vm…] [--ref R]` | Pushes XenoNet at git ref R into running VMs, rebuilds it, and restarts its services |
| `check [vm…] [--only C,…]` | Each VM's checks, then the transport checks across vm1 and vm2 (below). `--only` runs just the transport checks named |
| `teardown [--yes]` | Deletes the lab's VMs, by name |

## What's in each VM

- **The lab user**, `will`, full name "Will", home `/home/will`. Lima
  would otherwise copy the Mac's user name and full name into the guest.
  git's identity is the demo world's made-up one (Nova).
- **git**, from the distribution, and **Node 22.23.3** from the official
  tarball, in `~/.local/node`, checked against `versions.json` and the
  release's `SHASUMS256.txt`.
- **Claude Code**, from its native installer, pinned to the Mac's version,
  with `DISABLE_AUTOUPDATER=1` (in `~/.claude/settings.json` and every login
  shell). **The Codex CLI**, pinned the same way, from npm into `~/.local`.
  `versions.json` holds the versions; `build` installs them.
- **XenoNet**, pushed from this repo with `git push` over SSH into `~/xenonet`
  (the VMs have no GitHub access), built there (`target/debug/xenonet`, with
  rustup and the toolchain `rust-toolchain.toml` pins; the first build takes
  several minutes, later ones only the crates that changed), and the
  `xenonet` command in `~/.local/bin`.
- **Two XenoNets**, as systemd user services that run without anyone logged
  in (linger): `xenonet.service` on 127.0.0.1:47840, which reads the VM's own
  sessions and is the one that joins a network, and `xenonet-demo.service`
  on 127.0.0.1:47841, with made-up data. They serve the API only; the page
  isn't built. `~/.config/xenonet-lab/xenonet.env` holds a VM's settings:
  `vm2` has `XENONET_CLAUDE_USAGE=off`, so only `vm1` asks Anthropic for
  plan usage.
- **iroh-relay** in `vm1`, once a relay check has run: built in the VM at
  the version `versions.json` pins (the iroh XenoNet uses), with
  `--features server --locked`, into `~/.local/bin`. The first build takes
  a few minutes. The checks run three from config files, with TLS, never
  `--dev`, on certificates for vm1's lima0 address from **the lab's CA**,
  which the first run makes in `~/.local/share/xenonet-lab/relay/`:

  | Relay | URL | The network |
  |---|---|---|
  | `a` | `https://<vm1>:3443` (QUIC address discovery on UDP 7842) | lists it |
  | `b` | `https://<vm1>:4443` | lists it |
  | `elsewhere` | `https://<vm1>:5443` | doesn't list it; vm2 adds it as its own in `relay-unlisted` |

  Each runs as a transient user service, `xenonet-lab-relay-<name>`, with
  its metrics on 127.0.0.1 (ports 9090 to 9092). The checks put the CA in
  each VM's `~/.local/state/xenonet/network/relay-ca.pem`, the setting
  XenoNet trusts a relay's own CA by ([own-relay.md](../../docs/guide/own-relay.md)),
  and start that VM's XenoNet again when it changed.
- **openssl and tcpdump**, for the lab's CA and `relay-sealed`'s capture.
- **Made-up repos** in `~/Projects` (`fixtures.mjs`), from the demo world's
  names and remotes: `atlas-api` on both VMs with the same remote and the
  same commits; `atlas-web` and `labs/geofence` on `vm1`; `ledger` and
  `docs` on `vm2`; and on both, `Café Zones` (a space and an accent), a
  worktree beside its repo (`atlas-api-refunds`) and a pnpm workspace
  (`atlas-kit`). The remotes are never fetched from or pushed to.

## On the Mac

- **Each VM's Lima folder**, `~/.lima/<instance>`, which also holds
  `xenonet-lab-password`: the VM's sudo password (mode 0600). Only its hash
  went into the VM.
- Nothing is added to `~/.ssh/config`.

## Networking and root

- **Plain mode**: no mounts, no automatic port forwards, no guest agent.
  Nothing in the Mac's home folder is visible in a VM, and nothing a VM
  listens on is forwarded to the Mac's 127.0.0.1.
- **Two interfaces.** `eth0` is Lima's user network, for SSH from the Mac
  and the internet. `lima0` is Lima's vzNAT network (Apple's shared NAT):
  the VMs and the Mac are on one LAN there, the Mac at the subnet's `.1`,
  with UDP and multicast between all three. That's where XenoNet's direct
  paths and mDNS run. `status` shows each VM's address on it.
- **The Mac's loopback is blocked.** Lima's user network maps 192.168.5.2 in
  a guest to the Mac's 127.0.0.1, where XenoNet's API listens. nftables
  rules, loaded before the network at every boot
  (`xenonet-lab-firewall.service`), drop the TCP connections and UDP a guest
  starts toward it. They look at TCP's SYN flag rather than conntrack's
  state, so the SSH connections the Mac opens into the VM are never caught,
  even one open before the rules load. DNS (Lima serves it at 192.168.5.2
  too) and DHCP still work. XenoNet's API listens on 127.0.0.1 in the VMs
  and on the Mac, so it isn't on the LAN either.
- **Root needs the VM's password**, which only the Mac holds, so an agent
  in the VM can't lift the rules. The two exceptions are commands Lima runs:
  `sudo cat /mnt/lima-cidata/param.env` at every start, and
  `sudo /sbin/shutdown -h now`. To run something as root yourself, with
  the password on standard input and no terminal to echo it:
  `ssh -F ~/.lima/xenonet-vm1/ssh.config -T lima-xenonet-vm1 sudo -S -p "''" <command> < ~/.lima/xenonet-vm1/xenonet-lab-password`.
- **SSH** is Lima's own channel: `limactl shell xenonet-vm1`, or
  `ssh -F ~/.lima/xenonet-vm1/ssh.config lima-xenonet-vm1`. It takes keys
  only, with no agent or X11 forwarding into a VM. Lima gives a VM a new
  instance id at every start, which would have cloud-init make new host
  keys each time; the templates keep them (`ssh_deletekeys: false`).

## Each VM's checks

`npm run lab -- check` first runs these in each running VM, one `ok` or
`FAIL` line apiece:

1. both XenoNets answer, on 127.0.0.1:47840 and :47841 (two checks);
2. a server on the Mac's 127.0.0.1 can't be reached through 192.168.5.2,
   while the internet can (two);
3. the VM has an address on the lab's LAN (`lima0`);
4. XenoNet's API listens on 127.0.0.1 only, and nothing is mounted from
   the Mac (two);
5. only made-up names: the lab user is "Will", git's is Nova, and the
   Mac's own names appear nowhere in the VM's users, groups, git config or
   authorized keys;
6. `sudo` without the password is refused, and with it works.

## The transport checks

Then it runs the transport checks (`transport.mjs`) across vm1 and vm2.
Each prints one line:

- `ok` or `FAIL`;
- `wait`: not in this build yet. The check needs phase 5 code that the
  VMs' XenoNet was built without. The lab looks for the commit that lands
  each piece in the history the VM was built from, by its subject
  (`LANDS` in `transport.mjs`): `feat(net): … link/2 …` (commit 9),
  `… holder …` (10), `… mailboxes …` (11), `… several networks …` (13)
  and `… self-hosted relays …` (13b, the own relays the relay checks need).
  `push` the VMs once it lands;
- `owner`: the check needs something only you can set up, and says what;
  with the stand-in an agent uses instead;
- `skip`: vm1 and vm2 aren't both running. The line still says what this
  checkout waits on.

Only `FAIL` makes `check` fail.

| Check | What it shows | Waits on | You set up | An agent uses | What it changes, and puts back |
|---|---|---|---|---|---|
| `lan-udp` | vm1 and vm2 reach each other over UDP on the LAN | | | | nothing |
| `lan-multicast` | multicast from vm1 reaches vm2, as mDNS needs | | | | nothing |
| `mac-udp` | the Mac reaches each VM over UDP on the LAN | | run it: `--only mac-udp`, from your own terminal | | nothing |
| `mdns` | mDNS finds the other machines, and the path to each is direct | link/2 | the network | `simulate net-founder` | nothing |
| `lan-message` | a message on the LAN arrives in under 1 s | link/2 | the network | the delivery tests | two stand-in sessions |
| `relay` | with vm1↔vm2 UDP dropped by nft, they link through relay `a` or `b` in vm1, TLS from the lab's CA, and vm2's doctor rows for both say ok, "a CA you added" | link/2, own relays | the network, and relays `a` and `b` in it | the relay tests (`relay_tests.rs`) | starts the relays, puts the CA in each VM, drops the UDP; then stops the relays and lifts the UDP |
| `relay-sealed` | the text of a message through the relay is nowhere in a capture of vm1's lima0, while the relays' byte counters show it went through them | link/2, own relays | as `relay` | `a_relay_sees_who_talks_never_what`, which reads what the relay itself sees | as `relay`, and a tcpdump of vm1's lima0, deleted afterwards |
| `relay-unlisted` | vm2 adds `elsewhere` as its own relay; messages still come, and `elsewhere` never sees vm1 (its `unique_client_keys` stays at 1) | link/2, own relays | as `relay` | `a_relay_the_network_doesnt_list_is_never_used` | as `relay`, and `elsewhere` as vm2's own relay, taken out afterwards |
| `relay-down` | stopping vm2's home relay moves vm2 to the other in 60 s, messages still come, and vm2's doctor names the one that's down | link/2, own relays | as `relay` | `a_relay_going_down_moves_relay_only_machines_to_another_listed_one` | as `relay`, and stops vm2's home relay |
| `mailbox` | after `down vm2`, a mailbox delivers to vm2 once it's back, the sender offline | link/2, mailboxes | the network, and the Mac as its mailbox | `simulate host-offline` | stops vm2 and vm1's XenoNet, starts both again |
| `gap` | after a partition, vm2 fills its gap from vm1 | link/2, holders | the network | the turmoil tests | cuts vm2 off the LAN, lets vm1 alone back, then lifts it |
| `skew` | vm2's clock 3 min ahead is ordered by it; 6 min ahead is accepted, clamped to 5 min, and the doctor shows the skew | link/2 | the network | `simulate host-skew` | vm2's clock, with NTP off, then back on |
| `networks` | vm1 in N1 and N2, vm2 in N2 only: vm2 sees nothing of N1 | link/2, networks | the network (N2), and N1 | the several-networks tests | two stand-in sessions |
| `restore` | a restored state dir starts a new incarnation and doesn't freeze | | the network | `simulate net-frozen` | copies vm2's `~/.local/state/xenonet` aside, writes past it, restores it, drops the copy |

**How they test it.** A check messages and hands off between *stand-in
sessions*: a made-up Claude Code session in a VM's `~/Projects/atlas-api`
(or `atlas-web`), which XenoNet hears of through `xenonet hook claude` and
which calls the engine's tools (`peers`, `send`, `inbox`, `handoff`,
`claim`) through `xenonet mcp`, as a real session would. A message is a
loose entry, a handoff a kept one. The partitions are nft rules in a table
of their own (`xenonet_lab_check`), which every `check` lifts first and a
reboot forgets. The clock is set as root, from the Mac's time.

**What they read** from `xenonet net status --json` in each VM, besides
what's there today (`network`, `hosts[].id`, `hosts[].online`, `frozen`):
`networks[]` once a machine may be in several, the network's `relays`
(URLs) and `mailboxes` (host ids), `hosts[].path` (`direct` or `relay`),
`hosts[].found` (how its address was found, `mdns` among them),
`hosts[].skewMs` (that machine's clock ahead of this one's, measured on the
hello) and `self.hlcAheadMs` (this machine's HLC ahead of its own clock).
The skew check also looks for vm2 in `xenonet doctor --json`, and the relay
checks read its relay rows (`network.relay.<host:port>`), `relaysHere.url`
(the machine's home relay) and the relays' metrics. The phase 5
commit that adds each one names it so, or changes `transport.mjs` with it.

### Setting up the network (you)

XenoNet refuses creating a network, joining one, and widening one (a relay,
a mailbox) from under an agent, so these are yours. The checks read the
VMs' state and say which is missing.

1. **The network.** In the Mac's XenoNet window, start a network, then
   **Add a machine** with host id `vm1`, and run the line it shows inside
   the VM: `limactl shell xenonet-vm1`, then paste it. The same for `vm2`.
   Say yes to finding machines on the LAN (mDNS) when it asks.
2. **The relays** (for `relay`, `relay-sealed`, `relay-unlisted` and
   `relay-down`). Run `npm run lab -- check --only relay` once: it builds
   and starts relays `a` and `b` in vm1, has both VMs trust the lab's CA,
   and says their URLs, `https://<vm1's lima0 address>:3443` and `:4443`.
   Add both as relays of the network in the Mac's window (**Network
   settings › Relays › Add to** the network; it asks you, and the founder
   or a second window host approves), then run
   `npm run lab -- check --only relay,relay-sealed,relay-unlisted,relay-down`.
   The Mac doesn't need the lab's CA: only the VMs reach the relays. Golden
   copies built before the relay checks lack tcpdump: build them again, or
   install it in vm1 (`sudo apt-get install -y tcpdump`), and
   `relay-sealed` says so until then.
3. **The mailbox** (for `mailbox`). Name the Mac as a mailbox of the
   network in its window (Network settings).
4. **A second network** (for `networks`). Make N1 of the Mac and vm1 only,
   and share `atlas-web` into N1 and no other network. The first network
   stays N2, with vm2 and `atlas-api`.

An agent doesn't do these. It looks at the same states in XenoNet Dev
with the `simulate` kinds in the table (AGENTS.md), and at the tests named
there.

macOS may ask once, in a window, whether an app may reach the local
network: your terminal, the first time you run `--only mac-udp` (which
`check` skips unless it's named, so an agent's run never opens that
window), and XenoNet when its endpoint first uses the LAN. Both are yours
to allow.

## Signing in (the owner, after the VMs are built)

The lab never signs in to anything and never copies a sign-in between
machines. Sign in inside each VM yourself, with the same accounts as the
Mac. Run these in a terminal on the Mac; for `vm2`, write `xenonet-vm2` for
`xenonet-vm1`.

**Claude Code**

1. `limactl shell xenonet-vm1` opens a shell in the VM, as `will`.
2. Run `claude`. Answer the first-run questions (the theme, and whether to
   trust the folder), and pick the claude.ai account (Pro, Max, Teams or
   Enterprise) when it asks how to log in.
3. No browser can open in the VM, so Claude Code prints a login URL.
   Press `c` to copy it (or select it in the terminal), and open it in the
   Mac's browser. Sign in with the Mac's account and authorize.
4. The page shows a code. Paste it into the VM's terminal at
   `Paste code here if prompted`, and press Enter. Claude Code says
   `Login successful`; press Enter again.
5. Leave with `/exit`, then `exit`. The sign-in is in the VM's
   `~/.claude/.credentials.json`, readable by `will` alone.

**Codex**

1. On the Mac, make sure nothing is using port 1455 (the Mac's own
   `codex login` uses it while it runs): `lsof -nP -iTCP:1455 -sTCP:LISTEN`
   prints nothing.
2. Connect with the port forwarded into the VM, on a connection of its
   own: through Lima's shared connection, the forward would stay open
   after you leave.
   `ssh -F ~/.lima/xenonet-vm1/ssh.config -o ControlPath=none -L 1455:localhost:1455 lima-xenonet-vm1`
3. In that shell, run `codex login`. It starts a login server on the VM's
   port 1455 and prints a URL.
4. Open the URL in the Mac's browser and sign in with the Mac's ChatGPT
   account. The browser's redirect to `localhost:1455` reaches the VM
   through the forward, and Codex says it's signed in.
5. `exit`. The sign-in is in the VM's `~/.codex/auth.json`; the lab's
   `~/.codex/config.toml` keeps it in that file rather than a keyring, as on
   the Mac.

Then `npm run lab -- status` shows both VMs as before, and the real XenoNet
in each (port 47840) starts showing that VM's sessions.

## Changing things

- **Another branch of XenoNet**: `npm run lab -- push --ref <branch>`.
- **Newer tools**: change `versions.json` and build the golden copies again.
- **A VM without `lima0`** (made from a template without `networks`):
  `limactl stop xenonet-vm1`, then
  `limactl edit --set '.networks=[{"vzNAT": true}]' xenonet-vm1`, then
  `npm run lab -- up vm1`.

## Resets and teardown

- **Hard reset of a working VM** (it loses its sign-ins): sign out inside
  it, `limactl stop xenonet-vm1`, `limactl delete xenonet-vm1`, then
  `npm run lab -- clone vm1`.
- **Checkpoint and restore**, by hand: stop the VM, then
  `limactl clone xenonet-vm1 xenonet-vm1-ckpt`. To restore,
  `limactl delete xenonet-vm1`, then
  `limactl clone xenonet-vm1-ckpt xenonet-vm1 --ssh-port 60851` and
  `npm run lab -- up vm1`. The restored VM keeps its host key.
- **Rebuild a golden copy**: `limactl unprotect xenonet-base-ubuntu`,
  `limactl delete xenonet-base-ubuntu`, then `npm run lab -- build ubuntu`.
- **Teardown**: sign out inside each VM you signed in to (`/logout` in
  Claude Code, and `codex logout`), so its tokens don't outlive it. Then
  `npm run lab -- teardown --yes`. It deletes the lab's instances by name,
  checkpoints named as above included, never by pattern, and refuses while
  a running VM still has a sign-in.
  Lima's cached OS images stay; don't run `limactl prune`, which empties
  Lima's whole cache.

## Disk

Clones share blocks with their golden copy, and `du` counts shared blocks
once for each clone, so check free space with `df`. A VM's disk file
doesn't shrink when the guest deletes files; cloning it again from its
golden copy does.
