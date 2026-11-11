# Schemas

XenoNet's own data, and the vendor data it reads, as versioned JSON Schemas.
[contracts.md](contracts.md), section 1, fixes the ids and the version
rules. The definitions are Rust types in `crates/xenonet-model`, and the
files are written from them.

## Layout

| Where | What |
|---|---|
| `crates/xenonet-model/src/` | The definitions, the source of truth: plain Rust types with serde and schemars, one module per area (`actor.rs`, `network.rs`, `live.rs`, `graph.rs`…), with no IO and no vendor in them |
| `schemas/<path>/v<major>.json` | XenoNet's own JSON Schema files, written from the types and committed. The path is the `$id`'s |
| `schemas/vendor/<vendor>/<product>/…` | The vendor schemas, hand-kept JSON Schema (see [Vendor data](#vendor-data)) |
| `crates/xenonet-model/tests/schemas.rs` | Every file is what its type writes, none is stale and none is left over. `XENONET_SCHEMAS_WRITE=1 cargo test -p xenonet-model` writes the missing and stale ones |
| `crates/xenonet-model/src/ids.rs` | Typed ids: `$id`s and short ids (`SchemaId`), host-qualified ids (`parse_qualified`, `qualify`), sessions (`SessionId`), actors (`ActorId`, `YOU`), repos (`RepoId`), and the store's ids (`ProjectId`, `CheckoutId`, `AgentId`, `IntentId`, `SpaceId`) |

In `crates/xenonet-model/src/schema.rs`:

| Item | What |
|---|---|
| `Document` | A type that is a schema document: its `ID` (`engine/claim/v1`), `TITLE` and `DESCRIPTION` |
| `Ref<T>` | A field whose schema refers to another document by its `$id`, and whose value is that document's type |
| `Opt<T>` | A field that may be left out and is never null when it's there (`Option<T>` is nullable) |
| `Record<V, K>` | A record: `propertyNames` from the key's schema, `additionalProperties` from the value's |
| `tag!` | A unit type that is always one string (a document's `schema` field) |
| `Shapes` | The transform that writes the shapes the files have always had: a loose object says `"additionalProperties": {}`, numbers carry no `format`, subschemas no `title` |
| `documents()` | Every document's file, by id |

The files are read back by `crates/xenonet-schema` (the lenient vendor
readers) and the page's tests (`test/harness/schema.js`).

## The internal schemas

Each is modeled on what the code produces, not on what it should.
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
| `engine/message/v1`, `engine/claim/v1`, `engine/contact/v1` | The engine's records, as the store keeps them and as the API and events send them | Each is a union of its forms |
| `doctor/result/v1`, `doctor/run/v1` | The doctor's checks and runs | |
| `theme/v1` | A theme file, or a theme resolved | Every field is optional, since a theme names only what it changes. The new tokens' values are open |
| `state/v1`, `state/v2` | What XenoNet saves for you, in the shape the store's state core answers it (`state/v2`) | v1's parts are all optional |
| `actor/*/v1`, `drift/record/v1` | As the contracts define them | The actors' enums are open |
| `drift/log/v1` | drift.json: when you last looked, and the `drift/record/v1`s kept | |
| `app/v1` | An entry of the app registry (`src/shared/apps.js`, embedded in `crates/xenonet-adapter` as `apps.json`): an app XenoNet knows sessions of, and what's fixed about it. `test/apps.test.js` checks every entry | The apps XenoNet only observes have their id, name and numbat's names only |

## Versions

- A schema's `$id` carries its major version: a `Document` whose `ID` is
  `state/v2` is the file `https://xenonet.dev/schemas/state/v2.json`.
- Additive changes keep the major: a new optional field, a new value of
  an enum, a new variant of a union. Change the schema file in place.
- Anything else makes a new major, defined beside the old one: removing,
  renaming or retyping a field, making an optional field required, or
  taking a value away. XenoNet keeps no compatibility with data an
  earlier build stored ([direction.md](direction.md), "Order of work").
- Every object is loose: unknown keys pass, and the files allow them. So
  a reader checking against v1 isn't broken by a field a later v1 adds.
- Enums in the internal schemas list the values XenoNet produces, so a test
  catches a value it didn't mean to. The actors' and the theme tokens'
  are open sets (`Open<T>`): any string passes, since a client draws a
  value it doesn't know as the default.
- What XenoNet writes says what it is: doctor.json, drift records, actor
  payloads and the network's documents by a `"schema"` field with the
  short id. API responses keep their shape, which the golden files pin.

### Stored data

| Store | Version | What happens to another |
|---|---|---|
| `xenonet.sqlite` (`crates/xenonet-store`) | `PRAGMA user_version`, the hash of the store's schema | No migrations: a database of another version, or an `engine.sqlite` from before the store, is left on disk as it is and refused, with why. The machine is set up again |
| A versioned JSON file (`xenonet-blocks`' `read_versioned`, [blocks.md](blocks.md)) | `"version"` | A newer version is read and never written back; one that isn't JSON or an object is moved aside to `<name>.broken-<time>.json` |
| `drift.json` | `drift/log/v1` | Read as below |

## Adding a schema

1. A new internal schema goes in the contracts first
   ([contracts.md](contracts.md), section 1). A vendor schema goes under `schemas/vendor/`, made as
   [Vendor data](#vendor-data) says.
2. Define it as a type in its area's module of `crates/xenonet-model`,
   deriving `Serialize`, `Deserialize` and `JsonSchema`, and implement
   `Document` for it:

   ```rust
   /// What it is, and where XenoNet sends or keeps it.
   #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
   pub struct Thing {
       pub id: String,
       pub at: Ms,
       pub note: Option<String>,
   }

   impl Document for Thing {
       const ID: &'static str = "area/thing/v1";
       const TITLE: &'static str = "A thing";
       const DESCRIPTION: &'static str = "What it is, and where XenoNet sends or keeps it.";
   }
   ```

   - Objects are loose unless the type says `#[serde(deny_unknown_fields)]`.
   - A field that may be left out is `Opt<T>`; a nullable one is `Option<T>`
     (with `#[schemars(required)]` when it must be there).
   - Another document inside this one is `Ref<Other>`, and the file refers
     to it with a `$ref`, rather than copying it.
   - A doc comment on a field or type is its description: write one where
     a reader of the file needs it.
   - A string's pattern or length, a number's range: schemars's `regex`,
     `length` and `range` attributes.
3. Add `schema::file::<Thing>` to the module's `DOCUMENTS` list.
4. Write the file: `XENONET_SCHEMAS_WRITE=1 cargo test -p xenonet-model`,
   and commit it with the type.
5. Check what XenoNet really produces against it (the route and golden
   tests do, through `xenonet_schema::typed`).

## Checking data

- `xenonet_schema::typed::read` reads a JSON value as one of the model's
  types: the type is the check. What fails is the first problem serde
  meets, with the path to it (`events.0.kind`) and serde's own words.
- The page's tests check what they read against the files
  (`test/harness/schema.js`).
- Vendor data is checked against the files under `schemas/vendor/` by the
  lenient readers (see [Vendor data](#vendor-data)).

## Vendor data

What XenoNet reads of other apps' data has a schema too, so a vendor's
change to a format shows as drift, by source, before a reader quietly
stops seeing something. [contracts.md](contracts.md), section 1, fixes
the sources, the drift record and the rules; this is how they're built.

### The sources

| Path (`vendor/…/v1`) | What | Read by | Version, and where it's found | Range |
|---|---|---|---|---|
| `anthropic/claude-code/transcript-line` | A line of a Claude Code transcript (`~/.claude/projects/**.jsonl`, subagents' included) | The live tailer, stats and search, through `adapters/claude-code`'s line format | Claude Code's, on each line (`version`); a line without one has its file's last | `>=1.0.0 <3.0.0` |
| `anthropic/claude-code/config` | `~/.claude.json`'s `oauthAccount`, and XenoNet's own `mcpServers` entry | `adapters/claude-code` (the account) | None in the file | `*` |
| `anthropic/claude-desktop/session` | Claude desktop's `claude-code-sessions/**/local_*.json` | `adapters/claude-code/src/reader/listing.rs` | None in the files | `*` |
| `anthropic/claude-desktop/plan-usage` | Claude desktop's `plan-usage-history.json` | Plan usage (`xenonet-state`'s live runtime) | The file's own `version` | `2` |
| `anthropic/agent-sdk/session-listing` | What `listSessions()` returns | `adapters/claude-code/src/reader/listing.rs` | The installed SDK's (its `package.json`) | `>=0.3.0 <0.4.0` |
| `anthropic/agent-sdk/session-message` | What `getSessionMessages()` returns | `adapters/claude-code/src/reader/listing.rs` | The installed SDK's | `>=0.3.0 <0.4.0` |
| `anthropic/oauth/usage` | Anthropic's `/api/oauth/usage` answer | Plan usage (`xenonet-sources`' `claude_usage`) | None in the answer (the request pins `anthropic-beta: oauth-2025-04-20`) | `*` |
| `openai/codex/rollout-line` | A line of a Codex rollout (`~/.codex/sessions/**`, and archived ones) | The live tailer, stats and search, through `adapters/codex`'s line format; a thread whose items the app-server can't list | Codex's, from the rollout's first line (`session_meta`'s `cli_version`), kept per file | `<1.0.0` |
| `openai/codex/app-server` | The `codex app-server` results XenoNet reads, by method: `initialize`, `thread/list`, `thread/items/list` | `adapters/codex/src/reader/app_server.rs` | Codex's, from `initialize`'s `userAgent` (`<client>/<version> (…)`) | `<1.0.0` |
| `openai/codex/state-db` | The rows XenoNet queries in `state_5.sqlite`: `projects` (id, name) and `project_roots` (project_id, path) | `adapters/codex/src/reader/state_db.rs` | The file's number; a `state_<n>.sqlite` of another number is a new version | `5` |
| `openai/codex/auth-claims` | The claims XenoNet reads from `auth.json`'s ID token (never the tokens) | `adapters/codex` (the account) | None in the claims | `*` |
| `github/gh/hosts` | gh's `hosts.yml`, as a keys-only view of its YAML (`ghHostsShape`: every value is `""`, so no token or name is in it) | Identities (`xenonet-state`) | None in the file | `*` |
| `nous/hermes-agent/state-db` | What Hermes Agent's observer reads of `state.db`: sessions by named columns, messages' roles, tool names and times, the `path` argument of `write_file` and `patch` calls (taken out by SQLite's `json_extract`), and the backends' heartbeats | `adapters/hermes` (`files.rs`) | The database's own `schema_version` | `<=31` |
| `nous/hermes-agent/spawn-ledger` | `spawn-ledger.json`: Hermes's long-lived processes, never their `argv` | `adapters/hermes` (`files.rs`) | Hermes's, from its build stamp's `baseVersion` | `<1.0.0` |
| `nous/hermes-agent/gateway-state` | `gateway_state.json`: the gateway's state, heartbeat and platforms' states | `adapters/hermes` (`files.rs`) | Hermes's | `<1.0.0` |
| `nous/hermes-agent/processes` | `processes.json`: the background processes Hermes runs, never their commands | `adapters/hermes` (`files.rs`) | Hermes's | `<1.0.0` |
| `nous/hermes-agent/cron-jobs` | `cron/jobs.json`: when each job runs, how its last run went, and its folder; never names or prompts | `adapters/hermes` (`files.rs`) | Hermes's | `<1.0.0` |
| `nous/hermes-agent/cron-ticker` | `cron/ticker_heartbeat` and `ticker_last_success`, one line each, read into a shape | `adapters/hermes` (`files.rs`) | Hermes's | `<1.0.0` |
| `nous/hermes-agent/install-stamp` | `hermes-agent/install-stamp.json`, Hermes's build stamp | `adapters/hermes` (`files.rs`) | Its own `schemaVersion` | `<=2` |
| `anomaly/opencode/db` | `opencode.db`, OpenCode's session database (experimental reader) | `adapters/opencode` | The newest session's own `version` | `*` |
| `google/gemini-cli/chat-record` | `~/.gemini/tmp/*/chats/session-*.jsonl`, Gemini CLI's chat records (experimental reader) | `adapters/gemini-cli` | None; the newest schema | `*` |
| `github/copilot-cli/session-event` | `~/.copilot/session-state/<id>/events.jsonl`, Copilot CLI's session events (experimental reader) | `adapters/copilot` | `session.start`'s `version` | `1` |
| `anysphere/cursor/hook-input` | The file `scripts/cursor/hook.mjs` writes from Cursor's hook input (experimental reader) | `adapters/cursor` | `cursor_version` | `*` |
| `spacexai/grok-build/session-summary` | `~/.grok/sessions/<encoded cwd>/<id>/summary.json`, a Grok Build session's index entry | `adapters/grok-build/src/reader/listing.rs` | None in the file | `*` |
| `spacexai/grok-build/update-line` | A line of a Grok Build session's `updates.jsonl`, a session update in its envelope | The live tailer, stats and search, through `adapters/grok-build`'s line format | None in the lines | `*` |
| `spacexai/grok-build/usage` | `~/.grok/sessions/<encoded cwd>/<id>/usage.json`, a Grok Build session's token and cost ledger | `adapters/grok-build`, for the live tailer and stats | None in the file | `*` |
| `spacexai/grok-build/subagent-meta` | `~/.grok/sessions/<encoded cwd>/<id>/subagents/<subagent id>/meta.json`, a subagent a Grok Build session ran | `adapters/grok-build/src/reader/listing.rs` | None in the file | `*` |

Not covered, since the contracts don't list them: the Remote Control
owner line the identities reader finds in a transcript's first 64 KB, the
settings files and `codex mcp get` that connecting reads, hook payloads,
the CLIs' version and sign-in outputs, git's files and output, project
manifests and context pages. Of Hermes's sources in
[actors/hermes.md](actors/hermes.md), the
observer doesn't read a backend's `/api/status`, which says what
`state.db` and `gateway_state.json` already do, or `tool_calls.log`,
which only a gateway writes, and only when it's switched on. Grok Bot's
observer reads processes only, so it has no vendor schema.

### Making a vendor schema

Vendor schemas are hand-kept JSON Schema files, with XenoNet's own
keywords under `x-xenonet`.

- It declares only the fields XenoNet's code reads, with their types. A
  field is required only when the vendor always writes it and XenoNet needs
  it (a line's `type` and `timestamp`); the rest are optional, and often
  nullable, since other apps' fields come and go.
- Objects are open (`"additionalProperties": {}`): keys the schema doesn't
  declare pass without a word, since a vendor record has dozens of keys
  XenoNet never reads and the set changes every release.
  `"x-xenonet": {"closed": true}` marks the small objects whose keys are
  all the format's, where a key it doesn't list is drift; `known` lists
  their keys XenoNet doesn't read. An object is closed only
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
- A discriminated union is a `oneOf` whose options each give the key a
  `const` (or name it with `"x-xenonet": {"discriminator": <key>}`): a
  value of the key the schema doesn't list is drift, and the only value a
  drift record keeps. Kinds XenoNet passes over are listed as options
  too, so they aren't drift.
- The root carries `"x-xenonet": { "vendorVersions": <range>, "lenient":
  true }`, and the description says where the version comes from.
- Schemas are made from XenoNet's code and the vendors' public formats
  (the Messages API, Codex's open protocol, the SDK's `sdk.d.ts`). Real
  data may be read for types, whether a field is always there, and
  versions, but no name or value from it goes in a schema, a fixture or
  a test: a transcript can hold unreleased features' names as well as
  people's work.
- The lenient readers' tests in `crates/xenonet-schema` use made-up
  records, ones that fit and odd ones with the drift each should show.

### Reading leniently

- At runtime the readers walk the JSON Schema files
  (`crates/xenonet-schema/src/lenient.rs`), which are what anyone else
  reads too; `package.json`'s `build.files` ships them.
- Each source has a reader (`VendorReader`, in
  `crates/xenonet-schema/src/vendor.rs`). It
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
  - transcripts: the 1st, 2nd, 4th, 8th… line of each
    kind in each file (Claude Code's by `type`; Codex's by `type` and
    payload type, and an item's type);
  - lists read again and again (sessions, desktop files): every 16th in
    turn;
  - single answers (the app-server's pages, plan usage, sign-ins): every
    one;
  - within a record, the first 16 elements of an array, then every 8th
    and the last, and 32 entries of a map.

  A type change found once in a kind of record is looked for in every
  record of that kind after it, along the path it was found at
  (`recheck`), which costs a few property reads.

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
  and more; `is_field` in `crates/xenonet-schema/src/fields.rs`).
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

Some made up, as `xenonet drift` would list them:

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
  share it (the service and `xenonet drift`): each adds the
  counts it saw since it last wrote, and `lookedAt` only moves forward.
- **The drift report** (`crates/xenonet-schema/src/drift.rs`) says, for each source: its schema and range, the
  records read (and checked whole), the versions they said, and the drift
  that's new since you last looked, changes to what XenoNet reads told apart
  from additions; with `all`, every record kept. It has the time it was
  made (`at`), and counts: `new`, the `changes` among them, and `known`.
- **`xenonet drift`** gives the report: from a running XenoNet, with what it has read since it
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
  under way looked before the mark. The page has no view of the report.
- **The doctor's `contracts.drift`** check looks at what's first been seen
  since you last looked: a change to what XenoNet reads (a field gone or of
  another type, a line that isn't JSON, a version the schema wasn't made
  for) is a warning; an addition (a new field or kind of record) can't
  break a reader, so it's a note, which sends no notification. It names
  the first two and says how to see the rest (`xenonet drift`).

### When the report shows drift

- Something XenoNet should read: declare it in the schema (optional, so
  older data still fits), and read it.
- Something XenoNet passes over: list it, a kind as an option of its
  union, a key of a closed object in `known`.
- A field gone or retyped for good: a new major of the schema with the
  vendor's new version range, beside the old one, so data from before
  still reads with the schema it was written for.
