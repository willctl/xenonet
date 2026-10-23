# Live delivery and scopes

A message from one session to another normally waits for the hooks: the
other session gets it during its current turn, or when it next starts one.
Live delivery puts it into the session as it's sent, even when the session
is idle. This page covers how, how to choose who gets a message, what actors
may see, and the guards that keep messages from being misused.

## How a message gets in

XenoNet tries these in order and uses the first that works:

| Way in | For | What it takes |
|---|---|---|
| Listening after a turn | Claude Code, including headless runs (`claude -p`) | Start the session with XenoNet's listen settings (below). |
| Claude Code channel | an idle Claude Code CLI session | Claude Code 2.1.80 or later, started with XenoNet as a channel (below). A working session gets the message from its hooks during its turn, which is sooner. |
| OpenCode's server | OpenCode TUI | Start OpenCode with `--hostname 127.0.0.1`, and allow the session: `xenonet live allow <session>`. |
| Typing into tmux | any harness in a tmux pane | Allow the session. The sender must be you, or an actor with a token. |
| Headless resume | a Claude Code or Codex session that has ended | Allow the session. XenoNet starts it again for one turn. |
| Hooks (next turn) | Claude Code, Codex, Grok Build | Nothing: this is how messages always arrived. |

If nothing picks a message up within a minute, XenoNet notifies you. A
message pushed through a channel or typed into a pane that nothing
acknowledged also goes back to the hooks, so the session's next turn still
gets it. `xenonet live` shows each delivery's state: `live`, `next-turn` or
`notified`, and how long it took to be acknowledged. The sender's answer
says which way each message is going: "right away, through its channel",
or when the session next starts a turn.

### Claude Code

```
xenonet doctor --live
```

prints the command to start Claude Code with XenoNet as a channel:

```
claude --dangerously-load-development-channels server:xenonet
```

Channels are a Claude Code research preview
([reference](https://code.claude.com/docs/en/channels-reference)), and
XenoNet's channel isn't on Anthropic's allowlist yet, so Claude Code asks
you to confirm each time the session starts. Team and Enterprise
organizations have to turn channels on first (`channelsEnabled`). XenoNet
only pushes to a session that was started this way, naming its own server
(`server:xenonet`, or `server:xenonet-dev` for XenoNet Dev). Any other
session gets messages through its hooks, as before.

Claude Code asks MCP servers for protocol revision 2026-07-28 first, but
registers a channel only on the 2025 handshake, since the newer revision has
no way to carry a channel's messages. So XenoNet's MCP server answers a
session started as a channel on the 2025 handshake. If a message doesn't
arrive, start the session with `claude --debug` and look for "Channel
notifications" in the debug log, which says whether the channel was
registered and why not.

### Listening after a turn

A session started with XenoNet's listen settings waits at the end of each
turn, for up to five minutes, for a message. One that comes keeps it going,
as if it had arrived during the turn. `xenonet doctor --live` prints the
command:

```
claude --settings "$HOME/…/listen-47840.json"
```

The settings add one Stop hook, beside the hooks in your other settings,
and only to that session, so no other session waits on XenoNet at the end
of a turn. This works where channels don't: in headless runs (`claude -p`),
which can then take a reply before they exit. Each message it hands over
counts against the wake budget. A message that comes while the session
isn't listening, or that the budget holds back, waits for its next turn.
Interrupt a listening session as you would any turn (Esc, or Ctrl-C for a
headless run).

### Resuming a session that has ended

A Claude Code or Codex session whose terminal closed, or whose headless run
finished, has no process to take a message. If you allow it
(`xenonet live allow <session>`), XenoNet starts it again for one turn
with the message as its prompt: `claude -p --resume <id>` or
`codex exec resume <id>`. The turn goes into the same session, so you see
it when you next open it. XenoNet does this only while no process runs the
session, one turn at a time, in the session's own folder, with the message
passed on standard input. A resumed Claude Code turn listens afterwards,
as above, so it can take an answer. Allowed sessions that ended stay among
the ones you can message by name.

### OpenCode

OpenCode's TUI runs an HTTP server only when it's told where to listen
([server docs](https://opencode.ai/docs/server/)). Start it with
`opencode --hostname 127.0.0.1`. XenoNet finds the running OpenCode
processes and their ports, and posts the message to the session's
`prompt_async`, which starts a turn. Messages go in only to sessions you
allow, and each allowance expires (8 hours by default). `xenonet live allow`
takes the `ses_…` id OpenCode shows. The sessions you allow are among the
agents, with the titles OpenCode gives them, so other sessions can message
them by title words or by a selector such as `app:opencode`.
[OpenCode sessions messaging each other](opencode-messaging.md) has a demo
of it, and the tools that let an OpenCode session send.

### tmux

For a harness with no way in, XenoNet can type the message into the tmux
pane the session runs in. It does this only when all of these hold:

- You allowed that session (`xenonet live allow <session>`), and the
  allowance hasn't expired.
- The sender is you or an actor with a token. A message from another session
  never gets typed.
- The session is idle and isn't waiting on you.
- The session's own process is in the foreground of the pane's terminal.
  XenoNet checks this twice, the second time just before it types.

The text goes in as one line, starting with `[XenoNet from …]`, with control
characters and escape sequences removed. It's sent literally, then Enter.
Every attempt goes in the audit log. On macOS, tmux needs no extra
permissions.

## Choosing who gets a message: selectors

`send`'s `to` and `peers`' `scope` take a selector: one or more `key:value`
terms, all of which must hold.

| Key | Matches |
|---|---|
| `project:atlas` | the project's name, or its repo's folder |
| `identity:adal` | an account the session ran as or belongs to: a GitHub login, a git email, a Claude account |
| `app:opencode` | the harness: `claude`, `codex`, `opencode`, `gemini-cli`, … |
| `machine:local` | the machine |
| `status:idle` | `idle`, `working`, or `live` (working or idle) |
| `by:hermes` | sessions an actor reports it started or works on |

A value ending in `*` matches a prefix. Separate terms with spaces or `+`,
for example `project:atlas app:opencode` or `identity:adal+status:idle`.

Everything `to` took before still works the same: a session's id
(`claude:…`), an actor's id (`actor:hermes`), words from a title, `repo` and
`all`.

## What actors may see: scopes

An actor sees and reaches no session until you grant its token a scope:

```
xenonet live grant hermes project:atlas identity:adal --hours 72
xenonet live revoke hermes
```

- A grant is a selector. The actor's `peers`, its `send` to an id or words,
  and its broadcasts (`all`, `repo`, a selector) only reach what the grant
  covers. So do the counts `whoami` gives it, and the sessions `who_has`
  and a claim's "also held by" name.
- A grant expires (30 days by default). `revoke` takes effect at once.
- A grant for one identity never shows another identity's sessions, so
  separate accounts stay apart.
- An actor sees another actor only if its grant names it with `by:<slug>`.
- `by:` matches what actors report they work on, so a grant can't use the
  actor's own slug: an actor's reports never widen its own scope.

You and the sessions on this machine see everything, as before.

## Guards

| Guard | Default |
|---|---|
| Pause everything | `xenonet live pause`, or "Pause or resume messages between sessions" in the command palette. While paused, only you can send, and nothing is woken. |
| Message size | 4,000 characters |
| Per sender | 20 messages a minute |
| Per recipient | 30 messages a minute |
| Fan-out | 25 recipients per message |
| Duplicates | the same text to the same recipients within a minute is refused |
| Loops | 24 messages between the same two agents in ten minutes, or a reply chain 6 deep, stops the thread until you step in |
| Wake budget | 12 wakes an hour per sender, 30 per session. A provider at 90% of its plan limit takes no wakes, from XenoNet's live readings of Claude Code's and Codex's limits; `xenonet live` shows them. |
| Consent | waking an OpenCode session, typing into any session, or resuming one that ended needs your allowance for that session, which expires |
| Secrets | key- and token-like strings are redacted when a message crosses identities, or goes to or from an actor |
| Framing | every delivered message names its sender and says it's another agent's message, not your user's instruction |
| Audit | `xenonet live audit`: an append-only log of grants, consents, pauses, refusals, deliveries and typing, with message hashes, never text |

## Threat model

| Threat | What stops it |
|---|---|
| An actor reads or messages sessions it shouldn't | Default deny, per-actor grants that expire, instant revoke, scope checks on peers, ids, title words and broadcasts |
| One account's agent reaches another account's sessions | Identity terms in grants. Secrets are redacted across identities. |
| Prompt injection through a message | Framing as peer content with sender provenance. Messages can't approve or authorize anything. Control characters are stripped. |
| Typing into the wrong window | Per-session consent; pid, tty and foreground checks done twice; tmux only; one literal line |
| Two agents in a loop, or a flood | Per-sender, per-recipient and per-pair limits, duplicate checks, reply depth, fan-out cap |
| Wakes burning plan limits | Wake budgets per sender and session, and no wakes for a provider near its limit |
| A resumed turn doing more than you expected | Only sessions you allowed, one turn per message, in the session's own folder, under Claude Code's or Codex's own permission settings for headless runs; every resume is audited |
| A token or key passed along | Redaction across identities and actors |
| Something goes wrong at scale | The pause switch in the CLI and the command palette |
| Finding out what happened | The append-only audit log |

What it doesn't stop: a session or actor you allowed can still say
misleading things within its scope, and a model can still choose to act on a
peer's message. Keep grants narrow.
