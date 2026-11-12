# Contracts

What XenoNet's parts and the programs around it rely on staying the same:
schema ids and versions, host-qualified and stored ids, what an actor
looks like on the wire, the theme tokens, what XenoNet writes into other
programs, and the ids and documents of remote actors, the network and the
tray. The code and the golden files hold these
([development.md](../guide/development.md)); a change to one is a new
version beside the old, or a golden file changed on purpose, with the
reason in the commit.

## 1. Schemas

### Ids

- JSON Schema draft 2020-12. Each schema's `$id` is
  `https://xenonet.dev/schemas/<path>/v<major>.json`. The ids don't
  need to resolve.
- Inside data, the short form `<path>/v<major>` names a schema, for
  example `"schema": "actor/snapshot/v1"`.
- XenoNet's own schemas, by path: `session`, `project`, `graph`,
  `live/snapshot`, `live/session`, `live/activity`, `identity`,
  `engine/message`, `engine/claim`, `engine/contact`, `engine/agent`,
  `doctor/result`, `doctor/run`, `theme`, `state` (what XenoNet saves for
  you, in the shape the store's state core answers it), `actor/snapshot`,
  `actor/event`, `actor/list`, `actor/registration`,
  `actor/credentials`, `actor/gateway-credentials`, `actor/gateway-ask`,
  `actor/report`, `actor/me`, `actor/events`, `actor/console`,
  `actor/requests`, `actor/tokens`, `actor/connection`, `drift/record`,
  `drift/log`, `host/hello`, `app` (an entry of the app registry,
  `src/shared/apps.js`), `assistant/result` and `assistant/error`
  (section 6), the `network/…` documents (section 7), `tray/status` and
  `tray/event` (section 8), and `system`, a computer's load as
  `GET /api/system` answers it. `schemas/` has the full list.
- Vendor schemas, at `vendor/<vendor>/<product>/<source>`: Anthropic's
  Claude Code, Claude desktop, Agent SDK and OAuth usage; OpenAI's Codex;
  SpaceXAI's Grok Build; Anysphere's Cursor; Google's Gemini CLI;
  GitHub's Copilot CLI and `gh`; Anomaly's OpenCode; and Nous Research's
  Hermes Agent. [schemas.md](schemas.md) says how they're kept.

### Versions

- The `$id` carries the major version. Adding an optional field, a new
  value of an enum or a new variant keeps the major. Removing, renaming
  or retyping a field makes a new major. XenoNet keeps no compatibility
  with data an earlier build wrote: the store refuses a file of another
  schema, and the machine is set up again
  ([direction.md](direction.md), "Order of work").
- A vendor schema carries
  `"x-xenonet": {"vendorVersions": "<semver range>", "lenient": true}`.
  When a vendor changes a format, the schema gets a new major with its
  own range, and the reader picks by the version it detects (or tries
  the newest first when the data carries no version).
- What XenoNet writes says what it is: `doctor.json`, drift records,
  script files, actor payloads and the network's documents have a
  `"schema"` field. The answers of the HTTP API and the command are pinned
  by golden files (`crates/xenonet-cli/tests/golden/`).

### Layout

- `crates/xenonet-model/src/`: the definitions, the source of truth, one
  module per area, as Rust types with serde and schemars; `ids.rs` builds
  `$id`s and parses and qualifies ids (section 2). The vendor schemas
  are hand-kept JSON Schema under `schemas/vendor/`.
- `schemas/`: XenoNet's own JSON Schema files, written from the types,
  mirroring the `$id` paths, committed. `XENONET_SCHEMAS_WRITE=1 cargo test
  -p xenonet-model` writes them, and the same test fails when they're out
  of date.
- [schemas.md](schemas.md): the layout and the rules.

### Reading vendor data leniently

- Vendor records parse with loose objects: unknown keys pass through, and
  XenoNet reads only the fields its schema declares.
- A record that doesn't fully match is still used for the fields that do.
  It's dropped only when nothing XenoNet needs can be read.
- Every difference becomes a drift record (below): an unknown key, a
  declared field gone missing, a field with another type, an unknown
  value of a declared discriminator (such as a line's `type`), a vendor
  version XenoNet hasn't seen, or a line that isn't JSON.
- In hot paths (tailing), validation runs on a sample per line type and
  file, and shapes are cached.

### Drift records (`drift/record/v1`)

```json
{
  "schema": "drift/record/v1",
  "source": "anthropic/claude-code/transcript-line",
  "vendorVersion": "2.1.286",
  "kind": "new-field",
  "path": "message.content[].citations",
  "shape": "array<object{type:string,url:string}>",
  "firstSeen": 1791230000000,
  "lastSeen": 1791234567000,
  "count": 12
}
```

- `kind`: `new-field`, `missing-field`, `type-change`, `new-variant`,
  `new-version`, `unparsable`.
- `shape` holds types only: object keys and their value types, array
  element types, at most four levels deep. Never a value, except the
  values of fields a schema declares as discriminators (a line's `type`)
  and vendor version strings, kept only when they read as a version. An
  optional `variant` holds those discriminators' values around a drift,
  such as `["event_msg", "agent_message"]`. A key that reads as data (a
  path, an id, an email, a number, a token by its prefix, a hex id
  anywhere in it) is never kept, and nor is any key's name inside an
  object a schema declares as a record, keyed by data.
- One record per drift across vendor versions: `vendorVersion` is the
  version it was first seen in, so near-daily releases don't fill the
  log with repeats. An unknown key counts only in an object a schema
  closes.
- Additions (`new-field`, `new-variant`) are notes; changes
  (`missing-field`, `type-change`, `unparsable`, `new-version`) warn.
- Kept in the cache folder (`drift.json`, a `drift/log/v1`: `{version,
  lookedAt, records}`; at most 500 records, oldest dropped first,
  `crates/xenonet-schema/src/drift.rs`). The doctor warns about drift seen
  since you last looked. Only `xenonet drift` marks drift as looked at
  (`POST /api/drift/seen`), never the doctor. `xenonet drift --exit-code` exits 1 when there's new
  drift and leaves it unmarked, so a script goes on exiting 1 until you
  look with plain `xenonet drift` or the console.

## 2. Host-qualified and stored ids

### Host ids

- A machine's host id is the name the person gives it in a network
  (`xenonet net create --host <id>`, or Add a machine):
  `^[a-z][a-z0-9-]{1,31}$`, such as `studio` or `vm1`. It never defaults
  to the machine's own name, which often holds a person's name. The
  machine's keys, not its host id, are what the network's chain
  recognizes ([network.md](network.md)).
- An unqualified id means this XenoNet's own machine. One machine on its
  own works with unqualified ids throughout.
- A node from another machine is `@<host>:<local id>`:
  `@vm1:claude:4f1c0000-0000-4000-8000-000000000001`,
  `@vm1:project:~/projects/atlas`. To parse one: it starts with `@`, and
  the host runs to the first `:`. A machine turns its path keys into `~`
  before they leave it, so no home folder (a user name) crosses machines
  or lands in the console's layout keys.
- A repo merged across machines by its git remote is `repo:<repoKey>`,
  for example `repo:github.com/willctl/xenonet`. The `repoKey` is computed
  on the machine that has the checkout: SSH aliases resolved, `ssh://`,
  ports and user names dropped, then `normalize_remote`
  (`crates/xenonet-model/src/ids.rs`).
- Identities have no host: account ids are already global, so one Claude
  account signed in on two machines is one identity. The exception is
  `openai:api-key`, which belongs to its machine and is qualified. Plan
  limits in the live snapshot name the identity they belong to.
- Actors are `actor:<slug>` (`^[a-z0-9][a-z0-9-]{0,31}$`), or
  `@<host>:actor:<slug>` from another machine. The person at the console
  is `actor:you`.
- Another machine's map reaches a console only through the network
  (`crates/xenonet-runtime/src/remote/`). It sends titles, states and
  counts unless the network allows more, and transcripts, stats and
  diffs cross only when one is opened. A console keeps what a machine
  sends only as the schemas declare it (`remote/declared.rs`), so no
  field of a machine's own reaches the page. A demo console shows demo
  machines only.

### Stored ids

What XenoNet mints and keeps in its store: a prefix and the lowercase,
unpadded base32 of 20 bytes, 32 characters.

| Id | What | Made from |
|---|---|---|
| `prj_` | A project | The HMAC-SHA256 of its normalized git remote under the owner's salt, which their person chain holds, so one repo is one project on all their machines. A project with no remote gets one at random |
| `chk_` | A checkout: one folder on one machine | Random |
| `agt_` | An agent: a harness session as the store knows it, with the app's own id beside it | Random, the first time XenoNet sees the session |
| `int_` | An intent | The hash of the act that made it |
| `spc_` | A space of shared context | SHA-256 over `xenonet-space/1`, a zero byte and the hash of the space chain's first entry |
| `prs_` | A person | Their person chain |
| `xns1_` | A machine, by its signing key | The key |

A space says what it's about with an `EntityRef`: `prj_…`, `prs_…`,
`xns1_…`, `actor:<slug>@xns1_…`, `identity:<id>`, or `name:<kind>:<name>`
until it's bound to an id. Claims are `(prj_, relative path)`.

## 3. Actors

### The snapshot (`actor/snapshot/v1`)

`GET /api/actors` returns an `actor/list/v1`:
`{"schema": "actor/list/v1", "at": <ms>, "actors": [<snapshot>, ...]}`.

```json
{
  "schema": "actor/snapshot/v1",
  "id": "actor:wren",
  "kind": "agent",
  "name": "Wren",
  "product": { "id": "hermes-agent", "vendor": "Nous Research", "version": "0.21.5" },
  "host": null,
  "via": ["observed", "attributed"],
  "presence": "working",
  "presenceSince": 1791230000000,
  "lastSeen": 1791234567000,
  "heartbeatMs": 60000,
  "status": "Triaging issues in atlas-api",
  "capabilities": ["spawn-sessions", "edit-repos", "run-commands", "message-people", "schedule"],
  "targets": [
    { "id": "project:~/projects/atlas-api", "relation": "works-on", "since": 1791231000000, "lastAt": 1791234500000 },
    { "id": "claude:6c1f0000-0000-4000-8000-000000000001", "relation": "spawned", "since": 1791233000000, "lastAt": 1791234400000 }
  ],
  "recent": [],
  "counts": { "events24h": 42, "spawned24h": 3, "messages24h": 5 },
  "identity": null
}
```

| Field | Notes |
|---|---|
| `kind` | `agent` or `human` |
| `product` | What it is, or null for a custom actor. `id` is a slug such as `hermes-agent`, `grok-bot`, `muse` or `dots` |
| `host` | null on this machine, else the host id (and `id` is then qualified) |
| `via` | How XenoNet knows of it: `observed` (reads its state), `attributed` (traces work here back to it), `registered` (it reports itself), `remote` (it reads through a connection, section 6), `demo` |
| `presence` | `working`, `idle`, `away` (missed two heartbeats), `offline`, `unknown`. "Working for 90 seconds after an action" counts only a registered actor's own events. You go offline ten minutes after XenoNet's window is hidden |
| `status` | What it says or XenoNet infers it's doing, at most 140 characters, or null. A registered actor's `status` events may carry its own line here, as plain text |
| `needsYou` | `{kind, text, since}` when the actor waits on you (it reaches the alert strip), else null or absent. `kind` is an open set (`question`, `permission`, `review`); `text` is at most 140 characters, and "Waiting for you" stands in for a line that's refused |
| `capabilities` | What the product can do, as words for the map: any of `spawn-sessions`, `edit-repos`, `run-commands`, `message-people`, `message-agents`, `schedule`, `browse`, `use-computer`. What it may do here is its grant's, not these words ([authorization.md](authorization.md)) |
| `targets` | Graph node ids it's acting on (never actor ids; an event may still target an actor), each with a `relation`: `works-on`, `spawned`, `edited`, `messaged`, `watches`, `reviews` |
| `recent` | Its last events, newest last, at most 20 |
| `identity` | An identity id (`claude:…`, `chatgpt:…`, `github:…`) if XenoNet knows which account it acts as |
| `remote` | For an actor that reads through connections: its connections, which are active, the bridge, the account link, its permissions and the `transports` its active connections take. Left out otherwise |

Enums are open sets: a value a client doesn't know is drawn as the
default and isn't an error.

### Events (`actor/event/v1`)

```json
{ "schema": "actor/event/v1", "id": "wren:41", "actor": "actor:wren", "at": 1791234400000,
  "kind": "spawned", "target": "claude:6c1f0000-0000-4000-8000-000000000001",
  "detail": "Started a Claude Code session in atlas-api", "via": "attributed" }
```

- `kind`: `presence`, `status`, `spawned`, `edited`, `ran`, `messaged`,
  `said`, `finished`, `failed`.
- `target`: a graph node id, an actor id, or null.
- `detail`: at most 200 characters, written by XenoNet from facts (what,
  where). Never the text of a message or conversation, and never a secret.

### How the console gets them

- SSE `actors`: the list (as above) whenever it changes, at most every
  500 ms. SSE `actor-event`: each event, which drives beams and reactions.
  SSE `remote-read`: `{actor, transport, reach, at}`, a remote actor's
  read through one of its connections, at most one an actor a second:
  the route it took and up to 8 node ids it read (none for made-up
  data). The map draws it as light through the actor's tunnel; it is
  never kept and never in the feed. SSE `refusal`: `{id, at, provider,
  kind, category, session, context}`, a provider refusal XenoNet just kept
  ([refusals.md](../guide/refusals.md)); `live/activity` has the `refusal`
  kind. SSE `assistant`: `{at}`, only that a tunnel XenoNet runs changed;
  the window reads how with its key (`GET /api/assistant/tunnels`). SSE
  `actor-requests`: `{at, waiting}`, counts only, so who asked stays the
  window's to see.
- `GET /api/actors/you`: with the window's credential, your whole
  `actor/snapshot/v1` (with `?since=`, your `actor/events/v1`); without
  it, the public placeholder the list shows. `POST /api/actors/you` takes
  the window's reports and answers an `actor/me/v1` with `unread: 0`.
  Turning off "Show me on the map" deletes your record:
  `DELETE /api/actors/you` answers `{"deleted": "actor:you"}`.
  `POST /api/actors/you/message {to, text}` writes to an actor as you.
- The window's: `GET /api/actors/requests` answers an `actor/requests/v1`
  (each program waiting for an Allow: what it asks, who asked, when it
  lapses). `GET /api/actors/tokens` answers an `actor/tokens/v1`:
  `{"schema": "actor/tokens/v1", "at": <ms>, "tokens": [{actor, state,
  expiresAt, usedAt}]}` for each actor that reports itself, oldest first,
  where `state` is `works`, `expired` (30 days unused) or `none` (the
  demo's made-up actor), `expiresAt` is when it stops working unless it's
  used (null unless it works), and `usedAt` is when it was last used,
  exactly the first time and to within the hour after, or null for a
  token never used. Never a token. `GET /api/actors/observers` lists the
  observers and whether each is on.
- Driver (`src/driver.js`): `actors()`, `actorRequests()`, `actorTokens()`
  and `feed()` (the activity feed's lines, sessions' and actors'), and
  `select("actor:…")`; `find` matches actors as kind `actor`.
- Console tools (`crates/xenonet-mcp/src/console.rs`): an `actors` tool,
  which brings the `requests`, the `tokens` and the `tunnels` too, and a
  `feed` tool; `simulate` has `actor-spawn`, `actor-edit`,
  `actor-message`, `actor-online`, `actor-offline`, and in the demo and
  test instances only `actor-request`, `actor-needs-you`, `actor-tunnel`,
  `actor-tunnel-down` and `actor-read`.
- The demo world has `actor:you` and three made-up agents with scripted
  activity: a local agent that works on projects and spawns sessions, a
  cloud agent that works through an app on this machine, and a custom
  actor that registered itself.

### Registering

[actors/interface.md](actors/interface.md) has the full API. The
essentials:

- `POST /api/actors/register` with an `actor/registration/v1` body
  (`{name, slug?, product?, capabilities?, heartbeatMs?, pid?}`) answers
  `202` with an id and a token that works only once you Allow the actor in
  XenoNet's window; every new actor waits for that, even one you register
  from your own terminal. Allowing it takes your presence.
- The token is the actor's grant: a Biscuit rooted at this machine,
  written `xenonet_actor_` and base64url, the same kind for every actor,
  local, bot or cloud assistant. XenoNet keeps its revocation id and the
  scopes the receiver holds for it, never the token
  ([authorization.md](authorization.md)).
- Then `POST /api/actors/heartbeat {presence?, status?}`,
  `POST /api/actors/events {events}`, `POST /api/actors/targets {targets}`
  and `DELETE /api/actors/me`, each with `Authorization: Bearer <token>`.
  `GET /api/actors/me/stream` is its live stream of what's for it.
- The same calls as `actor_*` tools on the `xenonet` MCP server
  (`actor_heartbeat`, `actor_events`, `actor_targets`), and as
  `xenonet actor …` on the command line. Registering and unregistering
  stay yours: there's no tool for them.
- `actor:<slug>` is a valid agent id for the engine's messages and claims.
  A message is information: no actor or agent approves anything by what
  it says.
- A token unused for 30 days is revoked as `expired`. Over the bad-token
  limit, only that process's further bad tokens get 429 for a minute.
- An actor XenoNet knows only by observing it can't be removed, whoever
  asks: a 409 with the code `observed`, whose body also names its
  `product`. Its observer's switch takes it off the map. Nor can it be
  messaged, since it reads no inbox: `send` refuses it. Installed products
  stay listed however long they haven't run, as offline, except Grok Bot,
  whose Bots work in the cloud with its app closed: it's listed as
  unknown for as long as it's installed, and never offline.
- An observed actor's edited paths (Hermes's) count in agents' edit
  warnings and `who_has` for 30 minutes, like a registered actor's,
  marked as observed.

## 4. Theme tokens

All optional. A missing field takes XenoNet's value, so every theme file
stays valid, and `theme/v1` only grows.

```json
{
  "colors": { "actor": "#c9a0ff", "human": "#fff3d6" },
  "motion": { "pace": 1, "ease": "smooth", "breathe": 0.1 },
  "stars": { "shape": "point", "drift": [0, 0, 0], "twinkle": 0.5 },
  "grid": { "style": "polar" },
  "sweep": { "shape": "beam" },
  "sonar": { "style": "ring" },
  "lines": { "style": "solid", "curvature": 0, "width": 1 },
  "actors": {
    "form": "orb",
    "params": {},
    "palette": ["#5fe1ff", "#b18cff", "#ffd76a", "#ff7ad9", "#7dffb2", "#ff9d5c"],
    "human": { "form": "orb" },
    "glow": { "opacity": 0.5, "scale": 2.4 },
    "breathe": { "period": 4.5, "depth": 0.12 },
    "beam": { "style": "pulse", "width": 1.4, "speed": 1 },
    "react": { "flash": 1.6, "ripple": true }
  }
}
```

| Token | Values |
|---|---|
| `colors.grok` | Grok Build's sessions, as `colors.claude` and `colors.codex` are Claude Code's and Codex's |
| `motion.pace` | A multiplier for ambient motion (1 is XenoNet's own) |
| `motion.ease` | `smooth`, `stepped` (everything moves in ticks) |
| `motion.breathe` | How much nodes swell and fade at rest: 0.1 is XenoNet's bob (4.5 × breathe), 0 holds still |
| `stars.shape` | `point`, `flake`, `mote` |
| `stars.drift` | Units per second; `[0.6, -4, 0.3]` is falling snow at the map's scale |
| `grid.style` | `polar`, `hex`, `cells`, `none` |
| `sweep.shape` | `beam`, `aurora`, `flare`, `hand`, `ring`, `none` |
| `sweep.opacity` | A strength relative to XenoNet's beam (0.03), so a theme that only changes the shape still shows it |
| `sonar.style` | `ring`, `ripple`, `ping`, `tick` (drawn as the ring for now) |
| `sonar.every` | Seconds between the core's waves, never under one apart; 0 or less sends none |
| `lines.style` | `solid`, `dashed`, `arc`, `filament`, `seam`, `hairline` |
| `fonts.body` | The font for body text in the panels; `fonts.display` when left out. Every `fonts` value is a list of font names, quoted or not: one holding anything but letters, digits, spaces, quotes, commas and `. _ + & -` keeps the value underneath, since it could end the style it goes in |
| `identities` | Five ring colors for named identities; derived when left out |
| `actors.form` | `orb`, `cursor`, `crystal`, `flame`, `swarm`, `jelly` |
| `actors.params` | Knobs for the form (rings, cells, facets, tendrils), free-form numbers and booleans |
| `actors.glow` | The glow round each actor: its `opacity`, and its `scale` against the actor's size |
| `actors.beam.style` | `pulse`, `stream`, `bolt` |

- An unknown value falls back to the value of the theme being extended
  (XenoNet's, for a built-in), and new values can be added later without a
  new major.
- Each agent actor takes a color from `actors.palette` by a stable hash of
  its id; the person at the console takes `colors.human`. The page's CSS
  gets `--actor` and `--human`.
- The built-in themes keep the contrast rules in
  [themes.md](../guide/themes.md), "Contrast", which `test/themes.test.js`
  checks.

## 5. Integration points

What XenoNet writes into other programs outlives any one version of its
Desktop, Service and Tools, so these stay as they are, and a change to
one is a new version of it beside the old, never an edit:

- The launcher, at `<state>/runtime/launcher/xenonet` (`xenonet.exe` on
  Windows): the xenonet binary, a copy of the Tools' own, copied again
  through a temporary file and a rename when the Tools change. Hooks, MCP
  servers, the `xenonet` command on PATH, actors' scripts and OpenAI's
  tunnel-client all run this path, so it never moves. `--launcher` prints
  `{launcher, protocol, tools, node}`: launcher 3, with no node.
- The running Service, at
  `<state>/runtime/components/service/active/bin/xenonet`: a copy of the
  selected version's folder, which `active/.from` names
  (`<version>-<sha256>`), swapped in through `active.next` and
  `active.prev` with the service stopped whenever an install, update or
  rollback selects a version. Firewalls let a program in by this path, so
  it doesn't move between versions. When `active` isn't a copy of the
  selected version, the service runs from that version's own folder.
- The node launcher, `<state>/runtime/launcher/cli/xenonet.js` beside a
  node of its own, where an install of an earlier XenoNet put one: entries
  that install wrote run it. The binary carries the script
  (`crates/xenonet-components/src/launcher.cjs`); its first line says its
  version (`// xenonet-launcher <n> (protocol <p>)`), and XenoNet replaces
  it only with a newer one, through a temporary file and a rename. It
  passes `hook` on itself and runs the Tools' binary for anything else.
  The Tools carry no node, and XenoNet never puts one there itself.
  Connecting again moves those entries to the binary.
- Its argument forms: `hook <app>` (protocol 1: the event on stdin is
  posted to `/api/hook/<app>` on `XENONET_PORT`, or `--port`, and the
  answer printed; always exit 0, nothing on stderr), `mcp`,
  `mcp console`, `mcp assistant --grant <id> [--port <n>]`, `actor …`,
  `disconnect --yes --port <n>`.
- `XENONET_PORT` says which XenoNet; each instance has its own port
  (`crates/xenonet-components/src/instances.rs`). Hook routes are
  `/api/hook/claude`, `/api/hook/codex` and `/api/hook/grok`; a change to
  what one takes is a new route beside it.
- Entries in Claude Code's, Codex's and Grok Build's settings belong to
  the XenoNet whose port they name. Connecting or disconnecting one
  XenoNet changes its own and those whose files are gone, never another
  live XenoNet's. MCP servers are named by instance: `xenonet` and
  `xenonet-console` for the installed XenoNet, `xenonet-<instance>`
  (`xenonet-dev`) for the others. Unchanged entries are left byte for
  byte, which keeps Codex's trust in its hooks.
- Uninstalling with your data removed runs `disconnect` first, so no
  hook is left running a deleted launcher.

## 6. Remote actors

Remote actors, such as a cloud assistant, read through connections the
owner pairs, under the same actor grant as every other actor (design:
[cloud-assistants.md](cloud-assistants.md); guide:
[cloud-assistants.md](../guide/cloud-assistants.md)).

- Schemas: `assistant/result/v1` and `assistant/error/v1`, the envelopes
  of the tools of `xenonet mcp assistant`, whose answers carry
  `"schema": "xenonet.assistant.result.v1"` and
  `"xenonet.assistant.error.v1"`; and `actor/connection/v1`, the owner's
  view of a connection. Unlike every other schema they're strict
  (`additionalProperties: false`): data that leaves the machine is an
  allowlist, and a field the contract doesn't name is refused, in
  arguments and answers alike. A new field is a new version.
- Ids: a project as remote readers see it is the store's `prj_` (section
  2), and in made-up answers alone `fixture_<name>`, which no project of
  the store's can be; a grant `gr_<24 hex>`, a connection `cn_<24 hex>`
  and a request `r_<uuid>`.
  Sessions keep their `claude:`/`codex:` ids when they're UUIDs.
- `identity` stays null on the snapshot for an account link that's only
  the owner's word. Products have `remote` beside `registers`, and
  `dots-<suffix>` slugs are dots'.
- A message carries its envelope, `message/envelope/v1`
  (`crates/xenonet-api/src/attest.rs`): how XenoNet knew who sent it, from
  its own checks, never from the sender's words. The inbox and
  `engine/message/v1` carry it.
- The connections, grants, account links and audit are tables in the one
  store, `xenonet.sqlite` (`crates/xenonet-store`).

## 7. The network

One network across machines ([network.md](network.md); what's next is in
[direction.md](direction.md), phase 5).

- Schemas: `network/frame/v1`, a line of a link's stream, signed by the
  machine that sent it and, between machines, sealed for the one it's
  for. The engine's side is the bodies: `network/presence/v1`,
  `network/message/v3` (a kept or loose entry of the author's feed, with
  the act's authority), `network/receipt/v1`, `network/claim/v1`,
  `network/handoff/v1` and `network/operator/v2` (answers only). Then
  `network/roster-entry/v2`, one entry of the network's chain;
  `network/relay/v1`, `network/send/v1` and `network/svc/v2`; and
  `network/state/v1`, what `GET /api/net/state` answers the window.
- Ids: a network is `net_<32 hex>`, a frame `frm_<uuid>`, a message
  `msg_<uuid>`, a handoff `hof_<uuid>`, a machine's transport key id
  `xnk1_<32 base32>` and its signing key `xns1_<32 base32>`, a person
  `prs_<32 base32>` and a delegation `dlg_<hex>`. Remote agents keep the
  `@<host>:<id>` form, and are targets and senders, never callers.
- The engine reaches the network through `xenonet_model::ops` (`Net` and
  `Sink`). With no network nothing is behind it.

## 8. The tray app

XenoNet's own menu bar app on macOS (design: [always-on.md](always-on.md),
"The tray app").

- Schemas: `tray/status/v1`, what `GET /api/tray/v1/status` answers:
  that the service runs, its version, what needs you as the console's
  Needs you queue has it (each item's title and the words after it, such
  as "asks"), the actor gateway's state and this machine's link to its
  network. `tray/event/v1` is an alert on `GET /api/tray/v1/events`: an
  item that started (`needs-you`) or stopped (`resolved`) waiting on you.
  The stream also sends the whole status, as `status`, when it changes.
- `POST /api/tray/v1/open-console`, with an optional `id`, shows
  Desktop's window at that session or actor when a Desktop is connected,
  and says `{ "opened": null }` when none is, so the tray opens Desktop or
  the browser itself.
- The credential: the tray connects to the component socket
  (`runtime/service-<port>.json`) as role `tray`. The hello's answer
  carries `trayKey`, new at each start of the service, only when the
  engine's `verify` passes the tray (`crates/xenonet-engine/src/verify.rs`):
  the process on the other end is
  `XenoNet Tray.app/Contents/MacOS/<executable>` by its real path, in
  `/Applications` or the home folder's `Applications`, with no agent or
  actor product above it. The routes take the key as `x-xenonet-tray`,
  never with an `authorization` header, and check that the request is
  yours on every request. Nothing is written to disk.
- One tray at a time. The hello's answer and `status` say `tray: true`
  while one is connected, and the Desktop hears a `tray` event,
  `{ "connected": true | false }`, when it comes and goes.
- Fixtures both sides read: `test/fixtures/tray/status.json` and
  `event.json`. The JS, Rust and Swift tests check them.
