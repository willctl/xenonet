# Sandboxed starts

A session that isn't yours to watch should run where it can't do much harm. XenoNet uses each app's own sandbox for that: it turns the app's sandbox on for a run, with limits you can read below, and refuses the start if the app has none to turn on. XenoNet has no sandbox of its own.

## Levels

A start names how confined it is, one of three levels. The level is part of the start you sign, with the domains for `vendor` and the wall for `cell`, so nothing between your signature and the run can change it.

| Level | What it is |
| --- | --- |
| `off` | The app's own settings. Only you can start at this level. |
| `vendor` | The app's own sandbox, which the rest of this page describes. The only level a start from anyone but you may have. |
| `cell` | A sealed Adytum cell behind the wall of the identity that signs the start ([confinement](../design/confinement.md)). It is yours alone, and cell starts aren't wired yet: asking for one is refused in words and nothing runs. |

The wall of a `cell` start is worked out from your identity on the machine that runs the start. A request never names one: a request that does is refused. A start without a level is refused, never defaulted, so an older XenoNet that doesn't know the field is refused too, unless the start is yours.

## What starts sandboxed

- **Always `vendor`:** a start from another machine that isn't yours, a delegated start, the run of an intent, and a start from a session or an actor. This is checked by the machine that runs it, on the signed start itself: a start that says nothing about its level, or says `off` or `cell`, is refused, so an older XenoNet that doesn't know the field is refused too, never run bare. A machine whose XenoNet is too old to say it takes a sandboxed start is never given one.
- **Yours to choose:** a start you make yourself at a console. On this machine the level is `off` by default; on another machine it is `vendor`. In the console, pick **Off** or **Its app's sandbox** under Confinement in Start a session. In a terminal it is `--confine off|vendor|cell`, with `--sandbox` and `--no-sandbox` as older ways to say `vendor` and `off`. The `start_session` tool takes `confine` (or the older `sandbox`) and `domains`, but a session can only say `vendor`: a session or an actor that asks for `off` or `cell` is refused before anything goes out.
- **Older machines:** a machine whose XenoNet is too old to take a sandboxed start is never given one. So your own start on another machine, and a delegation to it, fail with a 409 until it is updated, unless you start your own with `--confine off`; a delegation has no such way. An intent for Grok Build or Hermes Agent always fails, because the intent's run is always `vendor` and those two have no sandbox to drive.
- **Folder:** a sandbox is a project folder. A sandboxed start whose folder is your home folder (`~`, which is the default when none is named), the root of the disk, a folder that holds your home folder, or a folder under a hidden folder of home (`~/.config/x`) or its Library (AppData on Windows) is refused, by the machine that runs it, on the folder as it resolves (a link to home is home) and with case ignored on macOS and Windows. Otherwise the run could write all of home, or what outlives it.
- **Intents:** the run of an intent carries your id but is unattended and its last words go out as the outcome, so it is checked like a run that isn't yours: no Codex, and only the domains you allowed.
- **Domains:** a sandboxed run reaches none by default. Name the ones it may reach with `--allow github.com,*.npmjs.org`, the page's domains box, or `domains`. Your own start may name any. A start from anyone else (a session, an actor, another machine, a delegation) may name only domains you allowed on the machine that runs it: `xenonet runs domains set github.com '*.npmjs.org'`, `xenonet runs domains` to read them and `xenonet runs domains clear` to take them away. A name is allowed by the same name, or by a `*.` entry that holds it as a subdomain. Only you set the list.

A sandboxed run's first prompt says so and says what it may reach, the answer to the start says so, and so does the receipt on the machine that asked.

## What each app does

| App | Writes | Reads | Network |
| --- | --- | --- | --- |
| Claude Code | only in the folder it started in | none of the home folder but that folder | only the domains given |
| Codex | only in the folder it started in | the whole disk (its sandbox can't narrow reads) | none; a start with domains is refused |

A run's last words can carry any file it read. Codex's sandbox can't deny reads, so a sandboxed Codex start from anyone but you (a session, an actor, another machine, a delegation, an intent) is refused; start Codex from your own console, or use Claude Code.
| Grok Build, Hermes Agent | no sandbox to drive: a sandboxed start is refused in words | | |

On native Windows Claude Code has no sandbox, and Codex's sandbox there isn't one XenoNet has checked, so a sandboxed start of either is refused there. Run them under WSL2.

### Claude Code

XenoNet writes one settings file for the run and passes it with `--settings`, with `--setting-sources=` (the user's, the project's and the local settings are left out) and `--strict-mcp-config` with a configuration that holds only XenoNet's server. On a machine you don't own the repo is someone else's content, and its hooks and servers would run outside the sandbox. XenoNet's own hooks are in the file, so the run still shows on the map.

The file turns on `sandbox.enabled`, `sandbox.failIfUnavailable` (no sandbox means no run) and `sandbox.allowUnsandboxedCommands: false`, sets `sandbox.filesystem.allowWrite` to the folder, `denyRead` to the home folder with `allowRead` for the folder, and `sandbox.network.allowedDomains` with `strictAllowlist`. Claude Code's sandbox wraps shell commands only, so the same file sets `permissions.blockReadsOutsideWorkingDirectories` and `blockWritesOutsideWorkingDirectories` for the file tools, denies `WebSearch`, and denies `WebFetch` unless domains were given. With domains, a fetch of another domain asks, and no delegation or grant answers it.

Its `sandbox.filesystem.denyWrite` names every `.claude` (the folder's own and `**/.claude/**`), `.mcp.json`, `.claude.json`, and git's hooks and config, which win over the folder's `allowWrite`, so a command of the run can't write what Claude Code loads on a later turn outside the sandbox.

The file also sets `sandbox.autoAllowBashIfSandboxed` to false, so a sandboxed command still goes through the permission prompt (you answer it, or the delegation's rules do: a delete, a push or anything that reaches out comes to you), and lists the sign-in variable (`CLAUDE_CODE_OAUTH_TOKEN`) under `sandbox.credentials.envVars` as deny, so Claude Code has it but its commands don't. It allows no Unix socket (`allowUnixSockets` empty, `allowAllUnixSockets` false), and the run's environment leaves out `SSH_AUTH_SOCK`, `DOCKER_HOST` and any name ending in `_SOCK` or `_SOCKET`.

Claude Code loads what is in `.claude` (the folder it starts in, and any under it: skills, agents, commands and plugins, at any depth) whatever settings sources it is given, and a hook there, or a skill that is a plugin, runs as Claude Code itself, outside the sandbox that wraps its commands. Every confined turn passes `--disable-slash-commands`, which leaves the folder's skills and commands out (checked on 2.1.327); its agents still load, so XenoNet also looks through every `.claude` folder under the project, and refuses to sandbox a Claude Code run in a folder where it finds any of these, naming the file; take them out or use another folder:

- a `hooks`, `hooks.json`, `plugin.json` or `.claude-plugin` (names are compared with case ignored, so `.CLAUDE`, `Hooks/` and `PLUGIN.JSON` count);
- a markdown file whose front matter has a `hooks`, `mcpServers` or `mcp_servers` key in any spelling (quoted, indented, in any case, with a byte order mark or Windows line ends in front), since an agent's own servers may start outside the sandbox even under `--strict-mcp-config`;
- a markdown file whose front matter isn't plain `key: value` lines starting at the left edge, with indented lines only under such a key (a flow mapping, a `?` key, a tag, anchor, alias or merge, a whole mapping indented, a missing closing line), since it can't be told what it holds.

The walk fails closed: the folder itself, a folder or a file it can't read, or an entry that vanished while it looked, is a refusal in words, never an empty folder. A link to a folder is read where it points: one that leads outside the project is refused, one inside is walked once, links into `.git` and a deep `node_modules` included. A `.claude` that is a link, and any link inside a `.claude`, is refused wherever it leads and even when it leads nowhere yet, since a command could write through it past the run's write rules. Elsewhere a link to a file or to nothing is left alone. A folder too deep (16 folders) or too big (50,000 entries) to look through is refused in words too; `node_modules` is looked at two levels down only, where a package's own `.claude` would be (`node_modules/@scope/pkg` too). The check is made again on every turn, resumed or copied, so a skill added between turns is refused on the next one.

`--safe-mode` would leave all of this out, but it also drops XenoNet's own hooks and its MCP server, and `--bare` takes no OAuth sign-in, so neither is used. Claude Code keeps a `permission-mode` entry in a session's transcript, so every turn of a run that isn't yours passes `--permission-mode default`, and a resumed or copied session can't bring a laxer mode back.

The run's sign-in is not in any environment. On macOS a program of the same user can read another's environment (`kern.procargs`, `ps eww`), and Claude Code's sandbox profile for the Mac allows it, so a command in the sandbox could read the token of the process that runs it. XenoNet instead makes a pipe, starts the run with its read end as descriptor 3, and only then writes the token and a newline and closes its own end. The run has descriptors 0 to 3 and no other: every descriptor from 4 up is close-on-exec or closed in the run, even one some code of the service left open. The run's environment only says where to read (`CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR=3`), and Claude Code reads one line (opening `/dev/fd/3`, which a pipe allows and a socket doesn't on Linux), closes the descriptor and deletes the variable (as of 2.1.327). A run never keeps a sign-in variable the service itself was started with. That holds for every run XenoNet starts, resumes or copies on macOS and Linux, confined or not. What is left: on Linux the pipe's ends are close-on-exec from the start (`pipe2`); on macOS there is no `pipe2`, so they are made so by a second call, and a process another thread of the service forks in the microseconds between the two calls holds the read end too and could read the line before the run does, and holds the write end too, so the run reads the end of the line only when that process exits. The line is written only after the run has started, so such a process holds an empty pipe until then. These runs are started with std's `Command` and a `pre_exec` in `xenonet-os`, because Adytum's exec has no way to pass a descriptor. On Windows the token is in `CLAUDE_CODE_OAUTH_TOKEN`, which the commands of a confined run are not given (`sandbox.credentials.envVars`, deny) but which a program of the same user might read. Give the runs a token you can revoke at Anthropic (`claude setup-token`); `xenonet runs sign-in forget` makes XenoNet drop it.

Shell commands can't read the rest of the home folder, so a toolchain installed there (`~/.cargo`, `~/.nvm`) isn't available to them. Put what a guest run needs in its folder or a system path. On Linux, Claude Code's sandbox needs `bwrap` and `socat` (`sudo apt-get install bubblewrap socat`); `xenonet doctor` checks. Claude Code keeps a command off Unix sockets with a seccomp filter it ships for x64 and arm64. Whether it loads can't be tested from outside Claude Code (its `/sandbox` shows it), so the doctor says so, and warns on any other CPU.

### Codex

The run is `codex exec --sandbox workspace-write` with approvals off (`approval_policy="never"`), network off, `/tmp` left out of what it may write, and no extra writable folders. Codex's sandbox can't name domains, so a start with domains is refused.

XenoNet also runs it with `--ignore-user-config` and `--ignore-rules`, so your `config.toml` (its `notify` command, its MCP servers, the projects it trusts, and with them a repository's own `.codex/config.toml`, hooks and rules) isn't loaded, and gives it XenoNet's MCP server alone. `codex exec resume` takes no `--sandbox`, so a resumed turn carries `sandbox_mode` and the limits through `-c` as well. The arguments parse with Codex 0.162.1; a sandboxed Codex run itself has not been run on a signed-in machine.

## After the start

- A turn resumed for the session later, or a copy forked from it, runs sandboxed the same way. If its sandbox can't be made again, the turn isn't run.
- What holds a session to its sandbox is a record in XenoNet's state folder (`sandboxed/`). It is written before the run starts, and a start whose record can't be written doesn't run. A copy of a confined session is given its own id before it runs (`--session-id` beside `--fork-session`) and gets its own record and mark before it is spawned, so a restart in the middle of its first turn leaves it held; every turn renews the record, and none goes by age (past 5,000 the longest unused go). A record kept before the store marks existed has no mark of its own: at service start it is marked once, as not yours, so none can be resumed bare.
- A start for anyone but you (a session, an actor, another machine, a delegation, an intent) also writes a mark in XenoNet's store, "confined: vendor, for <who>", before the run and with the record; a start whose mark can't be written doesn't run. The mark never expires and a copy gets one too. Only you pick a permission mode: a start for anyone else that names one (`acceptEdits`, `auto`, `dontAsk`, even `plan`) is refused by the machine that runs it, because in those modes Claude Code approves edits without asking, so the delegation's rules would never see them. A start that carries your id but isn't yours (an intent's) with no level, or `off`, is refused, not run bare. A turn for a session with a mark and no readable record is refused, and so is a turn when the store can't be read. Only a start of your own has the record alone: deleting it by hand lets a turn of yours run without the sandbox.
- The run's environment is the one an ACP agent gets: the service's without XenoNet's own variables, session marks and anything named for a secret or pointing at a Unix socket, plus the app's sign-in (`xenonet runs sign-in`).
- Under a delegation or a grant, a command that asks to run outside the sandbox (`dangerouslyDisableSandbox`) is never answered for you. It comes to Needs you and only you can allow it.

See [Starting sessions](network.md#starting-sessions) and [Delegating work to another machine](delegation.md).
