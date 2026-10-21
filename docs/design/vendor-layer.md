# The vendor layer, version 2

A design for how XenoNet maps each app's data to its own: one registry of
the apps it knows, one stream of events read from their transcripts, and
shared types. Proposed, for review; nothing here is built yet.

**In short:**

- "The vendor layer" is every piece of code that knows a particular app:
  where its data lives, how its lines read, what its tools are called, how
  to connect it, resume it, open it and tell its processes apart.
- Claude Code and Codex are special-cased throughout. The strings
  `"claude"` and `"codex"` appear in about 90 Rust files and 70 JS files.
  About 15 places pick one of the two by testing for the other, and about
  40 lists and objects in the JS hold exactly those two. Adding an app
  that XenoNet tails and connects, as it does those two, means changing
  more than 50 files.
- **An app registry.** One module of plain data says what's fixed about
  an app. `SESSION_APPS` grows into it, Rust gets a generated copy, and
  the lists that hold two apps come from it.
- **An event stream.** Each tailed app has one mapper from its transcript
  lines to shared events. Live, stats, search and refusals read the
  events, not the vendor's lines. Stats already works this way.
- **Shared types and capabilities.** One token-usage type, one patch
  parser, one drift-reader pool, and small interfaces for what differs
  per app in code: a line format, a session source, a native reader, a
  connect format, delivery, process matching, caller identification.
- What XenoNet shows doesn't change. The golden files, the parity tests
  and `test:bin` hold at every step, and each of the agreements in
  `blocks.md` on how live, stats and search read a line is kept as it is,
  including where each reader keeps its own way. A change to output is
  its own step, for the owner to decide.

## What's there now

Apps reach XenoNet in three ways:

| Way in | Apps | Where they're known |
|---|---|---|
| Tailed and connected | Claude Code: its transcripts, sessions listed through the Agent SDK and Claude desktop's files, plan usage from Anthropic's API and Claude desktop. Codex: its rollouts, threads through `codex app-server`, projects from its state database | `core/lines.js`, `live.js`, `stats.js`, `search.js`, `claude.js`, `codex.js`, `identities.js`, `engine/connect.js`, `delivery/*`, `procs.js`, `engine/verify.js`, `open.js`, `doctor/`, `mcp/xenonet.js`, and their Rust twins |
| Experimental native readers, on only when `XENONET_READERS` names them | OpenCode, Gemini CLI, Copilot CLI, Cursor | `core/native/*.js`, `crates/xenonet-sources/src/native/*`, wired in `core/state.js` and `crates/xenonet-wire-map/src/observed.rs` |
| Observed or reported runs | about 20 more apps through numbat's records and actors' reports | `core/numbat.js` (with its own app names and a version allowlist), `core/reported-runs.js` |

The first two read vendor data checked against vendor schemas as
`schemas.md` describes, with drift kept per source. Numbat has a version
allowlist instead, and actors' reports aren't vendor data.

What they map into:

| Shape | JS | Rust |
|---|---|---|
| A session (`session/v1`) | `core/schema/session.js`, whose app enum already lists every app in `SESSION_APPS` | No struct: ordered key lists (`sources/src/codex.rs` `Obj`, `native/mod.rs` `Out`), in each app's own key order. Claude's go through a function named `codex_obj` |
| A transcript line | `ClaudeLine`, `CodexLine` through a `Normalizer` | `sources/src/lines.rs`; `wire-map/src/vendor.rs` holds exactly two normalizers, picked by `codex: bool` |
| One call's tokens | `{ input, cacheRead, cacheWrite5m, cacheWrite1h, output }`, as stats and pricing take it; `ClaudeLine.usage` adds `cacheWrite` and `fast` | Three shapes and a wrapper: `lines::Usage`, `pricing::Tokens` (wrapped by `telemetry::Call`), `stats::Event.tokens` |
| A stats event | `stats.js` `claudeEvents` (call, tool, result, prompt, end, spawn, task, compact) and `codexEvents` (prompt, end, call, tool, result) | `state/src/stats.rs` |
| Plan limits | Claude's and Codex's own shapes in `live/snapshot/v1` | The same |

What repeats:

- A Codex `token_count` becomes tokens twice, in `live.js` and `stats.js`
  (and in `telemetry.rs` and `stats.rs`), and each reads it its own way.
- A transcript's path becomes a session id in five places with different
  patterns; content becomes text in four to six.
- `apply_patch` headers are parsed in four places in JS and three in
  Rust; live's pair leaves out `*** Move to:`.
- Source names are kept in six tables: `core/drift.js`, `drift.rs`,
  `scripts/provider-drift/registry.mjs` (which is already a registry of
  providers, with their sources, release feeds and inspectors), the
  readers, `observed.rs` and `wire-map/src/vendor.rs`.
- The Rust binary keeps two drift-reader pools, so the drift report lists
  no read counts or versions for what the actors' observers read (their
  drift itself is logged).

## Goals

1. Adding an app touches a short, known list of files.
2. A fact about an app that is data (its folders, ids, signing team, hook
   events, edit tools) lives in one place. Behaviour stays in code, in
   the capability it belongs to.
3. A rule the transcript readers share is written once. Where `blocks.md`
   says each keeps its own way, each still does, from the same events.
4. Output, and behaviour on the wire, don't change. Every step passes the
   golden files, `npm test`, `test:rust`, `test:bin` and `test:e2e`
   unchanged.

Not in scope: the vendor schemas' rules (`schemas.md`), the actors'
observers beyond the tables they share, the places where XenoNet runs
Codex or Claude Code rather than reads them (narration and
classification, diagnostics, handoff, the integrations' sandboxes), and
retiring the JS core.

## 1. The app registry

**Where.** `core/apps.js`: plain data, no imports, so the page and
`core/schema/ids.js` can both use it. As with themes, JS is the source
and Rust gets a generated copy: `scripts/apps.mjs` writes
`crates/xenonet-base/src/apps.json`, which `xenonet-base` embeds and
parses once into typed entries, and `scripts/apps.mjs --check` in a test
fails when the copy is stale. `ids.rs`'s hand-kept `SESSION_APPS` goes.
The registry gets a schema, `app/v1`, listed in `contracts.md` section 1
first, as `schemas.md` asks of a new schema, and each entry is checked
against it.

**An entry** holds only what is data:

| Field | What | Today in |
|---|---|---|
| `id`, `name` | `codex`, `Codex` | `SESSION_APPS` |
| `numbat` | the names numbat records it under (`claude-code` for `claude`) | `core/numbat.js` `LEFT_OUT` and its mapping |
| `products` | its vendor products (`openai/codex`); each product's sources stay in `scripts/provider-drift/registry.mjs`, the one list of sources, which `core/drift.js` and `drift.rs` are checked against | the six tables above |
| `roots` | the env variable that moves the app's folder, and its default (`CODEX_HOME`, `~/.codex`) | `config.js`, `connect.js`, `claude-usage.js`, `delivery/resume.js` |
| `context` | the default context window (200000, 258400) | `live.js` |
| `identity`, `limits` | which identity and plan-limits reader it has | `identities.js`, `live.js`, `delivery/usage.js` |
| `team` | the team that signs it (codesign's TeamIdentifier) | `procs.js` `TEAMS` |
| `connect` | its config format, and the hook events with their matchers | `connect.js` `CLAUDE_EVENTS`, `CODEX_EVENTS` |
| `editTools` | for a native reader, the tools whose calls are edits | each reader's `EDIT_TOOLS` |
| `has` | the capabilities it has (section 4) | spread across the code |

For example, Codex:

```js
{
  id: "codex",
  name: "Codex",
  numbat: ["codex"],
  products: ["openai/codex"],
  roots: { env: "CODEX_HOME", default: "~/.codex" },
  context: 258400,
  identity: "chatgpt",
  limits: "codex",
  team: "2DC432GLL2",
  connect: { format: "codex-hooks", events: [["SessionStart"], ["UserPromptSubmit"], ["PreToolUse", "apply_patch|Edit|Write"], ["PostToolUse"], ["Stop"], ["SessionEnd"]] },
  has: ["lines", "sessions", "connect", "delivery", "processes", "caller"],
}
```

Claude Code's entry has four products (`anthropic/claude-code`,
`claude-desktop`, `agent-sdk`, `oauth`), and its `roots` say that
`CLAUDE_CONFIG_DIR` moves two things with different defaults
(`config.js`). Finding Claude desktop's folder, a search of candidates per
platform with its own override, stays code (`core/dirs.js`), behind the
session source. The apps XenoNet only observes keep entries with `id`,
`name` and their numbat names.

**Which reader a session came from is the session's, not the app's.**
Gemini CLI's runs come from its native reader when `XENONET_READERS` names
it and from numbat otherwise, and an actor can report a Claude Code run
through the gateway, which has no transcript. The session's own
`observed` and `reader` fields keep deciding what XenoNet can do with it;
the registry says what the app can do at most.

**What's generated from it.** The lists that hold the tailed apps: `live/*`'s
`App`, `series` and `costs.byApp`, and the page's per-app series and
filters. They come from the entries that have `lines`, so the files stay
as they are until an app gains it. These stay as they are, since they
aren't lists of apps or are public inputs:

- `identity/v1`'s kinds (accounts: `claude`, `chatgpt`, `openai`,
  `github`, `git`);
- `engine/agent/v1`'s `App` (`claude`, `codex`, `actor`; the engine
  already produces others);
- `mcp/xenonet.js`'s tool input `app` (`claude`, `codex`, `other`);
- the assistant protocol's `SOURCES` and `SessionSummary.app`;
- the theme tokens `--claude` and `--codex` (`theme/v1`).

Widening any of these is a contract change of its own when an app needs
it. Grok Build widened two, by adding to them: `engine/agent/v1`'s `App`
has `grok`, since its sessions connect to the engine, and `theme/v1` has
`colors.grok` (see [Grok Build in XenoNet](grok-build.md)).

**The fallbacks stay, as one lookup.** About 15 places pick one of the two
apps by testing for the other: most read an app that isn't Codex as
Claude Code; a few (`cli/commands/hook.js`, `/api/diagnose/ask`,
`cli/commands/diagnose.js`, `search.js`, `src/doctor.js`) read one that
isn't Claude Code as Codex. Each becomes a lookup in the registry with the
same fallback as today, so `xenonet hook <app>` and the routes behave as
they do now. Refusing an app the registry doesn't have is a later step
(step 6), since it changes behaviour on the wire.

## 2. One event stream

**Today** stats turns each app's lines into events, while live
(`applyClaude`, `applyCodex`, `codexItem`), search (`docsFromLine`) and
refusals (`claudeSignal`, `codexSignal`) each read the vendor's lines
again.

**Proposed.** Stats' events grow into the shared stream, in a new module
(`core/events.js`, `crates/xenonet-sources/src/line_events.rs`). Each
tailed app has one mapper:

```
mapper.events(line, file) → [LineEvent]
```

`line` is the normalized line (`ClaudeLine`, `CodexLine`). `file` is what
the reader keeps per file: the Codex prompt mode, the turn marks live
uses, and the last token total. What is kept per session stays the
reader's (live's last 400 message ids across a session's files, stats'
ids across all of them).

**The events.** Each carries `t` (the line's time, `NaN` when it can't be
read), `agentId` (a subagent's file, else null) and `origin` (the line's
kind, as the sampling hooks name it):

| Event | Carries |
|---|---|
| `prompt` | `parts`: every content block in order, with its kind and its text (`""` for a block without text); `command` (a slash command's name and args); `meta`, `sidechain`, `compactSummary`, `interruption` |
| `said` | the assistant's blocks as `parts`; `sidechain` |
| `call` | `messageId`, `model`, `effort`, `usage` (section 3), `window`, `fast` or `priority`, `provider`; for Codex also the running `total` |
| `tool` | `id`, `name`, the tool's input as written |
| `result` | `id`, `error` |
| `item` | a Codex completed item: its type (command, MCP call, file change, web search, image, subagent), its status, and what it holds |
| `turnStart`, `turnEnd` | for `turnEnd`, `reason` (`end_turn`, `task_complete`, `interrupted`, `refusal`) and Codex's `duration_ms` |
| `settings` | model, effort, service tier and provider, as `turn_context`, `thread_settings_applied` and `session_meta` say them |
| `compact`, `spawn`, `task` | as in stats |
| `limits`, `context` | the windows and context a line reports |
| `refusal` | the signal, as `core/refusals/detect.js` builds it |
| `broken` | a field the mapper couldn't read (below) |

**Where the readers agree, the mapper decides.** `blocks.md` lists what
all three share, and the mapper does it once:

- a compacted summary and an interruption aren't prompts;
- a Codex rollout's prompts are its completed user messages, or its
  `user_message` events in a rollout with none;
- Codex messages that are blank or start with `<` aren't prompts
  (`isCodexPrompt`).

The whole-file readers (stats and search) set the Codex mode from the
whole file before mapping, as `hasUserMessages` does now; live sets it as
lines come.

**Where each keeps its own way, each still does.** The events carry what
each reader's rule needs, and the rule stays in the reader:

| `blocks.md` difference | The event carries | The reader keeps |
|---|---|---|
| A line whose time can't be read | `t: NaN` | Live takes it as written when read; stats and search leave it out |
| A user line's text | `parts`, with every block | Live and stats join text blocks with spaces; search joins every block with newlines, an image adding an empty line |
| What's a prompt beyond the shared rules | `meta`, `sidechain`, `command`, the raw text | Stats' slash-command form, search's `# AGENTS.md` rule and `subagent` role, live's subagent rule |
| An interruption, an aborted Codex turn, a refusal | `turnEnd` with its `reason` | Live ends the turn under way; stats counts only `end_turn` and `task_complete` |
| A message's usage | `messageId` on every `call` | Live keeps the last 400 ids per session; stats keeps them all |
| Codex tool calls | `tool` and `result` from response items; `item` from completed items | Stats counts calls; live counts completed items, and `apply_patch` calls |
| Codex tokens | `call.usage` (the last call) and `call.total` | Live goes by the total's growth, output and reasoning output kept; stats prices the last call |
| Codex agent messages | `said` with its `origin` | Live reads completed AgentMessage items and `task_complete`'s last message; search also `agent_message` and assistant response items |
| Subagent files | `agentId`, as the reader found it | Each reader's own rule for which files are subagents' (`blocks.md`'s table) |
| Archived rollouts | nothing new | Live doesn't read them; stats and search do |

**Errors.** `lines.js` reads a record when a reader asks, so a line that
broke a reader breaks it at the same field it did before, and search,
reading only as far as a user line's first tool result, never reaches the
fields after it. An eager mapper would read fields some readers never
read. So the mapper never throws: when a field can't be read, the line's
events end with `broken`, naming where. A reader that would have reached
that field treats it as it treats a throw today (live counts an error and
goes on to the next line); one that wouldn't ignores it.

**The test contract stays.** The tests call the per-app entry points
directly: about 70 calls to `applyClaude`, `applyCodex` and `codexItem`,
and `docsFromLine`, `claudeSignal`, `codexSignal` and `categorize`. They
stay, as thin wrappers over the mapper and the reader, with their twins'
exports.

**What stays code, by design.** These read a tool's input or a line's
detail and stay with the reader that uses them: `AskUserQuestion` and
`ExitPlanMode` raising needs-you, `toolDetail`, `filesOf` and
`claudeEdits`, stats' spawn rule (`Agent` or `Task`), and `categorize`,
which is one case-insensitive table for every app (its agent rule is a
prefix). Per-app tool lists would change today's categories, so
`categorize` stays as it is.

**Other readers of transcripts** stay outside the stream, as session or
identity sources reading through the normalizer: the console's Codex
thread from its rollout (`readRolloutPage`), the Agent SDK's listing as
the binary reads it (`wire-map/src/sdk.rs`), and `identities.js`'s
transcript owner.

**Native readers** keep their session-level shape (section 4). One with
per-turn data could emit events later, which would give its sessions
stats and search; that isn't part of this change.

## 3. Shared types

In Rust, with the JS shapes they mirror:

- **`TokenUsage`** (`xenonet-base`): `input`, `cache_read`,
  `cache_write_5m`, `cache_write_1h`, `output`, and the call's speed
  (standard, fast, priority). It replaces `lines::Usage`,
  `pricing::Tokens` and `stats::Event.tokens`, and `telemetry::Call`
  wraps it as it wraps `Tokens` now.
- **`LineEvent`** (`xenonet-sources/src/line_events.rs`): the events of
  section 2. Named apart from `stats::Event`, which becomes a view of it.
- **Session records**: a typed builder per app that writes `session/v1`
  in that app's own key order, over the ordered map used now, so the
  output stays byte for byte. A new app's fields are declared in
  `session/v1`, in zod, as Claude's and Codex's are.
- **`LimitsReading`**: the reading as XenoNet keeps it (its source,
  `stale` and reason, when observed, identity and host) holding a list of
  windows (`label`, `used_pct`, `window_minutes`, `resets_at`).
  `live/snapshot/v1` still sends today's per-app shapes, built from it.
- **The patch parser**: one parser reads `apply_patch` text into the
  paths it adds, updates, deletes and moves to. Live, the engine and the
  native readers use it and keep their own records (live's
  `{ path, kind, diff }`, the engine's `{ ts, path }`). Live still leaves
  out `*** Move to:` until step 6.
- **One drift-reader pool**: `wire-map`'s `Vendor` is made once and
  handed to the actors' observers, which drop their own pool.

## 4. Capabilities

Code that differs per app sits behind a small interface. An entry lists
the ones it has (`has`), and code that needs one loops over those apps.
In Rust these are traits; in JS, objects with the same methods:

| Capability (`has`) | What it does | Today |
|---|---|---|
| `lines` | The sampling hooks (`kindOf`, `versionOf`), the mapper, which files are its transcripts and the session id of each (for each reader, as `blocks.md` records), and a session's files from its id, as stats needs | `lines.js`, `lines.rs`, `wire-map/src/vendor.rs`, `live.js` `classify`, `stats.js`, `search.js` |
| `sessions` | List sessions, rename one, read its transcript; for Claude Code through the Agent SDK and Claude desktop's files, for Codex through `codex app-server` (with its lifecycle and its state database's projects) | `claude.js`, `codex.js`, `wire-map/src/sdk.rs`, `wire-map/src/codex.rs` |
| `native` | `read`, `sessions`, `live`, `ids`, `status` | `core/native/*.js`; the `Reader` enum's five variants in `observed.rs` (four readers and numbat) |
| `connect` | Preview, apply and remove XenoNet's entries in one config format | `engine/connect.js`, `connect.rs` (`Kind::Claude`, `Kind::Codex`) |
| `delivery` | Resume a turn (with the environment it keeps), or reach a running session | `delivery/resume.js`, `channel.js`, `opencode.js`, `router.js` |
| `processes` | Tell its processes apart (names, bundled binaries, `node` or `bun` running its package) | `procs.js` `agentOf`, `engine/verify.js` |
| `caller` | Find the calling session in an MCP request: Claude Code's `CLAUDE_CODE_SESSION_ID`; Codex's `threadId`, `x-codex-turn-metadata` or `CODEX_THREAD_ID`; channel support | `mcp/xenonet.js` |
| `open` | The URLs that open a session or project in the app | `open.js` |
| identity, limits | The account a session runs under, and its plan limits (named by the entry's `identity` and `limits`) | `identities.js`, `claude-usage.js`, `live.js` |

Hook payloads stay as the engine reads them: one set of fields Claude
Code and Codex share (`engine/engine.js`), with no vendor schema, as
`schemas.md` notes.

## Hosts on different versions

A host's map merges other hosts' data and checks it against its own
schemas (`core/hosts/merge.js`): nodes, live sessions, activity and
agents, each copied by what its schema declares. An app a newer host
knows and an older one doesn't is dropped there, silently, as an app
outside today's enums is now. The rule stays:

- registry changes are additive;
- an older host drops what it doesn't know;
- nothing it does know changes shape.

The places that assume two apps (`readingOf`, `addSeries`, the limits,
the stats counts) take the list from the registry.

## A new app, before and after

| | Today | With this design |
|---|---|---|
| Read by an experimental native reader, like Gemini CLI | About 20 files: `SESSION_APPS` twice, the schema in zod and its file, `vendor/index.js`, `drift.js`, `drift.rs`, provider drift, the reader in JS and Rust, the twin, `parity.json`, `config.js`, `state.js`, `observed.rs`, fixtures and tests | A registry entry, its product in provider drift, the schema in zod and its file, the reader in JS and Rust with its twin and `parity.json` entry, fixtures and tests |
| Tailed and connected, as Claude Code and Codex are | More than 50 files, and every list of the two apps | The above, with its `lines` capability in place of a reader; `connect`, `sessions`, `delivery`, `processes` and `caller` only if it has them; its `session/v1` fields; demo transcripts; and on the page, its colour (a theme token, a `theme/v1` change) |

While the JS core is the test contract, a mapper or reader is written in
JS and in Rust. Desktop runs only the binary, so the JS ones exist for the
tests. The registry, the schemas and everything that reads them are
written once.

## The order of work

Each step is its own PR. Steps 1 to 5 leave output unchanged:

1. **The registry.** `core/apps.js`, the generated Rust copy and its
   check, `app/v1` in the contracts, `SESSION_APPS` from it on both
   sides, the generated lists, and the fallbacks as lookups with the
   same results.
2. **The shared types.** `TokenUsage`, the patch parser, session record
   builders, and one drift-reader pool. The pool adds the observers' read
   counts to the drift report, which only adds to it.
3. **The event stream, one reader at a time.** Stats first, since its
   events are the model; then search, refusals and live. The golden files
   stay byte for byte at each step.
4. **The native readers** behind `native`, with `observed.rs`'s table and
   `state.js`'s reader list from the registry.
5. **Connect, delivery, processes, caller and open** behind their
   capabilities.
6. **Changes to output, each for the owner to decide**, with its golden
   files:
   - Codex's live token count, which adds `reasoning_output_tokens` to
     `output_tokens`. XenoNet's pricing takes reasoning to be part of
     output (`core/pricing.js`), but `blocks.md` records live keeping
     both as its own way, so this is a decision to revisit, not a bug fix
     on its own say.
   - `*** Move to:` in live.
   - OpenCode ids that don't round-trip with numbat.
   - Refusing an app the registry doesn't have, instead of falling back.
   - What `engine/verify.js` does with a calling app that has no
     `processes` capability. Today a caller outside Claude Code, Codex and
     OpenCode isn't process-checked, and its messages are marked
     unchecked. Refusing such callers would keep a new app from adding to
     that.

## Testing

- Each of steps 1 to 5 passes the golden tests (`blocks.md`, "The golden
  tests"), `npm test`, `npm run test:rust`, `npm run test:bin` and
  `npm run test:e2e` unchanged; `core/apps.js` reaches the page.
- `app/v1` in the contracts test, and the registry checked against it.
- `scripts/apps.mjs --check`: the Rust copy is current.
- Every product an entry names is in provider drift, and every source in
  `drift.js` and `drift.rs` belongs to one.
- A made-up app, in a test registry with a small line format and its own
  transcripts in a temporary home, is tailed, counted and searched with
  no change to live, stats or search.
- The fuzz targets (`fuzz/`) cover each mapper as they cover the line
  readers now.

## Not decided yet

- Whether plan limits go out as one list in a `live/snapshot/v2`, or keep
  today's per-app shapes.
- Whether the JS tests should read Rust's mappers through the addon, so
  each mapper is written once, before the JS core is retired.
- Whether native readers emit events, which would give their sessions
  stats and search.
- Whether the page's per-app colours become one token per app in
  `theme/v1`, or stay `--claude` and `--codex` with a shared colour for
  the rest.
