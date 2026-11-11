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
