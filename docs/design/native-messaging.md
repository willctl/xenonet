# Native messaging

This proposes that XenoNet use the messaging that Claude Code and Codex already
have, and keep its own inbox for the cases they don't cover. It is a design for
review, and nothing in it is built yet. The facts about the two tools were
checked on 2026-10-09 against Claude Code's published docs and Codex 0.159.3.
Where something could not be checked, the text says so.

**In short:**

- Today every message goes through XenoNet's store and reaches an agent through a
  hook. An idle agent hears nothing until someone types to it.
- Claude Code sessions on one computer can message each other directly. A
  message to an idle session starts a turn, and a message to a busy one arrives
  between tool calls. XenoNet can't call that from outside, but the agents can.
- So for a Claude to Claude message, XenoNet steps aside. `send` tells the agent
  to use `SendMessage`, and XenoNet watches the transcripts so its feed and map
  still show the message.
- XenoNet's own delivery stays for Codex, for messages across tools, for repo and
  all broadcasts, for actors, and as a fallback when a native message doesn't
  arrive.
- Codex has no stable way to reach another session. The design leaves a place
  for one and says what to check first.

## Why

XenoNet's messages reach an agent at one of four moments: a session starts, the
user submits a prompt, a tool finishes, or the agent is about to stop. Those
come from the hooks in `core/engine/engine.js`. Between them the message waits.
`delivery()` already says so in its answer: "when it next starts a turn (it's
idle now)".

That is a poor fit for a Claude Code session in the Desktop app, left idle
while you do something else. A message from a second session could wait for
hours, and the sender gets no sign of that beyond the delivery note. The
`inbox` tool is a workaround: the agent has to remember to call it, and it
can't wait on anything that isn't running.

Claude Code now ships what XenoNet was approximating. Using it removes a whole
class of "the message is sitting in an inbox" cases, and XenoNet keeps doing the
part nothing else does: showing who is talking to whom across every tool.

## What the tools offer

### Claude Code

| Mechanism | Reaches an idle session | Who can use it | Status |
|---|---|---|---|
| `SendMessage` and `ListAgents` ([docs](https://code.claude.com/docs/en/cross-session-messaging)) | Yes. Claude Code starts a turn with the message. A busy session reads it between tool calls. | An agent inside a session. XenoNet can't call it. | Stable. Version 2.1.224 or later, 2.1.234 on Windows. |
| The session's inbox socket | Same path as `SendMessage` | A process on the same computer. Claude Code exports the socket's path as `CLAUDE_CODE_MESSAGING_SOCKET` and a token as `CLAUDE_CODE_MESSAGING_TOKEN` to its hooks and shell commands. | The docs describe the auth line only. The message format is not documented. |
| Channels ([docs](https://code.claude.com/docs/en/channels-reference)) | Yes, as the examples show: a POST to a channel server makes an idle Claude start work. | An MCP server that Claude Code spawns. It declares `claude/channel` and sends `notifications/claude/channel`. | Research preview. The user must start the session with `--channels`, or with `--dangerously-load-development-channels server:<name>` for a server not on Anthropic's list, which works in interactive sessions only. Organization policy can block it. |
| Agent teams mailbox | Yes | Teammates in one team | Experimental, behind an environment variable. No documented way for an outside process to write to it. |
| Hooks (`additionalContext`) | No. They fire at events. | XenoNet, today | Stable |

Three details of `SendMessage` matter for the design.

- **Holds.** Each session has an inbound setting, `crossSessionInbound`, with
  `accept`, `hold` and `refuse`. With no setting, Claude Code compares the two
  sessions' permission modes. It holds a message for approval when the receiver
  skips permission prompts and the sender doesn't, and the other way round.
  The Desktop app and the VS Code extension can't show the approval dialog, so a
  held message waits for `dialogExpiry`, 5 minutes by default, and is dropped.
  A send can succeed and the message still never reach the model.
- **What the receiver sees.** The message arrives as
  `<cross-session-message from="local_<id>" name="<session name>">text</cross-session-message>`
  with a line saying it came from another Claude session and not from the
  user. It can't approve a permission prompt or change configuration. A real
  transcript has exactly this shape in a user turn.
- **Addresses.** An agent sends to a session's name, with a short `[ref]` when
  two share a name. `from` carries the sender's session id.

### Codex

| Mechanism | Reaches an idle thread | Who can use it | Status |
|---|---|---|---|
| `turn/start`, `turn/steer`, `thread/inject_items` in the app-server protocol | Yes, through the process that owns the thread | A client of that app-server | Experimental protocol |
| A shared local app-server daemon: `codex app-server daemon`, `codex app-server proxy --sock`, `codex agents`, `--remote unix://PATH` | Yes, for threads that run on it | Any client that reaches its control socket | Experimental. Whether Codex Desktop and the terminal client run their threads on it is **unchecked**. On the computer where this was checked the daemon wasn't running. |
| `multi_agent` (stable) | Within one thread's own subagents | The model | Not a link between sessions |
| `agent_message_board` | Unknown | Unknown | Listed as under development and off |
| Hooks | No | XenoNet, today | Stable |

XenoNet's Codex client starts a private app-server that only reads. It receives
no events from the app-server a Codex client runs for its own threads, so it
could not steer those threads today. The two ways in are the shared daemon,
if clients use it, and a client that Codex itself starts with a socket.

## The design

### Choose the route per recipient

`send` decides, for each recipient, how a message should travel, and says so in
its answer.

| Recipient | Route |
|---|---|
| A Claude Code session on this computer, with messaging on | **Native.** The sender uses `SendMessage`. |
| A Claude Code session without it: an old version, a bare-mode run, `refuse`, or a session Claude Code can't list | **Hooks**, as now |
| A Codex thread | **Hooks**, as now. Later, **steer** if the thread runs on the shared daemon |
| An actor | **Inbox**, as now |
| An agent on another computer | **Native** if both sessions are connected to Remote Control, otherwise **relay** and hooks |
| Everyone in a repo, or everyone | XenoNet's own delivery to those that need it, and a `SendMessage` for each Claude Code session that has messaging |

For a native route, `send` does not store the message. It returns the recipient
and the exact call to make, for example `SendMessage to "api-worker"`, and the
text to send. The agent makes the call. This keeps a message from being sent
twice and keeps the delivery inside Claude Code's own controls.

`peers` gains a `native` field for each Claude Code session: the name `ListAgents`
shows for it, when XenoNet can tell. The `xenonet` MCP tool descriptions say the
rule in a sentence, so an agent that skips `peers` still gets it.

### Watch what the tools do

XenoNet already reads every transcript. It adds two readers for Claude Code's
messages.

- **Sent.** A `tool_use` named `SendMessage` in the sender's transcript, with
  its `to`, `summary` and `message`.
- **Received.** A `<cross-session-message from=… name=…>` block in the
  receiver's transcript.

XenoNet pairs the two by session, text and time, and then shows the message in the
feed and as an arc on the map like any other, marked **native**. The mark means
Claude Code delivered it and attributed the sender. XenoNet's `verified:` line
doesn't apply, because XenoNet never saw the sender's process. The feed says so.

The pairing gives XenoNet a receipt, which `SendMessage` itself doesn't give a
sender. A message seen in the sender's transcript but not in the receiver's
after a short wait (a minute by default) is shown as **not seen**. From there
XenoNet can offer the sender a fallback: store the same text for hooks to
deliver. It does so only when the sender asks, because the receiver may have
held the message on purpose.

### Keep the inbox for what's left

The store, `inbox`, claims, broadcasts and the hook delivery stay. They do
three jobs that nothing native does: reach Codex and actors, reach every agent
in a repo including ones that start later, and warn about a file another agent
is editing. The change is that Claude to Claude stops using them.

### What XenoNet does not do

- **It does not write to an inbox socket.** Claude Code exports the path and
  token to a session's own hooks, so XenoNet could pass them on and post from the
  service. But the format after the auth line isn't documented, and an
  undocumented wire format can change in any release. If Anthropic documents
  it, a XenoNet that sends for a recipient who can't call `SendMessage` becomes
  possible, and the table above gets one more row.
- **It does not turn on channels.** A `xenonet mcp` server could declare
  `claude/channel` and push a stored message into an idle session. That wakes
  the session, which is the missing piece for Codex to Claude messages. It also
  needs the user to start every session with a flag, and a development-channel
  warning on each start. The design keeps a place for it (see Phases) and
  waits for it to leave the research preview.
- **It does not steer a Codex thread it doesn't own.** A second client resuming
  a thread another process is running can fork the thread's history. Steering
  is only for threads that run on a daemon XenoNet reaches.

## Security

- **A message is data.** XenoNet's `verified:` envelope and quoting stay for
  XenoNet-delivered messages. Native messages keep Claude Code's own framing and
  limits: they can't approve anything and can't change configuration.
- **No permission laundering.** An agent must not ask a peer to do what its own
  session would block. `send`'s answer repeats the rule when it routes native.
- **Holds are visible.** XenoNet never tries to get a held message past Claude
  Code's inbound control. It reports "not seen", and the user decides.
- **Settings stay the user's.** XenoNet never edits `crossSessionInbound` or
  `dialogExpiry`. The doctor can say when a setting makes native messages
  between your sessions unlikely to arrive, for example sessions that skip
  permission prompts talking to ones that don't.

## Phases

1. **Route and hint.** Per-recipient routing in `send`, `native` in `peers`,
   the tool descriptions, and tests with fake sessions. Nothing changes for
   Codex, actors or broadcasts.
2. **Watch.** The two transcript readers, native entries in the feed and on the
   map, and the receipt with the "not seen" state and the fallback.
3. **Codex.** First check whether Codex's clients run threads on the shared
   daemon and what its control socket accepts. If they do, add a route that
   starts or steers a turn on a daemon-owned thread, labeled as coming from
   another agent, with the user's opt-in because it starts work.
4. **Idle Claude from outside.** The channel from `xenonet mcp`, once channels
   leave the research preview, and a socket sender if the format is published.

Each phase leaves the earlier behavior working.

## What to check before building

1. **Names and ids.** `from` is a `local_<id>` for sessions in the Desktop app,
   while XenoNet's agent ids use the transcript's session id. The mapping is
   unconfirmed, so phase 2 starts by finding it, with the session name as a
   second key.
2. **Whether a send reaches a Desktop session in a different permission mode.**
   The docs say it is held and expires. Try it between two sessions with
   different modes and record what each side shows.
3. **Whether Codex clients use the shared daemon.** Start a Codex terminal
   session and look for the control socket under the Codex home folder, then
   run `codex agents`.
4. **The transcript form on each Claude surface.** The Desktop app, the CLI and
   the VS Code extension should all write the same blocks. Confirm with one
   message on each.

## Decisions to confirm

Each has a default. Phase 1 starts with the defaults unless you change one.

1. **Native by default.** Default: `send` routes Claude to Claude messages
   native whenever it can. A setting turns that off for a user who wants
   XenoNet's delivery everywhere.
2. **No second copy.** Default: a native message is not also stored for hooks.
   The fallback is a separate, explicit action.
3. **The wait before "not seen".** Default: one minute.
4. **Codex steering.** Default: off until phase 3 finds a safe route, and then
   behind a setting that starts off.
5. **Channels.** Default: wait for general availability. Build nothing now.
