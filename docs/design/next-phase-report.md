# The next phase: what landed

The report on XenoNet's next phase, planned and approved on 2026-10-05 and
finished on 2026-10-06. `next-phase.md` has the plan and every decision
made along the way; `contracts.md` has what the workstreams agreed on.

## In short

- All four workstreams landed: eight themes that switch live, a
  schema-backed core that records vendor drift as shapes, actors on a layer
  of their own (you, Hermes, Grok Bot, programs you allow), the projects
  folder changed from the console, other machines as a prototype, and a VM
  lab to test them in.
- A code and security review found 52 issues; skeptics confirmed 50, and
  all 50 are fixed, five of them serious.
- 499 commits, all as Will. Unit tests went from 210 to 888 and
  end-to-end tests from 17 to 59, all passing.
- Nothing on the hot paths got more than 10% slower at any merge point.
  Against the code from before the phase, two did: page ready in XenoNet
  Dev (+14%, 49 ms, from loading 23 more files) and a full refresh (+12%,
  0.25 ms). Both are explained below. The actors layer and other
  machines stay inside their frame budgets.

## What landed

### Themes

- Four new themes, each a whole look (palette, type, motion, and what the
  map's sky, stars, floor, sweep, sonar and lines become): Helios, solar;
  Rime, ice; Trench, the deep sea; and Eclipse, for focus. With XenoNet's
  own and the three it had, that makes eight.
- Themes switch live, without a reload, and every window redraws in the
  new look. Theme… moves through them on the real map (Enter keeps one,
  Esc puts yours back); the gallery shows each as a card with swatches and
  a preview, and makes your own from any of them. A theme file of yours
  named for a built-in takes its place.
- New tokens: a body font (a theme's panels can be set in a plainer face
  than its titles), identity ring colors, and the actors' forms. The page's
  last colors that ignored the theme (diffs, panel tints) follow it, and
  the tray and the menu bar have a Theme menu.

### A strong core

- Versioned JSON Schemas (draft 2020-12, the major version in each `$id`)
  for XenoNet's own data, written from zod definitions and committed under
  `schemas/`, with a test that they're current. Vendor data has schemas
  of its own, by vendor, product and version: Claude Code's transcripts
  and config, Claude's desktop app, the Agent SDK, Anthropic's usage API,
  Codex's rollouts, app server, state database and sign-in claims, gh's
  hosts file, and Hermes's state. `adr/0001-schemas.md` says why zod.
- Lenient readers: a record that doesn't fully match is still used for
  what does, and every difference becomes a drift record that keeps
  shapes, never values (the review looked hard for a way a value could
  get through, and closed the ones it found: keys that are really data,
  such as logins and tokens). `xenonet drift` and the console show them, and
  the doctor warns about new ones.
- Building blocks: scanners, gatherers, tailers, normalizers and stores,
  each with a lifecycle and health the doctor reads, and test harnesses
  (fake process tables, clocks, ssh). The projects scan, the transcript
  readers, the search index and the engine's store moved onto them without
  changing what they do. The three transcript readers (live, stats,
  search) now agree on what a prompt, a turn and a bad timestamp are.
- `state.json` v2, migrated once with a backup; `engine.sqlite` copied
  before each migration, the copy kept for a week.

### Actors

- Persistent actors beside the session graph: you at the console, the
  always-on agents on this Mac (Hermes, observed through its state; Grok
  Bot, through its processes), and programs of your own that register
  through a generic interface. Muse is left out for now and dots are on
  hold, as decided.
- A layer of their own above the map, with beams into the sessions and
  projects they act on, drawn in each theme's own form, reacting when
  they act. The Actors panel lists each with how XenoNet knows it; a
  program becomes an actor only once you Allow it in XenoNet's window.
- The interface (`actors/interface.md`): registration that waits for your
  Allow, tokens kept only as hashes, a key only XenoNet's own window holds,
  process checks on macOS, rate limits, the `xenonet actor` command and the
  `xenonet` MCP server's actor tools. Agents see actors in `peers` and
  briefings, and actors' edits count in their warnings.
- The demo world has made-up actors with scripted activity, and the
  console's tools can simulate actors acting, asking to join, and waiting
  on you.

### Roots and hosts

- The projects folder changes from the console (or `xenonet root`), with
  each folder's layout, rules and Codex's picks kept with it.
- Other machines: after research (`hosts.md`), a prototype of the chosen
  design, a hub over SSH. Each machine runs its own XenoNet; `ssh <host>
  xenonet relay` streams its map read-only (titles, states and counts, unless
  asked for more); each host is a constellation of its own beside this
  one, bridged where two machines have checkouts of one repo. It's behind
  `XENONET_HOSTS=1`, and adding a host is yours alone.
- A VM lab (`scripts/lab`, `npm run lab`): two Linux VMs (Ubuntu in UTC,
  Fedora in Pacific/Auckland) cloned from golden copies nobody signs in
  to, each running XenoNet and a demo XenoNet, with Claude Code and Codex
  signed in as the same accounts as the Mac. They can't reach the Mac's
  loopback. The relay passed its privacy checks over real SSH on both,
  and all 18 of the lab's checks pass.

### The review, and Wave 3

- A review in six areas: who's calling (process checks, tokens, rate
  limits), the actors' model, other machines, the HTTP surface and the
  Electron shell, the core's data, and the page. Each pair of areas had a
  skeptic try to refute every finding. 52 findings (5 high, 21 medium, 26
  low; 15 security, 5 privacy); 50 confirmed, and every one is fixed.
- The five serious ones:
  - A hostile machine on the map could run script in XenoNet's window
    through a field the activity feed didn't escape. A host's data is now
    cut to the fields its schemas declare, and every field the page writes
    into markup is escaped.
  - A theme file whose `extends` named something every object has
    (`toString`) skipped the theme checks. Now `extends` can only name a
    theme.
  - `git status`, which XenoNet runs in every repo every 20 seconds, could
    still run programs a repo's own config names (filter drivers, an
    index hook). Those are off for its calls now; your own global git
    config keeps working.
  - On macOS, a process newer than XenoNet's cached process table, or a
    caller that closed its connection right after sending, passed the
    process checks. XenoNet now finds them, and refuses what it can't see.
- Also from the review: the page runs under a Content-Security-Policy; the
  desktop bridge answers only XenoNet's own page; the settings that change
  what XenoNet reads or shows (the projects folder, your rules, Codex's
  picks, identities, the theme) are yours, never an agent's; rate limits
  an actor can't dodge; look-alike names refused; drift keeps no keys that
  are really data; XenoNet's `ps` reads work on a Mac in any language (they
  came back empty in German, which the stricter checks would have turned
  into refusing everything).
- The same wave built the twenty decisions from merge point B (presence
  with the window hidden, observed actors, framing that leaves room for
  the alert strip, the readers' rules, the hosts' frame budget and cap,
  `xenonet drift --exit-code`, and the rest), and cleared the leftovers from
  both merge points: theme colors that ignored the theme, the Theme menu,
  flaky tests, a test that could print this machine's setup, a shared Vite
  cache, and a relaunch race in XenoNet Dev's control channel.

## Numbers

Made-up data only (`scripts/bench.mjs`), on AC power, in rounds that
alternate the checkouts; medians of three rounds.

**Before the phase and after**

| Hot path | Before | After | Change |
|---|---|---|---|
| Graph build, 4,000 sessions | 8.98 ms | 8.70 ms | −3% |
| Graph build, 300 sessions | 0.95 ms | 0.91 ms | −4% |
| Full refresh, demo world, warm | 2.02 ms | 2.27 ms | +12% |
| Tailing: start / a sweep / finding new files | 418 / 1.01 / 2.07 ms | 444 / 1.00 / 2.03 ms | +6% / −1% / −2% |
| Tailing: a line to its event | 44.1 ms | 44.4 ms | +1% |
| Server start, to listening | 738 ms | 803 ms | +9% |
| Page ready, in XenoNet Dev | 340 ms | 389 ms | +14% |
| Frame time, demo | 2.4 ms | 2.5 ms | +0.1 ms |

- **Page ready (+14%, 49 ms once per load).** XenoNet Dev serves the page
  as separate modules, and the page now loads 81 files at boot instead of
  58: the actors layer (8), the theme looks (7), other machines (3), the
  projects folder's panel, framing and the terminal's clipboard. A
  packaged build bundles them; the benchmark measures only XenoNet Dev.
  Loading the hosts code only when hosts are on, and the projects-folder
  panel, the actor inspector and the request view on first use, would
  take about eight files off; that's on the maintenance list.
- **Full refresh (+12%, 0.25 ms).** The rounds overlap (1.94 to 2.19 ms
  before, 2.13 to 2.36 after), and a quarter of a millisecond a refresh
  isn't worth chasing.
- At each merge point, against the main before it, nothing was more than
  10% slower once anything a load spike flagged was run again on a quiet
  machine (merge point A's +12% on page ready was answered by loading the
  theme switcher and the gallery on first use).

**What's new, and its budget**

| | Cost | Budget |
|---|---|---|
| The actors layer, demo (4 actors, 8 beams) | +0.1 ms a frame | 0.5 ms median, 1.0 ms p95 |
| The actors layer, 6 actors and 24 beams | +0.2 ms median, +0.1 ms p95 | same |
| Two other machines, capped (as when you look at neither) | +1.5 ms median, +1.6 ms p95 | 2 ms median, 3 ms p95 |
| Two other machines, drawn in full | +3.0 ms median | |
| Switching themes, to settled | 222 ms, live | |
| Frame time in each theme | 2.2 to 2.6 ms | |
| Hermes's observer: a quiet look / 20 new rows / its first look | 1.8 / 2.6 / 25 ms | |
| Grok Bot's observer: a look / a poll | 1.2 / 0.01 ms | |

## Decisions

Every decision is in `next-phase.md`, under the gate it was made at:
gate 0 (the themes, which actors), merge point A (the hosts
design, the VM lab, the actors' order and privacy, the theme tokens, the
readers agreeing, the root), merge point B (twenty calls on presence,
observers, framing, readers, hosts and drift) and Wave 3 (the calls made
in review, which the owner agreed to). `contracts.md` sections 8 to 10
list what each changed in the contracts.

## What's left

**Next: the maintenance pass.** As you asked, the skills available here
applied to the whole codebase: code review and security review at high
effort, simplify, dataviz on the stats panel and gauges, claude-api on
the pricing table, fewer permission prompts, refreshing AGENTS.md, and
consolidating memory. The page's boot files go on that list too.

**Still open, by area** (from the lanes' notes; none blocks anything):

- *Security.* `/api/open`, `/api/handoff`, `/api/ack` and `/api/layout`
  still answer any local program. An agent's request in its own session's
  name passes unchecked when netstat has no line for the connection at
  all (kept so agents' tools survive a change in netstat's output).
  iTerm2's shells run outside its app bundle, so an actor's own processes
  are cut there less tightly. Presence changes aren't counted against the
  events rate. On Linux and Windows an actor's token alone decides, as
  designed, and the Windows ACL code hasn't run on a real Windows machine.
  Git older than 2.44 can still fetch lazily in a partial clone, and your
  own git-lfs reads a repo's `lfs.storage`.
- *Other machines.* Schemas for what the relay reads through (transcripts,
  stats, diffs), so the hub can strip those too; events that arrive while
  the relay sends its snapshot are lost; remote alerts don't reach the
  tray's count, and remote engines and actors aren't relayed; collisions
  across machines, the merged repo view and a layout for many hosts are
  still to come; Phase B of the lab, a macOS guest, waits for disk space.
- *Actors.* Muse and dots, and Hermes reporting itself through its plugin;
  `lastSeen` still moves on XenoNet's own presence changes.
- *Core.* The console's drift view, decided at merge point A, wasn't
  built: the doctor's drift warning shows in the console, and `xenonet
  drift` shows the records and marks them seen. Drift records the schemas
  have since learned stay in `xenonet drift --all` until newer ones push
  them out; stats don't end a turn at an interruption; the tailer doesn't
  notice a file rewritten to the same size or larger; with actors off, a
  long-running XenoNet deletes engine.sqlite's week-old copy only when it
  next opens the database.

**Housekeeping.** About 6,700 `xenonet-*` folders that test runs left in the
temp folder; the main checkout's old shared `node_modules/.vite`, unused
now; this phase's worktrees and branches, once main has everything; the
VM lab keeps running until you tear it down (sign out in each VM first).
