# Sandboxed starts

A session that isn't yours to watch should run where it can't do much harm. XenoNet uses each app's own sandbox for that: it turns the app's sandbox on for a run, with limits you can read below, and refuses the start if the app has none to turn on. XenoNet has no sandbox of its own.

## What starts sandboxed

- **Always sandboxed:** a start from another machine that isn't yours, a delegated start, the run of an intent, and a start from a session or an actor. This is checked by the machine that runs it, on the signed start itself: a start that says nothing about the sandbox, or that it is off, is refused, so an older XenoNet that doesn't know the field is refused too, never run bare. A machine whose XenoNet is too old to say it takes a sandboxed start is never given one.
- **Yours to choose:** a start you make yourself at a console. On this machine the switch is off by default; on another machine it is on. In the console, tick **Run it in its app's sandbox** in Start a session. In a terminal it is `--sandbox` or `--no-sandbox`. The `start_session` tool takes `sandbox` and `domains`, but a session can only say true.
- **Domains:** a sandboxed run reaches none by default. Name the ones it may reach with `--allow github.com,*.npmjs.org`, the page's domains box, or `domains`.

A sandboxed run's first prompt says so and says what it may reach, the answer to the start says so, and so does the receipt on the machine that asked.

## What each app does

| App | Writes | Reads | Network |
| --- | --- | --- | --- |
| Claude Code | only in the folder it started in | none of the home folder but that folder | only the domains given |
| Codex | only in the folder it started in | the whole disk (its sandbox can't narrow reads) | none; a start with domains is refused |
| Grok Build, Hermes Agent | no sandbox to drive: a sandboxed start is refused in words | | |

On native Windows Claude Code has no sandbox, so a sandboxed start of it is refused there. Run it under WSL2.

### Claude Code

XenoNet writes one settings file for the run and passes it with `--settings`, with `--setting-sources=` (the user's, the project's and the local settings are left out) and `--strict-mcp-config` with a configuration that holds only XenoNet's server. On a machine you don't own the repo is someone else's content, and its hooks and servers would run outside the sandbox. XenoNet's own hooks are in the file, so the run still shows on the map.

The file turns on `sandbox.enabled`, `sandbox.failIfUnavailable` (no sandbox means no run) and `sandbox.allowUnsandboxedCommands: false`, sets `sandbox.filesystem.allowWrite` to the folder, `denyRead` to the home folder with `allowRead` for the folder, and `sandbox.network.allowedDomains` with `strictAllowlist`. Claude Code's sandbox wraps shell commands only, so the same file sets `permissions.blockReadsOutsideWorkingDirectories` and `blockWritesOutsideWorkingDirectories` for the file tools, denies `WebSearch`, and denies `WebFetch` unless domains were given. With domains, a fetch of another domain asks, and no delegation or grant answers it.

Shell commands can't read the rest of the home folder, so a toolchain installed there (`~/.cargo`, `~/.nvm`) isn't available to them. Put what a guest run needs in its folder or a system path. On Linux, Claude Code's sandbox needs `bwrap` and `socat` (`sudo apt-get install bubblewrap socat`); `xenonet doctor` checks.

### Codex

The run is `codex exec --sandbox workspace-write` with approvals off (`approval_policy="never"`), network off, and `/tmp` left out of what it may write. Codex's sandbox can't name domains, so a start with domains is refused.

## After the start

- A turn resumed for the session later, or a copy forked from it, runs sandboxed the same way. If its sandbox can't be made again, the turn isn't run.
- The run's environment is the one an ACP agent gets: the service's without XenoNet's own variables, session marks and anything named for a secret, plus the app's sign-in (`xenonet runs sign-in`).
- Under a delegation or a grant, a command that asks to run outside the sandbox (`dangerouslyDisableSandbox`) is never answered for you. It comes to Needs you and only you can allow it.

See [Starting sessions](network.md#starting-sessions) and [Delegating work to another machine](delegation.md).
