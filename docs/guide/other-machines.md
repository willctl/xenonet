# Other machines over SSH

The main way to use XenoNet on several machines is a [network](network.md): each machine links to one gateway, the sessions on every machine see and message each other, and you can act from any machine you use a XenoNet window on.

This page covers the other way, a prototype: XenoNet can show other machines' XenoNets beside its own, read-only, over SSH, with no network. It's off unless you start XenoNet with `XENONET_HOSTS=1`. You can read another machine's transcripts and stats in XenoNet's window. Nothing from it can be opened in its app, dismissed, handed off or messaged. A machine that's both in your network and added here is shown once, from here.

## How XenoNet reaches a host

Each machine runs its own XenoNet, which stays the only reader of its files. The XenoNet you look at runs `ssh <host> xenonet relay` for each host you add.

The relay is a small read-only command. It reads the XenoNet on its own machine at 127.0.0.1, as `xenonet agents` does, and streams that XenoNet's map back over the SSH connection.

Nothing listens on a network, and XenoNet holds no keys. SSH signs in, and checks each machine's host key.

## Adding a host

1. On each host, install XenoNet and the `xenonet` command, keep XenoNet running, and give the host a name, for example:

   ```bash
   xenonet hosts self vm1
   ```

   You choose the name. XenoNet never uses the machine's own name, which often holds a person's name.

   Instead of keeping XenoNet running on the host, you can add the host with `--start` in step 2. The relay then starts a XenoNet there, for as long as the relay runs.

2. On the machine you look at, start XenoNet with `XENONET_HOSTS=1`, then add the host, for example:

   ```bash
   xenonet hosts add vm1 lab@192.168.5.15
   ```

   Add `--ssh-port`, `--identity` and `--known-hosts` where the host needs them. XenoNet keeps its hosts in an ssh config of its own, `ssh_config` in its state folder, and runs ssh with it. So XenoNet never reads or changes your `~/.ssh/config`.

3. Accept the host's key once, in a terminal, with the command that `xenonet hosts add` prints. On a Mac it looks like this:

   ```bash
   ssh -F ~/'Library/Application Support/XenoNet/ssh_config' vm1
   ```

   The `~` stays outside the quotes, where the shell expands it. XenoNet never accepts a host key it doesn't know.

## Hosts on the map

This machine stays at the center of the map, and each host is a constellation of its own beside it. The host node has a ring that's green while the host is live, amber when it has gone quiet, and grey once it's offline. The host's projects and sessions sit on smaller rings around it.

A faint dashed bridge joins checkouts of one repo on different machines. It lights up when you select or hover either end.

Selecting a host's project gathers its sessions, as it does for a project on this machine. The inspector says where a node lives, such as `on vm1 · live`, and offers Preview and Stats. Under Filters, a chip for each machine shows only that machine's work.

To keep the frame rate up, the map draws only part of a host while you're not looking at it. It draws every host node, and at most 24 of the hosts' other nodes in all. Sessions that wait on you or are stuck come first, then places. Selecting one of a host's nodes, or picking its chip, draws all of that host in place. The chip's tooltip says how much is drawn.

A host's alerts carry its name. While the host is live, its alerts count in the tray and the menu bar. A host that goes quiet keeps its constellation in place, dimmed and still, and its alerts say when it was last heard.

## Checking on hosts

`xenonet hosts` says how each host is doing, and what to do when one needs you. `xenonet hosts test vm1` makes one try, and says how far it got.

`xenonet doctor` checks each host:

- that it's reachable
- that its relay answers in a protocol this XenoNet speaks
- that its clock is within two minutes of this one's

The doctor also warns when two hosts answer with the same key, since the two would share one snapshot in the cache. That happens when one XenoNet is added twice, or when a machine's state folder was copied from another's.

## What crosses between machines

A host sends titles, states and counts. That covers projects, sessions and how they're doing, token counts, and who each session ran as.

Prompts, messages, a waiting question's text, tool details and the files sessions touch stay on their machine, unless you add the host with `--detail`. A transcript, a session's stats or a collision's diffs cross only when you open one in XenoNet's window, and XenoNet doesn't keep them. A browser tab, a session or an actor on this machine can't open them: XenoNet answers those reads only to its own window.

A host writes paths under its home folder as `~` before they leave it, so no user name crosses machines.

Another machine's nodes have ids of the form `@<host>:<id>`, such as `@vm1:project:~/projects/atlas-api`. This machine's ids, layout and pins don't change.

XenoNet bridges the checkouts of one repo on two machines. It matches them by their remote, as each machine resolves it, with that machine's SSH aliases.

Sessions updated in the last 7 days cross, and `--since` changes how far back that goes. Archived sessions stay on their machine. A host has at most 5,000 places and sessions on the map.

## When a host goes away

XenoNet shows a host that goes away as offline, with its last snapshot, marked stale. XenoNet connects again by itself, every 2 seconds at first, then less often, up to a minute apart.

XenoNet keeps each host's last snapshot, so offline hosts are still there after a restart.

## Accounts on several machines

One account signed in on several machines is one identity, with its counts added up. Its plan limits are the newest reading from any of the machines.

An account signed in only on another machine gets plan bars of its own, under that machine's name. An OpenAI API key belongs to its machine.

## Limiting the console's key

To limit what the console's key can do on a host, install the key there with `restrict,command="xenonet relay --port 47840"` in front of it, in `~/.ssh/authorized_keys`. That key can then run the relay and nothing else.

## Which machines can be hosts

A Mac needs Remote Login to be a host. You turn it on in System Settings, under General, then Sharing. A Linux machine usually has an SSH server already. Windows hosts aren't supported yet.

The design is in [docs/design/hosts.md](../design/hosts.md).
