# Building blocks

XenoNet's subsystems are made of a few building blocks, each with a clear
interface, a lifecycle and a health report: scanners walk folders,
gatherers take snapshots from sources, tailers read files as they grow,
normalizers turn a vendor's records into XenoNet's, and stores keep what
XenoNet remembers. They live in `core/blocks/`, with test harnesses in
`test/harness/`. `next-phase.md` has the plan they belong to.

| Block | File | What it is |
|---|---|---|
| `Block`, `Blocks` | `lifecycle.js` | What every block has: start, stop and health; a list of blocks |
| `Scanner` | `scanner.js` | A walk for the files wanted, within budgets, reading only folders that changed: depth first, or breadth first and asynchronous |
| `Gatherer` | `gatherer.js` | A snapshot from a source, on demand or on a cadence, with backoff and timing |
| `Tailer` | `tailer.js` | Files read a line at a time as they grow: offsets, half lines, watch and sweep |
| `Normalizer` | `normalizer.js` | A vendor's record to XenoNet's, with the hook for lenient checks and drift |
| `JsonStore`, `readVersioned` | `store.js` | A versioned JSON file: migrations, a copy kept, atomic saves |
| `SqliteStore`, `migrateSqlite` | `sqlite.js` | A versioned SQLite database: numbered steps in transactions |
| `systemClock` | `clock.js` | The clock blocks keep time with |

Import a block from its own file. `core/blocks/index.js` loads them all,
SQLite too, and the doctor and the `xenonet` command must never load SQLite
(it prints an experimental warning), which is why `store.js` and
`sqlite.js` are apart.

## Lifecycle and health

Every block extends `Block` (`lifecycle.js`):

- `name` and `kind` (`scanner`, `gatherer`, `tailer`, `normalizer`,
  `store`, `client`), set when it's made.
- `start()` (async) runs the kind's `onStart(run)`, where `run` names
  this start; starting what's started does nothing. A stopped block is
  idle again while it starts. A start that throws leaves the block
  failed, and one that reports a problem leaves it degraded, the first
  start or a restart alike. `stop()` (synchronous, so it can run as the
  process exits) runs `onStop()`.
- A start overtaken by `stop()`, or by a stop and a new start, sets no
  state: `current(run)` says whether the start that's `run` still stands,
  and a kind whose `onStart` awaits asks it after each await and gives up
  when it doesn't. So a stopped block stays stopped, whatever its last
  start was doing.
- `health()` returns `{ state, since, lastOk, lastError, counters,
  budget }`:

| Field | What |
|---|---|
| `state` | `idle` (made, not used yet), `running` (the last thing it did went well), `degraded` (working, but the last thing had a problem: it serves what it had, or less than all), `failed` (the last thing failed and it has nothing to serve), `stopped` |
| `since` | When it came to be in that state (ms) |
| `lastOk` | When it last did its work well, or null |
| `lastError` | `{ at, message }` of the last problem, or null. It stays after the block recovers; `state` says whether it's current |
| `counters` | Numbers its kind keeps; every block counts `errors` |
| `budget` | What it may spend and what it spent last (the scanner), else null |

- The kinds report with `ok()`, `problem(err)` (degraded), `fail(err)`
  and `count(name, n)`. A stopped block stays stopped. A block that's
  used without being started (a source read on demand) counts as running
  from its first success.
- `Blocks` is a list of them: `add(block, { source })` (returns it),
  `get(name)`, `remove(name)`, `list()`, and `health()`, each block's
  health with its name and kind, and `source` for a block that reads for
  one of the map's sources (`XenoNetState.sources`: `claude-sessions` for
  `claude`, `codex-threads` and `codex-app-server` for `codex`,
  `codex-projects`, `transcripts` for `live`, `identities` and
  `projects`). `XenoNetState.blocks` lists XenoNet's, and the server adds the
  engine's store.
- `GET /api/health` has them as `blocks: [{ name, kind, source?, state,
  since, lastOk, lastError, counters }]`, and the doctor's `xenonet.blocks`
  check warns about each one that has been failed or degraded for a
  minute, failed first, with its last error (paths under home as `~`);
  one that went wrong just now is a note, since it's tried again by
  itself. A block whose source `xenonet.sources` already reports as failed
  is left to that check, so a source that fails is said once.
- Health is about the machinery, not the news it brings: a source that
  answers "nothing to report" (plan usage without a sign-in) is fine, and
  the doctor's own checks judge what sources say. An idle block isn't a
  problem: the Codex client stays idle until something asks Codex.
- Every block takes a `clock` (`now`, `setTimeout`, `clearTimeout`,
  `setInterval`, `clearInterval`); tests pass one they move by hand.

## Scanner

```js
const s = new Scanner({ name, root, depth: 6, folders: 2000, ms: 500, pick: (name, dir) => name.endsWith(".jsonl"), enter: (name, dir, level) => name !== "node_modules", cache: true });
s.scan(); // the picked files in the folders this walk read
```

- Depth first, in each folder's own order: a folder's picked files, then
  its subfolders. `depth` counts levels below `root`; `pick` chooses
  files, `enter` subfolders. Synchronous: a stat or a readdir takes
  microseconds, and a trip to the thread pool for each costs many times
  that.
- With `cache` (the default) it remembers each folder's mtime and
  subfolders, reads a folder again only when its mtime changed (a file
  added, removed or renamed; not a file written to), and forgets folders
  that went and what was under them. `forget()` drops it all. `folders`
  is the cache.
- A walk stops at `folders` read or `ms` spent, and is degraded
  ("stopped at the 2000-folder budget"); the walks after it go on from
  where the cache says. A folder it can't read is passed over, and
  degrades it too. A root that isn't there holds nothing, and that's
  fine.
- `last` and `budget.last`: `{ files, read, seen, truncated, ms }` of
  the last walk. Counters: `scans`, `read`.

```js
await s.scanBreadth({ root, depth, folders, ms, pick, enter, look: async (dir, entries, level) => {}, concurrency: 16 });
```

- Breadth first and asynchronous, for a walk the server makes while it
  serves (the projects scan reads up to 2,000 folders on every refresh):
  the root, then the folders below it a level at a time, each level in
  order (a folder's subfolders in its own order, after those of the
  folders before it), `concurrency` folders read at once.
- `look(dir, entries, level)` is handed each folder read, with its
  entries (`fs.Dirent`), and may be async; `enter` is asked about the
  folder's subfolders once `look` is done with it, so it can go by what
  `look` found. A folder that can't be read isn't handed to `look`, and
  one `look` throws on is passed over; either degrades the scanner.
- The root is always read; `folders` budgets the folders below it, spent
  in order (the first folders on a level), and `read` and `seen` count
  those. A walk that runs out stops after the level it ran out on, and is
  degraded as above.
- The options given are that walk's own, in place of the scanner's: a
  scanner whose root and rules change from walk to walk (the projects
  scan follows the folder in use and your rules) passes them each time,
  and `budget` shows what the last walk was allowed. With `cache`, a
  folder that didn't change is only stat'ed; without, nothing is kept.
- `signal` (an `AbortSignal`) stops a walk no longer wanted: it reads no
  folder after the signal aborts, its `last` says `truncated:
  "cancelled"`, and it says nothing of the scanner's health. A check of
  another projects folder stops its scan this way when it stops waiting
  for it.

## Gatherer

```js
const g = new Gatherer({ name, gather: async ({ now, last }) => value, everyMs, nextIn, backoff, stamp, retry, keep, fallback });
await g.run({ force, now }); // what it serves
g.get();                     // the same, for a synchronous source
```

- When it's due: with `stamp` (a function, such as a file's mtime), when
  the stamp differs from the last gather's; otherwise always, unless
  `everyMs` (due that long after the last gather), `nextIn(value)` (the
  source says when, such as Retry-After) or `backoff` (`{ baseMs, maxMs,
  factor }`: after failures, a wait that grows) say otherwise. `due(now)`
  says; a run that isn't due serves what it has (counted as `skipped`),
  and `force` gathers anyway. `now` lets a caller run it on its own
  clock.
- Runs at once share one gather. Each is timed: `last` is `{ value, at,
  ms, ok, error, stamp }`.
- A failure is a gather that throws. With `keep`, it serves the last good
  value and is degraded; otherwise (or with nothing to keep) it serves
  `fallback` and is failed. With a stamp, a failed gather waits for the
  stamp to change, like a good one, unless `retry`.
- `forget()` drops what it gathered. `start({ immediately })` runs it on
  its own cadence (needs `everyMs`), `stop()` ends that. Each start has a
  cadence of its own, which its stop ends: a tick of a start that no
  longer stands neither gathers nor sets another timer, so a gatherer
  stopped and started again while it gathers keeps one cadence. It emits
  `gathered` with `last`. Counters: `runs`, `skipped`.

## Tailer

```js
const t = new Tailer({ name, roots: [{ dir, ...anything }], classify: (file) => entry | null, pick: (name) => name.endsWith(".jsonl"), onLine: (line, entry, prime) => …, onNewFile, onPrimed, primeWindowMs, sweepMs, discoverEvery, debounceMs, depth, watch, status });
await t.start();
```

- `roots` must exist, as real paths (on Windows, libuv aborts on a
  recursive watch of an 8.3 short name). Each root gets a scanner.
- At start it reads every file written within `primeWindowMs` as history
  (`prime` true) and leaves older ones dormant, with their size then; a
  dormant file written to again has what it held read as history first.
  A file it can't read at all (another user's, say) waits as a dormant
  one does, with the tailer degraded, so the rest are still read and
  followed. `onPrimed()` runs then, before the watches start.
- Stopped while it reads the history, it reads no more, and neither
  watches nor sweeps. `stop()` closes the watches, ends the sweep and
  lets go of the reads waiting on a watch event, so a start after it
  begins with none, and a watch it closed can't count as one that works.
- New lines come three ways: a recursive watch per root (debounced per
  file), a sweep every `sweepMs` over the files it reads, and a walk for
  new files every `discoverEvery` sweeps. `sweep({ discover })` runs one
  now (one at a time); `track(file, { prime })` reads one file (one read
  of a file at a time, so two can't take the same lines twice);
  `schedule(file)` is a watch event; `idle()` settles once no read or
  sweep is under way.
- It hands on whole, non-blank lines, decoded from whole lines only (a
  read can end inside a character, so the half line waits as bytes). A
  file that got shorter is read again from the start.
- `files` (file → `{ ...entry, offset, rest }`), `dormant`, `folders`
  (all the scanners' caches), `reads`, `pending`, `watchers`.
- `status` is `{ watching, files, lines, errors }`, shared with its
  owner, which counts the lines it parsed and the errors in them (live.js
  reports it in its snapshot). The tailer is degraded after a watch
  error until a read works, and stays degraded while a root has no
  watch. Counters: `sweeps`, `walks`, `files`, `dormant`, `lines`,
  `watchers`.

## Normalizer

```js
const n = new Normalizer({ name, source: "anthropic/claude-code/transcript-line", parse: JSON.parse, normalize: (raw, where) => record });
n.line(text, where);  // parsed and normalized, or null
n.record(raw, where); // already parsed
n.use({ inspect: (raw, where) => raw | null, unparsable: (text, where, err) => {} });
```

- `line()` parses; a line that doesn't parse goes to `unparsable` and is
  null. Then `inspect` (when set) gets the record and `where` it came
  from, and returns the record to use (the same, or with what doesn't
  fit taken out) or null to drop it. Then `normalize`.
- It never throws for a record: what's wrong with one is the reader's to
  find, where it found it before. A record the hooks or `normalize` throw
  on is dropped (null) and counted as an error, with `lastError` saying
  what; the reader goes on to the next, and one record doesn't change the
  block's state.
- Counters: `records`, `unparsable`, `dropped`. Records go through by the
  hundred thousand, so `lastOk` is when records were last seen going
  through, as of the last health report.

## Stores

`JsonStore` (`store.js`):

```js
const s = new JsonStore({ name: "state", file: () => path, version: 2, migrations, versionOf, context: () => ({}), fresh, prepare: (doc) => {}, debounceMs: 200, what: "its state" });
s.load(); s.loadReport(); s.save(); s.flush();
```

- `readVersioned(file, { version, migrations, versionOf, context, fresh,
  prepare, now })` reads: no file is a fresh document; not JSON, not an
  object or a version it doesn't know is moved aside to
  `<name>.broken-<time>.json`; an older version is migrated (see
  `core/migrate.js`) and kept as it was in `<name>.v<n>.json`; a newer one
  is read and never written back. Its report: `{ file, version, migrated,
  backup, broken, readOnly, problem }`, and `repaired` when there are any,
  its messages naming the file.
- `prepare(doc)` runs before a document is used, as it's read: it fills in
  what the document should have, and may return the parts it had to start
  afresh, such as one a hand edit left a string (`state.json`'s
  `"identities": "x"`), which the report lists as `repaired`. A document
  `prepare` throws on isn't used: a fresh one is, read only, with the file
  left alone and the problem reported. A migration's copy is written only
  once the migrated document is prepared, so one that can't be isn't
  copied again at every start.
- `load()` reads once, logs the parts `prepare` repaired, and saves a
  migrated document. `save()` writes a moment later, several changes in one;
  `flush()` writes now, through a temporary file renamed into place. A
  save that fails is logged (`[xenonet] couldn't save <what> to <file>`),
  degrades the store and is tried again with the next change or at exit.
  `file` can be a function: the folder can move while XenoNet runs (in
  tests). Counters: `loads`, `saves`.

`SqliteStore` (`sqlite.js`):

```js
const db = new SqliteStore({ name: "engine", file, migrations: [(db) => …], pragmas: ["journal_mode = WAL"], backup: true, keepBackupMs }).open();
```

- `migrateSqlite(db, migrations)`: `PRAGMA user_version` counts the steps
  taken, each runs in a transaction of its own with the version it
  reaches, and a database a newer XenoNet moved further is left at its
  version (the store is degraded). `version` is `{ from, version, newer
  }`.
- With `backup`, a database with data about to move up is copied first
  with `VACUUM INTO <name>.v<n>.sqlite` (`<name>.v<n>-<time>.sqlite` when
  that name is taken), the way state.json keeps `state.v1.json`, and the
  copy is deleted `keepBackupMs` (a week) after it was written. `open()`
  deletes the copies that old before it migrates, and `pruneBackups(now)`
  does it for an owner that prunes its database as it runs; it returns
  the files deleted.
- The engine's store has `backup` on (merge point A's call) and prunes its
  copies with its hourly prune, so `engine.v<n>.sqlite` appears beside
  engine.sqlite before a migration and goes a week later: it holds what
  the database keeps only so long (agents' messages, actors' events, your
  own record), so it mustn't keep them for good.
- `close()` closes as DatabaseSync does (closing a closed one throws).

## Test harnesses

| Harness | What |
|---|---|
| `test/harness/clock.js` | `fakeClock(start)`: `now()` is what the test sets; timers fire in order as `advance(ms)` moves it, with what they start let run, and what's under way runs first. `flush()`, `set(t)`, `pending()` |
| `test/harness/files.js` | `tempTree()`: real files in a temp folder, written as text, Buffers or JSON lines, with the mtimes a test sets (files and folders), and `cleanup()` (`cleanupAll()` for an `after` hook). `fakeWatch()`: an `fs.watch` that sends the events a test says (`emit`, `error`, `refuse`) |
| `test/harness/procs.js` | `fakeProcesses({ clock })`: a process table (pid, parent, start, program, args), `spawn` and `execFile` running scripts in place of programs (output, input a line at a time, exit, kill, timeouts), and the `ps`, `netstat` and `codesign` answers `core/procs.js` reads |

## What moved onto the blocks

Each move kept its module's API, one commit each, with the golden tests
(below) and the module's own tests unchanged.

| Subsystem | Blocks | Name in `/api/health` |
|---|---|---|
| state.json (`core/store.js`, `statefile.js`) | JsonStore | `state` |
| engine.sqlite (`core/engine/store.js`) | SqliteStore | `engine` |
| The transcript tailer (`core/live.js`) | Tailer, with a Scanner per root | `transcripts` |
| Transcript lines (`core/lines.js`, for live, stats and search) | Normalizer per app | `claude-lines`, `codex-lines` |
| Claude Code's sessions (`core/claude.js`) | Gatherer | `claude-sessions` |
| Codex's threads and desktop projects (`core/codex.js`) | Gatherers | `codex-threads`, `codex-projects` |
| The Codex app-server client (`core/codex.js`) | Block | `codex-app-server` |
| Identities' sources (`core/identities.js`) | A block with a Gatherer per account file and per repo | `identities` |
| Claude plan usage (`core/claude-usage.js`, read by live.js) | Gatherers | `claude-usage`, `claude-desktop-usage` |
| Git status (`core/git.js`) | A block with a Gatherer per repo | `git-status` |
| The projects scan's top-down walk (`core/projects.js`, Wave 2) | Scanner, breadth first, without the cache | `projects` |

The projects scan walks the projects folder with the scanner's
breadth-first walk (`projectsScanner()`, 16 folders at once): `look`
reads each folder's markers and workspace files, and `enter` keeps out
dot folders, dependencies and build output, and the insides of repos and
of folders made one project. It reads every folder on every scan, as
before, since what a folder is depends on its files' contents (a
package.json's workspaces) and on your rules, which a folder's mtime
doesn't follow. XenoNetState keeps one scanner for the map's scans, so a
scan stopped by its folder budget, or a folder it can't read, shows in
`/api/health` and the doctor; checks of another folder, and the Codex
outline, use one of their own. A folder you ignore is still read before
it's left out, as before, so the folders read and the budget come out
the same. The bottom-up pass (from where sessions ran) and submodules
read the folders they need on their own, as before.

What stayed, and why:

- **Claude desktop's session walk** (`readDesktopSessions` in
  `core/claude.js`) stays its own walk: it reads entries in each folder's
  own order and stops at the first folder it can't read, while the
  scanner reads a folder's files before its subfolders and passes over a
  folder it can't read, which could change which file wins when two name
  the same session.
- **The transcripts' owners** (`transcriptOwner` in identities.js), and
  **the file listings of stats and search** (`Transcripts` in stats.js,
  `listFiles` in paths.js), are as they were; they're candidates for the
  scanner.

Fixed on the way: the app-server handshake sends XenoNet's version, not
"0.1.0"; a gatherer without a cadence is always due, even if the system
clock steps back; and stopping the app-server lets go of it at once, so
its exit can't fail the next one's requests.

## The golden tests

`test/golden.test.js` pins what XenoNet makes of the demo world: the live
tailer's snapshot, history, notes and collision diffs after reading the
history, and the events and snapshot after lines come in; each session's
stats; the search index and a few searches; the map with its identities,
repos, the projects scan and state.json as saved and backed up; and the
projects scan in full, of the demo's two projects folders and of a
made-up one with what they don't have, with sessions, rules and other
limits. The world comes out the same every run: the clock is fixed with
node's mock timers (2026-10-03 12:00 UTC), the demo's `Math.random` and
`randomUUID` are seeded, the time zone is UTC and every file's mtime is
set. Made-up transcripts add what the demo never writes (bad, missing
and numeric timestamps, lines that aren't JSON or aren't objects, meta
and compacted lines, slash commands, a message in two lines, Codex's
other line types, an archived `.jsonl.zst` rollout).

`XENONET_GOLDEN=update node --test test/golden.test.js` rewrites
`test/golden/*.json`; a mismatch saves what came out to
`.xenonet/golden-<name>.json` to diff. Update them only for a change meant
to change what XenoNet produces, and say so in the commit.

## Where the three transcript readers agree, and where they don't

The live tailer (`live.js`), a session's stats (`stats.js`) and search
(`search.js`) read the same lines through the same normalizers, and
share what they read alike: the line types, times, flags, a Claude line's
tool results, text and usage, a Codex line's envelope. They read each line
as one stream of events (`core/events.js`, `line_events.rs`), as the
refusals do: the mapper makes what they agree on, and each keeps its own
way, below, over the events. Wave 1 wrote up
where they read a line differently, and the owner decided at merge point
A how they should agree. In Wave 2 they do, each change with its golden
files:

- A line whose time can't be read is left out of stats and search; the
  live tailer takes it as written when it's read.
- A compacted summary and an interruption aren't prompts in any of them.
- A Codex rollout's prompts are its completed user messages
  (`item_completed` UserMessage) in all three, not the user_message
  events and user response items that may say the same again. Rollouts
  since Codex 0.130 rarely write user_message, so stats found no prompt
  in most; `codex exec` runs (Codex 0.146 here) write only user_message,
  so they have none now.
- A Codex shell call's output ends the call in the live tailer too,
  which left it in flight until the turn ended or went stale.

The owner decided more at merge point B, and in Wave 3 they agree on
these too:

- A Claude Code interruption, or a Codex turn you aborted (`turn_aborted`
  with the reason `interrupted`, or none, as older rollouts write it),
  ends the turn under way in the live tailer: a `turn-end` marked
  `interrupted`, and the session's `interruptedAt` until its next prompt,
  with no `needsYou`, so it isn't ready for review and doesn't chime.
  `replaced` and `review_ended` aren't you stopping it.
- A Codex rollout with no completed user message at all (`codex exec`
  writes none, nor did Codex before 0.130) takes its prompts from its
  `user_message` events, in all three.
- Codex user messages that are blank (an image alone) or start with `<`
  (a delegation envelope, a note Codex wrote for itself) aren't prompts
  in any of them (`isCodexPrompt` in `core/lines.js`).

Where they still read a line differently, each keeps its own way:

### Claude Code transcripts

| What | live | stats | search |
|---|---|---|---|
| A line whose time can't be read: none, one `Date.parse` can't read, or a number (which `Date.parse` would read as a year) | Written when it's read | Left out | Left out |
| Line kinds | `user`, `assistant` | `user`, `assistant`; a compacted summary is a compaction | `user` (not meta), `assistant` |
| A user line's text | Text blocks joined with spaces | The same | Every block's text joined with newlines (an image adds an empty line) |
| What's a prompt | Not meta, a compacted summary or an interruption (`[Request interrupted…`); not starting with `<`; a sidechain line only in a subagent's file, where the first names the subagent | Not meta, sidechain, a compacted summary or an interruption; a slash command as `/name args`; not `<…`; a subagent's first line is its task | Everything but compacted summaries, interruptions, `<…` and `# AGENTS.md…`; sidechain lines as role `subagent` |
| A compacted summary, an interruption | A compacted summary is left out. An interruption ends the turn under way (`turn-end` interrupted, `interruptedAt`, no `needsYou`), unless it's in a subagent's file | A compaction; left out | Left out |
| Assistant text | Each text block a message (main file only) | None | Text blocks joined with newlines |
| A user line with tool results | Each result | Each result | Skipped whole, read only as far as its first result |
| A message's usage | Once per id; the last 400 ids per session | Once per id, all of them | None |
| Subagent files | `<id>/subagents/agent-<agent>.jsonl` | Any `.jsonl` in `<id>/subagents/`, `agent-` dropped from the id | Any `.jsonl` under `<id>/subagents/` |

A line that's JSON but not an object (`null`, a number) used to break
each reader its own way: live counted an error, a session's stats failed
(the API answered 502) and search skipped the file. The lenient readers
(`core/vendor.js`, through the normalizers) now drop it for all three,
since nothing in it can be read, and keep it as drift.

### Codex rollouts

| What | live | stats | search |
|---|---|---|---|
| A line whose time can't be read | Written when it's read | Left out | Left out |
| Prompts | Completed user messages (`item_completed` UserMessage), joined with spaces. In a rollout with none, its `user_message` events, until its first completed one: an event just before that message in the same turn is one prompt, not two. Not blank, not starting with `<` | Completed user messages, or in a rollout with none at all, its `user_message` events, read from the whole file. Not blank, not starting with `<` | The same as stats, joined with newlines, and not `# AGENTS.md…` either (as for Claude); the same text in a file is indexed once. A rollout is indexed by its events while it has no completed user message, and again, whole, once it has one |
| A turn you stopped | `turn_aborted` with the reason `interrupted`, or none, ends the turn under way, as a Claude interruption does | None | None |
| Agent messages | `item_completed` AgentMessage (spaces), task_complete's last message | None | AgentMessage (newlines), agent_message, `response_item` message (role assistant) |
| Tool calls | function_call, custom_tool_call and local_shell_call start one, and their outputs end it (by call_id, else id) | The same; a local_shell_call, which has no name, is a shell call | None |
| Commands, MCP calls, file changes, web searches, images, subagents | From `item_completed`; a web search with an empty query shows its action's; a subagent is done once its activity says it completed or was interrupted, until it's given more (`interacted`) | None | None |
| Tokens | The growth of total_token_usage, with its output_tokens as the output: OpenAI counts reasoning inside output_tokens, and pricing takes it so (the owner changed this on 2026-10-21; live used to add reasoning_output_tokens on top). The context from last_token_usage | A call each time total_token_usage grows, priced from last_token_usage's parts | None |
| Model | turn_context's model | turn_context's model and effort; the service tier from the thread's settings (`thread_settings_applied`, where a tier not said is the default), or from turn_context in older rollouts; session_meta's provider (a provider other than OpenAI names the model) | None |
| Rate limits | token_count's rate_limits, in XenoNet's shape (`limits.codex`: each window's `pct`, `windowMinutes` and `resetsAt` in ms) | None | None |
| Archived and compressed rollouts | Not read | Read | Read |

One known difference comes of the live tailer reading lines as they
come: in a rollout whose first completed user message comes turns later
(an old one picked up again), it keeps those earlier turns' prompts from
their events, which stats and search, reading the whole file, leave out.
An index from before Wave 3 has its Codex rollouts indexed again, once
(search's `PRAGMA user_version` 1), for the events rule.

The console shows a Codex thread from the app-server's
`thread/items/list`, and when it can't have that (this Mac's Codex
answers "not supported yet", and the app-server may not start at all),
from the thread's rollout, read through the same normalizer
(`readRolloutPage` in `codex.js`): its completed items, each as the
app-server's would show, or for a rollout without any (`codex exec`
writes the older lines) what was said and each call and its output. The
rollout is read whole the first time and then only what's been added
(`readNewLines` in `lines.js`): an offset and a half-written last line
kept as bytes, as the tailer reads; a rollout written again is read from
the start, a compressed archive is read whole again only when it
changes, and one read happens at a time. A Codex whose app-server said
it can't isn't asked again.

## Lenient reading and drift

The normalizers' hooks are set: `claudeLines` and `codexLines` in
`core/lines.js` use `transcriptHooks` from `core/vendor.js`, so every
line the three readers read is counted, its file's version followed, a
sample checked against the vendor schema (the 1st, 2nd, 4th, 8th… line of
each kind in each file, sampled by `where`: the tailer's entry for the
file in the live tailer, `{ file }` in stats and search; a line whose
`type` isn't a string counts by what it is, rather than throwing), and
the rest guarded for type changes found before. A field of another type
is taken out of the record before the view reads it, so the readers fall
back the way they do for a missing field; a line that isn't an object is
dropped.
`schemas.md` (Vendor data) has the rest: the sources, the sampling and
its cost, drift records, `drift.json`, the report and the doctor's check.
