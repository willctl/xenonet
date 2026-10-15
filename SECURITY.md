# Security

XenoNet runs on your machine and reads the files Claude Code and Codex already write. If you find a way to make it read, send or change something it shouldn't, tell Will before telling anyone else.

## Reporting a problem

Email will@willctl.com. Don't open a public issue or pull request for a security problem, so that the fix can be out before the problem is.

Say what you found, how to reproduce it, which version you ran (`xenonet --version`, or Updates in the command palette) and which operating system. Say whether a program had to be running on the machine already, whether it ran under your account, and what it could read or change. A proof of concept helps, as text or a short script.

You'll get an answer within seven days. The fix ships in a new release, and its notes credit you if you want that, by name, by handle or not at all. Please keep the details private until that release is out.

## Supported versions

XenoNet is at 0.x. Fixes go into the latest release only, and installed copies move to it: Windows downloads it, and a Mac links to its download (see [Install](docs/guide/install.md#updates)). Older versions don't get patches, so update first and check whether the problem is still there.

## What's in scope

Everything in this repository as it ships from [GitHub Releases](https://github.com/willctl/xenonet/releases): the background service (`server/`, `core/`), the console page (`src/`), Desktop (`electron/`), the `xenonet` command (`cli/`), the hooks it installs and the two MCP servers (`mcp/`), the actor gateway, the installers and the update path, and the Service and Tools archives. The JevRouter SDK in `vendor/` is in scope too; `vendor/SOURCE.md` says which upstream commit it is.

Examples of what to report:

- A page, repo, transcript, actor or other machine that makes XenoNet run a program, or run script in the console.
- A way to read transcripts, prompts, keys or tokens through the HTTP API, an MCP server, the gateway or a remote actor's endpoint, beyond what [Privacy and security](docs/guide/privacy.md) says each may read.
- A way for one session or actor to act as another, or to allow an actor, connect Claude Code and Codex, or change your settings from under a session, where XenoNet says it refuses that.
- A request accepted from another host, another origin or another web page.
- An update that installs without matching its manifest's hash.

Problems in Claude Code, Codex, Electron, Node or the apps XenoNet observes belong to those projects. Something you can only do as an administrator, or with physical access to the machine, is usually not a XenoNet bug. Write anyway if you're unsure.

## Known limits

These are how XenoNet works today. They're in the docs, not secret, and a report that only restates one won't lead to a fix on its own.

- The Mac app isn't signed or notarized by Apple yet, and the Windows installer isn't code-signed. macOS and SmartScreen warn the first time you open each version. Signing waits on choosing a publisher identity ([Install](docs/guide/install.md), [Releases](docs/guide/releases.md#signing)).
- XenoNet checks which process sent a coordination request on macOS only. On Windows and Linux it doesn't check requests yet, so there a program running under your account can act in a session's name. On macOS the check can't tell apart threads of one Codex process, and a session no running process has been seen to hold yet goes to whoever names it first ([Privacy and security](docs/guide/privacy.md#telling-sessions-apart-on-macos)).
- Updates and component archives are checked against the release manifest's hashes over HTTPS, not a publisher signature. Unsigned Mac builds only check for releases and never install one ([Components](docs/guide/components.md#updates)).
- A gateway token lets whoever holds it do what that actor can: see who is at work here, send messages and read the ones sent to it. Keep each token in its own app's settings. Connect… makes a new one, and the old one stops working ([Privacy and security](docs/guide/privacy.md#what-leaves-this-computer)).
- The server listens on 127.0.0.1 only and answers only its own address, so any program running under your account on this machine can reach it. That is by design: the `xenonet` command, the hooks and the MCP servers are such programs.

XenoNet has no telemetry and no analytics. Everything that leaves your machine is listed in [Privacy and security](docs/guide/privacy.md#what-leaves-this-computer). Apart from the update check and the plan-usage request to Anthropic, which you can turn off, each item is something you connected or pressed yourself.
