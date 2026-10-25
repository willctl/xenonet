# Schemas

XenoNet's own data, and the vendor data it reads, as versioned JSON Schemas.
`contracts.md` section 1 fixes the ids and the version rules;
`adr/0001-schemas.md` says why the definitions are zod.

## Layout

| Where | What |
|---|---|
| `core/schema/` | The definitions, the source of truth, one module per area |
| `schemas/<path>/v<major>.json` | The JSON Schema files, written from the definitions and committed. The path is the `$id`'s |
| `test/schemas.test.js` | Every schema the contracts list is defined, their `$ref`s lead somewhere, and the files are current |
| `test/schema-output.test.js` | What XenoNet really produces, checked against the definitions and the files |
| `test/fixtures/schemas.js` | Made-up data for the edge cases the demo world doesn't reach, and cases that mustn't pass |

In `core/schema/`:

| Module | What |
|---|---|
| `ids.js` | `$id`s and short ids (`schemaId`, `parseSchemaId`), host-qualified ids (`parseQualified`, `qualify`, `formatQualified`), session ids (`SESSION_APPS`, from the app registry, `core/apps.js`), actor ids (`actorId`, `parseActorId`, `YOU`) and repo ids (`repoId`, `normalizeRemote`). It imports only `core/apps.js`, which has no imports, so the page can use it |
| `registry.js` | `define(path, major, schema, { title, description })`, `schemaFor(id)`, `schemaIds()`, `jsonSchemas()` |
| `common.js` | What schemas are made of: `obj` (a loose object), `ms`, `count`, `nstr`, `openEnum`, `hex`, `passthrough` |
| `check.js` | `check(idOrSchema, data, { strict })` and `explain()` |
| `files.js` | The files in `schemas/` against the definitions, and writing them |
| `index.js` | Imports every area, so every schema is registered. Import this to look one up by id |
| `app.js`, `session.js`, `project.js`, `identity.js`, `graph.js`, `live.js`, `engine.js`, `doctor.js`, `theme.js`, `state.js`, `actor.js`, `drift.js` | The internal schemas, by area |
| `vendor/<vendor>/<product>.js` | The vendor schemas, made of `vendor/common.js`'s `vendor()`, `open`, `closed`, `variants` and `skipped` (see [Vendor data](#vendor-data)) |
| `lenient.js` | Vendor data checked against the schema files in `schemas/vendor/`, with no zod at runtime: `compile`, `inspect`, `strip`, `recheck`, `loadVendor`, `vendorSources`. No zod import, so the server can load it |

Beside them, `core/vendor.js` has a reader per vendor source and the
transcript normalizers' hooks, and `core/drift.js` keeps the drift they
find, in `drift.json` in the cache folder.

`npm run schemas` writes the files, and removes those of schemas no longer
defined; `npm run schemas -- --check` only says whether they're current.

## The internal schemas

Each is modeled on what the code produces today, not on what it should.
Where the code is loose, the schema is too:

| Id | What | Loose where |
|---|---|---|
| `session/v1` | A session as the adapters list it | App-specific fields are all optional: the demo's sessions carry both apps' |
| `project/v1` | A project or group as the scan finds it, with the rule that applies | |
| `graph/v1` | The map, as `/api/graph` and the `graph` event send it. Its nodes refer to `session/v1` and `project/v1`, its identities to `identity/v1` | Context page fields are front matter: text, or a JSON list when they start with `[` |
| `identity/v1` | An account work happens under | |
| `live/session/v1`, `live/snapshot/v1` | Live state, as `/api/live` and the `live` event send it. Codex's rate limits are in XenoNet's shape: `limits.codex.primary` and `.secondary` as `{ pct, windowMinutes, resetsAt }`, times in ms | |
| `live/activity/v1` | One thing that happened, in the history and the `activity` event | |
| `engine/agent/v1` | An agent: the brief form, the full view, or the console's summary | `editing` is a list, or in the console's snapshot a count |
| `engine/message/v1`, `engine/claim/v1`, `engine/contact/v1` | The engine's records, as engine.sqlite keeps them and as the API and events send them | Each is a union of its forms |
| `doctor/result/v1`, `doctor/run/v1` | The doctor's checks and runs | |
| `theme/v1` | A theme file, or a theme resolved | Every field is optional, since a theme names only what it changes. The new tokens' values are open |
| `state/v1`, `state/v2` | state.json before versions, and now | v1's parts are all optional |
| `actor/*/v1`, `drift/record/v1` | As the contracts define them | The actors' enums are open |
| `drift/log/v1` | drift.json: when you last looked, and the `drift/record/v1`s kept | |
| `app/v1` | An entry of the app registry (`core/apps.js`): an app XenoNet knows sessions of, and what's fixed about it. `test/apps.test.js` checks every entry | The apps XenoNet only observes have their id, name and numbat's names only |

## Versions

- A schema's `$id` carries its major version: `define("state", 2, …)` is
  `state/v2`, at `https://xenonet.dev/schemas/state/v2.json`.
- Additive changes keep the major: a new optional field, a new value of
  an enum, a new variant of a union. Change the definition in place and
  run `npm run schemas`.
- Anything else makes a new major, defined beside the old one: removing,
  renaming or retyping a field, making an optional field required, or
  taking a value away. Data XenoNet has stored in the old shape gets a
  migration.
- Every object is loose: unknown keys pass, and the files allow them. So
  a reader checking against v1 isn't broken by a field a later v1 adds.
- Enums in the internal schemas list the values XenoNet produces, so a test
  catches a value it didn't mean to. The actors' and the theme tokens'
  are open sets (`openEnum`): any string passes, since a client draws a
  value it doesn't know as the default.
- What XenoNet writes says what it is: state.json by its `version`;
  doctor.json, drift records and actor payloads by a `"schema"` field
  with the short id. API responses keep their shape, and tests check them
  against the schemas instead.

### Stored data and its migrations

| Store | Version | Migrations |
|---|---|---|
| state.json | `"version"`, none for v1 | `STATE_MIGRATIONS` in `core/statefile.js`: pure functions, run in order by `core/migrate.js`. The first load that migrates keeps the file as it was in `state.v<n>.json`, once the migrated document is prepared. A file that isn't JSON (or an object) is moved aside to `state.broken-<time>.json`; one from a newer XenoNet is read and never written back. A part that isn't an object (a hand edit that left `"identities": "x"`) starts afresh and the rest is kept. The doctor's `xenonet.state` check reports each |
| engine.sqlite | `PRAGMA user_version` | `ENGINE_MIGRATIONS` in `core/engine/store.js`, each step in a transaction with the version it reaches. Add a step at the end; never change one that has shipped. Before the first step, a database with data in it is copied beside it (`engine.v<n>.sqlite`, by `VACUUM INTO`), and copies a week old are deleted as it opens and with the engine's hourly prune, since the copy holds what the database keeps only a week or a day |

A migration takes a document from one version to the next and nothing
else: no reading files, no clock. Test it on made-up files of the old
version (`test/statefile.test.js`, `test/engine-store.test.js`).

## Adding a schema

1. A new internal schema goes in the contracts first (`contracts.md`
   section 1): the test fails for one it doesn't list. A vendor schema
   goes under `vendor/`, made as [Vendor data](#vendor-data) says.
2. Define it in its area's module, and use what `define` returns:

   ```js
   export const Thing = define("area/thing", 1, obj({ id: z.string(), at: ms, note: nstr }), {
     title: "A thing",
     description: "What it is, and where XenoNet sends or keeps it.",
   });
   ```

   - Objects are `obj(...)`, so they're loose.
   - No transforms, refinements or defaults: they don't survive the trip
     to JSON Schema. Check the data, then convert it in code.
   - Use a registered schema inside another (`z.array(Thing)`) and the
     file refers to it with a `$ref`, rather than copying it.
   - Describe what isn't obvious with `.describe()`.
   - An object holding another app's data as it came is
     `passthrough(obj(...))`, so strict checks don't count its keys.
3. Import a new module from `index.js`.
4. Run `npm run schemas` and commit the file with the definition.
5. Check what XenoNet really produces against it, strictly
   (`valid(id, data)` in `test/schema-output.test.js`), and add made-up
   cases the real data doesn't reach to `test/fixtures/schemas.js`,
   including some that mustn't pass.

## Checking data

- `check(id, data)` gives zod's issues as paths and messages. For a union
  that fits no option, it reports the option that came closest.
- `check(id, data, { strict: true })` also lists the keys the data has
  that its schema doesn't declare. Tests check strictly, so the schemas
  stay complete.
- zod costs about 30 ms to load, so nothing on the server's way up
  imports `core/schema/index.js`: the doctor imports it when it checks
  state.json. Vendor data is checked against the schema files instead,
  by `lenient.js`, which doesn't load zod (see [Vendor data](#vendor-data)).

## Vendor data

What XenoNet reads of other apps' data has a schema too, so a vendor's
change to a format shows as drift, by source, before a reader quietly
stops seeing something. `contracts.md` section 1 fixes the sources, the
drift record and the rules; this is how they're built.

### The sources

| Path (`vendor/…/v1`) | What | Read by | Version, and where it's found | Range |
|---|---|---|---|---|
| `anthropic/claude-code/transcript-line` | A line of a Claude Code transcript (`~/.claude/projects/**.jsonl`, subagents' included) | `live.js`, `stats.js` and `search.js`, through `lines.js`'s normalizer | Claude Code's, on each line (`version`); a line without one has its file's last | `>=1.0.0 <3.0.0` |
| `anthropic/claude-code/config` | `~/.claude.json`'s `oauthAccount`, and XenoNet's own `mcpServers` entry | `identities.js` (the account) | None in the file | `*` |
| `anthropic/claude-desktop/session` | Claude desktop's `claude-code-sessions/**/local_*.json` | `claude.js` | None in the files | `*` |
| `anthropic/claude-desktop/plan-usage` | Claude desktop's `plan-usage-history.json` | `claude-usage.js` | The file's own `version` | `2` |
| `anthropic/agent-sdk/session-listing` | What `listSessions()` returns | `claude.js` | The installed SDK's (its `package.json`) | `>=0.3.0 <0.4.0` |
| `anthropic/agent-sdk/session-message` | What `getSessionMessages()` returns | `claude.js` | The installed SDK's | `>=0.3.0 <0.4.0` |
| `anthropic/oauth/usage` | Anthropic's `/api/oauth/usage` answer | `claude-usage.js` | None in the answer (the request pins `anthropic-beta: oauth-2025-04-20`) | `*` |
| `openai/codex/rollout-line` | A line of a Codex rollout (`~/.codex/sessions/**`, and archived ones) | `live.js`, `stats.js` and `search.js`, through `lines.js`; `codex.js`, for a thread whose items the app-server can't list | Codex's, from the rollout's first line (`session_meta`'s `cli_version`), kept per file | `<1.0.0` |
| `openai/codex/app-server` | The `codex app-server` results XenoNet reads, by method: `initialize`, `thread/list`, `thread/items/list` | `codex.js` | Codex's, from `initialize`'s `userAgent` (`<client>/<version> (…)`) | `<1.0.0` |
| `openai/codex/state-db` | The rows XenoNet queries in `state_5.sqlite`: `projects` (id, name) and `project_roots` (project_id, path) | `codex.js` | The file's number; a `state_<n>.sqlite` of another number is a new version | `5` |
| `openai/codex/auth-claims` | The claims XenoNet reads from `auth.json`'s ID token (never the tokens) | `identities.js` | None in the claims | `*` |
| `github/gh/hosts` | gh's `hosts.yml`, as a keys-only view of its YAML (`ghHostsShape`: every value is `""`, so no token or name is in it) | `identities.js` | None in the file | `*` |
| `nous/hermes-agent/state-db` | What Hermes Agent's observer reads of `state.db`: sessions by named columns, messages' roles, tool names and times, the `path` argument of `write_file` and `patch` calls (taken out by SQLite's `json_extract`), and the backends' heartbeats | `core/actors/hermes-files.js` | The database's own `schema_version` | `<=31` |
| `nous/hermes-agent/spawn-ledger` | `spawn-ledger.json`: Hermes's long-lived processes, never their `argv` | `hermes-files.js` | Hermes's, from its build stamp's `baseVersion` | `<1.0.0` |
| `nous/hermes-agent/gateway-state` | `gateway_state.json`: the gateway's state, heartbeat and platforms' states | `hermes-files.js` | Hermes's | `<1.0.0` |
| `nous/hermes-agent/processes` | `processes.json`: the background processes Hermes runs, never their commands | `hermes-files.js` | Hermes's | `<1.0.0` |
| `nous/hermes-agent/cron-jobs` | `cron/jobs.json`: when each job runs, how its last run went, and its folder; never names or prompts | `hermes-files.js` | Hermes's | `<1.0.0` |
| `nous/hermes-agent/cron-ticker` | `cron/ticker_heartbeat` and `ticker_last_success`, one line each, read into a shape | `hermes-files.js` | Hermes's | `<1.0.0` |
| `nous/hermes-agent/install-stamp` | `hermes-agent/install-stamp.json`, Hermes's build stamp | `hermes-files.js` | Its own `schemaVersion` | `<=2` |
| `anomaly/opencode/db` | `opencode.db`, OpenCode's session database (experimental reader) | `native/opencode.js` | The newest session's own `version` | `<2.0.0` |
| `google/gemini-cli/chat-record` | `~/.gemini/tmp/*/chats/session-*.jsonl`, Gemini CLI's chat records (experimental reader) | `native/gemini-cli.js` | None; the newest schema | `*` |
| `github/copilot-cli/session-event` | `~/.copilot/session-state/<id>/events.jsonl`, Copilot CLI's session events (experimental reader) | `native/copilot-cli.js` | `session.start`'s `version` | `1` |
| `anysphere/cursor/hook-input` | The file `scripts/cursor/hook.mjs` writes from Cursor's hook input (experimental reader) | `native/cursor.js` | `cursor_version` | `*` |
| `spacexai/grok-build/session-summary` | `~/.grok/sessions/<encoded cwd>/<id>/summary.json`, a Grok Build session's index entry | `grok.js` | None in the file | `*` |
| `spacexai/grok-build/update-line` | A line of a Grok Build session's `updates.jsonl`, a session update in its envelope | `live.js`, `stats.js` and `search.js`, through `lines.js`'s normalizer | None in the lines | `*` |
| `spacexai/grok-build/usage` | `~/.grok/sessions/<encoded cwd>/<id>/usage.json`, a Grok Build session's token and cost ledger | `grok-usage.js`, for `live.js` and `stats.js` | None in the file | `*` |

Not covered, since the contracts don't list them: the Remote Control
owner line `identities.js` finds in a transcript's first 64 KB, the
settings files and `codex mcp get` that connecting reads, hook payloads,
the CLIs' version and sign-in outputs, git's files and output, project
manifests and context pages. Of Hermes's sources in `hermes.md`, the
observer doesn't read a backend's `/api/status`, which says what
`state.db` and `gateway_state.json` already do, or `tool_calls.log`,
which only a gateway writes, and only when it's switched on. Grok Bot's
observer reads processes only, so it has no vendor schema.

### Making a vendor schema

- It declares only the fields XenoNet's code reads, with their types. A
  field is required only when the vendor always writes it and XenoNet needs
  it (a line's `type` and `timestamp`); the rest are optional, and often
  nullable (`maybe()`), since other apps' fields come and go.
- Objects are `open`: keys the schema doesn't declare pass without a
  word, since a vendor record has dozens of keys XenoNet never reads and
  the set changes every release. `closed` marks the small objects whose
  keys are all the format's, where a key it doesn't list is drift;
  `known` lists their keys XenoNet doesn't read. An object is closed only
  where the vendor's own types give every key, and a closed object lists
  them all:
  - Claude Code's and the Agent SDK's messages, token usage and content
    blocks (a reply's text and tool calls, and the text and tool results
    you send), from the Anthropic SDK's types (`BetaMessage`,
    `BetaUsage` and the blocks, in `@anthropic-ai/sdk`);
  - the SDK's session listing and messages, from its `sdk.d.ts`;
  - in Codex's rollouts, token counts, a rate limit window, a file's
    change in a FileChange item, a web search's action, and tool calls
    and their outputs in response items; in its app-server's answers, a
    thread's git info and a file change; from the protocol's types
    (`codex app-server generate-ts` writes them);
  - Claude desktop's plan usage samples, and Anthropic's usage windows.

  Before closing one, check that recent data has no key outside it (count
  keys only, never values); where it does, the object stays open, since a
  key name from someone's data never goes in a schema.
- A discriminated union is `variants(key, [...])`: a value of `key` the
  schema doesn't list is drift, and the only value a drift record keeps.
  Kinds XenoNet passes over are listed with `skipped(key, ...values)`.
- `vendor(path, major, schema, { title, description, versions })` adds
  `"x-xenonet": { "vendorVersions": <range>, "lenient": true }`, and the
  description says where the version comes from.
- Schemas are made from XenoNet's code and the vendors' public formats
  (the Messages API, Codex's open protocol, the SDK's `sdk.d.ts`). Real
  data may be read for types, whether a field is always there, and
  versions, but no name or value from it goes in a schema, a fixture or
  a test: a transcript can hold unreleased features' names as well as
  people's work.
- Made-up records for each source, ones that fit and odd ones with the
  drift each should show, are in `test/fixtures/vendor.js`;
  `test/vendor-schemas.test.js` checks zod and the files agree on them.

### Reading leniently

- At runtime the readers walk the JSON Schema files (`lenient.js`), not
  zod: zod costs about 30 ms to load, and the files are what anyone
  else reads. A unit test keeps the files in step with the definitions,
  and `package.json`'s `build.files` ships them.
- Each source has a reader (`reader(source)` in `core/vendor.js`). It
  picks the schema for the version the data carries: the newest major
  whose range has it; for a version none has, the newest, and the version
  is drift (`new-version`); for data that says none, the newest. A
  prerelease counts as the version it leads to (`0.155.0-alpha` is in
  `<1.0.0`).
- A whole check finds every difference, keeps each as drift, and gives
  back the record to use: the same, or a copy without the fields of
  another type, so a reader falls back the way it does for a missing
  field. A record is dropped only when it isn't an object at all (a
  transcript line that's `null`), since nothing in it can be read.
- Records come by the hundred thousand, so a sample is checked whole and
  the rest guarded:
  - transcripts (`transcriptHooks`): the 1st, 2nd, 4th, 8th… line of each
    kind in each file (Claude Code's by `type`; Codex's by `type` and
    payload type, and an item's type);
  - lists read again and again (sessions, desktop files): every 16th in
    turn (`sample`);
  - single answers (the app-server's pages, plan usage, sign-ins): every
    one (`check`);
  - within a record, the first 16 elements of an array, then every 8th
    and the last, and 32 entries of a map.

  A type change found once in a kind of record is looked for in every
  record of that kind after it, along the path it was found at
  (`recheck`), which costs a few property reads. On the benchmark's
  114,000 lines at start, the hooks cost about 0.06 µs a line, about 1.6%
  of tailing.

### Drift records

A `drift/record/v1` is a difference between vendor data and its schema,
kept as a shape and never a value:

| `kind` | When | `path` | `shape` |
|---|---|---|---|
| `new-field` | A key a closed object doesn't list | Where it is | Its value's shape |
| `missing-field` | A required field that isn't there; a column or table a query needs that's gone | Where it should be | null |
| `type-change` | A declared field of another type; a record that isn't an object (path null) | Where it is | What's there instead |
| `new-variant` | A discriminator's value the schema doesn't list | The discriminator's | The value, as a JSON string, when it's a short name; else its type |
| `new-version` | A version outside every range the source's schemas were made for | null | null |
| `unparsable` | A line, or an answer, that isn't JSON | null | null |

- **Paths** are dotted: `message.content[].citations` (`[]` for an array's
  elements), `payload.item.changes.*.unified_diff` (`*` for a map's
  entries), `["thread/list"].data[].id` for a declared key that isn't a
  plain name. A key that reads as data shows as `*`, even where the schema
  closes the object: a path, an email or anything with a slash, dot, `@`,
  colon or space; a run of four digits; a hex id anywhere in it (all hex,
  or seven hex characters in a row with a digit among them, such as a
  commit's short hash); or a token, by its prefix (GitHub's, GitLab's,
  `sk-`, Stripe's, Slack's, AWS's, Google's, npm's, PyPI's, Hugging Face's
  and more; `isField` in `core/drift.js`).
- **Shapes** are types: `string`, `number`, `object{type:string,url:string}`,
  `array<object{…}|string>`, at most four levels deep and about 400
  characters, key names counted. An object with a key that reads as data,
  or with more than 32 keys, is `record<…>` of its values' shapes, so no
  key that's data is kept either.
- **Inside a record, no key's name is kept.** Logins, repos and branches
  can read like field names, so an object keyed by data is declared a
  record in its schema (gh's hosts and logins, a file change's paths, MCP
  servers), and a new field or a type change anywhere inside one is shaped
  as `record<…>` of its types, as is the value of a key that reads as data
  in a closed object.
- **Variants**: drift inside a kind of record says which, as `variant`:
  the values of the discriminators the check went through to reach it,
  outermost first, such as `["event_msg", "item_completed",
  "UserMessage"]` in a Codex rollout or `["assistant", "text"]` for a text
  block in a Claude Code reply. The same path can be another field in
  another kind (`payload.message` in a `user_message` and in an
  `agent_message`), so it says where the path is. They're values the
  schema lists, never data, and a new variant is in the kind around it.
  It's left out where there's none (sources whose records aren't of
  kinds, a line's own `type`).
- The same drift seen again adds to one record: a new or missing field by
  its path and variant, a type change by its path, variant and the type
  found, a new variant by its value and the variant around it, a new
  version by the version, lines that aren't JSON by source.
  `vendorVersion` is the version it was first seen in, kept only when it
  reads as one (`2.1.288`, `0.155.0-alpha.9.2`, Python's `0.22.0rc1` and
  `0.22.0.dev0`, `31`: numbers, the usual tags and commit hashes), since
  anything else there, a branch's name or a person's, is data; `count` is
  how many records checked showed it (a sample, in the tailer). A record kept
  before records said their variant is taken over by the first one in a
  variant with the same path, with its history, so drift you've looked at
  isn't new again for saying where it is.

Some made up, as `npm run contracts` would list them:

| Source | Record |
|---|---|
| Claude Code's transcripts | `new-field message.content[].footnotes array<object{type:string,url:string}>`, in `assistant › text`, since 2.1.290 |
| Claude Code's transcripts | `new-variant type "progress"` |
| Codex's rollouts | `type-change payload.rate_limits.primary.resets_at string`, in `event_msg › token_count` |
| Codex's rollouts | `missing-field payload.message`, in `event_msg › agent_message` |
| Codex's app-server | `new-version 1.0.0` |
| Codex's state database | `missing-field projects[].name` |

### Where drift is kept, and seen

- **`drift.json`** in the cache folder, a `drift/log/v1`: `{ "version": 1,
  "lookedAt": <ms or null>, "records": [<drift/record/v1>, …] }`, at most
  500 records, the ones seen least lately dropped first. The server
  writes it a moment after it changes and at shutdown. Several processes
  share it (the server, `xenonet drift`, `npm run contracts`): each adds the
  counts it saw since it last wrote, and `lookedAt` only moves forward.
- **The drift report** (`driftReport()` and `reportLines()` in
  `core/vendor.js`) says, for each source: its schema and range, the
  records read (and checked whole), the versions they said, and the drift
  that's new since you last looked, changes to what XenoNet reads told apart
  from additions; with `all`, every record kept. It has the time it was
  made (`at`), and counts: `new`, the `changes` among them, and `known`.
- **`xenonet drift`** gives the report without npm, for a XenoNet installed
  as an app: from a running XenoNet, with what it has read since it
  started, or with none running, from the `drift.json` of the XenoNet it's
  asked about (XenoNet Dev's with `--dev`), where only sources with drift
  show. `--all` and `--json` as below. It marks what it showed as seen,
  except with `--exit-code`, for scripts: that exits 1 when there's drift
  new since you last looked and leaves it unmarked, so a script goes on
  exiting 1 until you look with plain `xenonet drift`.
- **`GET /api/drift`** is the report as a running XenoNet has it (`?all=1`
  for every record), for `xenonet drift`; **`POST /api/drift/seen {at}`**
  marks it seen up to `at`, never past now, so drift that turns up
  meanwhile stays new, and runs the doctor afresh, since a run already
  under way looked before the mark. The contracts let the console mark
  drift seen too, but the page has no view of the report yet.
- **`npm run contracts`** runs the live checks against your data, then
  gives the report with what this run read. `--all` lists every record
  kept, `--json` prints it all as JSON. It doesn't mark what it showed as
  seen: agents run it too, and the doctor's warning would go quiet before
  you'd seen it. What it finds goes into the `drift.json` of the XenoNet it
  runs as (XenoNet Dev's, from a checkout; `XENONET_INSTANCE` picks another),
  where `xenonet drift` and that XenoNet's doctor read it. It builds a search
  index of its own, in a temporary folder that it deletes after.
- **The provider drift check** (`npm run provider-drift`, and daily in
  GitHub Actions) runs the providers' newest releases, and the versions
  XenoNet pins, against made-up data in a made-up home, reads what they
  write through these readers, and compares their published types with
  these schemas. It keeps nothing in `drift.json`: its report is the run's
  artifact and a tracking issue, and it widens a range by pull request
  when a new version reads without a change ([Provider
  drift](../guide/provider-drift.md)).
- **The doctor's `contracts.drift`** check looks at what's first been seen
  since you last looked: a change to what XenoNet reads (a field gone or of
  another type, a line that isn't JSON, a version the schema wasn't made
  for) is a warning; an addition (a new field or kind of record) can't
  break a reader, so it's a note, which sends no notification. It names
  the first two and says how to see the rest (`xenonet drift`).

### When the report shows drift

- Something XenoNet should read: declare it in the schema (optional, so
  older data still fits), run `npm run schemas`, and read it.
- Something XenoNet passes over: list it, a kind with `skipped`, a key of a
  closed object in `known`.
- A field gone or retyped for good: a new major of the schema with the
  vendor's new version range, beside the old one, so data from before
  still reads with the schema it was written for.

In Wave 3 the report showed only additions, all things newer releases
write that XenoNet passes over: Claude Code's newer kinds of transcript
line, and Codex's newer kinds of rollout line, event and response item.
They're listed now as kinds the schemas pass over, so they aren't drift,
and Claude Code's newer fields of a message and its usage are among the
keys their closed objects know. Nothing closed was opened: a key or a
kind neither schema lists is still drift. Codex's `turn_aborted` became a
kind XenoNet reads, since a turn you stop ends there.
