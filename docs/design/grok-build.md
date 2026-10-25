# Grok Build in XenoNet

Grok Build is SpaceXAI's coding agent for the terminal, the `grok` command
(open source, Apache-2.0, [xai-org/grok-build](https://github.com/xai-org/grok-build)).
XenoNet treats its sessions as it treats Claude Code's and Codex's. They're
on the map, in the live tailer, in stats, search and the doctor, and they can
see and message the other sessions once you connect Grok Build.

This note says what Grok Build keeps, how XenoNet reads it, what XenoNet
writes into Grok Build's settings, and what's still to do. It was written
against Grok Build 1.0.50, its source and the user guide that ships with it
in `~/.grok/docs/user-guide/`. The tests' Grok Build files are made up in
the shapes the source writes.

## Where it sits

Grok Build is an app in the registry (`core/apps.js`, schema `app/v1`), with
the id `grok`, like Claude Code's and Codex's entries:

- `roots`: `GROK_HOME`, else `~/.grok`.
- `context`: a 256,000-token window, until its usage updates say the size.
- `team`: the Apple team that signs the `grok` binary.
- `has`: `lines` (its updates go through the shared event stream),
  `sessions`, `connect`, `processes` and `caller`.

The Rust copy of the registry is generated from it (`node scripts/apps.mjs`).
Each capability is the same in JS and in Rust. The JS is the contract the
tests hold the binary to.

## What Grok Build keeps

Everything lives under `~/.grok`, or the folder `GROK_HOME` names.

| Path | What |
| --- | --- |
| `sessions/<group>/<session id>/` | One folder per session. The group is the working directory, URL-encoded. When that's over 255 bytes, the group is a slug and a hash, with the folder in a `.cwd` file inside it. |
| `…/summary.json` | The session's index entry: `info` (its id and folder), `generated_title`, `title_is_manual`, `session_summary`, `created_at`, `updated_at`, `last_active_at`, `current_model_id`, `parent_session_id`, `session_kind`, `hidden`, `source_workspace_dir`, `head_branch` and more |
| `…/updates.jsonl` | The conversation, one ACP session update a line, each in an envelope: `{ timestamp, method, params: { sessionId, update } }`, with the time in Unix seconds |
| `…/subagents/<subagent id>/meta.json` | A subagent the session ran, in snake case: `subagent_id`, `parent_session_id`, `child_session_id` (the subagent's own session, in the sessions folder like any other), `subagent_type`, `description`, `prompt`, `status`, `started_at` and more. Grok's cloud copy, `subagent.json`, has the same in camel case. |
| `…/usage.json` | The session's tokens and cost, written as each turn ends: `{ sessionId, updatedAt, session, turns: [{ turnNumber, endedAt, turnId, …usage }] }`, each usage with `inputTokens`, `outputTokens`, `cachedReadTokens`, `cacheCreationTokens`, `reasoningTokens`, `modelCalls`, `costUsdTicks` and the same by model (`modelUsage`) |
| `hooks/*.json` | Your own hooks, which Grok runs without asking |
| `config.toml` | Settings and MCP servers (`[mcp_servers.<name>]`) |
| `auth.json` | Grok's sign-in, tokens included |
| `disabled-hooks` | Hooks you turned off in `/hooks` |
| `bin/`, `downloads/` | The `grok` command its installer puts there, and the binaries it links to |

## Reading it

### Sessions

`core/grok.js` (and `crates/xenonet-sources/src/grok.rs`) lists the session
folders on every refresh. Grok Build has no listing to ask: `grok sessions
list` prints one folder's sessions, for people.

- A session is `grok:<session id>`, the id in `summary.json`'s `info`, or
  else its folder's name.
- Its title is `generated_title`, else the first line of `session_summary`,
  else "(untitled session)".
- Its folder is `info.cwd`, else its group decoded, else the group's
  `.cwd`. A session in one of Grok's worktrees (`source_workspace_dir`)
  belongs to the folder its work came from.
- A session whose `session_kind` starts with `subagent` and names a
  `parent_session_id` is that session's subagent, and so is one a
  session's `subagents/*/meta.json` names as its `child_session_id`,
  which Grok can write before the subagent's own summary. The meta.json
  gives its type (`subagent_type`, the session's `agentKind`), which the
  session's panel shows beside where it came from ("Grok Build subagent ·
  explore", with its parent linked), and what it was asked to do, its
  title until its summary has one. One with a parent and another kind is
  a fork.
- A session Grok marks `hidden`, or a `headless` one, is background work,
  as `codex exec` runs are.
- A folder with `updates.jsonl` and no `summary.json` yet is a session being
  made. It shows untitled, at the time its updates were written.

Both files, and `usage.json` and `meta.json`, go through lenient schemas
(`spacexai/grok-build/session-summary`, `spacexai/grok-build/update-line`,
`spacexai/grok-build/usage` and `spacexai/grok-build/subagent-meta`, in
`core/schema/vendor/spacexai/grok-build.js`), so a change in Grok's
formats shows as drift instead of quietly emptying the map. The daily
provider drift check watches Grok Build's releases too.

### The conversation and live activity

`GrokLine` (`core/lines.js`) reads a line of `updates.jsonl`, and the
`grok` mapper (`core/events.js`, and `line_events.rs`) turns it into the
events the tailer, stats and search read for every app.

| ACP update | Events |
| --- | --- |
| `user_message_chunk` | A prompt. A run of chunks is one prompt, all their text joined. |
| `agent_message_chunk` | The agent's message. A run of chunks is one message, all their text joined. |
| `agent_thought_chunk` | The agent's thinking, as Claude Code's is kept: shown folded in the session's transcript, and not indexed |
| `tool_call` | A tool call. Its name is the one Grok gives its own tools (`_meta`'s `x.ai/tool`), or an MCP tool's `server__tool` title written `mcp__server__tool`, or else ACP's kind. Its input names its file as `file_path`, from the call's locations when the input doesn't. A call to one of Grok's edit tools (the registry's `editTools`: `search_replace`, `write`, `edit`, `hashline_edit`, `apply_patch`) is an edit: one given `content` is a write, another an edit, so the map draws collisions with it and other sessions are warned about the file. |
| `tool_call_update` | How the call went, once it's completed or failed |
| `usage_update` | How full the context is, and its window |
| Grok's own `_x.ai/session/update` lines | Nothing |

A run of chunks is one message with all its chunks' text, in the live
tailer, stats, search and the session's transcript. The mapper reads a
line at a time: a run's first chunk makes the message's events, and each
chunk after it says what it adds and the message's text so far, which
each reader puts in place of what it kept. A tool call, one of Grok's own
updates (a turn's end is one), or a chunk of another kind or prompt ends
the run; the context's fill doesn't. [blocks.md](blocks.md#grok-build-updates)
has what each reader does with it. The activity feed names a message once,
as it begins, by its first chunk's text.

The updates don't say when a turn ends. Grok's hooks do. `Stop`,
`StopCancelled` and `StopFailure` end it. A session XenoNet hasn't heard a
hook from in the last hour ends a turn after a minute with no new line and
no tool call open.

### Tokens and cost

The updates say how full the context is (`usage_update`), not what a call
took. Grok keeps that in the session's `usage.json`, which it writes as
each turn ends, before the update that ends the turn
(`session/usage_file.rs` in its source). XenoNet reads it with
`grokUsageTurns` (`core/grok-usage.js`, `usage_turns` in `grok.rs`), through
its lenient schema (`spacexai/grok-build/usage`), and counts it as it
counts Claude Code's calls:

- A turn is a call on each model it used (`modelUsage`, else the turn's
  own figures on `primaryModelId`), at the time it ended (`endedAt`), with
  its model calls (`modelCalls`). A turn with no end time is left out, as
  a line with no time is.
- `inputTokens` is the whole prompt. Grok's ledger sums the API's
  `prompt_tokens`, which hold the cache reads, and adds cache reads and
  writes into it for a model on Anthropic's API. So XenoNet's input is
  `inputTokens` less `cachedReadTokens` and `cacheCreationTokens`, with
  those two apart. The output is `outputTokens`, reasoning included, as
  the Responses API counts it; `reasoningTokens` isn't added again.
- The cost is `costUsdTicks / 1e10` dollars, as Grok recorded it. XenoNet
  has no xAI prices and doesn't guess one. Grok records a cost only when a
  call says it, which an API key's calls do and a SuperGrok or X Premium
  sign-in's often don't. With none, or with only some (`costIsPartial`),
  the cost isn't known: it's null, never 0, wherever XenoNet shows a cost.
- The session totals (`session`) include history a session took over, so
  XenoNet counts turns, not totals. A fork, and a session resumed into a
  new one, starts with a copy of its parent's ledger, so in a session that
  came from another (`parent_session_id`), the turns that ended before it
  was made (`forked_at`, else `created_at`) are the parent's and aren't
  counted again.
- Grok adds a subagent's usage into the turn of its parent's that ran it,
  and the subagent's own session writes its own ledger too. So the live
  tailer, Token use and `xenonet spend` count a subagent's tokens once,
  through its parent, and the subagent's own stats show its own turns.

The live tailer reads `usage.json` again after each of the session's lines
when it was written since (its size and time), and counts what each turn
adds: Grok folds a message you send during a turn into that turn, which
writes it again with more. Token use's line for Grok Build, the session's
tokens and the day's tokens come from it. Today's cost for Grok Build is
what Grok recorded; when none of its calls today has a cost, it's null
(`costs.byApp.grok`), and so is the day's (`costs.today`) when no call of
any app has a price. The model has no price either way, so it's named
under "not counted". `xenonet spend` shows it as unknown.

A session's stats (`core/stats.js`) put each turn's calls among its lines'
events, when the turn ended. Their cost isn't split by kind of token, so
the kinds' costs are null; a turn whose calls have no cost has a null
cost; and the context's fill comes from the usage updates.

## What XenoNet writes

Connecting Grok Build (`xenonet connect grok`, or `Connect your agents to
XenoNet` in the palette) writes two things, both under `GROK_HOME`.
Connecting every app leaves Grok Build out until it's installed, which
XenoNet tells by the `grok` command or its folder.

- `[mcp_servers.xenonet]`, with its `env` table, in `config.toml`, and
  `xenonet-console` beside it for the installed XenoNet. `grok mcp add`
  rewrites the whole file and drops your comments, so XenoNet edits the
  text itself (`core/engine/toml.js`, `toml.rs`). It takes out its own
  tables and adds them at the end. Every other line stays as it was, it
  keeps the file's mode, and it saves a copy as
  `config.toml.before-xenonet` the first time. A server of that name
  written another way is left alone, and the panel says so.
- `hooks/xenonet.json`, a file of XenoNet's own. Grok runs the hooks in
  `~/.grok/hooks` without asking, unlike Codex. Every XenoNet on the
  machine shares the file, each with its own port. Disconnecting takes
  out its own entries, and the file goes with the last of them.

The hooks run `xenonet hook grok`, which posts the event to
`/api/hook/grok`. Grok's payload has camelCase keys (`sessionId`,
`toolName`, `toolInput`, `stopHookActive`), with the event in Claude
Code's spelling in `hook_event_name`. XenoNet answers in Claude Code's
format, which Grok reads, but Grok doesn't hear an answer at every event:

| Event | What Grok does with an answer | What XenoNet answers |
| --- | --- | --- |
| `SessionStart` | Ignores it | Nothing |
| `UserPromptSubmit` | Throws away `additionalContext` | Nothing, and the session's messages stay unread |
| `PreToolUse` (edits) | Gives `additionalContext` with the call's result | A warning when another session has a hand in the file |
| `PostToolUse`, `PostToolUseFailure` | Gives `additionalContext` with the result | The session's messages |
| `Stop` | Keeps the turn going with `decision: block` and the reason. It also fires once as the session closes (`reason` `channel_closed` or `shutdown`), and reads no answer to that one | The session's messages, when `reason` is `end_turn` and the stop hook didn't already run this turn. At the session's close, nothing, so its messages wait for its next turn |
| `StopCancelled`, `StopFailure`, `Notification`, `SessionEnd` | Nothing | Nothing. They tell the tailer when a turn ended or a permission prompt waits, and the engine when a session ended. |

So a Grok Build session doesn't hear who else works in its repo when it
starts. It finds them with the `peers` tool.

Grok also runs Claude Code's hooks (`[compat.claude] hooks`, on unless you
turn it off), with its own payload. XenoNet's Claude Code routes pass over
an event with Grok's camelCase keys (`hookEventName`, or `sessionId`
without `session_id`), unchecked, so a Grok session is heard once, on its
own route (`hook_written_by` in `xenonet-base`'s apps, `hookWrittenBy` in
`core/apps.js`).

### Which session is calling

Grok Build runs a session's MCP servers with `GROK_SESSION_ID` set, so the
xenonet MCP server knows its session (`CALLERS` in `mcp/xenonet.js`, and
`xenonet.rs`). A server Grok started for a session serves no actor.

Requests in a Grok Build session's name must come from a `grok` process,
or from under one, as Codex's must come from Codex
(`core/engine/verify.js`). XenoNet tells Grok's processes apart by their
program (`core/processes.js`): `grok`, the `grok-<os>-<arch>` binaries in
`~/.grok/downloads`, and what `~/.grok/bin` links, where the installer
also puts an `agent` command that only counts there, since Cursor's CLI is
called `agent` too.

## The doctor

`xenonet doctor` says whether Grok Build is installed (on PATH or in
`~/.grok/bin`), signed in (that `auth.json` is there, never what's in it,
or an API key in `XAI_API_KEY`), whether XenoNet can read its sessions
folder, whether its sessions are connected, and whether Grok runs XenoNet's
hooks. An organization's `allow_managed_hooks_only` (in
`/etc/grok/requirements.toml`, `/etc/grok/managed_config.toml` or
`~/.grok`'s copies), or XenoNet's hooks turned off in `/hooks`, stop them.
Grok Build is optional, so not having it is a note, and the checks that
need it skip.

## In the console

Grok Build's sessions have a color in every theme (`colors.grok`, `--grok`),
a toggle under Filters that shows once there are any, a place in the legend,
and a line in Token use while they use tokens. The demo has three made-up
Grok Build sessions, written as Grok keeps them, so they go through the real
readers: their replies come in a chunk a sentence after a thought, and
their ledgers record tokens and no cost, as on a SuperGrok sign-in.

No desktop app hosts Grok Build's sessions, so they have no "open in app",
and XenoNet shows the session instead. If one does, it fits in beside `grok.js`
the way Claude desktop fits beside Claude Code, with an `open-app` action in
`core/open.js` and its name in `APP_DESKTOP` in `src/theme.js`. Grok Bot,
SpaceXAI's always-on agent, stays an actor.

## Still to do

- Renaming a session, by its `summary.json`'s title.
- Questions waiting on you read from the updates, as they are from Claude
  Code's transcripts.
- Whether a subagent's hook events carry its own session's id or its
  parent's. XenoNet answers a hook that names a `subagentType` as it answers
  a Claude Code subagent's, with edit warnings and no messages.

## Checking it against a real install

Once `grok` is signed in and has run a session or two, in a repo, with
XenoNet connected:

1. Run `xenonet doctor`. The Grok Build checks pass, or say what to fix.
2. Run `xenonet connect grok`, then
   `diff ~/.grok/config.toml.before-xenonet ~/.grok/config.toml`. Only
   XenoNet's tables are new, and `grok mcp list --json` lists them.
3. In a Grok session, run `/hooks`. XenoNet's hooks are listed and enabled.
4. Run `xenonet drift`. Anything under Grok Build's sources names the field
   that moved.
5. Ask the session to call XenoNet's `whoami`. It names `grok:<its id>`.
6. From another session, `send` the Grok session a message, then have the
   Grok session run a tool. The message arrives with the result, once.
