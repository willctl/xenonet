# The XenoNet lab

Linux VMs on the development Mac, so one XenoNet can be tested against
several hosts. Each VM runs Claude Code, the Codex CLI and a headless
XenoNet, and the XenoNet on the Mac reaches it over SSH with `xenonet relay`
(`docs/design/hosts.md`). `docs/design/vm-lab.md` is the plan; this is
Phase A of it, the two Linux hosts.

```sh
npm run lab -- build ubuntu --ref wA/merge   # the golden copies, once
npm run lab -- build fedora --ref wA/merge
npm run lab -- clone vm1                     # the working VMs
npm run lab -- clone vm2
npm run lab -- check                         # the plan's checks
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
- **Time zones** come from a template parameter, `labTimezone`, which the
  VM's root setup applies at every boot: Lima's own `timezone` setting does
  nothing in plain mode.
- `scripts/lab/config.mjs` has the table, and `scripts/lab/lima/` the
  templates (`lab.yaml` holds what the two share).

## Commands

| `npm run lab -- …` | What it does |
|---|---|
| `status` | The lab's VMs, the XenoNets in the running ones, and the lab's files |
| `build ubuntu\|fedora [--ref R]` | Builds a golden copy: creates it, installs the tools and XenoNet at git ref R (HEAD by default), checks the demo XenoNet answers, then stops and protects it. A build that stopped halfway picks up where it was |
| `clone vm1\|vm2` | Clones a working VM, starts it, records its host key, installs the console key, puts made-up repos in `~/Projects`, and starts its two XenoNets |
| `up [vm…]` / `down [vm…]` | Starts or stops working VMs. `up` records the host key again |
| `push [vm…] [--ref R]` | Pushes XenoNet at git ref R into running VMs, reinstalls its dependencies, and restarts its services |
| `keys [vm…] [--relay-port N]` | Points the console key at the XenoNet under test: 47841, the demo XenoNet (the default), or 47840 |
| `check [vm…]` | The plan's checks (below) |
| `teardown [--yes]` | Deletes the lab's VMs, by name, and the lab's files |

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
  (the VMs have no GitHub access), with `npm ci --omit=dev` and the `xenonet`
  command in `~/.local/bin`.
- **Two XenoNets**, as systemd user services that run without anyone logged
  in (linger): `xenonet.service` on 127.0.0.1:47840, which reads the VM's own
  sessions, and `xenonet-demo.service` on 127.0.0.1:47841, with made-up data
  for the hosts prototype's first runs. They serve the API only; the page
  isn't built. `~/.config/xenonet-lab/xenonet.env` holds a VM's settings: `vm2`
  has `XENONET_CLAUDE_USAGE=off`, so only `vm1` asks Anthropic for plan usage.
- **Made-up repos** in `~/Projects` (`fixtures.mjs`), from the demo world's
  names and remotes: `atlas-api` on both VMs with the same remote and the
  same commits; `atlas-web` and `labs/geofence` on `vm1`; `ledger` and
  `docs` on `vm2`; and on both, `Café Zones` (a space and an accent), a
  worktree beside its repo (`atlas-api-refunds`) and a pnpm workspace
  (`atlas-kit`). The remotes are never fetched from or pushed to.

## On the Mac

- **The lab's folder**, `~/Library/Application Support/XenoNet Dev/lab`
  (`XENONET_LAB_DIR` moves it):
  - `ssh_config`, with an alias per VM (`vm1`, `vm2`);
  - `console_ed25519`, the console key, made for the lab;
  - `known_hosts`, with each VM's host key.
- **Each VM's Lima folder**, `~/.lima/<instance>`, which also holds
  `xenonet-lab-password`: the VM's sudo password (mode 0600). Only its hash
  went into the VM.
- Nothing is added to `~/.ssh/config`.

## SSH

- **XenoNet's hosts use the lab's own config:** `ssh -F <the lab's ssh_config>
  vm1`. It offers the console key alone, never the Mac's agent, and checks
  host keys against the lab's `known_hosts`.
- **The console key can run the relay and nothing else.** In each VM's
  `~/.ssh/authorized_keys` it reads
  `restrict,command="/home/will/.local/bin/xenonet relay --port 47841" …`,
  so connecting with it gets the relay's hello, whatever command is
  asked. (On a branch from before the relay, it prints
  `xenonet: no command "relay"`, which still shows the key and the host key
  work.)
- **The command `status` prints for each VM** runs as printed:
  `ssh -F ~/'Library/Application Support/XenoNet Dev/lab/ssh_config' vm1`,
  the `~` outside the quotes, where the shell expands it, since XenoNet
  Dev's state folder has a space in its name.
- **Host keys are recorded through Lima's channel**, from the VM's
  `/etc/ssh`, so nothing is trusted blindly the first time. Lima gives a VM
  a new instance id at every start, which would have cloud-init make new
  host keys each time; the templates keep them (`ssh_deletekeys: false`).
- **For a shell in a VM**, use Lima's channel: `limactl shell xenonet-vm1`,
  or `ssh -F ~/.lima/xenonet-vm1/ssh.config lima-xenonet-vm1`.

## Networking and root

- **Plain mode**: no mounts, no automatic port forwards, no guest agent.
  Nothing in the Mac's home folder is visible in a VM, and nothing a VM
  listens on appears on the Mac.
- **The Mac's loopback is blocked.** Lima's network maps 192.168.5.2 in a
  guest to the Mac's 127.0.0.1, where XenoNet's API listens. nftables rules,
  loaded before the network at every boot (`xenonet-lab-firewall.service`),
  drop the TCP connections and UDP a guest starts toward it. They look at
  TCP's SYN flag rather than conntrack's state, so the SSH connections the
  Mac opens into the VM are never caught, even one open before the rules
  load. DNS (Lima serves it at 192.168.5.2 too) and DHCP still work.
- **Root needs the VM's password**, which only the Mac holds, so an agent
  in the VM can't lift the rule. The two exceptions are commands Lima runs:
  `sudo cat /mnt/lima-cidata/param.env` at every start, and
  `sudo /sbin/shutdown -h now`. To run something as root yourself, with
  the password on standard input and no terminal to echo it:
  `ssh -F ~/.lima/xenonet-vm1/ssh.config -T lima-xenonet-vm1 sudo -S -p "''" <command> < ~/.lima/xenonet-vm1/xenonet-lab-password`.
- SSH takes keys only, with no agent or X11 forwarding into a VM.

## The checks

`npm run lab -- check` runs the plan's checks in each running VM, nine in
each (eighteen with both VMs up), one `ok` or `FAIL` line apiece, and
fails if any does:

1. both XenoNets answer, on 127.0.0.1:47840 and :47841 (two checks);
2. a server on the Mac's 127.0.0.1 can't be reached through 192.168.5.2,
   while the internet can (two);
3. XenoNet listens on 127.0.0.1 only, and nothing is mounted from the Mac
   (two);
4. the console key gets the relay, not a shell;
5. only made-up names: the lab user is "Will", git's is Nova, and the
   Mac's own names appear nowhere in the VM's users, groups, git config or
   authorized keys;
6. `sudo` without the password is refused, and with it works.

## Adding the VMs to a console (the owner)

XenoNet refuses `xenonet hosts add` from under an agent, so adding the lab's
VMs to a console is yours; an agent checking the page uses XenoNet Dev's
`host-add` simulation instead. On the Mac, with XenoNet Dev started with
`XENONET_HOSTS=1`, give `xenonet hosts add` (with `--dev` while the installed
XenoNet runs too) what the lab's ssh config says of each VM:
`will@127.0.0.1` as the target, its SSH port (`--ssh-port`,
from the table above), the console key (`--identity`, `console_ed25519`
in the lab's folder), the lab's `known_hosts` (`--known-hosts`), and the
name its host key is kept under there (`--host-key-alias xenonet-lab-vm1`,
or `xenonet-lab-vm2`). The host key is in that file already, so there's
none to accept. The console key runs the relay for the demo XenoNet
whatever is asked, until `npm run lab -- keys --relay-port 47840` points
it at the real one.

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
  `push` names each host with `xenonet hosts self` (`vm1`, `vm2`), on a
  branch that has it.
- **The relay's port**: `npm run lab -- keys --relay-port 47840` once tests
  move from the demo XenoNet to the real one.
- **Newer tools**: change `versions.json` and build the golden copies again.

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
