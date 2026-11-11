# Building blocks

XenoNet's readers are made of a few building blocks, each with a lifecycle
and a health report: scanners walk folders, gatherers take snapshots from
sources, tailers read files as they grow, normalizers turn a vendor's
records into XenoNet's, and the versioned file store keeps what isn't in
the one SQLite store. They live in `crates/xenonet-blocks`, which depends
only on `xenonet-os` and `xenonet-text` ([rust-core.md](rust-core.md)).

Each block holds what it knows and decides, and leaves the doing to
whoever runs it: the runtime reads the files, keeps the timers and passes
the time in. So a block is plain state that a test drives with a clock it
moves by hand.

| Block | Module | What it is |
|---|---|---|
| `Lifecycle`, `Registry` | `lifecycle.rs` | What every block has: a state, its health, its counters; a list of blocks |
| `Scanner` | `scanner.rs` | A walk for the files wanted, within budgets, reading only folders that changed: depth first, or breadth first with the reading left to the caller |
| `Gatherer` | `gatherer.rs` | When a snapshot from a source is due, with backoff, and what it serves after a failure |
| `Tailer` | `tailer.rs` | Files read a line at a time as they grow: offsets, half lines, history and dormant files |
| `Normalizer` | `normalizer.rs` | A vendor's record to XenoNet's, with the hook for lenient checks and drift |
| `read_versioned`, `write_atomic` | `store.rs` | A versioned JSON file: migrations, a copy kept, atomic saves |
| `Clock`, `SystemClock`, `FixedClock` | `clock.rs` | The clock blocks keep time with |

Watching is not a block: each adapter says where its files live
(`Platform::watch`) and `xenonet-adapter`'s `watch` module turns the
system's file events into session events ([vendor-layer.md](vendor-layer.md)).
The scanner and the tailer's sweep are the backstop that catches up when
the system drops events.

## Lifecycle and health

- A block's state is `idle` (made, not used yet), `running` (the last
  thing it did went well), `degraded` (working, but the last thing had a
  problem: it serves what it had, or less than all), `failed` (the last
  thing failed and it has nothing to serve) or `stopped`.
- Its health is `{ name, kind, state, since, lastOk, lastError, counters,
  budget }`. `lastError` is `{ at, message }` and stays after the block
  recovers; `state` says whether it's current. Every block counts
  `errors`; `budget` is what a scanner may spend and spent last.
- A stopped block stays stopped whatever it reports, until it starts
  again. A block used without being started (a source read on demand)
  counts as running from its first success. Each change of state is kept
  until the runtime takes it (`take_changes`), to tell whoever listens.
- `GET /api/health` lists the blocks, with `source` for one that reads
  for one of the map's sources, and the doctor's `xenonet.blocks` check
  warns about each one that has been failed or degraded for a minute,
  failed first, with its last error (paths under home as `~`). One that
  went wrong just now is a note, since it's tried again by itself. A
  block whose source `xenonet.sources` already reports as failed is left
  to that check, so a source that fails is said once.
- Health is about the machinery, not the news it brings: a source that
  answers "nothing to report" (plan usage without a sign-in) is fine. An
  idle block isn't a problem: the Codex client stays idle until something
  asks Codex.

## Scanner

- `scan` walks depth first, in each folder's own order: a folder's picked
  files, then its subfolders. Limits count levels below the root, folders
  read and time spent; rules pick files and choose which subfolders to
  enter.
- With the cache, it remembers each folder's mtime and subfolders, reads
  a folder again only when its mtime changed (a file added, removed or
  renamed; not a file written to), and forgets folders that went and what
  was under them. A folder read within `RACY_MS` (2 s) of its last change
  is read again on the next walk whatever its mtime says, since coarse
  timestamps can hide a file added that soon.
- A walk stops at its folder or time budget and is degraded; the walks
  after it go on from where the cache says. A folder it can't read is
  passed over and degrades it too. A root that isn't there holds nothing,
  and that's fine.
- The breadth-first walk goes a level at a time, shallow levels first,
  and leaves the reading to the caller, which may read several folders at
  once and look at each before the walk decides which subfolders to
  enter: `begin_breadth`, then per level `plan`, each folder's
  `begin_read`, `read_failed` or `look_failed`, and `finish_read`, then
  `end_level`, and `finish_breadth`. A cancelled walk reads no more
  folders and says nothing of the scanner's health.

## Gatherer

- Due when its stamp (a file's mtime, say) differs from the last
  gather's; otherwise once `every_ms` has passed, or the time the source
  asked for (`next_in`, such as Retry-After), or, after failures, the
  backoff's delay, which grows with each one.
- A failure is a gather that fails. With `keep` it serves the last good
  value and is degraded; otherwise, or with nothing to keep, it serves its
  fallback and is failed. The next success makes it running again.
- It keeps the schedule and decides; the values, the stamps and the
  gather itself are the caller's.

## Tailer

- It keeps an offset in each file and hands on whole lines only, keeping
  a half-written last line as bytes, since a read can end inside a
  character. A file that got shorter is read again from the start.
- At start it reads every file written within `prime_window_ms` as
  history and leaves older ones dormant; a dormant file written to again
  has what it held read as history first. A file it can't read waits the
  same way, with the tailer degraded, so the rest are still read.
- Started again after a stop, a file it followed before is caught up by
  the sweep, so what was written meanwhile reads as live, not history.
- The caller watches, sweeps and reads, and asks at each step:
  `listed_at_start` and `at_start` for what the start's walk lists,
  `in_sweep` for each file it follows, `may_discover` and `discovered`
  for what a later walk lists, `add_file` and `take_held` for a file it
  reads for the first time, then `plan_take` and `took` for each read.

## Normalizer

- `line` parses a line; one that doesn't parse goes to the `unparsable`
  hook. Then `inspect` gets the record and where it came from, and
  returns the record to use (the same, or with what doesn't fit taken
  out) or none to drop it. Then `normalize`.
- It never fails for a record. A record the hooks or `normalize` fail on
  is dropped and counted as an error, with the last error saying what;
  the reader goes on, and one record doesn't change the block's state.

## The versioned file

`read_versioned` reads a JSON file with a version, for what XenoNet keeps
in files rather than in `xenonet.sqlite`:

- no file: a fresh document;
- not JSON, not an object, or a version it doesn't know: moved aside to
  `<name>.broken-<time>.json`, and a fresh document; never replaced by an
  empty one and written over;
- an older version: kept as it was in `<name>.v<n>.json`, then migrated
  and saved;
- a newer version: read as it is and never written back;
- a file it can't read, back up or move aside: the document in memory,
  and the file left alone.

`write_atomic` writes through a temporary file and a rename, so a crash
can't leave half a file. What XenoNet keeps of its own lives in the store
([rust-core.md](rust-core.md), `xenonet-store`).

## The golden tests

`crates/xenonet-state/tests/golden.rs` pins what XenoNet makes of a fixed
world, the demo's world and the odd transcripts it adds, frozen in
`crates/xenonet-state/tests/fixtures/golden-world`: the live tailer's state
and history, and lines as they come; each session's stats; the search
index; the map's refresh; and the projects scan. The clock is fixed at
2026-10-03 12:00 UTC and every file's mtime is set, so the world comes out
the same every run. Made-up transcripts add what the demo never writes
(bad, missing and numeric timestamps, lines that aren't JSON or aren't
objects, meta and compacted lines, slash commands, a message in two
lines, Codex's other line types, an archived `.jsonl.zst` rollout).

`XENONET_GOLDEN_OUT=<dir>` writes what came out to
`<dir>/golden-<name>.json`, to diff with `tests/golden/<name>.json`.
Change a golden file only for a change meant to change what XenoNet
produces, and say so in the commit.

## Where the three transcript readers agree, and where they don't

The live tailer, a session's stats and search read the same lines through
the same normalizers, as one stream of events
([vendor-layer.md](vendor-layer.md), "One event stream"), as the refusals
do: the mapper makes what they agree on, and each keeps its own way over
the events. They agree that:

- A line whose time can't be read is left out of stats and search; the
  live tailer takes it as written when it's read.
- A compacted summary and an interruption aren't prompts in any of them.
- A Codex rollout's prompts are its completed user messages
  (`item_completed` UserMessage), not the `user_message` events and user
  response items that may say the same again. A rollout with no completed
  user message at all (`codex exec` writes none) takes its prompts from
  its `user_message` events.
- Codex user messages that are blank (an image alone) or start with `<`
  (a delegation envelope, a note Codex wrote for itself) aren't prompts
  (`is_codex_prompt` in `xenonet-sources`' `lines`).
- A Codex shell call's output ends the call.
- A Claude Code interruption, or a Codex turn you aborted (`turn_aborted`
  with the reason `interrupted`, or none), ends the turn under way in the
  live tailer: a `turn-end` marked `interrupted`, and the session's
  `interruptedAt` until its next prompt, with no `needsYou`, so it isn't
  ready for review and doesn't chime. `replaced` and `review_ended`
  aren't you stopping it.
- A line that's JSON but not an object is dropped by the lenient readers
  for all three, and kept as drift.

Where they read a line differently, each keeps its own way:

### Claude Code transcripts

| What | live | stats | search |
|---|---|---|---|
| A line whose time can't be read: none, one that can't be parsed as a date, or a number | Written when it's read | Left out | Left out |
| Line kinds | `user`, `assistant` | `user`, `assistant`; a compacted summary is a compaction | `user` (not meta), `assistant` |
| A user line's text | Text blocks joined with spaces | The same | Every block's text joined with newlines (an image adds an empty line) |
| What's a prompt | Not meta, a compacted summary or an interruption (`[Request interrupted…`); not starting with `<`; a sidechain line only in a subagent's file, where the first names the subagent | Not meta, sidechain, a compacted summary or an interruption; a slash command as `/name args`; not `<…`; a subagent's first line is its task | Everything but compacted summaries, interruptions, `<…` and `# AGENTS.md…`; sidechain lines as role `subagent` |
| A compacted summary, an interruption | A compacted summary is left out. An interruption ends the turn under way, unless it's in a subagent's file | A compaction; left out | Left out |
| Assistant text | Each text block a message (main file only) | None | Text blocks joined with newlines |
| A user line with tool results | Each result | Each result | Skipped whole, read only as far as its first result |
| A message's usage | Once per id; the last 400 ids per session | Once per id, all of them | None |
| Subagent files | `<id>/subagents/agent-<agent>.jsonl` | Any `.jsonl` in `<id>/subagents/`, `agent-` dropped from the id | Any `.jsonl` under `<id>/subagents/` |

### Codex rollouts

| What | live | stats | search |
|---|---|---|---|
| A line whose time can't be read | Written when it's read | Left out | Left out |
| Prompts | Completed user messages, joined with spaces. In a rollout with none, its `user_message` events, until its first completed one: an event just before that message in the same turn is one prompt, not two. Not blank, not starting with `<` | Completed user messages, or in a rollout with none at all, its `user_message` events, read from the whole file. Not blank, not starting with `<` | The same as stats, joined with newlines, and not `# AGENTS.md…` either; the same text in a file is indexed once. A rollout is indexed by its events while it has no completed user message, and again, whole, once it has one |
| A turn you stopped | `turn_aborted` with the reason `interrupted`, or none, ends the turn under way | None | None |
| Agent messages | `item_completed` AgentMessage (spaces), `task_complete`'s last message | None | AgentMessage (newlines), `agent_message`, `response_item` message (role assistant) |
| Tool calls | `function_call`, `custom_tool_call` and `local_shell_call` start one, and their outputs end it (by `call_id`, else `id`) | The same; a `local_shell_call`, which has no name, is a shell call | None |
| Commands, MCP calls, file changes, web searches, images, subagents | From `item_completed`; a web search with an empty query shows its action's; a subagent is done once its activity says it completed or was interrupted, until it's given more | None | None |
| Tokens | The growth of `total_token_usage`, with its `output_tokens` as the output, since OpenAI counts reasoning inside it. The context from `last_token_usage` | A call each time `total_token_usage` grows, priced from `last_token_usage`'s parts | None |
| Model | `turn_context`'s model | `turn_context`'s model and effort; the service tier from the thread's settings (`thread_settings_applied`), or from `turn_context` in older rollouts; `session_meta`'s provider (a provider other than OpenAI names the model) | None |
| Rate limits | `token_count`'s `rate_limits`, in XenoNet's shape (`limits.codex`: each window's `pct`, `windowMinutes` and `resetsAt` in ms) | None | None |
| Archived and compressed rollouts | Not read | Read | Read |

One difference comes of the live tailer reading lines as they come: in a
rollout whose first completed user message comes turns later (an old one
picked up again), it keeps those earlier turns' prompts from their
events, which stats and search, reading the whole file, leave out.

### Grok Build updates

Grok Build streams what you and the agent say as chunks, one ACP update a
line (`user_message_chunk`, `agent_message_chunk`,
`agent_thought_chunk`), and a run of chunks of one kind is one message.
The mapper reads a line at a time, so it keeps the run open in its file
state (its kind, a key, and its text so far), and the readers agree on
this:

- A run's first chunk makes the message's events, with that chunk's text,
  as a Claude Code line would: a prompt, or a reply's call and words (its
  text, or a thought as a thinking block).
- Each chunk after it makes `More`: what it carries on (prompt, text or
  thinking), its own text, and the message's text so far (the chunks
  joined as written). Each reader puts the whole text in place of what it
  kept of the message, or keeps it then, when the run's first chunks had
  nothing in them.
- A run goes on past the updates that say nothing of the conversation (the
  context's fill, the plan, the mode, the commands). Any other line ends
  it: a chunk of another kind, a tool call or its update, or one of Grok's
  own updates (its turn's end is one). So does a chunk of another key: a
  prompt of another `promptIndex`, a chunk you didn't ask (a host turn, a
  shell command), and each interjection's text, as Grok's own replay
  splits them.

| What | live | stats | search |
|---|---|---|---|
| A prompt | Its whole text (`prompts`, `firstPrompt`), and a turn begins at its first chunk with something in it | Its whole text, trimmed, names its turn | One record, its whole text |
| A reply | Its whole text (`messages`, `lastMessage`) | None | One record, its whole text, rewritten as its chunks come |
| A thought | None | None | None, as Claude Code's thinking isn't indexed |
| Its chunks after the first | Each counts as a line of the turn (`lastEventAt`) | None | The record's text grows; the same text in a file is indexed once, as far as it's gone |
| The activity feed | A message is named once, by its text when it's first kept: its first chunk's, clipped | None | None |
| A run a read leaves open | The tailer keeps the run as the lines come | Read whole | Kept in the search index's files table, and the next read goes on with it |

The console's transcript of a Grok session
(`adapters/grok-build/src/reader/entries.rs`) reads the same events: a run
is one entry at its first chunk's time, with its whole text, and a thought
shows as reasoning, folded, as Claude Code's thinking does.

The console shows a Codex thread from the app-server's
`thread/items/list`, and when it can't have that (the app-server may not
list items, or not start at all), from the thread's rollout, read through
the same normalizer: its completed items, each as the app-server's would
show, or for a rollout without any what was said and each call and its
output. The rollout is read whole the first time and then only what's
been added: an offset and a half-written last line kept as bytes, as the
tailer reads; a rollout written again is read from the start, a
compressed archive is read whole again only when it changes, and one read
happens at a time.

## Lenient reading and drift

The normalizers' hooks are set for every transcript line the three
readers read: each line is counted, its file's version followed, a sample
checked against the vendor schema (the 1st, 2nd, 4th, 8th… line of each
kind in each file), and the rest guarded for type changes found before. A
field of another type is taken out of the record before the reader reads
it, so the readers fall back the way they do for a missing field; a line
that isn't an object is dropped. [schemas.md](schemas.md), "Vendor data",
has the rest: the sources, the sampling, drift records, `drift.json`, the
report and the doctor's check.
