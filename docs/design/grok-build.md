# Grok Build in XenoNet

Grok Build is SpaceXAI's coding agent for the terminal, the `grok` command
(open source, Apache-2.0, [xai-org/grok-build](https://github.com/xai-org/grok-build)).
XenoNet treats its sessions as it treats Claude Code's and Codex's. They're
on the map, in the live tailer, in stats, search and the doctor, and they can
see and message the other sessions once you connect Grok Build.

This note says what Grok Build keeps, how XenoNet reads it, what XenoNet
writes into Grok Build's settings, and what still needs a real session to
confirm. It was written against Grok Build 1.0.50 (2026-10-06) and the user
guide that ships with it in `~/.grok/docs/user-guide/`. No real Grok Build
session was read. The formats beyond the guide come from the guide's
examples, `grok --help`, `grok mcp add` run against a temporary
`GROK_HOME`, and the field names the binary carries.

## What Grok Build keeps

Everything lives under `~/.grok`, or the folder `GROK_HOME` names.

| Path | What | Source |
| --- | --- | --- |
| `sessions/<group>/<session id>/` | One folder per session. The group is the working directory, URL-encoded. When that's over 255 bytes, the group is a slug and a hash, with the folder in a `.cwd` file inside it. | User guide, sessions |
| `…/summary.json` | The session's index entry: `info` (its id and folder), `generated_title`, `title_is_manual`, `session_summary`, `created_at`, `updated_at`, `current_model_id`, `parent_session_id`, `last_turn_summary` and more | User guide, sessions |
| `…/updates.jsonl` | The conversation, one ACP session update a line | User guide, sessions |
| `…/usage.json` | Tokens and cost, for the session and each turn, in the shape `grok usage <id>` prints | The binary's file and field names |
| `…/signals.json` | Turn and tool counts, and token usage | User guide, sessions |
| `…/subagents/<id>/meta.json` | A subagent's record. The subagent's own session is in the sessions folder like any other. | User guide, sessions |
| `worktrees/<repo>/<name>/` | Worktrees Grok makes for a session (`grok -w`, `/fork --worktree`, a subagent with `isolation: worktree`) | User guide, sessions |
| `hooks/*.json` | Your own hooks, which Grok runs without asking | User guide, hooks |
| `config.toml` | Settings, MCP servers (`[mcp_servers.<name>]`) and hooks (`[[hooks.<Event>]]`) | User guide, configuration |
| `auth.json` | Grok's sign-in, tokens included | User guide, authentication |
| `disabled-hooks` | Hooks you turned off in `/hooks` | User guide, hooks |

## How XenoNet reads it

`core/grok.js` lists the session folders on every refresh. Grok Build has no
listing API, and `grok sessions list` prints one folder's sessions for
people.

- A session is `grok:<session id>`, the id in `summary.json`'s `info`, or
  else its folder's name.
- Its title is `generated_title`, else the first line of `session_summary`,
  else "(untitled session)". `title_is_manual` says you named it.
- Its folder is `info.cwd`, else its group decoded, else the group's
  `.cwd`. A session in one of Grok's worktrees belongs to the repo the
  worktree came from, read from the worktree's `.git` file, as Claude
  desktop's worktree sessions belong to theirs. One XenoNet can't place
  goes to the Grok worktrees zone.
- A session a subagent `meta.json` names is that parent's subagent. One
  with `parent_session_id` and no subagent record is a fork.
- A session Grok marks `hidden`, or whose `session_kind` reads as headless,
  is background work, as `codex exec` runs are.
- A folder without `summary.json` but with `updates.jsonl` is a session
  being made. It shows untitled, at the time its updates were written.

Every file goes through a lenient schema (`spacexai/grok-build/*` in
`core/schema/vendor/spacexai/grok-build.js`), so a change in Grok's
formats shows as drift instead of quietly emptying the map.

### The conversation and live activity

`core/lines.js`'s `GrokLine` reads a line of `updates.jsonl`. The guide says
only that each line is "a self-contained ACP session update event". The
binary builds an envelope for each line, so XenoNet reads the update from
`update`, from `params.update`, or from the line itself, and the time from
`timestamp` and the other places an envelope might keep one.

| ACP update | XenoNet |
| --- | --- |
| `user_message_chunk` | A prompt. A run of them is one prompt. |
| `agent_message_chunk` | The agent's message. A run of them is one message. |
| `agent_thought_chunk` | Reasoning, in the transcript |
| `tool_call`, `tool_call_update` | A tool call, and how it went. The tool's name comes from `_meta`, else from ACP's kind (`execute` is `run_terminal_command`). An MCP tool's `server__tool` is written `mcp__server__tool`, as the rest of XenoNet reads it. |
| a `diff` in a tool call's content | An edit, for collisions and the files a session has in hand |
| `usage_update` | How full the context is, and its window |
| `plan` | A plan, in the transcript |

The updates don't say when a turn ends, and Grok's hooks do. `Stop`,
`StopCancelled` and `StopFailure` end it, `UserPromptSubmit` starts one,
and a `permission_prompt` notification waits on you. Without hooks, a turn
whose last word was the agent's, with no tool call open, ends after 30
quiet seconds.

On macOS, the process table says which sessions are open. A grok process
holds the session in its folder written to last since it started. That
is the `running` flag, shown as "open in Grok Build" in the inspector.
XenoNet tells `grok` apart by its path. The `agent` command Grok's
installer links beside it is told apart from Cursor's `agent` by the
executable it runs.

### Tokens and cost

`core/grok-usage.js` reads `usage.json`, or `signals.json` when there's no
ledger. Grok's input count is the whole prompt in some places and only the
uncached part in others. The total says which, and XenoNet takes the cache
out of input when the parts add up that way. Grok's own cost
(`costUsdTicks`, 10^10 to the dollar) wins. Without it, XenoNet prices the
tokens with SpaceXAI's published prices in `core/pricing.js`, checked
2026-10-10 against docs.x.ai's pricing page.

The tailer reads each working session's ledger every 30 seconds. The first
read counts the last day's turns when each ended. Later reads count what's
new, now. Session stats read the same ledger, with each turn as a call.

## What XenoNet writes

Connecting Grok Build (`xenonet connect grok`, or the palette) writes two
things, both under `GROK_HOME`.

- `[mcp_servers.xenonet]` (and `xenonet-console` for the installed
  XenoNet) in `config.toml`. `grok mcp add` rewrites the whole file and
  drops your comments, so XenoNet edits the text itself
  (`core/engine/toml.js`). It takes out its own tables and adds them at
  the end. Every other line stays as it was, it keeps the file's mode, and
  it saves a copy as `config.toml.before-xenonet` the first time. A server
  of that name written another way is left alone, and the panel says so.
  Grok parsed the result in a temporary `GROK_HOME` (`grok mcp list
  --json`).
- `hooks/xenonet.json`, a file of XenoNet's own. Grok runs the hooks in
  `~/.grok/hooks` without asking, unlike Codex. Every XenoNet on the
  machine shares the file, each with its own port, as with Codex's
  `hooks.json`. Disconnecting takes out its own entries, and the file
  goes with the last of them.

The hooks run `xenonet hook grok`, which posts the event to
`/api/hook/grok` (launcher 3). XenoNet answers in Grok's terms, which
differ from Claude Code's.

| Event | What Grok does with an answer | What XenoNet answers |
| --- | --- | --- |
| `SessionStart` | Ignores it | Nothing. The session hears who's here with its first tool result instead. |
| `UserPromptSubmit` | Throws away `additionalContext` | Nothing, and the session's messages stay unread |
| `PreToolUse` (edits) | Gives `additionalContext` with the call's result | A warning when another session has a hand in the file |
| `PostToolUse`, `PostToolUseFailure` | Gives `additionalContext` with the result | Who's here, the first time, and the session's messages |
| `Stop` | Keeps the turn going with `decision: block` and the reason | The session's messages, unless the stop hook already ran this turn or the session is closing (`reason` other than `end_turn`) |
| `StopCancelled`, `StopFailure`, `Notification`, `SessionEnd` | Nothing | Nothing. They tell the tailer when a turn ended, and the engine when a session ended. |

Grok marks each event inside a subagent's own session with
`subagentType`. XenoNet answers those with nothing and marks nothing read,
so a message for the session reaches the conversation you're in, never a
subagent that would read it and drop it. The tailer ignores a subagent's
turn ends too.

Grok also runs Claude Code's hooks from `~/.claude/settings.json`. XenoNet's
Claude Code hooks pass over Grok's events, which carry Grok's
`hookEventName` and `workspaceRoot`, and a command hook sees
`GROK_HOOK_EVENT` in its environment.

### Which session is calling

Requests in a Grok session's name must come from a grok process, or from
under one, as Codex's must come from Codex (`core/engine/verify.js`). The
xenonet MCP server finds its session in this order:

1. `XENONET_GROK_SESSION`, which connecting writes as `{{session_id}}`. Grok
   fills that placeholder in for a remote server's headers. Whether it does
   for a local server's environment isn't documented.
2. `GROK_SESSION_ID`, which Grok sets for hooks and may set for MCP servers.
3. The call's `_meta`.
4. Otherwise the server says only that a grok calls. XenoNet then finds the
   grok process the call comes from, and takes the session that process's
   hooks last pinned to it. This needs Grok's hooks and, for now, macOS.

## A desktop host later

No official desktop app hosts Grok Build sessions yet. If one does, it fits
in beside `grok.js` the way Claude desktop fits beside Claude Code. It would
need a reader of its own records for titles, renames and lineage, an
`open-app` action in `core/open.js` with its link, and its name in
`APP_DESKTOP` in `src/theme.js`. Grok Bot, SpaceXAI's always-on agent, stays an actor.

## Open questions

These need a real Grok Build session to settle.

- The envelope around each line of `updates.jsonl`, its keys, and where it
  keeps the time.
- Whether a tool call's `_meta` names the tool, and what ACP `kind` each
  of Grok's tools reports.
- `usage.json`'s exact shape, and whether its `inputTokens` include the
  cache.
- `subagents/<id>/meta.json`'s fields, and whether a subagent's summary
  says its parent.
- The values of `session_kind`, and whether headless runs set `hidden`.
- Whether `{{session_id}}` is filled in in a local MCP server's
  environment, and whether Grok starts one MCP server per session.
- Whether a hook's `sessionId` inside a subagent is the subagent's own
  session or the parent's. XenoNet answers neither.
- Whether writing `summary.json` while the session is open loses the new
  name when Grok writes its own copy.
- The format of `disabled-hooks`. The doctor looks for "xenonet" in it.
- How Grok reports a refusal. The tailer doesn't tell them apart yet.
- Grok's sign-in as an identity. XenoNet doesn't read `auth.json`.

## Checking it against a real install

Once `grok` is signed in and has run a session or two, in a repo, with the
installed XenoNet connected:

1. Run `xenonet doctor`. The Grok Build checks pass, or say what to fix.
2. Run `xenonet connect grok`, then
   `diff ~/.grok/config.toml.before-xenonet ~/.grok/config.toml`. Only
   XenoNet's tables are new, and `grok mcp list --json` lists them.
3. In a Grok session, run `/hooks`. XenoNet's hooks are listed and enabled.
4. Run `ls ~/.grok/sessions/*/` and `head -c 2000 <session>/updates.jsonl`.
   Compare the envelope's keys and the updates' kinds with `GrokLine` in
   `core/lines.js`.
5. Run `cat <session>/summary.json` and `ls <session>`. Check for a
   `usage.json`, and compare it with `spacexai/grok-build/usage`.
6. Run `grok usage <session id>`. Its totals match what
   `xenonet session <id>` shows.
7. Run `xenonet drift`. Anything under Grok Build's sources names the field
   that moved.
8. Ask the session to call XenoNet's `whoami`. It names `grok:<its id>`. If
   it fails, Grok didn't fill in `{{session_id}}` and the hooks haven't
   pinned the process yet.
9. From another session, `send` the Grok session a message, then have the
   Grok session run a tool. The message arrives with the result, once. Run a
   subagent in it, and the subagent doesn't get it.
10. Rename the session in XenoNet while Grok has it open, send a prompt,
    and look at the title in `summary.json`.
11. Run `ps -axo pid,comm | grep -E 'grok|agent'`. The session's process is
    there, and the inspector says the session is open.
