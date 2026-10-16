# The xenonet command

`xenonet` does from a terminal what the console does. It says what XenoNet sees, checks your setup and connects Claude Code and Codex. It works on macOS and Linux.

`Install the xenonet command` in the palette puts it in `~/.local/bin`, or `%LOCALAPPDATA%/XenoNet/bin` on Windows, as a small script that runs on XenoNet's own runtime. In a checkout, `node cli/xenonet.js install` does the same with your Node. On Windows, XenoNet's first launch installs it, and Setup puts its folder on your user Path. Elsewhere, if that folder isn't on PATH, the command tells you how to add it.

```console
$ xenonet
● XenoNet 0.1.0 on 127.0.0.1:47840
  Reads     48 projects, 251 Claude sessions, 2,444 Codex threads
  Sessions  1 working, 18 idle, 3 connected to XenoNet
  Actors    1 idle, 1 unknown
  Doctor    everything XenoNet needs is in place (checked 4 min ago)
```

## Commands

| Command | What it does |
| --- | --- |
| `xenonet`, `xenonet status` | Shows which XenoNet is running, what it reads, the sessions at work, which sessions need you, and the doctor's last word. The actors get a line of their own. |
| `xenonet doctor` | Checks your setup. Each problem comes with a fix, and a failed check exits 1. See [The doctor](#the-doctor). |
| `xenonet logs` | What went wrong lately, by code, with a fix when there's one; works with XenoNet stopped. See [When something goes wrong](troubleshooting.md). |
| `xenonet diagnose` | A summary of XenoNet's state that's safe to share. `--ask` has a Codex or Claude Code session on this computer look at it with you. |
| `xenonet decide` | The decision model XenoNet can ask instead of Codex, and its OpenRouter key, kept in your credential store. See [Decision models](decisions.md). |
| `xenonet refusals` | The provider refusals XenoNet kept: list, show, export for a dataset, add context, scan older transcripts. See [Provider refusals](refusals.md). |
| `xenonet drift` | Lists what has changed in the data XenoNet reads from Claude Code, Codex, Claude desktop and gh since you last looked. It goes by source, and shows shapes, never values. Then it marks that seen, which quiets the doctor. `--all` lists every difference kept. `--exit-code`, for scripts, exits 1 on new drift and leaves it unmarked. See [Drift](how-it-works.md#drift). |
| `xenonet agents` | Lists the sessions at work in the last two hours, with where each is, what it says it's doing, and the files it's editing or has claimed. The actors at work come after them, in a list of their own. `--repo` keeps only those in the current repo. |
| `xenonet who <path>` | Says which sessions and actors have a hand in a file. |
| `xenonet session [words]` | Shows a session's models, tokens, estimated cost, turns and tools. Without words, it shows the latest session in the current folder. With words, it shows the newest session whose title has them. |
| `xenonet messages` | Shows what sessions and actors said to each other in the last day. |
| `xenonet search <words>` | Searches every Claude Code and Codex transcript. |
| `xenonet root [folder]` | Shows the projects folder XenoNet reads and where that comes from, or reads another folder. `--check` only says what XenoNet would find there. `--reset` goes back to `~/Projects`. See [Changing the projects folder](projects.md#changing-the-projects-folder). |
| `xenonet film` | Renders a film of today in a hidden XenoNet Dev window, as a 15 to 30 second story (9:16), and saves the MP4 with no tags. Projects show as star names and no clock appears unless `--names` or `--tokens` say otherwise. It also takes `--seconds`, `--theme`, `--size` and `--out`. |
| `xenonet script record`, `xenonet script run <file>` | Saves what you did in XenoNet Dev as a script, and plays scripts back. See [Driving XenoNet](development.md#driving-xenonet-from-agents-scripts-and-tests). |
| `xenonet theme [name]` | Lists the themes, or switches to one. `xenonet theme copy <theme> <name>` starts one of your own. |
| `xenonet connect`, `xenonet disconnect` | [Connects Claude Code and Codex](agents.md#connecting-claude-code-and-codex), or takes XenoNet out again, after showing each change. |
| `xenonet actor list`, `xenonet actors` | Lists the [actors](actors.md), with each one's presence and status, what it acts on, and how XenoNet knows it. `xenonet actor show <slug>` adds what one did lately. |
| `xenonet actor register --name <name>` | Asks to add a program as an actor, and waits for your Allow in XenoNet's window. Then it writes the actor's token to `--token-file`, readable only by you. |
| `xenonet actor heartbeat`, `event`, `targets`, `status`, `inbox`, `send`, `leave` | These are how an actor reports and acts, with its token from `XENONET_ACTOR_TOKEN_FILE`. |
| `xenonet actor listen [--exec <program>]` | For an actor's program that stays running: it waits for messages and reads each as it arrives. It prints them (JSON lines with `--json`), or runs the program once per message with the message as JSON on its standard input. The program doesn't get the actor's token. |
| `xenonet actor wake <slug> [test\|off]` | How an actor is woken when a message arrives. `test` calls its webhook now; `off` stops waking it. You set a webhook in XenoNet's window. |
| `xenonet actor observe [<product> on\|off]`, `xenonet actor remove <slug>` | `observe` lists the products XenoNet observes, and turns one on or off. `remove` removes an actor that reports itself. An actor XenoNet only observes goes off the map when you turn its observer off. Turning an observer on or off, and removing an actor, are yours to do, from your own terminal. |
| `xenonet gateway` | The [actor gateway](actors.md#actors-outside-this-machine-the-gateway) for actors outside this machine: whether it's on, its tunnel, its address and whether that reaches it, and each app's connection. `tunnels` lists the tunnels XenoNet can run and which this computer has; `on --tunnel <kind> [--host <name>] [--name <tunnel>]` turns it on through one; `check` and `repair` check the address and set the tunnel up again; `address <slug>` says what to tell an app whose address is old; `token <slug>` asks for a new gateway token for an actor, and `off` closes the gateway. `revoke <slug>` takes an actor's gateway token away. Gateway tokens are made in XenoNet's window, so `token` only leaves a card there to Allow, and prints no token. |
| `xenonet assistant` | Lists your [remote actors'](cloud-assistants.md) connections, with each one's projects, expiry and when it was last used. `audit` lists what was read lately, without content. `revoke <grant>` stops one for good, and `disable` turns them all off; both work with XenoNet stopped. Pairing one, and turning access on again, happen in XenoNet's window. |
| `xenonet mcp assistant --grant <grant>` | The read-only MCP server a tunnel client runs for a remote actor you paired. `--fixture` serves made-up data with no XenoNet, to try a client first. |
| `xenonet hosts` | Shows [other machines' XenoNets](other-machines.md) beside this one, a prototype. `self` names this XenoNet as a host. `add`, `remove`, `test` and `reconnect` manage the hosts it shows. |
| `xenonet console <steps>` | Drives XenoNet Dev, to check a change to XenoNet. See [Driving XenoNet](development.md#driving-xenonet-from-agents-scripts-and-tests). |
| `xenonet install`, `xenonet uninstall` | Puts the command on your PATH, or takes it off. |
| `xenonet completion zsh` | Prints tab completion for zsh. Add `eval "$(xenonet completion zsh)"` to `~/.zshrc`, or the `bash` one to `~/.bashrc`. |

Every command takes `--json`, for scripts. A command talks to XenoNet, or to XenoNet Dev when XenoNet isn't running. `--dev`, `--demo` or `--port` picks one.

## The doctor

`xenonet doctor` checks these:

- that Claude Code and Codex are installed and signed in
- that XenoNet can read their sessions and your projects folder
- git, the runtime, XenoNet itself and the command
- whether Claude Code and Codex are connected
- for actors, tokens left in repos or expired, what the observers read, and that tokens are held to their processes

Each problem comes with a fix, and a failed check exits 1. With no XenoNet running, `xenonet doctor` runs the checks itself, and skips the ones that need XenoNet. With [other machines](other-machines.md#checking-on-hosts) turned on, it checks each host too.

XenoNet runs the same checks a minute after it starts, and every half hour. When they find a new problem, a chip appears in the alert strip. Unless XenoNet's window is in front, a notification appears too. Either one opens the results, and so does `Check XenoNet's setup` in the palette.

Connected sessions can run the checks too, with the `doctor` tool.
