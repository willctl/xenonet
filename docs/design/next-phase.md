# The next phase

The plan for XenoNet's next phase, approved on 2026-10-05: four workstreams,
built by subagents working in parallel in git worktrees and merged at two
merge points. `contracts.md` has what the workstreams agree on up front.
It finished on 2026-10-06: `next-phase-report.md` says what landed, the
numbers before and after, and what's left.

## The workstreams

1. **Themes.** Four new themes, each a whole look: palette, type, motion,
   and what the map's sky, stars, floor, sweep, sonar and lines become.
   Themes switch live, without a reload. Moving through them previews each
   one on the real map (Enter keeps it, Esc puts the old one back), and a
   gallery shows swatches and previews, along with making your own.
2. **A strong, consistent core.**
   - Versioned JSON Schemas for XenoNet's own data, and for the vendor data
     it reads.
   - Lenient adapters that record drift in vendor data as shapes, never
     values.
   - A drift report, and a doctor check for it.
   - Building blocks (scanners, gatherers, tailers, normalizers and
     stores), with the existing subsystems moved onto them without
     changing what they do.
3. **Actors.** Persistent entities beside the session graph:
   - the person at the console;
   - always-on agents;
   - custom actors, through a generic interface.

   They're drawn on a layer of their own, with beams into the sessions and
   projects they act on.
4. **Roots and hosts.** Changing the projects folder from the console, a
   recommendation for multi-host workspaces and a prototype of it, and a
   VM lab to test it in.

## Decisions at gate 0

- **Themes:** Helios, Rime, Trench and Eclipse (see `theme-slate.html`).
  - Orrery, Kintsugi and Mycelium are next in line.
  - Vellum, a light theme, needs a rendering path of its own first.
- **Actors:**

| Name | Product | Status |
|---|---|---|
| hermes | Hermes Agent, Nous Research | Installed on the development Mac; fully observable |
| grok bok | Grok Bot, SpaceXAI | Goes ahead |
| dots | OpenAI's dots | On hold |
| muse | Meta's Muse | Decided at merge point A |

- **Schemas:** zod 4 with `z.toJSONSchema`, unless core-1's spike finds a
  reason not to.
- **Vite:** the dev server ignores `.claude/`, so worktrees never reload
  XenoNet Dev's page (landed on main before Wave 1).

## Decisions at merge point A (2026-10-05)

Everything recommended at merge point A was accepted. Contract changes are
listed in section 8 of `contracts.md`.

- **Hosts:** a hub over SSH. `ssh <host> xenonet relay` streams a host's
  snapshots read-only, each host is a constellation of its own, and
  checkouts of one repo are bridged. SSH only for now. Titles, states and
  counts cross machines; transcripts only when opened. Remote hosts stay
  read-only through Wave 2.
- **VM lab:** Lima, with two Linux VMs first; a macOS guest later, once
  disk is free. XenoNet's hosts get an ssh config file of their own. Codex
  signs in through a forwarded port, and the VMs use the same accounts as
  the development Mac.
- **Actors:** built in the order `interface.md` gives.
  - Every new actor waits for an Allow in XenoNet's window.
  - The person at the console is an actor, kept 24 hours and never named.
  - Agents see actors in `peers`, and a registered actor's edits count in
    the warnings.
  - Grok Bot is presence and process ancestry. Hermes is observed and
    attributed, reading only file paths from its tool calls. Muse is
    left out for now; dots are on hold.
- **Themes:**
  - The bloom fix stays, with Gridline brought down to about 1.1.
  - New tokens: `fonts.body` (a plainer face for the panels' text than for
    titles) and `identities` ring colors.
  - "Make your own" keeps putting the copy in use, and the switcher and
    gallery stay at the right.
  - The terminal keeps fixed ANSI greens, blues, magentas and cyans.
  - The switcher and the gallery load on first use: that answers the
    page's +12% on driver-ready.
- **Core:**
  - The three transcript readers are made to agree in Wave 2: a bad
    timestamp is skipped by stats and search; compacted summaries aren't
    prompts; Codex prompts come from completed user-message items
    everywhere; a shell call's output ends it in live.
  - engine.sqlite is copied before a migration.
  - `xenonet.blocks` leaves out what a failed source already reports.
  - Drift gets `xenonet drift` and a console action.
  - No Ajv for now.
- **Root:** "Clear your rules" clears the folder in use, with a separate
  action for every folder. `git status` runs with
  `core.fsmonitor=false`. `xenonet root` needs a running XenoNet.

## Decisions at merge point B (2026-10-05)

Everything recommended at merge point B was accepted, along with Wave 2
as built. Contract changes are listed in section 9 of `contracts.md`;
Wave 3 builds the rest.

- **You:** you go offline ten minutes after XenoNet's window is hidden, and
  turning off "Show me on the map" deletes your record at once.
- **Actors:**
  - One XenoNet only observes (Hermes, Grok Bot) can't be removed: its
    observer's switch takes it off the map. Installed products stay
    listed, as offline, when they haven't run for a week.
  - Hermes's edited paths count in agents' edit warnings, marked as
    observed.
  - Grok Bot's version comes from its app's `Info.plist`, that one key.
    Its observer polls every 3 s, not every second.
  - The doctor's scan for actor tokens in repos runs once a day and when
    you run the doctor.
  - A request card uses the attention color in every theme, and the
    camera leaves room for the alert strip and panels when it frames the
    map.
- **Readers:**
  - A Claude interruption ends the turn in the live view, and its status
    says it was interrupted.
  - A Codex rollout with no completed user messages takes its prompts from
    `user_message` events (`codex exec`, older rollouts).
  - Codex user items that start with `<` aren't prompts in any reader.
- **Hosts:** grow the v1 schemas; give hosts a frame budget and cap the
  remote nodes drawn beyond what you're looking at, measured by a hosts
  part in the benchmark.
- **Drift:** `xenonet drift --exit-code` for scripts, and only `xenonet drift`
  and the console mark drift as looked at.
- **Security:** the Wave 3 review weighs git filter drivers, which a
  repo's own config can make `git status` run.
- **Lanes:** frame time is measured by the benchmark at merge points, not
  in windows of a lane's own.

## Decisions in Wave 3 (2026-10-06)

The calls the owner agreed to in Wave 3, or that its review made. Contract
changes are listed in section 10 of `contracts.md`.

- **Git status:** only the filter drivers a repo's own config names are
  emptied; your global and system ones keep working, and git-lfs's stay
  on. A submodule counts as changed only when it's on another commit.
- **Grok Bot** is listed as unknown, out of sight, for as long as it's
  installed, and is never offline.
- **Codex's picks are yours**, as the other standing settings are: asking
  Codex and forgetting its picks.
- **Themes:** a `sonar.every` of 0 or less sends no sonar.
- **Launch** ends a demo or test copy stuck holding the instance's lock by
  itself, since its data is made up, and says so.
- **Drift:** `xenonet drift --exit-code` leaves drift unmarked, and
  `npm run contracts` writes XenoNet Dev's drift when run from a checkout.
- **The process check:** on macOS, a request whose process XenoNet can't see
  is refused.
- **The standing settings are yours:** the projects folder, your rules
  and Codex's picks, identities' names, hidden flags and pins, and the
  theme, from XenoNet's window or your own terminal, never from under an
  agent or an actor product.
- **Names keep to one script**, as Unicode's highly restrictive profile
  has it.
- **A custom product can't name a maker** of a known product: Anthropic,
  OpenAI, Nous Research, SpaceXAI, xAI, Anysphere, Cursor, Meta or
  willctl.
- **On Windows,** a token file whose ACL can't be read is refused.
- **The doctor's token scan** skips dependency, build and cache folders
  when it looks past `.gitignore`, and only a run you ask for looks
  afresh; the others get the day's look.
- **Framing** keeps the map between the docks: the side panels, the alert
  strip and the terminal.
- **Hosts:** a frame budget of 2 ms median and 3 ms p95 for the hosts
  you're not looking at, and a cap of 24 of their nodes.
- **Readers:** only a Codex turn aborted as interrupted, or with no reason
  (older rollouts), counts as stopped.

## Waves

Every workstream works in `.claude/worktrees/<name>` on its own branch,
with `node_modules` linked to the main checkout's. Every Wave 1 branch
starts from `w0/plan`.

| Wave | Branch | Owners | What |
|---|---|---|---|
| 0 | `w0/plan` | orchestrator | This plan, the contracts, the theme slate, `scripts/bench.mjs` |
| 1 | `w1/core` | core-1, then core-2, then core-3 | Schema system, internal schemas, `state.json` v2 and migrations, the actor contract stub; building blocks and harnesses, subsystems moved onto them; vendor schemas, drift, the drift report, the doctor check |
| 1 | `w1/themes` | themes-1, then themes-2 | Live theme switching and the new tokens; the four themes, the switcher and the gallery. The Wave 1 UI lane |
| 1 | `w1/root` | root | Changing the projects folder: core, API, command line, picker, driver, console tool and e2e test |
| 1 | `w1/actors-research` | hermes and cloud, then interface | Deep-dives and install plans for the actors, then the generic actor interface |
| 1 | `w1/hosts-research` | hosts and vm-lab | Multi-host options with a recommendation, and the VM lab plan |
| A | main | orchestrator | Merges, tests, e2e, benchmarks, theme screenshots, and the next round of decisions |
| 2 | `w2/actors-core` | actors-core | The generic interface (engine, API, MCP tools, command line), the person as an actor, observers for the approved actors, demo actors |
| 2 | `w2/actors-ui` | actors-ui | The actors layer. The Wave 2 UI lane |
| 2 | `w2/hosts` | hosts-proto | The chosen multi-host option, and the VM lab |
| 2 | `w2/core-scan` | core-4 | The projects scan moved onto the scanner block |
| B | main | orchestrator | Merges, tests, e2e, benchmarks, the actors layer's frame budget |
| 3 | `w3/*` | reviewers, fixer, docs | Code and security review (find, then verify), fixes, docs, README, final report |

Wave 1 runs as two workflows side by side:
- **build:** core-1 → core-2 → core-3, themes-1 → themes-2, and root.
- **research:** hermes, cloud, hosts and vm-lab, then interface.

Each agent returns its commits, test results, benchmark numbers and open
questions.

### Merge points

- **Merge point A** merges `w0/plan`, `w1/core`, `w1/root`, `w1/themes`,
  then the research branches. Each is rebased onto main and fast-forwarded,
  so history stays linear.
- **Merge point B** merges the Wave 2 branches the same way.
- **After each code merge:** `npm test`, `npm run test:e2e`, and
  `node scripts/bench.mjs`, against the numbers from the last merge point.
- Anything more than 10% slower is fixed or justified.

## Baseline

On main at c57da05, with made-up data. `node scripts/bench.mjs` measures
all of these.

| Hot path | Median |
|---|---|
| Graph build, 40 projects and 300 sessions | 0.88 ms |
| Graph build, 400 projects and 4,000 sessions | 8.9 ms |
| Full refresh, demo world (warm) | 1.9 ms |
| Tailing: start over 92 recent files (54 MB) | 389 ms |
| Tailing: an appended line to its event | 43.7 ms |
| Tailing: a new transcript's first line to its event | 43.7 ms |
| Server start to first graph (demo) | 724 ms |

Page load and frame time join this table when the benchmark first runs
them on main.

### Budgets

- **The actors layer:** at most 0.5 ms median and 1.0 ms p95 per frame
  with the demo's actors (6 actors, up to 24 live beams), and close to
  nothing with no actors.
- **Hosts:** the hosts you're not looking at add at most 2 ms median and
  3 ms p95 a frame with two demo-sized hosts
  (`node scripts/bench.mjs hosts`); at the end of Wave 2 the two drawn in
  full added about 3.2 ms.
- **Validation and drift recording:** within 10% on tailing and refresh.
- **Every theme:**
  - text and labels at least 4.5:1 against their backgrounds;
  - muted text at least 4.5:1, dim text at least 3:1;
  - alert colors at least 3:1 against panels;
  - no page errors;
  - switching to it and back leaves frame time and memory as they were.

## Bugs found while planning

| Bug | Lane |
|---|---|
| Theme bloom is ignored after the first frame: `src/scene.js` resets it every frame | themes |
| Theme fonts never reach the map's labels: `src/nodes.js` hardcodes Chakra Petch | themes |
| The terminal, poster, replay, activity feed, identity rings and the Electron window background ignore the theme | themes |
| A theme of your own with a built-in's id is silently replaced by the built-in | themes |
| A theme switch reloads the page and wipes the op log. Theme commands aren't logged and can't run from the menu bar. Film is logged twice | themes |
| A `state.json` that isn't valid JSON is replaced by `{}` and overwritten | core |
| The three transcript readers (live, stats, search) disagree on line types and on bad timestamps | core: normalized where outputs stay the same; the rest is written up |
| Codex's raw `rate_limits` reach the API, with `resets_at` in seconds | core |
| The Codex app-server handshake claims version 0.1.0 | core |
| Codex's picks would be re-applied to the same relative paths under a new root. Rules outside the root can't be undone one at a time. The git status loop never drops old repos | root |

## Risks

- **Live theme switching touches every renderer.** Guarded by an e2e test
  that walks every theme and back, and by frame time and memory checks.
- **Moving subsystems onto building blocks can change behavior quietly.**
  Guarded by tests first, one move per commit, golden tests on the demo
  world, and the benchmark after each move.
- **The `state.json` migration runs on real state at the first start after
  merging.** Guarded by a backup beside the file, migrations tested on
  made-up v1 files, and never writing back a file XenoNet can't read.
- **Validation in the tailer's hot path.** Guarded by sampling, cached
  shapes, and the benchmark.
- **Three of the four actors live in their makers' clouds.** XenoNet may see
  only their presence and the work they do on this machine, and Wave 1
  settles what's observable before anything is modeled.
- **Registering an actor is a new way in for any local program.** Guarded
  by tokens, the macOS process check, size and rate limits, and actors
  approving nothing. Wave 3 reviews the security of all of it.
- **Remote hosts.** The API stays on 127.0.0.1; another machine reaches it
  only through an authenticated tunnel. The process checks can't see
  across machines, so remote input is trusted less.
- **The VM lab.** macOS guests are large, and Apple's license allows two
  per Mac.
- **Privacy.**
  - Fixtures are made up.
  - Drift keeps shapes only.
  - An actor's conversations are read for metadata only.
  - No machine name becomes a host id.
  - Screenshots come from the demo instance.

## Gates

- **Merge point A:**
  - theme screenshots;
  - the actor write-ups and install plans: which to install, where from,
    size and cost;
  - the host options, to pick one;
  - the VM lab plan;
  - the schema layout;
  - the go-ahead to migrate the development `state.json`.
- **Wave 2:** approved installs and VM downloads. Every sign-in is done by
  the owner.
- **Merge point B:** review and security findings that need a decision.
- **The end:** a final report.
