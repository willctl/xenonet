# The vendor layer

How XenoNet maps each app's data to its own: one registry of the apps it
knows, one adapter crate per platform behind one trait, watchers that push
what changed, and one stream of events read from transcripts. The core
asks the registry and never matches on an app's id; `npm run lint:rust`
fails a core crate that names a platform ([rust-core.md](rust-core.md)).

## The app registry

`src/shared/apps.js` is plain data with no imports, so the page and
`src/shared/ids.js` can use it. Rust gets a copy:
`node scripts/apps.mjs` writes `crates/xenonet-adapter/src/apps.json`,
which `xenonet-adapter` embeds and parses into typed entries, and a test
fails when the copy is stale. Each entry is an `app/v1`
([contracts.md](contracts.md), section 1).

An entry holds only what is data. The comment at the top of `apps.js`
says what each field means; in short:

| Field | What |
|---|---|
| `id`, `name` | `codex`, `Codex` |
| `numbat` | The names numbat records its runs under |
| `products` | Its vendor products (`openai/codex`), whose sources the drift log keeps |
| `roots` | The variable that moves its folder, and the folder otherwise (`CODEX_HOME`, `~/.codex`) |
| `context` | Its context window, until a line says otherwise |
| `identity`, `limits` | The kind of account it runs under, and its plan limits in `live/snapshot/v1` |
| `team` | The developer team that signs it (codesign's TeamIdentifier) |
| `connect` | Its config format, the hook events with their matchers, and the events whose answers it passes over |
| `editTools` | The tools whose calls are edits, where they aren't Claude Code's |
| `has` | What XenoNet can do with it at most (below) |
| `acp` | The command that runs it as an ACP agent over standard input and output, for an app XenoNet starts sessions of over ACP |
| `headless`, `routing`, `mcpStatus`, `reserved`, `reportsSpend`, `sorts`, `handoff` | Its headless turn's arguments, its place as a routing candidate, whether its own server lists its MCP servers, the names no actor may pass for, whether it reports spend, whether XenoNet runs turns of its own with it, and how a handoff carries one of its sessions |

Claude Code, Codex and Grok Build are tailed and connected. OpenCode,
Cursor, Gemini CLI and Copilot CLI have readers of their own records,
on when `XENONET_READERS` names them. About twenty more apps have an
entry with their name and numbat's only: XenoNet sees their runs through
numbat's records or an actor's report.

**What a session can do is the session's.** The registry says what an
app can do at most. Which reader a session came from, and so what XenoNet
can do with it, is the session's own `observed` and `reader` fields: an
actor can report a Claude Code run that has no transcript.

**An app the registry doesn't have is refused** where a command, a route,
another machine or a reader of transcripts names one: a 400 on routes, a
usage error from the command, and a machine's tokens left out with a note
in the log (`apps::tailed_or`). `xenonet hook` refuses quietly, with a
line in the log, and still exits 0. A request in the name of a session
whose app has no `processes` capability is refused, naming the app.

## Adapters

A platform is one crate under `adapters/` implementing `Platform`
(`crates/xenonet-adapter/src/platform.rs`), and `adapters/all`
(`xenonet-platforms`) installs every adapter, in the registry's order,
before anything asks the registry.

| Crate | What it brings |
|---|---|
| `adapters/claude-code`, `codex`, `grok-build` | Processes, ways in for live delivery, connect, readers, watchers, openers and doctor checks |
| `adapters/opencode` | Its processes, its sessions through its own server, and the reader of its database, which the watcher watches |
| `adapters/cursor`, `gemini-cli`, `copilot` | The reader of each app's own records |
| `adapters/hermes`, `grok-bot`, `muse` | The observer of each actor product; Hermes also XenoNet's plugin for Hermes |
| `adapters/acp` | The generic ACP client: starts and drives a session of any app the registry gives `acp` (Grok Build) or actor product the products table does (Hermes Agent), in its own process tree, with none of XenoNet's credentials in its environment |
| `adapters/a2a` | Outbound A2A actors: agents XenoNet calls by their pinned Agent Card |
| `adapters/git` | What XenoNet knows of a git checkout, read from git's own files, and what to watch for its head and refs to move |

The trait's methods each match a capability in `has`, so code that needs
one loops over the platforms that have it:

| `has` | `Platform` | What it does |
|---|---|---|
| `lines` | `tailed`, `transcript_dirs`, `session_files` | The mapper from its lines to events (below), where its transcripts are, a session's files for stats |
| `sessions` | `lister`, `renames` | List sessions, rename one, read its transcript (Claude Code through the Agent SDK and Claude desktop's files, Codex through `codex app-server`) |
| `native` | `native` | An experimental reader of the app's own records |
| `connect` | `connect` | Preview, apply and remove XenoNet's entries in its config format |
| `delivery` | `delivery` | Resume a turn, or reach a running session |
| `processes` | `processes` | Tell its processes apart |
| `caller` | `decode_hook` | Find the calling session in a hook or MCP request |
| `open` | `opener` | The URLs that open a session or project in its app |
| (identity) | `signin` | The account it's signed in to |
| (actor product) | `observer` | What its product does, for an actor XenoNet observes |
| (ACP) | `acp`, `program` | How XenoNet starts it as an ACP agent here |

Every adapter also brings its doctor rows (`checks`, with the facts they
read in `doctor_facts`).

## Watching

`Platform::watch` says where an app's sessions and transcripts are
written (a `WatchSpec` each), and `Platform::changed` says what a set of
changed paths means, as `SessionEvent`s, reading only what was added
since its `Cursor`. `xenonet-adapter`'s `watch` module runs the system's
file events (FSEvents, inotify, ReadDirectoryChangesW) with a 100 ms
debounce; on macOS a transcript being written is followed on its own with
kqueue too, since FSEvents can lag by seconds. When the system drops
events or a watch fails, the watcher asks for a reconcile, and the scan
([blocks.md](blocks.md)) catches up. An adapter whose sources can't be
watched gives no specs, and its scan stays the way they're read.

## One event stream

Each tailed app has one mapper from its normalized lines to shared events
(`crates/xenonet-sources/src/line_events.rs`), and the readers read those:
the live tailer, a session's stats and search, and the refusals.

- Every event has the line's time (`t`, NaN when it can't be read), its
  subagent, and its kind as the sampling hooks name it (`origin`).
- The kinds: `Prompt` (every content block in order, a slash command,
  and whether it's meta, a sidechain, a compacted summary or an
  interruption), `Said`, `Call` (the message id, model, effort, usage,
  window, speed, provider, and Codex's running total), `Tool`, `Result`,
  `Item` (a Codex completed item), `TurnStart`, `TurnEnd` (why: end of
  turn, task complete, interrupted, refusal), `Settings`, `Compact`,
  `Context`, `Limits`, `Refusal`, and `Broken`.
- Where the readers agree, the mapper decides once: a compacted summary
  and an interruption aren't prompts; a Codex rollout's prompts are its
  completed user messages, or its `user_message` events in a rollout with
  none; Codex messages that are blank or start with `<` aren't prompts.
- Where each reader keeps its own way, the event carries what that rule
  needs and the rule stays in the reader: live takes a line whose time
  can't be read as written when read, while stats and search leave it
  out; live keeps the last 400 message ids per session, stats keeps them
  all; live goes by the growth of Codex's token total, stats prices the
  last call.
- The mapper never fails. A field it can't read ends the line's events
  with `Broken`, naming the field, so a reader that would have read that
  field fails as it would reading the line itself, and one that wouldn't
  goes on.

Tokens are one type, `TokenUsage` (`xenonet_model::usage`): input, cache
reads, cache writes for 5 minutes and 1 hour, output, and the call's
speed. One parser (`xenonet_adapter::formats::patch`) reads `apply_patch` text into the paths it adds,
updates, deletes and moves to, for live, the engine and the native
readers.

## Machines on different versions

A machine's map merges what other machines of its network send and keeps
it only as its own schemas declare it
(`crates/xenonet-runtime/src/remote/`). An app a newer machine knows and
an older one doesn't is dropped there. Registry changes are additive, and
nothing a machine knows changes shape.

## Adding an app

- A registry entry in `src/shared/apps.js`, then `node scripts/apps.mjs`.
- Its vendor schemas under `schemas/vendor/` ([schemas.md](schemas.md)).
- An adapter crate under `adapters/` implementing `Platform` for what its
  `has` lists, added to `adapters/all`.
- For a tailed app: its line format and mapper, its watch specs, demo
  transcripts, and its colour as a theme token (a `theme/v1` change).
- Fixtures and tests. [grok-build.md](grok-build.md) is the latest app
  added this way.
