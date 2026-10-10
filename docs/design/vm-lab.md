# The VM lab

A plan for virtual machines on the development Mac, so XenoNet can be tested
against several hosts. Each VM runs Claude Code and the Codex CLI, signed in
to the same accounts as this Mac, with a headless XenoNet beside them. The
XenoNet on this Mac reaches each VM's XenoNet over SSH.

Written on 2026-10-05 for merge point A (workstream 4 in `next-phase.md`),
when nothing had been installed, downloaded or created. The owner approves
the plan, and then signs in inside each VM. Credential files are never
copied between machines.

The hosts lane's `hosts.md` recommends a hub over SSH. The console runs
`ssh <host> xenonet relay`, and the host's XenoNet stays on 127.0.0.1. The lab
is built for that, and its prototype runs ([below](#the-hosts-prototypes-runs))
are the lab's first job.

**Status.** Phase A, the two Linux hosts, is built: `scripts/lab/` and
`npm run lab`, which `scripts/lab/README.md` says how to use. Phase B, the
macOS guest, is later, once disk is free. The plan below is as it was
approved; merge point A and the build settled these:

- Linux VMs first, the Ubuntu and Fedora golden copies with `vm1` and
  `vm2` cloned from them, as section 2 has it.
- Nothing is added to `~/.ssh/config` (question 3): XenoNet's hosts get an
  ssh config of their own, and the lab writes its own, with an alias per
  VM, in XenoNet Dev's state folder under `lab/`.
- The VMs use this Mac's accounts (question 5), and Codex signs in through
  a forwarded port, with no change to the account (question 6). The owner
  signs in; the lab never does, and never copies a sign-in.
- `npm run lab -- check` runs the checks of section 2 as nine in each VM,
  with one more: root needs the VM's password, which only the Mac holds.
- Adding the VMs to a console as hosts is the owner's: XenoNet refuses it
  from under an agent.

**In short**

- **Lima for every VM.**
  - It's already installed.
  - It sets up Linux VMs from start to finish, from cloud images. The two
    images the lab needs are already in its cache.
  - Since version 2.1 it also runs macOS guests, as an experimental
    feature.
  - Tart is the fallback for the macOS guest. UTM, also installed, is the
    manual fallback.
- **Three working VMs.** Each is cloned from a stopped copy that was never
  signed in:
  - Ubuntu 24.04, with 2 vCPUs and 4 GiB of memory;
  - Fedora 44, with 2 vCPUs and 4 GiB;
  - macOS 26.5, with 4 vCPUs and 8 GiB.
- **Networking.**
  - Inside each VM, XenoNet listens on 127.0.0.1 only.
  - This Mac reaches it over SSH, through the relay's own connection:
    nothing is forwarded, and nothing new listens.
  - Lima's network also lets a guest reach this Mac's own 127.0.0.1,
    XenoNet's API included, so each guest blocks that, and an agent in the
    guest can't lift the block.
- **Disk is the limit.** This Mac has 78 GiB free.
  - The Linux half needs about 10 GB of disk and about 1.5 GB of
    downloads, none of them an OS image.
  - The macOS guest needs a 19.8 GB restore image, and about 30 GB once
    built.
  - The whole lab fits today, but it would leave the disk about 96% full.
    Build the Linux VMs first, and the macOS guest once space has been
    freed.

## This Mac today

Checked read-only on 2026-10-05. This Mac's disk is given in GiB; downloads
are in GB, as their sources give them.

| Item | Today |
|---|---|
| Hardware | Apple M3 Max: 16 cores (12 performance, 4 efficiency), 64 GB of memory |
| macOS | 26.5.2 (25F84) |
| Disk | 926 GiB, of which 78 GiB is free (the data volume is 92% full) |
| Lima | 2.1.2, from Homebrew. The latest stable release is 2.2.1, and 2.3.0-beta.1 came out the same day ([releases][lima-releases]) |
| UTM | 4.7.5, the latest stable release ([releases][utm-releases]), with `utmctl` on the PATH. 5.0.6 is in beta |
| Docker Desktop | 4.52.0. Its Linux VM holds about 7.6 GiB of memory, and its disk image takes 56 GiB |
| Not installed | Tart, OrbStack, Apple's `container` and QEMU. Lima's `vz` driver doesn't need QEMU |
| Lima's download cache | 2.5 GiB. It holds the exact Ubuntu 24.04 image (about 0.6 GB) and Fedora 44 image (about 0.5 GB) that Lima 2.1.2's templates pin, so those VMs download no image |
| Other VMs | UTM holds two QEMU Ubuntu VMs, using 7.4 GiB. The lab doesn't use them: they're the owner's other work, and they aren't scripted |
| Agents and tools | Claude Code 2.1.288 (the native install), Codex CLI 0.146.0 (from npm), Node 22.22.3, git 2.50.1 |

Two Virtualization.framework VMs are running. Neither is macOS, so both
macOS slots (see below) are free.

- **Docker Desktop's Linux VM**, running since September.
- **A Lima Linux VM from another project**, running since August.
  - Its instance folder has since been deleted, so `limactl list` no longer
    shows it and `limactl` can't stop it.
  - Its disk was deleted while still open, so the space stays in use until
    the VM's process ends.
  - Like any Lima guest, it can reach this Mac's loopback services (see
    [Networking](#networking)).
  - What to do with it is the owner's call. The lab doesn't touch it.

## 1. The tools, compared

| | Lima | UTM | Tart | OrbStack | Apple's `container` |
|---|---|---|---|---|---|
| On this Mac | 2.1.2 | 4.7.5 | No. 2.40.1 is a 23 MB download ([releases][tart-releases]) | No | No. 1.5.0 comes as a 118 MB package ([releases][container-releases]) |
| macOS guests | Yes, experimental since 2.1 ([docs][lima-macos]). See [the notes below](#notes-on-the-comparison) | Yes, with the Apple backend, set up in the GUI wizard ([docs][utm-macos]) | Yes, and mature. Prebuilt images (the Tahoe base image is about 25 GB, with user admin/admin, SSH and Homebrew), or `tart create --from-ipsw` ([quick start][tart-quick]) | No | No |
| Linux guests | Yes: cloud images of about ten distributions, Ubuntu, Debian and Fedora among them. The Tier 1 ones are tested in CI ([templates][lima-templates]) | Yes, with QEMU or the Apple backend, from ISOs | Yes: Ubuntu, Debian and Fedora images, each needing a disk of at least 20 GB | Yes, as "machines" that share one kernel inside one VM ([architecture][orb-arch]) | Containers from OCI images, each in its own lightweight VM ([overview][container-overview]) |
| Automation | `limactl` to create, start, stop, clone, copy, shell into and delete; YAML templates; cloud-init; provisioning scripts; plain mode ([docs][lima-plain]) | `utmctl` to start, stop, suspend, clone, delete, get IPs, and run commands through the QEMU guest agent. AppleScript can create VMs. There's no documented scripted macOS install ([reference][utm-scripting]) | A CLI: `clone`, `run --no-graphics`, `ip`, `exec` (through its guest agent), `set`, `suspend`, `push` and `pull` ([`run`][tart-run], [`exec`][tart-exec]) | The `orb` CLI and cloud-init ([docs][orb-machines]) | The `container` CLI, for OCI builds and runs |
| Networking | A user-mode network: the guest is 192.168.5.15. This Mac reaches the guest only over SSH or forwarded ports. The guest reaches this Mac's 127.0.0.1 through 192.168.5.2 ([docs][lima-user-net]). vzNAT and user-v2 are optional | Apple backend: shared or bridged. Port forwarding works only with QEMU's emulated VLAN ([docs][utm-ports]) | NAT by default, where a host service is reachable only if it listens on 0.0.0.0. Also Softnet isolation, bridged, and host-only ([FAQ][tart-faq]) | Ports appear on localhost, SSH is multiplexed, and the Mac's files are at `/mnt/mac` unless a machine is `--isolated` ([docs][orb-isolated]) | — |
| Snapshots and clones | No snapshots with `vz` (`limactl snapshot` is QEMU-only and experimental: [source][lima-vz-driver]). `limactl clone` copies a stopped VM copy-on-write on APFS ([source][lima-clone]) | None in 4.7.5. The 5.0 beta adds QEMU snapshots on macOS 26; snapshots of Apple-backend VMs need a macOS 27 host ([releases][utm-releases]) | Clones are copy-on-write ([source][tart-clone]). Suspend and resume need a macOS 14 host and `--suspendable` | `orb clone`, which copies data on demand; export and import | No |
| License and cost | Apache-2.0, free | Apache-2.0, free ([site][utm-site]) | FSL-1.1-ALv2: internal use is allowed, and each release becomes Apache-2.0 after two years ([LICENSE][tart-license]). Free since Cirrus Labs joined OpenAI ([announcement][cirrus]) | Free for personal, non-commercial use. Pro costs $8 per user per month ([pricing][orb-pricing]) | Apache-2.0, free ([repo][container]) |
| Fit for a scripted lab | Best for Linux, and good enough for macOS | A manual fallback for macOS | Best for macOS, but a new install | Poor. It has no macOS guests, the machines share a kernel, the Mac's files are mounted by default, and it overlaps Docker Desktop | Poor for long-lived hosts that need SSH and services. Fine for throwaway CLI tests |

### Notes on the comparison

- **Lima's macOS guests have limits** ([docs][lima-macos]):
  - the host must run the same macOS version as the guest, or a newer one;
  - the display window can't be turned off;
  - there's no automatic port forwarding;
  - the first-login setup screens are skipped only from Lima 2.3, which is
    still in beta.
- **Tart's license has changed.** Its website's licensing page
  ([page][tart-licensing-page]) still lists the old Fair Source tiers. Two
  newer sources replace it:
  - the repository's LICENSE (FSL-1.1-ALv2, copyright OpenAI);
  - Cirrus Labs' announcement of 2026-04-07, which says licensing fees have
    stopped.
- **Tart and UTM probably share Lima's version rule.** They use the same
  framework, so plan on the host being at least as new as the guest.
  Tart's prebuilt Tahoe images are rebuilt as new 26.x releases come out,
  so the newest may be newer than this Mac's 26.5.2. Check its version, or
  build from a 26.5.x restore image.

Other tools looked at:

| Tool | Why not here |
|---|---|
| Lume (MIT) | It runs scripted macOS VMs. Its unattended setup writes the account, SSH, automatic login and no-sleep settings straight to the guest's disk ([docs][lume]). It's another install for what Lima or Tart already do |
| Multipass | Canonical's tool for Ubuntu VMs, using QEMU by default on macOS ([docs][multipass]). Lima, already installed, does all it does and more |
| Docker Sandboxes (`sbx`) | It runs one agent in a microVM and needs a sign-in ([docs][docker-sbx]). A lab host needs SSH, a running service and a home folder of its own |
| Parallels Desktop | Paid. It supports macOS guests, with snapshots from version 20, under the same two-VM limit ([KB][parallels-macos]) |
| VMware Fusion | Free since November 2024 ([announcement][fusion-free]), but it runs no macOS guests on Apple silicon ([KB][fusion-arm]) |

### Apple's limit on macOS guests

- **What the license allows.** Section 2B(iii) of the macOS Tahoe 26
  license allows up to two more copies or instances of macOS in virtual
  machines on each Mac that already runs it ([license][apple-sla]).
  - The allowed purposes are software development, testing during software
    development, using macOS Server, and personal non-commercial use.
  - Service bureaus, time-sharing and similar services aren't allowed.
  - The lab counts as testing during development.
- **How macOS enforces it.** Virtualization.framework refuses to start a
  third macOS VM while two are running
  (`VZError.Code.virtualMachineLimitExceeded`, [docs][vz-limit]). Stopped
  VMs and Linux VMs don't count, so the lab's stopped copies and any number
  of Linux VMs are fine.
- **Today.** Neither slot is in use. The lab uses one and leaves the other
  free, for a second macOS host later or for other work.
- **A known bug.** On macOS 26.5, a reported bug keeps a slot taken after a
  guest shuts itself down, until the Mac restarts. An Apple engineer
  acknowledged it on the Apple Developer Forums (FB22967193,
  [thread][vz-slot-bug]); it isn't in Apple's documentation.
  - Stop macOS guests from this Mac, with `limactl stop`.
  - If a start fails with that error while no macOS VM is running, restart
    the Mac.

### Recommendation

- **Use Lima, already installed, for every VM.**
  - Linux VMs run in plain mode: no mounts, no automatic port forwarding,
    and no guest agent ([docs][lima-plain]). Each VM then looks like a
    remote machine that can be reached only over SSH.
  - The macOS guest uses Lima's experimental support, with its `macos-26`
    image (macOS 26.5, 25F71).
  - If the macOS guest fails on the first try, use Tart for it alone. Tart
    is one free 23 MB install. UTM's GUI wizard is the manual fallback,
    with nothing to install.
- **Stay on Lima 2.1.2.** It has everything this plan uses. Upgrading to
  2.2.1 is optional, and worth trying before Tart if the macOS guest
  fails. Don't use the 2.3 beta.
- **Don't install OrbStack or Apple's `container`.**

## 2. The lab

### The VMs

| Lima instance | OS | vCPU | Memory | Disk: virtual / used | Host id | SSH port | Role |
|---|---|---|---|---|---|---|---|
| `xenonet-base-ubuntu` | Ubuntu 24.04 LTS, arm64 | 2 | 4 GiB | 20 GiB / about 4–5 GB | | | Golden copy: stopped and protected, never signed in |
| `xenonet-base-fedora` | Fedora 44 Cloud, arm64 | 2 | 4 GiB | 20 GiB / about 4–5 GB | | | Golden copy |
| `xenonet-base-macos` | macOS 26.5 (25F71) | 4 | 8 GiB | 64 GiB / about 25–30 GB | | | Golden copy |
| `xenonet-vm1` | Clone of the Ubuntu golden copy | 2 | 4 GiB | Grows by 1–2 GB | `vm1` | 60851 | The common Linux host |
| `xenonet-vm2` | Clone of the Fedora golden copy | 2 | 4 GiB | Grows by 1–2 GB | `vm2` | 60852 | Another distribution: dnf, SELinux and a newer systemd |
| `xenonet-mac1` | Clone of the macOS golden copy | 4 | 8 GiB | Grows by 2–5 GB | `mac1` | 60853 | Claude desktop, the ChatGPT app, XenoNet Dev's desktop app and the macOS process checks |
| `xenonet-vm3` (optional) | Clone of the Ubuntu golden copy | 2 | 4 GiB | Grows by 1–2 GB | `vm3` | 60854 | The same OS twice, and scale |

- **Memory.** Claude Code needs at least 4 GB ([setup][claude-setup]).
  The macOS guest gets 8 GiB for Claude desktop, the ChatGPT app and XenoNet
  Dev.
- **The whole lab at once.** The three working VMs take 8 vCPUs and 16 GiB
  (20 GiB with `vm3`), out of 16 cores and 64 GB, next to Docker's
  7.6 GiB.
- **Ids and ports.**
  - Host ids follow `contracts.md`: `^[a-z][a-z0-9-]{1,31}$`, and never a
    machine's name. Each VM names itself with `xenonet hosts self <id>`, as
    `hosts.md` proposes.
  - The SSH port is where Lima's host agent forwards the guest's port 22,
    on this Mac's 127.0.0.1. It's fixed per VM (Lima's `ssh.localPort`),
    so each VM's SSH alias stays put.
  - Inside each VM, XenoNet uses its usual port: 47840, or 47841 when it runs
    with `--demo`.
- **Time zones.** `vm1` runs in UTC and `vm2` in a distant zone, such as
  Pacific/Auckland, so the hosts disagree on local time on purpose.
- **Golden copies are never signed in.** Each is built and stopped before
  anyone signs in, so a clone never carries a sign-in.

### What goes in each

| What | Linux VMs | macOS guest | Source | Size |
|---|---|---|---|---|
| A lab user | `will`, full name "Will", home `/home/will` | `will`, home `/Users/will` | Lima's `user` setting | |
| git | The distribution's package | Xcode Command Line Tools | apt or dnf; Apple | Small on Linux. The Command Line Tools take 1.8 GB on disk on this Mac |
| Node 22 (XenoNet needs 22.13 or later) | 22.23.3 from the official tarball, unpacked into `~/.local/node` and checked against `SHASUMS256.txt` | The same, for darwin-arm64 | [nodejs.org][node-22] | A 30 MB download (26 MB for macOS), about 0.2 GB on disk |
| Claude Code | The native installer, pinned to a version (`bash -s <version>`), with `DISABLE_AUTOUPDATER=1` | The same | [setup][claude-setup] | About 0.25 GB (the linux-arm64 build is 245 MB) |
| Codex CLI | `npm i -g @openai/codex@<version>`, into a user-level npm prefix | The same | [npm][codex-cli], [releases][codex-releases] | About 0.1 GB to download, 0.4 GB on disk |
| XenoNet | The branch under test, pushed from this Mac over SSH; `npm ci --omit=dev`; the `xenonet` command (`node cli/xenonet.js install`); a systemd user service | A full `npm ci` (Electron, for XenoNet Dev), the `xenonet` command, and a LaunchAgent for the headless server | This repo, npm | About 0.3 GB (the Agent SDK's linux-arm64 package is 244 MB). About 0.8 GB on macOS |
| Test repos | Made-up repos generated in `~/Projects` | The same | The demo world's fixtures | Under 10 MB |
| Claude desktop and the ChatGPT app | | From the makers' sites | claude.ai, openai.com | About 0.9 GB and 1.3 GB installed |

**Versions.**

- `scripts/lab/versions.json` pins Node, Claude Code and Codex.
- By default it pins what this Mac runs (Claude Code 2.1.288 and Codex
  0.146.0 today), so the lab tests what the owner uses.
- One VM can run the newest releases (2.1.289 and 0.160.0 today), to test
  versions drifting apart across hosts.

**Privacy and credentials.**

- **No real names in a guest.** Lima copies this Mac's user name and full
  name into each guest unless told otherwise. git falls back to that full
  name when `user.name` isn't set. The lab sets the user to `will`, named
  "Will", and gives git the demo world's made-up identities. That way no
  real name reaches a guest's paths, its `/etc/passwd`, or a commit made
  in it.
- **No mounts.** Lima's templates mount the home folder read-only by
  default. The lab's templates don't, so nothing from this Mac's home is
  visible in a guest: not `~/.claude`, not `~/.codex`, not SSH keys.
- **No borrowed credentials.**
  - There's no SSH agent forwarding and no GitHub sign-in in the VMs, and
    credential files are never copied in.
  - XenoNet's code reaches each VM by `git push` over SSH, so the VMs need
    no GitHub access.
  - The test repos' remotes are made up, and are never fetched from or
    pushed to.
- **Sign-ins.** The owner signs in inside each VM, after it's cloned.

**The headless XenoNet.**

```ini
# ~/.config/systemd/user/xenonet.service in a Linux VM
[Unit]
Description=XenoNet (lab host)

[Service]
ExecStart=%h/.local/node/bin/node %h/xenonet/server/index.js
Restart=on-failure

[Install]
WantedBy=default.target
```

- **When it runs.** Root runs `loginctl enable-linger will` during
  provisioning, so the service runs without anyone logged in.
- **What it serves.** The API only, on 127.0.0.1:47840 as XenoNet always
  does. `dist/` isn't built, so a request for the page gets a 503.
- **A demo XenoNet beside it.** The hosts prototype's first runs use
  `node server/index.js --demo` (127.0.0.1:47841): made-up data, and no
  sign-in needed. So those runs can start as soon as Phase A is built.
- **On the macOS guest.** A LaunchAgent runs the same server. When a test
  needs the UI, XenoNet Dev's desktop app runs beside it, on its own port.
- **Usage requests.** Every 5 minutes, XenoNet asks Anthropic for plan usage.
  So the lab doesn't multiply those requests, all VMs but one run with
  `XENONET_CLAUDE_USAGE=off`, unless a test is about usage across hosts.

**The test repos.**

- **Made up, from the demo world.** The repos are generated from the names
  and remotes in `core/demo.js`, which are already made up.
- **Where each repo goes:**
  - `atlas-api` is on all three hosts with the same remote, so its
    checkouts get one repo key and a bridge between them;
  - `atlas-web` and `labs/geofence` are only on `vm1`;
  - `ledger` and `docs` are only on `vm2`;
  - `pathfinder` is only on `mac1`.
- **Linux edge cases:**
  - a folder whose name has a space and an accent;
  - a worktree checked out beside its repo;
  - a pnpm workspace.
- **History.** Each repo gets a fixed, made-up history and a few
  uncommitted edits.

**Sessions.**

- **How they're made.** After sign-in, the lab script runs a few short
  sessions on each host, with made-up prompts (`claude -p …` and
  `codex exec …`). The owner can run them too.
- **Why.** They give the map live and finished sessions.
- **Cost.** They use the smallest model each CLI offers, and count against
  the owner's plans.

### Networking

```text
this Mac                                    vm1 (Lima's xenonet-vm1, plain mode)

XenoNet, the console                          XenoNet on 127.0.0.1:47840
  ssh -- vm1 xenonet relay  === SSH ===>        <- xenonet relay, a local client
  (the lab's ssh config, console key        Claude Code, the Codex CLI
   and known_hosts)                         ~/Projects: made-up repos

Lima's network
  192.168.5.2 -> this Mac's 127.0.0.1  <-X- nftables drops new connections
                                              to 192.168.5.2
```

**How this Mac reaches a VM.**

- **The SSH connection out is the relay's.** The console runs
  `ssh … -- vm1 xenonet relay --port 47840` (`hosts.md`). The stream comes back
  over that connection's standard output.
- **Inside the VM, the relay is a local client of its XenoNet**, so XenoNet's
  `Host` and `Origin` checks hold as they are.
- **Nothing is forwarded, and nothing new listens**, on this Mac or in the
  VM.

**SSH to each VM.**

- **One alias per VM.** The lab writes an ssh config of its own, with an
  alias per VM (`vm1`, `vm2`, `mac1`), in a `lab` folder in XenoNet Dev's
  state folder. One `Include` line in `~/.ssh/config` brings it in. Adding
  that line is the owner's call ([question 3](#5-questions-for-the-owner)).
- **Two keys.**
  - Lima's own key (`~/.lima/_config/user`) serves `limactl shell` and the
    lab script.
  - A console key, made for the lab, is installed in each VM restricted to
    the relay, as `hosts.md` suggests:
    `restrict,command="/home/will/.local/bin/xenonet relay --port 47840"`.
    It can run the relay and nothing else. The port is the XenoNet under
    test: 47841 for the demo runs, 47840 after them.
- **Host keys are checked.** Lima's own ssh config turns checking off
  (`StrictHostKeyChecking=no`, with known hosts in `/dev/null`:
  [source][lima-sshutil]); the lab's config doesn't.
  - The lab script records each VM's host key through Lima's own channel,
    so nothing is trusted blindly the first time.
  - A VM cloned again may get a new host key. The lab script replaces it
    the same way, or leaves it changed when a test wants that failure.
- **Logins.**
  - SSH takes keys only. The lab turns password logins off in every
    guest's `sshd`, since the lab user's password is there for sudo
    alone.
  - Agent forwarding is off everywhere.

```text
# The lab's ssh config (in XenoNet Dev's state folder, under lab/), written by the lab script
Host vm1
  HostName 127.0.0.1
  Port 60851
  HostKeyAlias xenonet-lab-vm1
  User will
  IdentityFile "~/Library/Application Support/XenoNet Dev/lab/console_ed25519"
  IdentitiesOnly yes
  UserKnownHostsFile "~/Library/Application Support/XenoNet Dev/lab/known_hosts"
```

**Port forwarding, for debugging only.**

- XenoNet answers only requests whose `Host` header is `127.0.0.1:<its port>`
  or `localhost:<its port>` (`server/index.js`). A TCP forward from
  another local port gets a 421, "bad host".
- So forward to a Unix socket and send the header yourself:
  `curl --unix-socket <socket> -H 'Host: 127.0.0.1:47840' http://vm1/api/health`.
  A Unix socket is also out of reach of this Mac's other users.
- Simpler still: `limactl shell xenonet-vm1 xenonet status`.

**Why plain mode.**

- Without it, Lima publishes every port a guest opens on 127.0.0.1 onto
  this Mac's 127.0.0.1, with no SSH in between. That would put each VM's
  XenoNet on this Mac's port 47840, where this Mac's own XenoNet listens.
- Plain mode turns that forwarding off ([docs][lima-port]).
- macOS guests have no automatic forwarding anyway.

**The other direction: the guest reaching this Mac.**

- **The gap.** Lima's user-mode network maps 192.168.5.2
  (`host.lima.internal`) to this Mac's 127.0.0.1 ([docs][lima-user-net],
  [source][lima-gvproxy]). So a guest can reach anything that listens on
  this Mac's loopback, XenoNet's API included. XenoNet's `Host` check stops
  browsers, but not a program that sets the header itself.
- **The block.**
  - In the Linux guests, an nftables rule, applied by root at every boot,
    drops new connections to 192.168.5.2. DNS still works, since Lima
    serves it on 192.168.5.3, and so do replies to SSH.
  - In the macOS guest, a pf anchor does the same, if the guest gets the
    same network. Check for 192.168.5.2 on the first build.
- **Keeping the block in place.**
  - The lab user's sudo needs a password, except to shut down. That's how
    Lima's macOS guests already work.
  - Each Linux VM's password is random. Only its hash goes into the VM, as
    a template parameter. The password itself stays on this Mac, in the
    VM's Lima folder (mode 0600), for the lab script's root tasks: the
    clock and slow-link tests, and updates.
  - So an agent in a guest can't lift the rule.
  - The macOS guest's generated password is changed, and its `~/password`
    file deleted, because any program in the guest can read that file.
- **For the hosts lane.**
  - This Mac's XenoNet should also refuse, or trust less, any connection that
    comes from a VM's network process.
  - On macOS, the process check can already name that process.

**Between guests, and the one forward.**

- **Guests can't reach each other.** Each Lima instance has its own
  user-mode network. If a later test needs guests to talk to each other,
  for example peer to peer, Lima's `user-v2` network adds that, with names
  such as `lima-xenonet-vm1.internal` ([docs][lima-user-v2]).
- **The one forward** is for Codex's browser sign-in, which needs this
  Mac's port 1455 forwarded into the VM once (see
  [Signing in](#signing-in-owner-only)).

A sketch of the Linux template. Fedora's differs only in its image, its
package manager, and its SSH service's name (`sshd`).

```yaml
# scripts/lab/lima/ubuntu.yaml
minimumLimaVersion: 2.1.0
base:
- template:_images/ubuntu-24.04   # images only, without Lima's home-folder mount
vmType: vz
cpus: 2
memory: 4GiB
disk: 20GiB
plain: true                       # no mounts, no automatic forwards, no guest agent
mounts: []
timezone: UTC
user:
  name: will
  comment: Will
  home: /home/will
ssh:
  overVsock: false                # always 127.0.0.1:<localPort>, set per clone
  loadDotSSHPubKeys: false
  forwardAgent: false
param:
  labSudoHash: ""                 # given at create or clone time: --param labSudoHash=…
provision:
- mode: data                      # written at every boot, before the scripts
  path: /etc/xenonet-lab.nft
  content: |
    table inet xenonet_lab
    delete table inet xenonet_lab
    table inet xenonet_lab {
      chain out {
        type filter hook output priority 0;
        ip daddr 192.168.5.2 udp dport { 53, 67 } accept
        ip daddr 192.168.5.2 tcp dport 53 accept
        ip daddr 192.168.5.2 ct state new drop
      }
    }
- mode: system                    # runs as root at every boot
  script: |
    #!/bin/bash
    set -eux -o pipefail
    # The firewall, git, and xz for Node's tarball, on the first boot.
    if ! command -v nft >/dev/null || ! command -v git >/dev/null; then
      apt-get update && apt-get install -y nftables git xz-utils
    fi
    nft -f /etc/xenonet-lab.nft
    # Root needs the lab's password, which only this Mac holds, except to
    # shut down. In sudoers the last matching line wins, so the exception
    # comes last.
    [ -n "$PARAM_labSudoHash" ] && usermod -p "$PARAM_labSudoHash" will
    printf '%s\n' 'will ALL=(ALL) ALL' 'will ALL=(root) NOPASSWD: /sbin/shutdown -h now' \
      > /etc/sudoers.d/90-cloud-init-users
    # That password is for sudo only: SSH takes keys and nothing else.
    printf '%s\n' 'PasswordAuthentication no' 'KbdInteractiveAuthentication no' \
      > /etc/ssh/sshd_config.d/10-xenonet-lab.conf
    systemctl try-reload-or-restart ssh
    loginctl enable-linger will
```

- **The tools** (Node, Claude Code, Codex, XenoNet and the `xenonet` command)
  go in once, over `limactl shell`, from the lab script, at the versions
  in `versions.json`. They aren't installed at every boot.
- **Clones** get their own SSH port and password:
  `limactl clone xenonet-base-ubuntu xenonet-vm1 --ssh-port 60851 --param labSudoHash=… --start`.
- **The macOS template** bases itself on `template:_images/macos-26`. It
  sets 4 vCPUs, 8 GiB, a 64 GiB disk, no mounts and the same user, and
  keeps the display on, which Lima needs for macOS guests.
- **Check on the first build:**
  - that `limactl stop` still shuts a VM down cleanly with this sudo rule;
  - that the user settings apply to the macOS guest as well.

**Checks when the lab is up.**

1. `limactl shell xenonet-vm1 xenonet status` shows the VM's projects and
   sessions. With the hosts prototype, `xenonet hosts` on the console shows
   `vm1` as live.
2. In each guest, while this Mac's XenoNet Dev is running,
   `curl -m 3 -H 'Host: 127.0.0.1:47842' http://192.168.5.2:47842/api/health`
   fails, while `curl -sI https://api.anthropic.com` works.
3. In each guest, `ss -ltn` shows XenoNet on 127.0.0.1 only, and `mount`
   shows nothing shared from this Mac.
4. `ssh vm1 true` with the console key gets the relay, not a shell.
5. In each guest, `getent passwd will` and `git config user.name` show only
   made-up names.

### What the lab tests

- **Several hosts at once:**
  - host-qualified ids;
  - one repo checked out on three hosts, joined by bridges;
  - one Claude account and one ChatGPT account across hosts as one
    identity each.
- **A host coming and going:**
  - a VM that stops and starts;
  - a relay that pauses;
  - a slow link, made with `tc netem` on `vm2`.
- **Clocks.**
  - Plain mode has no clock sync with this Mac, so after this Mac sleeps, a
    guest's clock runs behind until NTP catches up.
  - The hosts are also in different time zones.
- **XenoNet on Linux for the first time** (the README says it hasn't been
  tested there):
  - paths;
  - the doctor;
  - identities, read from `~/.claude/.credentials.json` and
    `~/.codex/auth.json`.
- **macOS-only code on a second Mac:**
  - the process checks, where the relay comes from under `sshd`;
  - Claude Code's sign-in in the keychain;
  - Claude desktop's and the ChatGPT app's files;
  - XenoNet Dev's window, Dock icon and notifications.
- **Where Codex keeps its sign-in.** On the macOS guest, the default
  `auto` setting tries the keychain first. This Mac uses
  `~/.codex/auth.json`.
- **Versions drifting across hosts.** One VM runs newer Claude Code and
  Codex releases, to exercise the drift report.

### The hosts prototype's runs

`hosts.md` lists eight runs for the VM lab. Here's how the lab does each.

| Run in `hosts.md` | In the lab |
|---|---|
| 1. Set up: a Linux guest with the branch, running `node server/index.js --demo`, added with `xenonet hosts add vm1 <alias>` | Phase A, with the demo XenoNet on 47841. No sign-in needed. The alias is `vm1` |
| 2. Watch for ten minutes; measure bytes with `-C` and without | Nothing special |
| 3. Suspend the guest, then resume it | Lima has no suspend. `pkill -STOP -f 'xenonet relay'` in the guest, through `limactl shell`, makes the host go stale, and `pkill -CONT` brings it back. `limactl stop` and `start` make it go offline and come back |
| 4. Set the guest's clock three minutes ahead | `timedatectl set-ntp false`, then `set-time`, as root through the lab script |
| 5. A restricted key | The console's key is restricted from the start |
| 6. Failures say what to do | A changed host key: clone the VM again. A refused key: take it out of `authorized_keys`. No relay: move the `xenonet` command aside. An unknown protocol: run an older branch |
| 7. Two guests at once | `vm1` and `vm2` |
| 8. Optional: Tailscale, and a macOS guest with real sessions | Phase B for the macOS guest. Tailscale would need the owner to sign in inside each guest, and isn't in this plan |

## 3. The steps

### First, the owner decides

| # | Decision | Why |
|---|---|---|
| 0.1 | Approve the plan, its downloads and its disk use | Nothing happens before this |
| 0.2 | Free disk before the macOS guest, or build only the Linux VMs for now | See [question 1](#5-questions-for-the-owner) |
| 0.3 | Add one `Include` line to `~/.ssh/config` for the lab's aliases | See [question 3](#5-questions-for-the-owner) |
| 0.4 | Which accounts sign in on which VM | The same as this Mac's, unless one VM tests a second account |
| 0.5 | How Codex signs in on the Linux VMs: device code, which is a ChatGPT account setting, or a port forward, which changes nothing | See [Signing in](#signing-in-owner-only) |
| 0.6 | Whether to keep Lima 2.1.2 or upgrade it to 2.2.1 | Optional |

### Phase A: the Linux hosts

Phase A downloads about 1.5 GB (the tools, once for each VM) and uses about
10 GB of disk.

| # | Step | Who | Download | Disk | Source |
|---|---|---|---|---|---|
| A1 | Put the lab into the repo under `scripts/lab/`: the Linux and macOS templates, `versions.json`, the fixture generator, and `lab.mjs`. `lab.mjs` builds, starts, stops, shows status, resets, checkpoints and tears down, and writes the lab's ssh config, console key and known hosts. Add an `npm run lab` script | Agent (Wave 2, `w2/hosts`) | None | None | This plan |
| A2 | `limactl create --name xenonet-base-ubuntu scripts/lab/lima/ubuntu.yaml`, then `limactl start xenonet-base-ubuntu` | Agent | None for the image, which is cached. Packages, about 0.1–0.3 GB | About 3 GB | [Ubuntu image][ubuntu-img] |
| A3 | Over `limactl shell`, install the pinned tools (Node, Claude Code, Codex), push XenoNet, run `npm ci --omit=dev`, and install the `xenonet` command and the service unit. Then `limactl stop` and `limactl protect` | Agent | Node 30 MB, Claude Code about 0.25 GB, Codex about 0.1 GB, npm about 0.1 GB | About 1–2 GB more | [Node][node-22], [Claude Code][claude-setup], [Codex][codex-releases] |
| A4 | Repeat A2 and A3 for `xenonet-base-fedora` | Agent | As A2 and A3, with the cached image | About 4–5 GB | [Fedora image][fedora-img] |
| A5 | Clone `xenonet-vm1` and `xenonet-vm2`, each with its SSH port and password. Install the console key, record the host keys, and run `xenonet hosts self vm1` (and `vm2`) | Agent | None | Close to nothing at first | [`clone`][lima-clone] |
| A6 | The hosts prototype's runs 1 to 7, on the demo XenoNets | Agent | None | Small | `hosts.md` |
| A7 | Sign in to Claude Code and Codex on `vm1` and `vm2` | **Owner** | None | None | [Signing in](#signing-in-owner-only) |
| A8 | Generate the test repos, start the real XenoNet, run the checks above, and run a few short sessions | Agent | None | Small | |

Cost: nothing for the tools. The sessions count against the owner's Claude
and ChatGPT plans.

### Phase B: the macOS guest

Phase B downloads about 23 GB and uses about 30 GB of disk, with a peak of
about 50 GB while the golden copy is built.

| # | Step | Who | Download | Disk | Source |
|---|---|---|---|---|---|
| B0 | Check the free space. On today's numbers, this Mac would have about 20–25 GB free at Phase B's peak, and about 40 GB (96% full) once it's done | **Owner** | | | [Question 1](#5-questions-for-the-owner) |
| B1 | `limactl create --name xenonet-base-macos scripts/lab/lima/macos.yaml`, then `limactl start xenonet-base-macos`. This downloads the macOS 26.5 (25F71) restore image and installs it | Agent | 19.8 GB (19,769,944,180 bytes, as Apple's CDN reports it) | 19.8 GB in Lima's cache, plus about 20 GB of disk | [Lima's macOS guests][lima-macos], [image][ipsw-265] |
| B2 | Log in for the first time in the VM's window and go through the setup screens. Change the generated password, and delete `~/password`. Turn off automatic macOS updates, so the guest never gets newer than this Mac. Optionally, turn on automatic login, so agents can drive XenoNet Dev's window | **Owner** | | | |
| B3 | Over SSH, install the Command Line Tools, Node, Claude Code and Codex. Add the pf anchor, turn off password logins in `sshd`, push XenoNet, run a full `npm ci`, and add the `xenonet` command and the LaunchAgent. Download Claude desktop and the ChatGPT app and copy them into `/Applications` | Agent, with sudo from the owner during this step only | Command Line Tools about 1 GB (1.8 GB on disk). Apps about 1 GB (2.2 GB on disk). Tools about 0.5 GB. npm about 0.3 GB | About 6 GB | [Claude Code][claude-setup] |
| B4 | `limactl stop`, then `limactl protect`. Delete the restore image from Lima's cache (see [Teardown](#teardown)) | Agent | | Frees 19.8 GB | |
| B5 | Clone `xenonet-mac1` with its SSH port, start it, install the console key, record the host key, and run `xenonet hosts self mac1` | Agent | | Close to nothing at first | |
| B6 | Sign in to Claude desktop and the ChatGPT app. Then sign in to Claude Code and Codex in Terminal, inside the VM's window | **Owner** | | | [Signing in](#signing-in-owner-only) |
| B7 | Start the headless XenoNet and add `mac1` to the console. XenoNet Dev runs when a test needs the UI | Agent | | | |

- **Newer restore images.** A 26.5.2 image (19.8 GB) would match this Mac
  exactly. A 26.6.2 or 27.0.1 guest (26.6 GB) needs this Mac updated
  first. The list of builds comes from ipsw.me ([list][ipsw-list]), a
  secondary source; the sizes come from Apple's CDN.
- **Cloning a macOS guest.** Lima gives each clone a new machine
  identifier. Check that `xenonet-mac1` boots on the first build. If it
  doesn't, `xenonet-mac1` becomes the only macOS instance, and resets are
  soft ones.
- **Cost.** Nothing. The macOS license allows this use, and Claude Code and
  Codex use the existing plans.

### Signing in (owner only)

| | On a Linux VM, over SSH | On the macOS guest |
|---|---|---|
| Claude Code | `limactl shell xenonet-vm1`, then `claude`, and choose the claude.ai account. Open the URL it prints in this Mac's browser and sign in. Paste the code the page shows at "Paste code here if prompted" ([docs][claude-auth]). The sign-in is saved to `~/.claude/.credentials.json` in the VM, with mode 0600 | Sign in from Terminal in the VM's window, so the sign-in goes to the keychain, as on this Mac. Over SSH, the keychain is locked and Claude Code falls back to the file |
| Codex | Two ways ([docs][codex-auth]). **Device code (beta):** run `codex login --device-auth`, then open the URL and enter the one-time code on this Mac. It needs device code login turned on in the ChatGPT account's security settings, which is the owner's decision. **Port forward**, which changes nothing in the account: connect with `ssh -F ~/.lima/xenonet-vm1/ssh.config -L 1455:localhost:1455 lima-xenonet-vm1`, run `codex login` there, and open the printed URL on this Mac. The browser's redirect reaches the VM through the forward | Run `codex login` in Terminal, in the VM's window. To match this Mac, which keeps `~/.codex/auth.json`, first set `cli_auth_credentials_store = "file"`. The default, `auto`, tries the keychain first |
| Claude desktop and the ChatGPT app | | Sign in in each app's window |

- **Never copy credentials between machines.** Don't copy `auth.json` or
  `.credentials.json` from one machine to another, even though Codex's
  docs describe how.
- **Accounts.** Every sign-in uses the same account as this Mac, unless the
  owner chooses otherwise (question 5).
- **Plan limits.** Sessions in the lab count against the same plan limits
  as this Mac's.
- **Signing out.** Before deleting a VM someone signed in to, sign out
  inside it (`/logout` in Claude Code, and `codex logout`), so its tokens
  don't outlive it.

## 4. Reset, snapshots and teardown

Lima's `vz` VMs have no snapshots, so the lab uses copy-on-write clones of
stopped VMs instead.

| Reset | What it keeps | How | Time |
|---|---|---|---|
| Soft reset | The VM and its sign-ins | `npm run lab -- reset --soft vm1`. It stops XenoNet and deletes its state and cache (`~/.local/state/xenonet` and `~/.cache/xenonet`), the agents' session history (`~/.claude/projects` and `~/.codex/sessions`) and the test repos. Then it generates the repos again and starts XenoNet | Seconds |
| Checkpoint and restore | A signed-in VM, as it was at the checkpoint | Checkpoint: `limactl stop xenonet-vm1`, then `limactl clone xenonet-vm1 xenonet-vm1-ckpt`. Restore: `limactl delete xenonet-vm1`, then `limactl clone xenonet-vm1-ckpt xenonet-vm1` with its SSH port. A checkpoint only ever replaces its own VM and never runs beside it. If a provider has rotated a token since the checkpoint, the owner may need to sign in again | Seconds |
| Hard reset | Only the golden copy | `limactl delete xenonet-vm1`, then clone the golden copy again as in step A5. The owner signs in again | About a minute, plus the sign-in |
| Rebuild a golden copy | Nothing | `limactl unprotect` and delete it, then build it again (steps A2–A3, or B1–B4) | Minutes. The macOS copy downloads 19.8 GB again unless the image was kept |

### Teardown

Before tearing down, the owner signs out inside each signed-in VM.

```sh
for vm in xenonet-vm1 xenonet-vm2 xenonet-vm3 xenonet-mac1 \
          xenonet-base-ubuntu xenonet-base-fedora xenonet-base-macos; do
  limactl list -q | grep -qx "$vm" || continue
  limactl stop "$vm" 2>/dev/null
  limactl unprotect "$vm" 2>/dev/null
  limactl delete "$vm"
done
# The lab's ssh config, console key and known hosts
rm -rf ~/"Library/Application Support/XenoNet Dev/lab"
# The macOS restore image, if it's still cached. Lima names its cache
# folders by the SHA-256 of the download's URL.
rm -rf ~/Library/Caches/lima/download/by-url-sha256/d03b3824e0aae6eea2d293a968593658f68b69f9ce45f6ab5e11587a51bbb7d4
```

- **Then the owner takes the `Include` line out of `~/.ssh/config`.**
- **Only delete the lab's instances, by name.** Never delete by wildcard:
  other Lima VMs on this Mac aren't the lab's.
- **Don't run `limactl prune` on its own.** It empties Lima's whole cache,
  including other projects' images. `--keep-referred` keeps whatever an
  instance or template still refers to.
- **The cached Linux images stay.** The Ubuntu and Fedora images remain in
  Lima's cache, as they were before the lab.

### What the lab costs in disk

| | Disk |
|---|---|
| Ubuntu and Fedora golden copies | About 8–10 GB |
| `vm1` and `vm2` (and `vm3`) | Nothing at first, then 1–2 GB each with use |
| macOS golden copy | About 25–30 GB |
| `mac1` | 2–5 GB with use |
| The restore image in Lima's cache | 19.8 GB, until step B4 deletes it |
| **The whole lab, after it's built** | **About 40–45 GB** |
| **Peak, while the macOS golden copy is built** | **About 60 GB** |
| Free on this Mac today | 78 GiB (about 84 GB) |

- **Measuring disk.** Clones share blocks with their golden copy on APFS.
  `du` counts shared blocks once for each file, so check free space with
  `df`.
- **Disk files don't shrink.** When a guest deletes files, its disk file
  may not get smaller. Cloning it again from the golden copy does.
- **Stopped VMs** use no memory or CPU. While running, the lab uses
  16 GiB and 8 vCPUs (20 GiB and 10 vCPUs with `vm3`).
- **Downloads.** About 1.5 GB for Phase A and about 23 GB for Phase B.

## 5. Questions for the owner

1. **Disk.** Before the macOS guest, should about 30 GB be freed, or
   should the lab be Linux-only for now? The candidates on this Mac:
   - Docker Desktop's disk image, 56 GiB (`docker system df` shows what can
     be reclaimed);
   - the disk of the orphaned Lima VM, which comes back when that VM stops;
   - UTM's two VMs, 7.4 GiB;
   - cached images that other Lima templates left, about 1.4 GB.
2. **The orphaned Lima VM.** Should it keep running, or be stopped? Its
   folder is gone, so `limactl` can't stop it, and it can reach this Mac's
   loopback services.
3. **SSH aliases.** May the lab add one `Include` line to `~/.ssh/config`,
   so `ssh vm1` reaches the lab's VMs as `hosts.md`'s relay expects? The
   other way is for `xenonet hosts add` to take an ssh config file of its
   own, which is the hosts lane's call.
4. **Lima.** Stay on 2.1.2, which is enough for this plan, or upgrade to
   2.2.1?
5. **Accounts.** Should every VM use the same Claude and ChatGPT accounts
   as this Mac, or should one VM use another account, to test telling
   identities apart?
6. **Codex on the Linux VMs.** Turn on device code login in the ChatGPT
   account's security settings, or use the port-1455 forward, which needs
   no change to the account?
7. **The macOS guest.**
   - Try Lima's experimental support first, with Tart (free, 23 MB) as the
     fallback?
   - Allow automatic login in the guest, so agents can drive XenoNet Dev's
     window there?
8. **The restore image.** Keep the 19.8 GB image after the build, to
   rebuild without downloading again, or delete it?
9. **Updating this Mac.** Will this Mac move to macOS 26.6 or 27 soon? A
   guest can't be newer than its host, and a macOS 27 guest needs a
   macOS 27 host and a 26.6 GB restore image.
10. **More VMs.**
    - A third Linux VM (`vm3`)?
    - Later, a Windows 11 on Arm guest? Lima 2.3, in beta, adds Windows
      guests, and the Windows license would be the owner's.
11. **Agents' permissions in the VMs.** Should agents ask permission as
    usual, or auto-accept edits, so lab sessions can run unattended?

## Sources

Checked on 2026-10-05. The sizes of downloads come from each source's
release assets, from npm's registry metadata, or from the `Content-Length`
headers of Apple's CDN. Sizes on disk were measured on this Mac.

- Lima: [releases][lima-releases], [macOS guests][lima-macos],
  [plain mode][lima-plain], [port forwarding][lima-port],
  [user-mode network][lima-user-net], [user-v2][lima-user-v2],
  [gateway NAT to the host's loopback][lima-gvproxy],
  [no `vz` snapshots][lima-vz-driver], [copy-on-write clones][lima-clone],
  [its SSH options][lima-sshutil], [experimental features][lima-experimental],
  [AI agents in Lima][lima-ai], [templates][lima-templates],
  [v2.1 on the CNCF blog][lima-v21]
- UTM: [releases][utm-releases], [macOS guests][utm-macos],
  [scripting][utm-scripting], [port forwarding][utm-ports], [site][utm-site]
- Tart: [quick start][tart-quick], [FAQ][tart-faq], [LICENSE][tart-license],
  [releases][tart-releases], [`run`][tart-run], [`clone`][tart-clone],
  [`exec`][tart-exec], [old licensing page][tart-licensing-page],
  [Cirrus Labs' announcement][cirrus], [images][tart-images]
- OrbStack: [machines][orb-machines], [isolated machines][orb-isolated],
  [architecture][orb-arch], [pricing][orb-pricing]
- Apple: [`container`][container], [its releases][container-releases],
  [its technical overview][container-overview], [macOS Tahoe 26
  license][apple-sla], [`virtualMachineLimitExceeded`][vz-limit],
  [the slot bug on the forums][vz-slot-bug], [macOS 26.5 restore
  image][ipsw-265]
- Agents: [Claude Code authentication][claude-auth],
  [Claude Code setup][claude-setup], [Codex authentication][codex-auth],
  [Codex CLI][codex-cli], [Codex releases][codex-releases]
- Others: [Node 22][node-22], [Lume][lume], [Multipass][multipass],
  [Docker Sandboxes][docker-sbx], [Parallels][parallels-macos],
  [VMware Fusion, free][fusion-free], [Fusion on Apple silicon][fusion-arm],
  [ipsw.me's list of builds (secondary)][ipsw-list]

[lima-releases]: https://github.com/lima-vm/lima/releases
[lima-macos]: https://lima-vm.io/docs/usage/guests/macos/
[lima-plain]: https://lima-vm.io/docs/config/plain/
[lima-port]: https://lima-vm.io/docs/config/port/
[lima-user-net]: https://lima-vm.io/docs/config/network/user/
[lima-user-v2]: https://lima-vm.io/docs/config/network/user-v2/
[lima-gvproxy]: https://github.com/lima-vm/lima/blob/master/pkg/networks/usernet/gvproxy.go
[lima-vz-driver]: https://github.com/lima-vm/lima/blob/master/pkg/driver/vz/vz_driver_darwin.go
[lima-clone]: https://github.com/lima-vm/lima/blob/master/pkg/instance/clone.go
[lima-sshutil]: https://github.com/lima-vm/lima/blob/master/pkg/sshutil/sshutil.go
[lima-experimental]: https://lima-vm.io/docs/releases/experimental/
[lima-ai]: https://lima-vm.io/docs/examples/ai/
[lima-templates]: https://github.com/lima-vm/lima/tree/master/templates
[lima-v21]: https://www.cncf.io/blog/2026/03/25/lima-v2-1-macos-guests-and-enhanced-ai-agent-safety/
[utm-releases]: https://github.com/utmapp/UTM/releases
[utm-macos]: https://docs.getutm.app/guest-support/macos/
[utm-scripting]: https://docs.getutm.app/scripting/reference/
[utm-ports]: https://docs.getutm.app/settings-qemu/devices/network/port-forwarding/
[utm-site]: https://mac.getutm.app/
[tart-quick]: https://tart.run/quick-start/
[tart-faq]: https://tart.run/faq/
[tart-license]: https://github.com/openai/tart/blob/main/LICENSE
[tart-releases]: https://github.com/openai/tart/releases
[tart-run]: https://github.com/openai/tart/blob/main/Sources/tart/Commands/Run.swift
[tart-clone]: https://github.com/openai/tart/blob/main/Sources/tart/Commands/Clone.swift
[tart-exec]: https://github.com/openai/tart/blob/main/Sources/tart/Commands/Exec.swift
[tart-licensing-page]: https://tart.run/licensing/
[cirrus]: https://cirruslabs.org/
[tart-images]: https://github.com/cirruslabs/macos-image-templates
[orb-machines]: https://docs.orbstack.dev/machines/
[orb-isolated]: https://docs.orbstack.dev/machines/isolated
[orb-arch]: https://docs.orbstack.dev/architecture
[orb-pricing]: https://orbstack.dev/pricing
[container]: https://github.com/apple/container
[container-releases]: https://github.com/apple/container/releases
[container-overview]: https://github.com/apple/container/blob/main/docs/technical-overview.md
[apple-sla]: https://www.apple.com/legal/sla/docs/macOSTahoe.pdf
[vz-limit]: https://developer.apple.com/documentation/virtualization/vzerror/code/virtualmachinelimitexceeded
[vz-slot-bug]: https://developer.apple.com/forums/thread/830119
[ipsw-265]: https://updates.cdn-apple.com/2026SpringFCS/fullrestores/122-58869/DFB1CEEF-5619-4591-9924-E20DB2C8FED0/UniversalMac_26.5_25F71_Restore.ipsw
[ipsw-list]: https://api.ipsw.me/v4/device/VirtualMac2,1?type=ipsw
[claude-auth]: https://code.claude.com/docs/en/authentication
[claude-setup]: https://code.claude.com/docs/en/setup
[codex-auth]: https://learn.chatgpt.com/docs/auth
[codex-cli]: https://learn.chatgpt.com/docs/cli
[codex-releases]: https://github.com/openai/codex/releases
[node-22]: https://nodejs.org/dist/latest-v22.x/
[ubuntu-img]: https://cloud-images.ubuntu.com/releases/noble/release-20260518/
[fedora-img]: https://download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/aarch64/images/
[lume]: https://cua.ai/docs/lume/guide/fundamentals/unattended-setup
[multipass]: https://canonical.com/multipass/docs/latest/explanation/driver/
[docker-sbx]: https://docs.docker.com/ai/sandboxes/
[parallels-macos]: https://kb.parallels.com/en/128867
[fusion-free]: https://blogs.vmware.com/cloud-foundation/2024/11/11/vmware-fusion-and-workstation-are-now-free-for-all-users/
[fusion-arm]: https://knowledge.broadcom.com/external/article/315602
